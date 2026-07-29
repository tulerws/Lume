use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

use notify::{RecursiveMode, Watcher};
use serde::Deserialize;
use serde_json::Value;
use tauri::AppHandle;

use crate::{
    domain::{
        AccessMode, AgentKind, HookEvent, HookEventKind, PermissionAction, PermissionProfile,
        SessionActivity, SessionSource,
    },
    event_server,
    state::{now_millis, AppState},
};

const RECOVERY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
struct SessionMetadata {
    id: String,
    cwd: Option<String>,
    started_at: Option<String>,
    source: SessionSource,
}

#[derive(Debug)]
struct ObservedFile {
    offset: u64,
    session: Option<SessionMetadata>,
    profile: Option<PermissionProfile>,
    pending_goal_tools: HashMap<String, PendingGoalTool>,
}

#[derive(Debug)]
struct PendingGoalTool {
    name: String,
    activity_id: String,
}

#[derive(Debug, Deserialize)]
struct CodexRecord {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    payload: RecordPayload,
}

#[derive(Debug, Default, Deserialize)]
struct RecordPayload {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    originator: Option<String>,
    #[serde(default)]
    source: Option<Value>,
    #[serde(default)]
    parent_thread_id: Option<String>,
    #[serde(default)]
    thread_source: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    approval_policy: Option<String>,
    #[serde(default)]
    approvals_reviewer: Option<String>,
    #[serde(default)]
    sandbox_policy: Option<Value>,
    #[serde(default)]
    last_agent_message: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    output: Option<Value>,
}

pub fn start(state: AppState, app: AppHandle) -> Result<(), String> {
    thread::Builder::new()
        .name("lume-codex-session-monitor".into())
        .spawn(move || monitor(state, app))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn monitor(state: AppState, app: AppHandle) {
    let Some(root) = sessions_root() else {
        return;
    };
    let mut observed = initialize(&root);
    loop {
        if watch_session_files(&root, &state, &app, &mut observed).is_ok() {
            return;
        }
        poll(&root, &state, &app, &mut observed);
        thread::sleep(RECOVERY_INTERVAL);
    }
}

fn watch_session_files(
    root: &Path,
    state: &AppState,
    app: &AppHandle,
    observed: &mut HashMap<PathBuf, ObservedFile>,
) -> Result<(), String> {
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .map_err(|error| error.to_string())?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|error| error.to_string())?;

    loop {
        match receiver.recv_timeout(RECOVERY_INTERVAL) {
            Ok(Ok(event)) => {
                for path in event.paths {
                    if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
                        poll_path(&path, state, app, observed);
                    }
                }
            }
            Ok(Err(error)) => return Err(error.to_string()),
            Err(RecvTimeoutError::Timeout) => {
                poll(root, state, app, observed);
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err("Monitor de sessões do Codex desconectado".into());
            }
        }
    }
}

fn initialize(root: &Path) -> HashMap<PathBuf, ObservedFile> {
    let mut observed = HashMap::new();
    for path in session_files(root) {
        let Ok(file_metadata) = fs::metadata(&path) else {
            continue;
        };
        let file = ObservedFile {
            offset: file_metadata.len(),
            session: read_session_metadata(&path),
            profile: None,
            pending_goal_tools: HashMap::new(),
        };
        observed.insert(path, file);
    }
    observed
}

fn poll(
    root: &Path,
    state: &AppState,
    app: &AppHandle,
    observed: &mut HashMap<PathBuf, ObservedFile>,
) {
    for path in session_files(root) {
        poll_path(&path, state, app, observed);
    }
}

