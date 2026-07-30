use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::{
    domain::{
        AgentKind, AgentSession, PermissionAction, PromptAttachmentInput, PromptDelivery,
        QuestionAnswer, SessionActivity, SessionSource, SessionStatus,
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
    "interactive_questions",
    "termination",
    "realtime_stream",
    "coordinated_updates",
    "work_status",
    "prompt_interruption",
    "prompt_delivery",
];
pub const STREAM_HEARTBEAT_INTERVAL_MS: u64 = 15_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptUnavailableReason {
    UnsupportedAgent,
    SessionNotConnected,
    WorkingDirectoryMissing,
    AgentBusy,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCapabilities {
    pub can_prompt: bool,
    pub prompt_unavailable_reason: Option<PromptUnavailableReason>,
    pub can_approve: bool,
    pub can_answer_question: bool,
    pub can_terminate: bool,
    pub can_open_source: bool,
    pub can_read_results: bool,
    pub can_attach_images: bool,
    pub can_interrupt: bool,
    pub prompt_deliveries: Vec<PromptDelivery>,
}

impl SessionCapabilities {
    pub fn for_session(session: &AgentSession) -> Self {
        let prompt_unavailable_reason = if session.source == SessionSource::Web
            && matches!(
                session.status,
                SessionStatus::Running | SessionStatus::PermissionRequired
            ) {
            Some(PromptUnavailableReason::AgentBusy)
        } else if session.source == SessionSource::Web {
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
            can_answer_question: session.pending_question.is_some(),
            can_terminate: session.source == SessionSource::Cli && session.process_id.is_some(),
            can_open_source: matches!(session.source, SessionSource::Web | SessionSource::Vscode),
            can_read_results: !session.results.is_empty() || session.last_response.is_some(),
            can_attach_images: session.source != SessionSource::Web
                && session.agent != AgentKind::Unknown,
            can_interrupt: matches!(
                session.status,
                SessionStatus::Running | SessionStatus::PermissionRequired
            ) && can_interrupt_session(session),
            prompt_deliveries: if session.agent == AgentKind::Codex
                && session.source != SessionSource::Web
            {
                vec![
                    PromptDelivery::NewTurn,
                    PromptDelivery::Steer,
                    PromptDelivery::Queue,
                ]
            } else {
                vec![PromptDelivery::NewTurn]
            },
        }
    }
}

fn can_interrupt_session(session: &AgentSession) -> bool {
    if session.source != SessionSource::Web
        && session.agent == AgentKind::Codex
        && session.native_session_id.is_some()
    {
        return true;
    }
    #[cfg(not(target_os = "windows"))]
    {
        session.source == SessionSource::Cli && session.process_id.is_some()
    }
    #[cfg(target_os = "windows")]
    {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    pub label: String,
    pub status: WorkItemStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoSummary {
    pub items: Vec<WorkItem>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Complete,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalSummary {
    pub objective: String,
    pub status: GoalStatus,
    pub started_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkSummary {
    pub todo: Option<TodoSummary>,
    pub goal: Option<GoalSummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubSession {
    #[serde(flatten)]
    pub session: AgentSession,
    pub capabilities: SessionCapabilities,
    pub work_summary: AgentWorkSummary,
}

impl From<AgentSession> for HubSession {
    fn from(session: AgentSession) -> Self {
        let capabilities = SessionCapabilities::for_session(&session);
        let work_summary = work_summary(&session.activities);
        Self {
            session,
            capabilities,
            work_summary,
        }
    }
}

fn work_summary(activities: &[SessionActivity]) -> AgentWorkSummary {
    AgentWorkSummary {
        todo: todo_summary(activities),
        goal: goal_summary(activities),
    }
}

fn todo_summary(activities: &[SessionActivity]) -> Option<TodoSummary> {
    let plan = activities
        .iter()
        .rev()
        .filter(|activity| activity.kind == "plan")
        .find_map(|activity| {
            let items = plan_items(activity.detail.as_deref()?);
            (!items.is_empty()).then_some(TodoSummary {
                items,
                updated_at: activity.created_at,
            })
        });
    let tool = activities
        .iter()
        .rev()
        .filter(|activity| activity.kind == "tool")
        .find_map(|activity| {
            let items = activity
                .detail
                .as_deref()
                .and_then(todo_items)
                .or_else(|| todo_items(&activity.title))?;
            (!items.is_empty()).then_some(TodoSummary {
                items,
                updated_at: activity.created_at,
            })
        });

    match (plan, tool) {
        (Some(plan), Some(tool)) => Some(if plan.updated_at >= tool.updated_at {
            plan
        } else {
            tool
        }),
        (plan, tool) => plan.or(tool),
    }
}

fn plan_items(detail: &str) -> Vec<WorkItem> {
    if let Ok(value) = serde_json::from_str::<Value>(detail) {
        let mut items = Vec::new();
        collect_plan_items(&value, &mut items);
        if !items.is_empty() {
            return items;
        }
    }

    detail
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (status, label) = if let Some(label) = line.strip_prefix('✓') {
                (WorkItemStatus::Completed, label)
            } else if let Some(label) = line.strip_prefix('●') {
                (WorkItemStatus::InProgress, label)
            } else if let Some(label) = line.strip_prefix('○') {
                (WorkItemStatus::Pending, label)
            } else {
                return None;
            };
            let label = label.trim();
            (!label.is_empty()).then(|| WorkItem {
                label: label.to_string(),
                status,
            })
        })
        .collect()
}

fn collect_plan_items(value: &Value, items: &mut Vec<WorkItem>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_plan_items(value, items);
            }
        }
        Value::Object(object) => {
            if let Some(label) = object.get("step").and_then(Value::as_str) {
                let status = object
                    .get("status")
                    .and_then(Value::as_str)
                    .map(work_item_status)
                    .unwrap_or(WorkItemStatus::Pending);
                items.push(WorkItem {
                    label: label.to_string(),
                    status,
                });
                return;
            }
            if let Some(plan) = object.get("plan") {
                collect_plan_items(plan, items);
            }
        }
        Value::String(value) => {
            if let Ok(nested) = serde_json::from_str::<Value>(value) {
                collect_plan_items(&nested, items);
            }
        }
        _ => {}
    }
}

fn todo_items(detail: &str) -> Option<Vec<WorkItem>> {
    let value = serde_json::from_str::<Value>(detail).ok()?;
    let todos = find_json_value(&value, &["todos", "tasks", "plan"])?;
    let entries = todos.as_array()?;
    let items = entries
        .iter()
        .filter_map(|entry| {
            let label = find_json_value(
                entry,
                &[
                    "content",
                    "subject",
                    "task",
                    "title",
                    "text",
                    "step",
                    "description",
                ],
            )
            .and_then(Value::as_str)?
            .trim();
            if label.is_empty() {
                return None;
            }
            let status = find_json_value(entry, &["status"])
                .and_then(Value::as_str)
                .map(work_item_status)
                .unwrap_or(WorkItemStatus::Pending);
            Some(WorkItem {
                label: label.to_string(),
                status,
            })
        })
        .collect::<Vec<_>>();
    (!items.is_empty()).then_some(items)
}

fn work_item_status(status: &str) -> WorkItemStatus {
    match status
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
        .as_str()
    {
        "completed" | "complete" | "done" => WorkItemStatus::Completed,
        "inprogress" | "in_progress" | "running" | "active" => WorkItemStatus::InProgress,
        _ => WorkItemStatus::Pending,
    }
}

fn goal_summary(activities: &[SessionActivity]) -> Option<GoalSummary> {
    let mut ordered = activities.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|activity| activity.created_at);
    let mut goal: Option<GoalSummary> = None;

    for activity in ordered {
        if activity.kind != "tool" {
            continue;
        }
        let title = activity.title.to_ascii_lowercase();
        let tool = if title.contains("create_goal") {
            "create"
        } else if title.contains("update_goal") {
            "update"
        } else if title.contains("get_goal") {
            "get"
        } else {
            continue;
        };
        let fields = activity
            .detail
            .as_deref()
            .and_then(goal_fields)
            .unwrap_or_default();
        let inferred_started_at = fields.started_at.or_else(|| {
            fields
                .elapsed_ms
                .map(|elapsed| activity.created_at - elapsed)
        });

        match tool {
            "create" => {
                let Some(objective) = fields.objective else {
                    continue;
                };
                goal = Some(GoalSummary {
                    objective,
                    status: fields.status.unwrap_or(GoalStatus::Active),
                    started_at: inferred_started_at.unwrap_or(activity.created_at),
                    updated_at: activity.created_at,
                });
            }
            "get" => {
                if goal.is_none() {
                    let Some(objective) = fields.objective.clone() else {
                        continue;
                    };
                    goal = Some(GoalSummary {
                        objective,
                        status: fields.status.clone().unwrap_or(GoalStatus::Active),
                        started_at: inferred_started_at.unwrap_or(activity.created_at),
                        updated_at: activity.created_at,
                    });
                }
                if let Some(goal) = goal.as_mut() {
                    if let Some(objective) = fields.objective {
                        goal.objective = objective;
                    }
                    if let Some(status) = fields.status {
                        goal.status = status;
                    }
                    if let Some(started_at) = fields.started_at {
                        goal.started_at = started_at;
                    }
                    goal.updated_at = activity.created_at;
                }
            }
            "update" => {
                if let Some(goal) = goal.as_mut() {
                    if let Some(status) = fields.status {
                        goal.status = status;
                    }
                    goal.updated_at = activity.created_at;
                }
            }
            _ => {}
        }
    }
    goal
}

#[derive(Default)]
struct GoalFields {
    objective: Option<String>,
    status: Option<GoalStatus>,
    started_at: Option<i64>,
    elapsed_ms: Option<i64>,
}

fn goal_fields(detail: &str) -> Option<GoalFields> {
    let value = serde_json::from_str::<Value>(detail).ok()?;
    Some(GoalFields {
        objective: find_json_value(&value, &["objective"])
            .and_then(Value::as_str)
            .or_else(|| find_json_value(&value, &["goal"]).and_then(Value::as_str))
            .map(str::to_string),
        status: find_json_value(&value, &["status"])
            .and_then(Value::as_str)
            .and_then(goal_status),
        started_at: find_json_value(
            &value,
            &["startedAt", "started_at", "createdAt", "created_at"],
        )
        .and_then(Value::as_i64)
        .map(timestamp_millis),
        elapsed_ms: find_json_value(
            &value,
            &[
                "elapsedMs",
                "elapsed_ms",
                "elapsedTimeMs",
                "elapsed_time_ms",
            ],
        )
        .and_then(Value::as_i64)
        .or_else(|| {
            find_json_value(
                &value,
                &[
                    "elapsedSeconds",
                    "elapsed_seconds",
                    "elapsedTimeSeconds",
                    "elapsed_time_seconds",
                ],
            )
            .and_then(Value::as_i64)
            .map(|seconds| seconds.saturating_mul(1_000))
        }),
    })
}

fn timestamp_millis(timestamp: i64) -> i64 {
    if timestamp.abs() < 100_000_000_000 {
        timestamp.saturating_mul(1_000)
    } else {
        timestamp
    }
}

fn find_json_value<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    match value {
        Value::Object(object) => keys.iter().find_map(|key| object.get(*key)).or_else(|| {
            object
                .values()
                .find_map(|value| find_json_value(value, keys))
        }),
        Value::Array(values) => values.iter().find_map(|value| find_json_value(value, keys)),
        Value::String(_) => None,
        _ => None,
    }
}

