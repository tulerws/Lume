use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, SystemTime},
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
const BOOTSTRAP_LOOKBACK: Duration = Duration::from_secs(5 * 60);
const BOOTSTRAP_TAIL_BYTES: u64 = 8 * 1024 * 1024;

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
    pending_tools: HashMap<String, PendingTool>,
}

#[derive(Debug)]
struct PendingTool {
    name: String,
    activity_id: String,
    kind: String,
    title: String,
    detail: Option<String>,
    files: Vec<String>,
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
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default)]
    summary: Option<Value>,
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    command: Option<Value>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    aggregated_output: Option<String>,
    #[serde(default)]
    stdout: Option<String>,
    #[serde(default)]
    stderr: Option<String>,
    #[serde(default)]
    changes: Option<Value>,
    #[serde(default)]
    success: Option<bool>,
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
    let mut observed = initialize(&root, &state, &app);
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

fn initialize(root: &Path, state: &AppState, app: &AppHandle) -> HashMap<PathBuf, ObservedFile> {
    let mut observed = HashMap::new();
    for path in session_files(root) {
        let Ok(file_metadata) = fs::metadata(&path) else {
            continue;
        };
        let mut file = ObservedFile {
            offset: file_metadata.len(),
            session: read_session_metadata(&path),
            profile: None,
            pending_tools: HashMap::new(),
        };
        if was_modified_recently(&file_metadata) && bootstrap_active_session(&path, &mut file) {
            if let Some(event) = event_for(&file, HookEventKind::Running, "Rodando", None) {
                let _ = event_server::publish_event(state, app, event);
            }
        }
        observed.insert(path, file);
    }
    observed
}

fn was_modified_recently(metadata: &fs::Metadata) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age <= BOOTSTRAP_LOOKBACK)
}

fn bootstrap_active_session(path: &Path, file: &mut ObservedFile) -> bool {
    let Ok(records) = read_tail_records(path, BOOTSTRAP_TAIL_BYTES) else {
        return false;
    };
    let mut running = false;
    for record in records {
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
        match record.payload.r#type.as_deref() {
            Some("task_started") => running = true,
            Some("task_complete" | "turn_aborted" | "stream_error" | "task_failed") => {
                running = false;
            }
            _ => {}
        }
    }
    running
}