fn poll_path(
    path: &Path,
    state: &AppState,
    app: &AppHandle,
    observed: &mut HashMap<PathBuf, ObservedFile>,
) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let length = metadata.len();
    if !observed.contains_key(path) {
        let mut file = ObservedFile {
            offset: 0,
            session: read_session_metadata(path),
            profile: None,
            pending_goal_tools: HashMap::new(),
        };
        if let Some(event) = session_started_event(&file) {
            let _ = event_server::publish_event(state, app, event);
        }
        publish_appended_events(path, state, app, &mut file);
        if file.offset == 0 {
            file.offset = length;
        }
        observed.insert(path.to_path_buf(), file);
        return;
    }

    let file = observed.get_mut(path).expect("verificado acima");
    if length < file.offset {
        file.offset = 0;
        file.profile = None;
        file.pending_goal_tools.clear();
        file.session = read_session_metadata(path);
        if let Some(event) = session_started_event(file) {
            let _ = event_server::publish_event(state, app, event);
        }
    }
    if file.session.is_none() && length > file.offset {
        file.session = read_session_metadata(path);
        if file.session.is_some() {
            file.offset = 0;
            if let Some(event) = session_started_event(file) {
                let _ = event_server::publish_event(state, app, event);
            }
        }
    }
    if length > file.offset {
        publish_appended_events(path, state, app, file);
    }
}

fn publish_appended_events(
    path: &Path,
    state: &AppState,
    app: &AppHandle,
    file: &mut ObservedFile,
) {
    if file.session.is_none() {
        file.offset = fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(file.offset);
        return;
    }
    let Ok((records, offset)) = read_records(path, file.offset) else {
        return;
    };
    for event in events_from_records(records, file) {
        let _ = event_server::publish_event(state, app, event);
    }
    file.offset = offset;
}

fn events_from_records(records: Vec<CodexRecord>, file: &mut ObservedFile) -> Vec<HookEvent> {
    let mut events = Vec::new();
    for record in records {
        if record.kind == "response_item" {
            match record.payload.r#type.as_deref() {
                Some("function_call") => remember_goal_tool(&record.payload, file),
                Some("function_call_output") => {
                    if let Some(event) = goal_tool_output_event(&record.payload, file) {
                        events.push(event);
                    }
                }
                _ => {}
            }
            continue;
        }
        if record.kind == "turn_context" {
            if let Some(session) = file.session.as_mut() {
                if record.payload.cwd.is_some() {
                    session.cwd = record.payload.cwd.clone();
                }
                file.profile = Some(profile_from_context(&record.payload));
            }
            continue;
        }
        if record.kind != "event_msg" {
            continue;
        }
        if matches!(
            record.payload.r#type.as_deref(),
            Some("user_message" | "agent_message")
        ) {
            if let Some(message) = record
                .payload
                .message
                .as_deref()
                .map(str::trim)
                .filter(|message| !message.is_empty())
            {
                let (kind, title) = if record.payload.r#type.as_deref() == Some("user_message") {
                    ("prompt", "Prompt enviado")
                } else {
                    ("message", "Resposta do agente")
                };
                if let Some(event) =
                    activity_event_for(file, kind, title, message, record.timestamp.as_deref())
                {
                    events.push(event);
                }
            }
            continue;
        }
        let (kind, label, last_response) = match record.payload.r#type.as_deref() {
            Some("task_started") => (HookEventKind::Running, "Rodando", None),
            Some("task_complete") => (
                HookEventKind::Completed,
                "Tarefa finalizada",
                record.payload.last_agent_message.as_deref(),
            ),
            Some("turn_aborted") => (HookEventKind::Completed, "Tarefa interrompida", None),
            Some("stream_error" | "task_failed") => {
                (HookEventKind::Failed, "Tarefa encerrada com erro", None)
            }
            _ => continue,
        };
        if let Some(event) = event_for(file, kind, label, last_response) {
            events.push(event);
        }
    }
    events
}

fn remember_goal_tool(payload: &RecordPayload, file: &mut ObservedFile) {
    let Some(name) = payload.name.as_deref().filter(|name| is_goal_tool(name)) else {
        return;
    };
    let Some(call_id) = payload.call_id.as_ref() else {
        return;
    };
    let activity_id = payload
        .id
        .as_ref()
        .and_then(|id| {
            file.session
                .as_ref()
                .map(|session| format!("codex:{}:{id}", session.id))
        })
        .unwrap_or_else(|| format!("codex-rollout-goal:{call_id}"));
    file.pending_goal_tools.insert(
        call_id.clone(),
        PendingGoalTool {
            name: name.into(),
            activity_id,
        },
    );
}