fn goal_status(status: &str) -> Option<GoalStatus> {
    match status.trim().to_ascii_lowercase().as_str() {
        "active" | "running" | "in_progress" | "inprogress" => Some(GoalStatus::Active),
        "complete" | "completed" | "done" => Some(GoalStatus::Complete),
        "blocked" | "failed" => Some(GoalStatus::Blocked),
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubSnapshot {
    pub protocol_version: u16,
    pub desktop_version: String,
    pub generated_at: i64,
    pub features: Vec<String>,
    pub sessions: Vec<HubSession>,
}

impl HubSnapshot {
    pub fn new(sessions: Vec<AgentSession>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            desktop_version: env!("CARGO_PKG_VERSION").to_string(),
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
        #[serde(default)]
        delivery: PromptDelivery,
    },
    ResolvePermission {
        session_id: String,
        permission_id: String,
        action: PermissionAction,
    },
    ResolveQuestion {
        session_id: String,
        question_id: String,
        answers: Vec<QuestionAnswer>,
    },
    TerminateSession {
        session_id: String,
    },
    InterruptPrompt {
        session_id: String,
    },
    OpenSessionSource {
        session_id: String,
    },
    RefreshRateLimits {
        agent: AgentKind,
    },
    ReportMobileVersion {
        version: String,
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
                ..
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
            HubCommand::ResolveQuestion {
                session_id,
                question_id,
                answers,
            } => {
                validate_identifier("session_id", session_id, 512)?;
                validate_identifier("question_id", question_id, 512)?;
                if answers.is_empty() {
                    return Err(ProtocolError::new(
                        "answers_empty",
                        "A resposta da pergunta está vazia",
                    ));
                }
                if answers.len() > 4 {
                    return Err(ProtocolError::new(
                        "too_many_answers",
                        "A solicitação aceita no máximo 4 perguntas",
                    ));
                }
                for answer in answers {
                    validate_identifier("answer.question_id", &answer.question_id, 256)?;
                    if answer.answers.is_empty()
                        || answer.answers.iter().all(|value| value.trim().is_empty())
                    {
                        return Err(ProtocolError::new(
                            "answer_empty",
                            "Uma das respostas está vazia",
                        ));
                    }
                    if answer.answers.len() > 8
                        || answer.answers.iter().any(|value| value.len() > 16 * 1024)
                    {
                        return Err(ProtocolError::new(
                            "answer_too_large",
                            "Uma das respostas excede o limite permitido",
                        ));
                    }
                }
            }
            HubCommand::TerminateSession { session_id }
            | HubCommand::InterruptPrompt { session_id }
            | HubCommand::OpenSessionSource { session_id } => {
                validate_identifier("session_id", session_id, 512)?;
            }
            HubCommand::RefreshRateLimits { .. } => {}
            HubCommand::ReportMobileVersion { version } => {
                validate_identifier("version", version, 32)?;
                if !version
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || ".-+".contains(character))
                {
                    return Err(ProtocolError::new(
                        "version_invalid",
                        "A versão informada é inválida",
                    ));
                }
            }
        }
        Ok(())
    }
}