fn read_tail_records(path: &Path, max_bytes: u64) -> Result<Vec<CodexRecord>, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let start = length.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file);
    if start > 0 {
        let mut partial = String::new();
        reader
            .read_line(&mut partial)
            .map_err(|error| error.to_string())?;
    }
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        if let Ok(record) = serde_json::from_str(&line) {
            records.push(record);
        }
    }
    Ok(records)
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
            pending_tools: HashMap::new(),
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
        file.pending_tools.clear();
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
                Some("function_call" | "custom_tool_call") => {
                    if let Some(event) = remember_tool(&record.payload, file) {
                        events.push(event);
                    }
                }
                Some("function_call_output" | "custom_tool_call_output") => {
                    if let Some(event) = tool_output_event(&record.payload, file) {
                        events.push(event);
                    }
                }
                Some("reasoning") => {
                    if let Some(detail) = reasoning_summary(&record.payload) {
                        if let Some(event) = activity_event_for(
                            file,
                            "analysis",
                            "Análise",
                            &detail,
                            record.timestamp.as_deref(),
                        ) {
                            events.push(event);
                        }
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
        if record.payload.r#type.as_deref() == Some("exec_command_end") {
            if let Some(event) = command_finished_event(&record.payload, file) {
                events.push(event);
            }
            continue;
        }
        if record.payload.r#type.as_deref() == Some("patch_apply_end") {
            if let Some(event) = patch_finished_event(&record.payload, file) {
                events.push(event);
            }
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
            Some("turn_aborted") => (HookEventKind::WaitingForInput, "Tarefa interrompida", None),
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

fn remember_tool(payload: &RecordPayload, file: &mut ObservedFile) -> Option<HookEvent> {
    let name = payload.name.as_deref()?;
    let call_id = payload.call_id.as_ref()?;
    let kind = tool_kind(name);
    let title = tool_title(name);
    let detail = tool_input_text(payload);
    let files = detail
        .as_deref()
        .map(files_from_patch_text)
        .unwrap_or_default();
    let activity_id = payload
        .id
        .as_ref()
        .and_then(|id| {
            file.session
                .as_ref()
                .map(|session| format!("codex:{}:{id}", session.id))
        })
        .unwrap_or_else(|| format!("codex-rollout-tool:{call_id}"));
    file.pending_tools.insert(
        call_id.clone(),
        PendingTool {
            name: name.into(),
            activity_id: activity_id.clone(),
            kind: kind.into(),
            title: title.clone(),
            detail: detail.clone(),
            files: files.clone(),
        },
    );
    if is_goal_tool(name) {
        return None;
    }
    let mut event = event_for(file, HookEventKind::Activity, &title, None)?;
    event.activity = Some(SessionActivity {
        id: activity_id,
        kind: kind.into(),
        title,
        detail,
        status: "running".into(),
        created_at: now_millis(),
        files,
        attachments: Vec::new(),
        append_detail: false,
    });
    Some(event)
}

fn tool_output_event(payload: &RecordPayload, file: &mut ObservedFile) -> Option<HookEvent> {
    let tool = file.pending_tools.remove(payload.call_id.as_deref()?)?;
    let output = payload.output.as_ref().and_then(record_value_text);
    let detail = combine_activity_detail(tool.detail.as_deref(), output.as_deref());
    let label = if is_goal_tool(&tool.name) {
        "GOAL atualizada"
    } else {
        &tool.title
    };
    let mut event = event_for(file, HookEventKind::Activity, label, None)?;
    event.activity = Some(SessionActivity {
        id: tool.activity_id,
        kind: tool.kind,
        title: if is_goal_tool(&tool.name) {
            format!("functions · {}", normalized_tool_name(&tool.name))
        } else {
            tool.title
        },
        detail,
        status: "completed".into(),
        created_at: now_millis(),
        files: tool.files,
        attachments: Vec::new(),
        append_detail: false,
    });
    Some(event)
}

fn command_finished_event(payload: &RecordPayload, file: &mut ObservedFile) -> Option<HookEvent> {
    let call_id = payload.call_id.as_deref()?;
    let tool = file.pending_tools.remove(call_id)?;
    let command = payload
        .command
        .as_ref()
        .and_then(command_value_text)
        .or(tool.detail);
    let output = payload
        .aggregated_output
        .as_deref()
        .and_then(response_text)
        .or_else(|| command_output_text(payload));
    let detail = combine_activity_detail(command.as_deref(), output.as_deref());
    let failed = payload.status.as_deref() == Some("failed")
        || payload.exit_code.is_some_and(|exit_code| exit_code != 0);
    let mut event = event_for(file, HookEventKind::Activity, "Comando", None)?;
    event.activity = Some(SessionActivity {
        id: tool.activity_id,
        kind: "command".into(),
        title: "Comando".into(),
        detail,
        status: if failed { "failed" } else { "completed" }.into(),
        created_at: now_millis(),
        files: tool.files,
        attachments: Vec::new(),
        append_detail: false,
    });
    Some(event)
}

fn patch_finished_event(payload: &RecordPayload, file: &mut ObservedFile) -> Option<HookEvent> {
    let changes = payload.changes.as_ref()?.as_object()?;
    if changes.is_empty() {
        return None;
    }
    let files = changes.keys().cloned().collect::<Vec<_>>();
    let mut diffs = Vec::new();
    for (path, change) in changes {
        if let Some(diff) = change.get("unified_diff").and_then(Value::as_str) {
            diffs.push(format!("*** Update File: {path}\n{diff}"));
        }
    }
    let detail = (!diffs.is_empty()).then(|| diffs.join("\n"));
    let session_id = file.session.as_ref()?.id.clone();
    let call_id = payload.call_id.as_deref().unwrap_or("patch");
    let pending = file.pending_tools.remove(call_id);
    let activity_id = pending
        .map(|tool| tool.activity_id)
        .unwrap_or_else(|| format!("codex:{session_id}:patch:{call_id}"));
    let failed = payload.success == Some(false) || payload.status.as_deref() == Some("failed");
    let mut event = event_for(file, HookEventKind::Activity, "Arquivos alterados", None)?;
    event.activity = Some(SessionActivity {
        id: activity_id,
        kind: "file".into(),
        title: "Arquivos alterados".into(),
        detail,
        status: if failed { "failed" } else { "completed" }.into(),
        created_at: now_millis(),
        files,
        attachments: Vec::new(),
        append_detail: false,
    });
    Some(event)
}

fn tool_kind(name: &str) -> &'static str {
    let name = normalized_tool_name(name);
    if matches!(name, "exec" | "exec_command" | "shell" | "terminal") {
        "command"
    } else if name == "apply_patch" {
        "file"
    } else if name == "update_plan" {
        "plan"
    } else {
        "tool"
    }
}

fn tool_title(name: &str) -> String {
    match normalized_tool_name(name) {
        "exec" | "exec_command" | "shell" | "terminal" => "Comando".into(),
        "apply_patch" => "Alteração de arquivo".into(),
        "update_plan" => "Plano atualizado".into(),
        "view_image" => "Imagem inspecionada".into(),
        "wait" => "Aguardando comando".into(),
        name => format!("functions · {name}"),
    }
}

fn normalized_tool_name(name: &str) -> &str {
    name.rsplit(['.', ':', '/']).next().unwrap_or(name)
}