fn goal_tool_output_event(payload: &RecordPayload, file: &mut ObservedFile) -> Option<HookEvent> {
    let tool = file
        .pending_goal_tools
        .remove(payload.call_id.as_deref()?)?;
    let detail = payload.output.as_ref().and_then(record_value_text)?;
    let mut event = event_for(file, HookEventKind::Activity, "GOAL atualizada", None)?;
    event.activity = Some(SessionActivity {
        id: tool.activity_id,
        kind: "tool".into(),
        title: format!("functions · {}", tool.name),
        detail: Some(detail),
        status: "completed".into(),
        created_at: now_millis(),
        files: Vec::new(),
        attachments: Vec::new(),
        append_detail: false,
    });
    Some(event)
}

fn is_goal_tool(name: &str) -> bool {
    matches!(name, "create_goal" | "get_goal" | "update_goal")
}

fn record_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => response_text(value),
        Value::Null => None,
        value => response_text(&value.to_string()),
    }
}

fn activity_event_for(
    file: &ObservedFile,
    kind: &str,
    title: &str,
    detail: &str,
    timestamp: Option<&str>,
) -> Option<HookEvent> {
    let session = file.session.as_ref()?;
    let mut hasher = DefaultHasher::new();
    session.id.hash(&mut hasher);
    kind.hash(&mut hasher);
    detail.hash(&mut hasher);
    timestamp.hash(&mut hasher);
    let mut event = event_for(file, HookEventKind::Activity, title, None)?;
    event.activity = Some(SessionActivity {
        id: format!("codex-rollout:{:x}", hasher.finish()),
        kind: kind.into(),
        title: title.into(),
        detail: response_text(detail),
        status: "completed".into(),
        created_at: now_millis(),
        files: Vec::new(),
        attachments: Vec::new(),
        append_detail: false,
    });
    Some(event)
}

fn session_started_event(file: &ObservedFile) -> Option<HookEvent> {
    event_for(file, HookEventKind::SessionStarted, "Esperando ação", None)
}

fn event_for(
    file: &ObservedFile,
    event: HookEventKind,
    label: &str,
    last_response: Option<&str>,
) -> Option<HookEvent> {
    let session = file.session.as_ref()?;
    let project = session
        .cwd
        .as_deref()
        .and_then(|cwd| Path::new(cwd).file_name())
        .and_then(|name| name.to_str())
        .map(str::to_string);
    Some(HookEvent {
        event,
        session_id: format!("codex-app-server:{}", session.id),
        agent: AgentKind::Codex,
        agent_label: Some("Codex".into()),
        session_name: None,
        project,
        source: Some(session.source.clone()),
        source_app: None,
        status_label: Some(label.into()),
        started_at: session.started_at.clone(),
        process_id: None,
        native_session_id: Some(session.id.clone()),
        working_directory: session.cwd.clone(),
        permission_profile: Some(file.profile.clone().unwrap_or_else(default_profile)),
        permission: None,
        question: None,
        last_response: last_response.and_then(response_text),
        activity: None,
        activities: Vec::new(),
        wait_for_decision: false,
    })
}

fn response_text(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    const LIMIT: usize = 32 * 1024;
    let mut response = value.chars().take(LIMIT).collect::<String>();
    if value.chars().count() > LIMIT {
        response.push('…');
    }
    Some(response)
}

fn read_session_metadata(path: &Path) -> Option<SessionMetadata> {
    let file = File::open(path).ok()?;
    let record = serde_json::Deserializer::from_reader(BufReader::new(file))
        .into_iter::<CodexRecord>()
        .next()?
        .ok()?;
    session_metadata(&record)
}