pub fn is_version_newer(candidate: &str, installed: &str) -> bool {
    let version_parts = |value: &str| {
        let stable = value.split('-').next().unwrap_or(value);
        let mut parsed = [0_u64; 4];
        for (index, part) in stable.split('.').take(parsed.len()).enumerate() {
            parsed[index] = part
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0);
        }
        parsed
    };
    version_parts(candidate) > version_parts(installed)
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
            session_name: "Codex · Lume".into(),
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
            pending_question: None,
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
        assert_eq!(snapshot.desktop_version, env!("CARGO_PKG_VERSION"));
        assert!(snapshot.features.contains(&"prompts".to_string()));
        assert!(snapshot
            .features
            .contains(&"coordinated_updates".to_string()));
        assert!(snapshot.features.contains(&"work_status".to_string()));
        assert!(snapshot.sessions[0].capabilities.can_prompt);
        assert!(snapshot.sessions[0].capabilities.can_terminate);
        let json = serde_json::to_value(snapshot).expect("snapshot");
        assert_eq!(json["sessions"][0]["nativeSessionId"], "thread-1");
        assert_eq!(json["sessions"][0]["capabilities"]["canPrompt"], true);
    }

    #[test]
    fn web_chatgpt_does_not_inherit_codex_runtime_capabilities() {
        let mut web = session();
        web.id = "web:chatgpt:tab-1".into();
        web.agent = AgentKind::ChatGpt;
        web.agent_label = "ChatGPT".into();
        web.source = SessionSource::Web;
        web.status = SessionStatus::Running;
        web.process_id = None;
        web.native_session_id = Some("tab-1".into());
        web.working_directory = None;

        let capabilities = SessionCapabilities::for_session(&web);
        assert!(!capabilities.can_prompt);
        assert_eq!(
            capabilities.prompt_unavailable_reason,
            Some(PromptUnavailableReason::AgentBusy)
        );
        assert!(!capabilities.can_interrupt);
        assert_eq!(
            capabilities.prompt_deliveries,
            vec![PromptDelivery::NewTurn]
        );
    }

    #[test]
    fn web_claude_is_not_treated_as_claude_code() {
        let mut web = session();
        web.id = "web:claude:tab-2".into();
        web.agent = AgentKind::Claude;
        web.agent_label = "Claude".into();
        web.source = SessionSource::Web;
        web.process_id = None;
        web.native_session_id = Some("tab-2".into());
        web.working_directory = None;

        let capabilities = SessionCapabilities::for_session(&web);
        assert!(capabilities.can_prompt);
        assert!(!capabilities.can_interrupt);
        assert_eq!(
            capabilities.prompt_deliveries,
            vec![PromptDelivery::NewTurn]
        );
    }

    #[test]
    fn cli_codex_keeps_queue_and_steer_capabilities() {
        let capabilities = SessionCapabilities::for_session(&session());
        assert_eq!(
            capabilities.prompt_deliveries,
            vec![
                PromptDelivery::NewTurn,
                PromptDelivery::Steer,
                PromptDelivery::Queue,
            ]
        );
    }

    #[test]
    fn snapshot_summarizes_latest_todo_plan() {
        let mut session = session();
        session.activities.push(SessionActivity {
            id: "plan-1".into(),
            kind: "plan".into(),
            title: "Plano atualizado".into(),
            detail: Some(
                "Preparando a entrega\n✓ Mapear eventos\n● Criar a bandeja\n○ Validar builds"
                    .into(),
            ),
            status: "running".into(),
            created_at: 12,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });

        let snapshot = HubSnapshot::new(vec![session]);
        let todo = snapshot.sessions[0]
            .work_summary
            .todo
            .as_ref()
            .expect("todo");
        assert_eq!(todo.items.len(), 3);
        assert_eq!(todo.items[0].status, WorkItemStatus::Completed);
        assert_eq!(todo.items[1].status, WorkItemStatus::InProgress);
        assert_eq!(todo.updated_at, 12);
    }

    #[test]
    fn snapshot_summarizes_todo_tool_items() {
        let mut session = session();
        session.activities.push(SessionActivity {
            id: "todo-1".into(),
            kind: "tool".into(),
            title: "TodoWrite".into(),
            detail: Some(
                r#"{"todos":[{"content":"Inspect hooks","status":"completed"},{"content":"Build tray","status":"in_progress"}]}"#
                    .into(),
            ),
            status: "completed".into(),
            created_at: 20,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });

        let snapshot = HubSnapshot::new(vec![session]);
        let todo = snapshot.sessions[0]
            .work_summary
            .todo
            .as_ref()
            .expect("todo");
        assert_eq!(todo.items.len(), 2);
        assert_eq!(todo.items[1].label, "Build tray");
        assert_eq!(todo.items[1].status, WorkItemStatus::InProgress);
    }

    #[test]
    fn snapshot_tracks_goal_lifecycle_and_start_time() {
        let mut session = session();
        session.activities.extend([
            SessionActivity {
                id: "goal-create".into(),
                kind: "tool".into(),
                title: "functions · create_goal".into(),
                detail: Some(r#"{"objective":"Ship Lume mobile"}"#.into()),
                status: "completed".into(),
                created_at: 100,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            },
            SessionActivity {
                id: "goal-update".into(),
                kind: "tool".into(),
                title: "functions · update_goal".into(),
                detail: Some(r#"{"status":"complete"}"#.into()),
                status: "completed".into(),
                created_at: 250,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            },
        ]);

        let snapshot = HubSnapshot::new(vec![session]);
        let goal = snapshot.sessions[0]
            .work_summary
            .goal
            .as_ref()
            .expect("goal");
        assert_eq!(goal.objective, "Ship Lume mobile");
        assert_eq!(goal.status, GoalStatus::Complete);
        assert_eq!(goal.started_at, 100);
        assert_eq!(goal.updated_at, 250);
    }

    #[test]
    fn snapshot_recovers_elapsed_goal_from_get_goal() {
        let mut session = session();
        session.activities.push(SessionActivity {
            id: "goal-read".into(),
            kind: "tool".into(),
            title: "functions · get_goal".into(),
            detail: Some(
                r#"{"objective":"Review agents","status":"active","elapsed_seconds":90}"#.into(),
            ),
            status: "completed".into(),
            created_at: 100_000,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });

        let snapshot = HubSnapshot::new(vec![session]);
        let goal = snapshot.sessions[0]
            .work_summary
            .goal
            .as_ref()
            .expect("goal");
        assert_eq!(goal.started_at, 10_000);
    }

    #[test]
    fn snapshot_normalizes_goal_start_time_from_seconds() {
        let mut session = session();
        session.activities.push(SessionActivity {
            id: "goal-read-seconds".into(),
            kind: "tool".into(),
            title: "functions · get_goal".into(),
            detail: Some(
                r#"{"goal":{"objective":"Test goal","status":"active","createdAt":1785190621}}"#
                    .into(),
            ),
            status: "completed".into(),
            created_at: 1_785_190_942_000,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });

        let snapshot = HubSnapshot::new(vec![session]);
        let goal = snapshot.sessions[0]
            .work_summary
            .goal
            .as_ref()
            .expect("goal");
        assert_eq!(goal.started_at, 1_785_190_621_000);
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
                delivery: PromptDelivery::NewTurn,
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
                delivery: PromptDelivery::NewTurn,
            },
        };
        assert_eq!(
            request.validate().expect_err("inválido").code,
            "prompt_empty"
        );
    }

    #[test]
    fn companion_versions_are_compared_numerically() {
        assert!(is_version_newer("0.10.0", "0.9.9"));
        assert!(is_version_newer("0.9.1", "0.9.0"));
        assert!(!is_version_newer("0.9.0", "0.9.0"));
        assert!(!is_version_newer("0.8.9", "0.9.0"));

        let invalid = HubCommandRequest {
            request_id: "mobile-version".into(),
            command: HubCommand::ReportMobileVersion {
                version: "../../update".into(),
            },
        };
        assert_eq!(
            invalid.validate().expect_err("inválido").code,
            "version_invalid"
        );
    }
}