fn tool_input_text(payload: &RecordPayload) -> Option<String> {
    payload
        .arguments
        .as_deref()
        .and_then(|arguments| {
            serde_json::from_str::<Value>(arguments)
                .ok()
                .as_ref()
                .and_then(tool_input_value_text)
                .or_else(|| response_text(arguments))
        })
        .or_else(|| payload.input.as_ref().and_then(tool_input_value_text))
}

fn tool_input_value_text(value: &Value) -> Option<String> {
    value
        .get("cmd")
        .and_then(Value::as_str)
        .and_then(response_text)
        .or_else(|| value.as_str().and_then(response_text))
        .or_else(|| record_value_text(value))
}

fn command_value_text(value: &Value) -> Option<String> {
    if let Some(parts) = value.as_array() {
        let command = parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        return response_text(&command);
    }
    record_value_text(value)
}

fn command_output_text(payload: &RecordPayload) -> Option<String> {
    let output = [payload.stdout.as_deref(), payload.stderr.as_deref()]
        .into_iter()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    response_text(&output)
}

fn combine_activity_detail(input: Option<&str>, output: Option<&str>) -> Option<String> {
    match (input, output) {
        (Some(input), Some(output)) if input.trim() != output.trim() => {
            response_text(&format!("{input}\n\n{output}"))
        }
        (Some(input), _) => response_text(input),
        (_, Some(output)) => response_text(output),
        _ => None,
    }
}

fn reasoning_summary(payload: &RecordPayload) -> Option<String> {
    payload
        .summary
        .as_ref()
        .and_then(value_text)
        .or_else(|| payload.content.as_ref().and_then(value_text))
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => response_text(value),
        Value::Array(values) => {
            let text = values
                .iter()
                .filter_map(value_text)
                .collect::<Vec<_>>()
                .join("\n");
            response_text(&text)
        }
        Value::Object(object) => object
            .get("text")
            .and_then(value_text)
            .or_else(|| object.get("summary_text").and_then(value_text)),
        _ => None,
    }
}

fn files_from_patch_text(value: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in value.lines() {
        let path = ["*** Add File: ", "*** Update File: ", "*** Delete File: "]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix));
        if let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) {
            if !files.iter().any(|existing| existing == path) {
                files.push(path.into());
            }
        }
    }
    files
}