fn session_metadata(record: &CodexRecord) -> Option<SessionMetadata> {
    if record.kind != "session_meta"
        || record.payload.parent_thread_id.is_some()
        || record.payload.thread_source.as_deref() == Some("subagent")
        || record
            .payload
            .source
            .as_ref()
            .and_then(|source| source.get("subagent"))
            .is_some()
    {
        return None;
    }
    let source = match (
        record.payload.originator.as_deref(),
        record.payload.source.as_ref().and_then(Value::as_str),
    ) {
        (Some("codex_vscode"), _) | (_, Some("vscode")) => SessionSource::Vscode,
        (Some("codex-tui" | "codex_cli_rs"), _) | (_, Some("cli")) => SessionSource::Cli,
        _ => return None,
    };
    Some(SessionMetadata {
        id: record.payload.id.clone()?,
        cwd: record.payload.cwd.clone(),
        started_at: record.payload.timestamp.clone(),
        source,
    })
}

fn read_records(path: &Path, start: u64) -> Result<(Vec<CodexRecord>, u64), String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut stream =
        serde_json::Deserializer::from_reader(BufReader::new(file)).into_iter::<CodexRecord>();
    let mut records = Vec::new();
    while let Some(record) = stream.next() {
        match record {
            Ok(record) => records.push(record),
            Err(error) if error.is_eof() => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok((records, start + stream.byte_offset() as u64))
}

fn profile_from_context(payload: &RecordPayload) -> PermissionProfile {
    let sandbox = payload
        .sandbox_policy
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("custom");
    let (mode, label) = match sandbox {
        "danger-full-access" => (AccessMode::FullAccess, "Acesso total"),
        "workspace-write" => (AccessMode::WorkspaceWrite, "Edições no projeto"),
        "read-only" => (AccessMode::ReadOnly, "Somente leitura"),
        "plan" => (AccessMode::Plan, "Modo de planejamento"),
        _ => (AccessMode::Custom, "Permissões da sessão"),
    };
    PermissionProfile {
        mode,
        label: label.into(),
        approval_policy: payload
            .approval_policy
            .clone()
            .unwrap_or_else(|| "Gerenciada na origem".into()),
        approvals_reviewer: payload.approvals_reviewer.clone(),
        can_respond_from_lume: false,
        available_actions: vec![PermissionAction::OpenSource],
    }
}

fn default_profile() -> PermissionProfile {
    PermissionProfile {
        mode: AccessMode::Custom,
        label: "Permissões da sessão".into(),
        approval_policy: "Gerenciada na origem".into(),
        approvals_reviewer: None,
        can_respond_from_lume: false,
        available_actions: vec![PermissionAction::OpenSource],
    }
}

fn sessions_root() -> Option<PathBuf> {
    let codex_home = env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| {
        env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .map(|home| home.join(".codex"))
    })?;
    Some(codex_home.join("sessions"))
}

fn session_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_session_files(root, &mut files);
    files
}

