use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::{
    domain::{
        AgentKind, AgentSession, PermissionAction, PromptAttachmentInput, SessionSource,
    },
    state::now_millis,
};

pub const PROTOCOL_VERSION: u16 = 1;
pub const PROTOCOL_FEATURES: &[&str] = &[
    "sessions",
    "activity",
    "results",
    "files",
    "prompts",
    "image_prompts",
    "rate_limits",
    "permissions",
    "termination",
    "realtime_stream",
];
pub const STREAM_HEARTBEAT_INTERVAL_MS: u64 = 15_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptUnavailableReason {
    UnsupportedAgent,
    SessionNotConnected,
    WorkingDirectoryMissing,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCapabilities {
    pub can_prompt: bool,
    pub prompt_unavailable_reason: Option<PromptUnavailableReason>,
    pub can_approve: bool,
    pub can_terminate: bool,
    pub can_open_source: bool,
    pub can_read_results: bool,
    pub can_attach_images: bool,
}

impl SessionCapabilities {
    pub fn for_session(session: &AgentSession) -> Self {
        let prompt_unavailable_reason = if session.source == SessionSource::Web {
            None
        } else if session.agent == AgentKind::Unknown {
            Some(PromptUnavailableReason::UnsupportedAgent)
        } else if session.native_session_id.is_none() {
            Some(PromptUnavailableReason::SessionNotConnected)
        } else if session.agent != AgentKind::Codex && session.working_directory.is_none() {
            Some(PromptUnavailableReason::WorkingDirectoryMissing)
        } else {
            None
        };
        Self {
            can_prompt: prompt_unavailable_reason.is_none(),
            prompt_unavailable_reason,
            can_approve: session.pending_permission.is_some()
                && session.permission_profile.can_respond_from_lume,
            can_terminate: session.source == SessionSource::Cli && session.process_id.is_some(),
            can_open_source: matches!(session.source, SessionSource::Web | SessionSource::Vscode),
            can_read_results: !session.results.is_empty() || session.last_response.is_some(),
            can_attach_images: session.source != SessionSource::Web
                && session.agent != AgentKind::Unknown,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubSession {
    #[serde(flatten)]
    pub session: AgentSession,
    pub capabilities: SessionCapabilities,
}

impl From<AgentSession> for HubSession {
    fn from(session: AgentSession) -> Self {
        let capabilities = SessionCapabilities::for_session(&session);
        Self {
            session,
            capabilities,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubSnapshot {
    pub protocol_version: u16,
    pub generated_at: i64,
    pub features: Vec<String>,
    pub sessions: Vec<HubSession>,
}

impl HubSnapshot {
    pub fn new(sessions: Vec<AgentSession>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            generated_at: now_millis(),
            features: PROTOCOL_FEATURES
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
            sessions: sessions.into_iter().map(HubSession::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum HubCommand {
    SubmitPrompt {
        session_id: String,
        prompt: String,
        #[serde(default)]
        attachments: Vec<PromptAttachmentInput>,
    },
    ResolvePermission {
        session_id: String,
        permission_id: String,
        action: PermissionAction,
    },
    TerminateSession {
        session_id: String,
    },
    OpenSessionSource {
        session_id: String,
    },
    RefreshRateLimits {
        agent: AgentKind,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubCommandRequest {
    pub request_id: String,
    #[serde(flatten)]
    pub command: HubCommand,
}

impl HubCommandRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identifier("request_id", &self.request_id, 128)?;
        match &self.command {
            HubCommand::SubmitPrompt {
                session_id,
                prompt,
                attachments,
            } => {
                validate_identifier("session_id", session_id, 512)?;
                if prompt.trim().is_empty() && attachments.is_empty() {
                    return Err(ProtocolError::new(
                        "prompt_empty",
                        "O prompt e os anexos estão vazios",
                    ));
                }
                if prompt.len() > 16 * 1024 {
                    return Err(ProtocolError::new(
                        "prompt_too_large",
                        "O prompt excede 16 KB",
                    ));
                }
                if attachments.len() > 4 {
                    return Err(ProtocolError::new(
                        "too_many_attachments",
                        "O prompt aceita no máximo 4 imagens",
                    ));
                }
            }
            HubCommand::ResolvePermission {
                session_id,
                permission_id,
                ..
            } => {
                validate_identifier("session_id", session_id, 512)?;
                validate_identifier("permission_id", permission_id, 512)?;
            }
            HubCommand::TerminateSession { session_id }
            | HubCommand::OpenSessionSource { session_id } => {
                validate_identifier("session_id", session_id, 512)?;
            }
            HubCommand::RefreshRateLimits { .. } => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
}

impl ProtocolError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn from_control(message: String) -> Self {
        let code = if message.contains("Sessão não encontrada") {
            "session_not_found"
        } else if message.contains("Aguarde o agente") {
            "session_busy"
        } else if message.contains("não informou") || message.contains("não possui") {
            "session_not_ready"
        } else if message.contains("não oferece") || message.contains("não permite") {
            "unsupported_action"
        } else if message.contains("processo isolado") {
            "unsafe_termination"
        } else {
            "command_failed"
        };
        Self::new(code, message)
    }
}

fn validate_identifier(field: &str, value: &str, max_len: usize) -> Result<(), ProtocolError> {
    if value.trim().is_empty() {
        return Err(ProtocolError::new(
            format!("{field}_empty"),
            format!("{field} está vazio"),
        ));
    }
    if value.len() > max_len {
        return Err(ProtocolError::new(
            format!("{field}_too_large"),
            format!("{field} excede o limite"),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubCommandResponse {
    pub protocol_version: u16,
    pub request_id: String,
    pub ok: bool,
    pub error: Option<ProtocolError>,
}

impl HubCommandResponse {
    pub fn success(request_id: String) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            ok: true,
            error: None,
        }
    }

    pub fn failure(request_id: String, error: ProtocolError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            ok: false,
            error: Some(error),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum HubEvent {
    SessionsChanged,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubEventEnvelope {
    pub protocol_version: u16,
    pub event_id: String,
    pub sequence: u64,
    pub occurred_at: i64,
    #[serde(flatten)]
    pub event: HubEvent,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum HubStreamMessage {
    Hello {
        protocol_version: u16,
        features: Vec<String>,
        heartbeat_interval_ms: u64,
    },
    Snapshot {
        sequence: u64,
        snapshot: HubSnapshot,
    },
    Update {
        events: Vec<HubEventEnvelope>,
        snapshot: HubSnapshot,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
}

impl HubStreamMessage {
    pub fn hello() -> Self {
        Self::Hello {
            protocol_version: PROTOCOL_VERSION,
            features: PROTOCOL_FEATURES
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
            heartbeat_interval_ms: STREAM_HEARTBEAT_INTERVAL_MS,
        }
    }
}

static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn event_journal() -> &'static Mutex<VecDeque<HubEventEnvelope>> {
    static JOURNAL: OnceLock<Mutex<VecDeque<HubEventEnvelope>>> = OnceLock::new();
    JOURNAL.get_or_init(|| Mutex::new(VecDeque::with_capacity(256)))
}

pub fn emit_sessions_changed(app: &AppHandle) {
    let sequence = EVENT_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    let occurred_at = now_millis();
    let event = HubEventEnvelope {
        protocol_version: PROTOCOL_VERSION,
        event_id: format!("{occurred_at}-{sequence}"),
        sequence,
        occurred_at,
        event: HubEvent::SessionsChanged,
    };
    if let Ok(mut journal) = event_journal().lock() {
        if journal.len() == 256 {
            journal.pop_front();
        }
        journal.push_back(event.clone());
    }
    let _ = app.emit("lume://sessions-changed", ());
    let _ = app.emit("lume://hub-event", event);
}

pub fn events_since(sequence: u64) -> Vec<HubEventEnvelope> {
    event_journal()
        .lock()
        .map(|journal| {
            journal
                .iter()
                .filter(|event| event.sequence > sequence)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub fn latest_event_sequence() -> u64 {
    EVENT_SEQUENCE.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AccessMode, PermissionProfile, SessionStatus};

    fn session() -> AgentSession {
        AgentSession {
            id: "codex:thread-1".into(),
            agent: AgentKind::Codex,
            agent_label: "Codex".into(),
            project: "Lume".into(),
            source: SessionSource::Cli,
            source_app: None,
            status: SessionStatus::WaitingForInput,
            status_label: "Esperando ação".into(),
            started_at: "1".into(),
            updated_at: 1,
            process_id: Some(42),
            native_session_id: Some("thread-1".into()),
            working_directory: Some("/work/lume".into()),
            permission_profile: PermissionProfile {
                mode: AccessMode::WorkspaceWrite,
                label: "Workspace".into(),
                approval_policy: "on-request".into(),
                approvals_reviewer: None,
                can_respond_from_lume: true,
                available_actions: Vec::new(),
            },
            pending_permission: None,
            last_response: None,
            results: Vec::new(),
            activities: Vec::new(),
            rate_limits: Vec::new(),
        }
    }

    #[test]
    fn snapshot_is_versioned_and_contains_capabilities() {
        let snapshot = HubSnapshot::new(vec![session()]);
        assert_eq!(snapshot.protocol_version, PROTOCOL_VERSION);
        assert!(snapshot.features.contains(&"prompts".to_string()));
        assert!(snapshot.sessions[0].capabilities.can_prompt);
        assert!(snapshot.sessions[0].capabilities.can_terminate);
        let json = serde_json::to_value(snapshot).expect("snapshot");
        assert_eq!(json["sessions"][0]["nativeSessionId"], "thread-1");
        assert_eq!(json["sessions"][0]["capabilities"]["canPrompt"], true);
    }

    #[test]
    fn realtime_stream_contract_is_versioned_and_additive() {
        let hello = serde_json::to_value(HubStreamMessage::hello()).expect("hello");
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(hello["heartbeatIntervalMs"], STREAM_HEARTBEAT_INTERVAL_MS);
        assert!(hello["features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|value| value == "realtime_stream")));

        let snapshot = serde_json::to_value(HubStreamMessage::Snapshot {
            sequence: 9,
            snapshot: HubSnapshot::new(vec![session()]),
        })
        .expect("snapshot");
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["sequence"], 9);
        assert_eq!(snapshot["snapshot"]["sessions"][0]["id"], "codex:thread-1");
    }

    #[test]
    fn process_only_session_explains_why_prompt_is_unavailable() {
        let mut session = session();
        session.native_session_id = None;
        let capabilities = SessionCapabilities::for_session(&session);
        assert!(!capabilities.can_prompt);
        assert_eq!(
            capabilities.prompt_unavailable_reason,
            Some(PromptUnavailableReason::SessionNotConnected)
        );
    }

    #[test]
    fn command_json_is_stable_and_validated() {
        let json = r#"{
            "requestId":"mobile-1",
            "type":"submit_prompt",
            "sessionId":"codex:thread-1",
            "prompt":"Continue"
        }"#;
        let request: HubCommandRequest = serde_json::from_str(json).expect("comando");
        request.validate().expect("válido");
        assert_eq!(
            request.command,
            HubCommand::SubmitPrompt {
                session_id: "codex:thread-1".into(),
                prompt: "Continue".into(),
                attachments: Vec::new(),
            }
        );
        let serialized = serde_json::to_value(request).expect("json");
        assert_eq!(serialized["type"], "submit_prompt");
        assert_eq!(serialized["sessionId"], "codex:thread-1");
    }

    #[test]
    fn empty_prompt_is_rejected_at_the_protocol_boundary() {
        let request = HubCommandRequest {
            request_id: "mobile-1".into(),
            command: HubCommand::SubmitPrompt {
                session_id: "codex:thread-1".into(),
                prompt: "  ".into(),
                attachments: Vec::new(),
            },
        };
        assert_eq!(
            request.validate().expect_err("inválido").code,
            "prompt_empty"
        );
    }
}