fn is_goal_tool(name: &str) -> bool {
    matches!(
        normalized_tool_name(name),
        "create_goal" | "get_goal" | "update_goal"
    )
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
        permission_profile: file.profile.clone(),
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
            .cwd
            .as_deref()
            .is_some_and(crate::session_filters::is_codex_internal_workspace)
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

    fn observed_file(source: SessionSource) -> ObservedFile {
        ObservedFile {
            offset: 0,
            session: Some(SessionMetadata {
                id: "chat-1".into(),
                cwd: Some("/work/lume".into()),
                started_at: None,
                source,
            }),
            profile: None,
            pending_tools: HashMap::new(),
        }
    }

    fn temporary_rollout(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lume-codex-session-{name}-{}-{}.jsonl",
            std::process::id(),
            now_millis()
        ))
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
        let memories = record(
            r#"{"type":"session_meta","payload":{"id":"chat-4","originator":"codex-tui","source":"cli","cwd":"/home/user/.codex/memories"}}"#,
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
        assert!(session_metadata(&memories).is_none());
    }

    #[test]
    fn lifecycle_records_become_realtime_vscode_events() {
        let mut file = observed_file(SessionSource::Vscode);
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
    fn an_interrupted_turn_returns_to_waiting_instead_of_completing() {
        let mut file = observed_file(SessionSource::Vscode);
        let events = events_from_records(
            vec![record(
                r#"{"type":"event_msg","payload":{"type":"turn_aborted"}}"#,
            )],
            &mut file,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event, HookEventKind::WaitingForInput));
        assert_eq!(
            events[0].status_label.as_deref(),
            Some("Tarefa interrompida")
        );
    }

    #[test]
    fn rollout_messages_become_chat_entries() {
        let mut file = observed_file(SessionSource::Vscode);
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
        let mut file = observed_file(SessionSource::Cli);
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
    fn command_and_patch_records_become_detailed_activities() {
        let mut file = observed_file(SessionSource::Vscode);
        let records = vec![
            record(
                r#"{"type":"response_item","payload":{"type":"function_call","id":"fc-command","name":"exec_command","arguments":"{\"cmd\":\"cargo test\"}","call_id":"call-command"}}"#,
            ),
            record(
                r#"{"type":"event_msg","payload":{"type":"exec_command_end","call_id":"call-command","command":["/bin/bash","-lc","cargo test"],"status":"completed","exit_code":0,"aggregated_output":"4 tests passed"}}"#,
            ),
            record(
                r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","call_id":"call-patch","input":"*** Begin Patch\n*** Update File: /work/lume/src/main.rs\n@@\n-old\n+new\n*** End Patch"}}"#,
            ),
            record(
                r#"{"type":"event_msg","payload":{"type":"patch_apply_end","call_id":"call-patch","status":"completed","success":true,"changes":{"/work/lume/src/main.rs":{"type":"update","unified_diff":"@@ -1 +1 @@\n-old\n+new\n"}}}}"#,
            ),
        ];

        let events = events_from_records(records, &mut file);

        assert_eq!(events.len(), 4);
        assert_eq!(
            events[0]
                .activity
                .as_ref()
                .map(|activity| activity.kind.as_str()),
            Some("command")
        );
        assert_eq!(
            events[0]
                .activity
                .as_ref()
                .map(|activity| activity.status.as_str()),
            Some("running")
        );
        assert_eq!(
            events[1]
                .activity
                .as_ref()
                .map(|activity| activity.status.as_str()),
            Some("completed")
        );
        assert_eq!(
            events[0]
                .activity
                .as_ref()
                .map(|activity| activity.id.as_str()),
            events[1]
                .activity
                .as_ref()
                .map(|activity| activity.id.as_str())
        );
        let patch_start = events[2].activity.as_ref().expect("patch start activity");
        let patch = events[3].activity.as_ref().expect("patch activity");
        assert_eq!(patch.kind, "file");
        assert_eq!(patch_start.id, patch.id);
        assert_eq!(patch.files, vec!["/work/lume/src/main.rs"]);
        assert!(patch
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("+new")));
    }

    #[test]
    fn namespaced_exec_and_plan_tools_get_semantic_activity_kinds() {
        assert_eq!(tool_kind("functions.exec"), "command");
        assert_eq!(tool_title("functions.exec"), "Comando");
        assert_eq!(tool_kind("functions.update_plan"), "plan");
        assert_eq!(tool_title("functions.update_plan"), "Plano atualizado");
        assert!(is_goal_tool("functions.get_goal"));
    }

    #[test]
    fn reasoning_summaries_are_visible_without_encrypted_reasoning() {
        let mut file = observed_file(SessionSource::Vscode);
        let events = events_from_records(
            vec![record(
                r#"{"timestamp":"2026-07-29T10:00:00Z","type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"Vou validar os eventos."}],"encrypted_content":"nao exibir"}}"#,
            )],
            &mut file,
        );

        assert_eq!(events.len(), 1);
        let activity = events[0].activity.as_ref().expect("analysis activity");
        assert_eq!(activity.kind, "analysis");
        assert_eq!(activity.detail.as_deref(), Some("Vou validar os eventos."));
    }

    #[test]
    fn startup_only_restores_a_rollout_with_an_active_turn() {
        let path = temporary_rollout("bootstrap");
        fs::write(
            &path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"chat-1\",\"originator\":\"codex_vscode\",\"source\":\"vscode\",\"cwd\":\"/work/lume\"}}\n",
                "{\"type\":\"turn_context\",\"payload\":{\"cwd\":\"/work/lume\",\"approval_policy\":\"on-request\",\"sandbox_policy\":{\"type\":\"workspace-write\"}}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n"
            ),
        )
        .expect("write active rollout");
        let mut file = observed_file(SessionSource::Vscode);
        assert!(bootstrap_active_session(&path, &mut file));
        assert_eq!(
            file.profile.as_ref().map(|profile| &profile.mode),
            Some(&AccessMode::WorkspaceWrite)
        );

        fs::write(
            &path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"chat-1\",\"originator\":\"codex_vscode\",\"source\":\"vscode\",\"cwd\":\"/work/lume\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n"
            ),
        )
        .expect("write completed rollout");
        assert!(!bootstrap_active_session(&path, &mut file));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn incremental_reader_retries_a_partial_json_record() {
        let path = temporary_rollout("partial");
        let complete = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n";
        let partial = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"";
        fs::write(&path, format!("{complete}{partial}")).expect("write partial rollout");

        let (records, offset) = read_records(&path, 0).expect("first read");
        assert_eq!(records.len(), 1);
        assert_eq!(offset, complete.len() as u64);

        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open rollout");
        file.write_all(b"}}\n").expect("finish record");

        let (records, final_offset) = read_records(&path, offset).expect("second read");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].payload.r#type.as_deref(), Some("task_complete"));
        assert_eq!(final_offset, fs::metadata(&path).expect("metadata").len());
        let _ = fs::remove_file(path);
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

    #[test]
    fn events_without_a_turn_context_do_not_publish_a_fallback_permission_profile() {
        let file = observed_file(SessionSource::Cli);
        let event =
            event_for(&file, HookEventKind::Running, "Rodando", None).expect("session event");

        assert!(event.permission_profile.is_none());
    }
}