fn collect_session_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_session_files(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(value: &str) -> CodexRecord {
        serde_json::from_str(value).expect("registro")
    }

    #[test]
    fn identifies_root_codex_sessions_without_creating_subagent_duplicates() {
        let vscode = record(
            r#"{"type":"session_meta","payload":{"id":"chat-1","originator":"codex_vscode","source":"vscode","cwd":"/work/lume"}}"#,
        );
        let cli = record(
            r#"{"type":"session_meta","payload":{"id":"chat-2","originator":"codex-tui","source":"cli","cwd":"/work/lume"}}"#,
        );
        let subagent = record(
            r#"{"type":"session_meta","payload":{"id":"chat-3","originator":"codex-tui","source":{"subagent":{"other":"guardian"}},"parent_thread_id":"chat-2","thread_source":"subagent","cwd":"/work/lume"}}"#,
        );

        assert_eq!(
            session_metadata(&vscode).expect("VS Code").source,
            SessionSource::Vscode
        );
        assert_eq!(
            session_metadata(&cli).expect("CLI").source,
            SessionSource::Cli
        );
        assert!(session_metadata(&subagent).is_none());
    }

    #[test]
    fn lifecycle_records_become_realtime_vscode_events() {
        let mut file = ObservedFile {
            offset: 0,
            session: Some(SessionMetadata {
                id: "chat-1".into(),
                cwd: Some("/work/lume".into()),
                started_at: None,
                source: SessionSource::Vscode,
            }),
            profile: None,
            pending_goal_tools: HashMap::new(),
        };
        let records = vec![
            record(r#"{"type":"event_msg","payload":{"type":"task_started"}}"#),
            record(
                r#"{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Resposta pronta"}}"#,
            ),
        ];

        let events = events_from_records(records, &mut file);

        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0].event, HookEventKind::Running));
        assert!(matches!(&events[1].event, HookEventKind::Completed));
        assert_eq!(events[0].source, Some(SessionSource::Vscode));
        assert_eq!(events[0].native_session_id.as_deref(), Some("chat-1"));
        assert_eq!(events[1].last_response.as_deref(), Some("Resposta pronta"));
    }

    #[test]
    fn rollout_messages_become_chat_entries() {
        let mut file = ObservedFile {
            offset: 0,
            session: Some(SessionMetadata {
                id: "chat-1".into(),
                cwd: Some("/work/lume".into()),
                started_at: None,
                source: SessionSource::Vscode,
            }),
            profile: None,
            pending_goal_tools: HashMap::new(),
        };
        let records = vec![
            record(
                r#"{"timestamp":"2026-07-24T10:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"Mostre os arquivos"}}"#,
            ),
            record(
                r#"{"timestamp":"2026-07-24T10:00:01Z","type":"event_msg","payload":{"type":"agent_message","message":"Alterei src/lib/TerminalWindow.svelte"}}"#,
            ),
        ];

        let events = events_from_records(records, &mut file);

        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0]
                .activity
                .as_ref()
                .map(|activity| activity.kind.as_str()),
            Some("prompt")
        );
        assert_eq!(
            events[0]
                .activity
                .as_ref()
                .and_then(|activity| activity.detail.as_deref()),
            Some("Mostre os arquivos")
        );
        assert_eq!(
            events[1]
                .activity
                .as_ref()
                .map(|activity| activity.kind.as_str()),
            Some("message")
        );
    }

    #[test]
    fn goal_tool_output_becomes_realtime_work_activity() {
        let mut file = ObservedFile {
            offset: 0,
            session: Some(SessionMetadata {
                id: "chat-1".into(),
                cwd: Some("/work/lume".into()),
                started_at: None,
                source: SessionSource::Cli,
            }),
            profile: None,
            pending_goal_tools: HashMap::new(),
        };
        let records = vec![
            record(
                r#"{"type":"response_item","payload":{"type":"function_call","id":"fc-goal","name":"get_goal","arguments":"{}","call_id":"call-goal"}}"#,
            ),
            record(
                r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call-goal","output":"{\"goal\":{\"objective\":\"Test goal\",\"status\":\"active\",\"createdAt\":1785190621}}"}}"#,
            ),
        ];

        let events = events_from_records(records, &mut file);

        assert_eq!(events.len(), 1);
        let activity = events[0].activity.as_ref().expect("goal activity");
        assert_eq!(activity.id, "codex:chat-1:fc-goal");
        assert_eq!(activity.kind, "tool");
        assert_eq!(activity.title, "functions · get_goal");
        assert!(activity
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("\"objective\":\"Test goal\"")));
    }

    #[test]
    fn reads_the_permission_profile_without_prompt_content() {
        let context = record(
            r#"{"type":"turn_context","payload":{"cwd":"/work/lume","approval_policy":"on-request","approvals_reviewer":"auto_review","sandbox_policy":{"type":"workspace-write"},"user_message":"nao deve ser guardada"}}"#,
        );
        let profile = profile_from_context(&context.payload);

        assert_eq!(profile.mode, AccessMode::WorkspaceWrite);
        assert_eq!(profile.approval_policy, "on-request");
        assert_eq!(profile.approvals_reviewer.as_deref(), Some("auto_review"));
        assert!(!profile.can_respond_from_lume);
    }
}
