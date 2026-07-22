use std::{
    collections::HashMap,
    io::ErrorKind,
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tauri::AppHandle;
use tungstenite::{accept, connect, stream::MaybeTlsStream, Message, WebSocket};

use crate::{
    domain::{
        AccessMode, AgentKind, HookEvent, HookEventKind, PermissionAction, PermissionProfile,
        PermissionRequest, SessionSource,
    },
    event_server,
    state::{now_millis, AppState},
};

const SERVER_ADDRESS: &str = "127.0.0.1:43130";
const SERVER_URL: &str = "ws://127.0.0.1:43130";
const PROXY_ADDRESS: &str = "127.0.0.1:43131";
pub const PROXY_URL: &str = "ws://127.0.0.1:43131";

pub struct CodexBridge {
    process: Mutex<Option<Child>>,
}

impl CodexBridge {
    pub fn start(state: AppState, app: AppHandle) -> Result<Self, String> {
        let listener = TcpListener::bind(PROXY_ADDRESS)
            .map_err(|error| format!("Could not start the Codex bridge: {error}"))?;
        thread::Builder::new()
            .name("lume-codex-proxy".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let state = state.clone();
                    let app = app.clone();
                    let _ = thread::Builder::new()
                        .name("lume-codex-client".into())
                        .spawn(move || {
                            if let Err(error) = proxy_connection(stream, state, app) {
                                eprintln!("Ponte do Codex encerrada: {error}");
                            }
                        });
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            process: Mutex::new(None),
        })
    }

    pub fn ensure_server(&self) -> Result<(), String> {
        if server_available() {
            return Ok(());
        }
        let mut process = command_for_server()?
            .spawn()
            .map_err(|error| format!("Could not start `codex app-server`: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if server_available() {
                *self
                    .process
                    .lock()
                    .map_err(|_| "Não foi possível guardar o processo do Codex".to_string())? =
                    Some(process);
                return Ok(());
            }
            if process
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err("O servidor do Codex encerrou antes de ficar disponível".into());
            }
            thread::sleep(Duration::from_millis(80));
        }
        let _ = process.kill();
        Err("O servidor do Codex não respondeu a tempo".into())
    }

    pub fn submit_prompt(
        &self,
        thread_id: &str,
        prompt: &str,
        profile: PermissionProfile,
        state: AppState,
        app: AppHandle,
    ) -> Result<(), String> {
        self.ensure_server()?;
        let mut server = prompt_connection(thread_id, prompt, profile, &state, &app)?;
        let thread_id = thread_id.to_string();
        thread::Builder::new()
            .name("lume-codex-prompt".into())
            .spawn(move || {
                if let Err(error) = monitor_prompt(&mut server, &thread_id, &state, &app) {
                    eprintln!("Prompt do Lume encerrado: {error}");
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

impl Drop for CodexBridge {
    fn drop(&mut self) {
        if let Ok(process) = self.process.get_mut() {
            if let Some(process) = process.as_mut() {
                let _ = process.kill();
                let _ = process.wait();
            }
        }
    }
}

fn command_for_server() -> Result<Command, String> {
    let mut command = crate::executables::command("codex")?;
    command
        .args(["app-server", "--listen", SERVER_URL])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    Ok(command)
}

fn server_available() -> bool {
    SERVER_ADDRESS
        .parse()
        .ok()
        .and_then(|address| TcpStream::connect_timeout(&address, Duration::from_millis(120)).ok())
        .is_some()
}

fn proxy_connection(stream: TcpStream, state: AppState, app: AppHandle) -> Result<(), String> {
    let mut client = accept(stream).map_err(|error| error.to_string())?;
    let (mut server, _) = connect(SERVER_URL).map_err(|error| error.to_string())?;
    configure_client_timeout(&mut client)?;
    configure_server_timeout(&mut server)?;
    let mut profiles = HashMap::new();
    let mut responses = HashMap::new();

    loop {
        match client.read() {
            Ok(message) => {
                let closing = matches!(message, Message::Close(_));
                observe_client_message(&message, &mut profiles);
                server.send(message).map_err(|error| error.to_string())?;
                if closing {
                    break;
                }
            }
            Err(tungstenite::Error::ConnectionClosed) => break,
            Err(tungstenite::Error::Io(error)) if transient(&error) => {}
            Err(error) => return Err(error.to_string()),
        }

        match server.read() {
            Ok(message) => {
                let closing = matches!(message, Message::Close(_));
                if let Some(response) =
                    intercept_server_message(&message, &state, &app, &profiles, &mut responses)?
                {
                    server.send(response).map_err(|error| error.to_string())?;
                } else {
                    client.send(message).map_err(|error| error.to_string())?;
                }
                if closing {
                    break;
                }
            }
            Err(tungstenite::Error::ConnectionClosed) => break,
            Err(tungstenite::Error::Io(error)) if transient(&error) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn configure_client_timeout(socket: &mut WebSocket<TcpStream>) -> Result<(), String> {
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(45)))
        .map_err(|error| error.to_string())
}

fn configure_server_timeout(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
) -> Result<(), String> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream
            .set_read_timeout(Some(Duration::from_millis(45)))
            .map_err(|error| error.to_string()),
        _ => Err("O Codex local deve usar uma conexão WebSocket sem TLS".into()),
    }
}

fn transient(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

fn prompt_connection(
    thread_id: &str,
    prompt: &str,
    profile: PermissionProfile,
    state: &AppState,
    app: &AppHandle,
) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let (mut server, _) = connect(SERVER_URL).map_err(|error| error.to_string())?;
    set_server_timeout(&mut server, Duration::from_secs(5))?;
    let mut profiles = HashMap::from([(thread_id.to_string(), profile)]);

    send_json(
        &mut server,
        json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": { "name": "lume", "title": "Lume", "version": env!("CARGO_PKG_VERSION") }
            }
        }),
    )?;
    wait_for_response(&mut server, 1, state, app, &profiles)?;
    send_json(
        &mut server,
        json!({ "method": "initialized", "params": {} }),
    )?;
    send_json(
        &mut server,
        json!({ "method": "thread/resume", "id": 2, "params": { "threadId": thread_id } }),
    )?;
    wait_for_response(&mut server, 2, state, app, &profiles)?;

    let turn = prompt_turn_request(thread_id, prompt);
    observe_client_message(&Message::Text(turn.to_string().into()), &mut profiles);
    send_json(&mut server, turn)?;
    wait_for_response(&mut server, 3, state, app, &profiles)?;
    set_server_timeout(&mut server, Duration::from_millis(200))?;
    Ok(server)
}

fn prompt_turn_request(thread_id: &str, prompt: &str) -> Value {
    json!({
        "method": "turn/start",
        "id": 3,
        "params": {
            "threadId": thread_id,
            "input": [{ "type": "text", "text": prompt }]
        }
    })
}

fn set_server_timeout(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) -> Result<(), String> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| error.to_string()),
        _ => Err("O Codex local deve usar uma conexão WebSocket sem TLS".into()),
    }
}

fn send_json(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    value: Value,
) -> Result<(), String> {
    socket
        .send(Message::Text(value.to_string().into()))
        .map_err(|error| error.to_string())
}

fn wait_for_response(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    expected_id: i64,
    state: &AppState,
    app: &AppHandle,
    profiles: &HashMap<String, PermissionProfile>,
) -> Result<(), String> {
    let mut responses = HashMap::new();
    loop {
        let message = socket.read().map_err(|error| error.to_string())?;
        if let Message::Text(text) = &message {
            if let Ok(value) = serde_json::from_str::<Value>(text) {
                if value.get("id").and_then(Value::as_i64) == Some(expected_id) {
                    if let Some(error) = value.get("error") {
                        return Err(error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("O Codex recusou o prompt")
                            .to_string());
                    }
                    return Ok(());
                }
            }
        }
        if let Some(response) =
            intercept_server_message(&message, state, app, profiles, &mut responses)?
        {
            socket.send(response).map_err(|error| error.to_string())?;
        }
    }
}

fn monitor_prompt(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    thread_id: &str,
    state: &AppState,
    app: &AppHandle,
) -> Result<(), String> {
    let profiles = HashMap::from([(thread_id.to_string(), direct_profile())]);
    let mut responses = HashMap::new();
    loop {
        match socket.read() {
            Ok(message) => {
                let completed =
                    match &message {
                        Message::Text(text) => serde_json::from_str::<Value>(text)
                            .ok()
                            .is_some_and(|value| {
                                value.get("method").and_then(Value::as_str)
                                    == Some("turn/completed")
                                    && value
                                        .get("params")
                                        .and_then(|params| text_at(params, "threadId"))
                                        == Some(thread_id)
                            }),
                        _ => false,
                    };
                if let Some(response) =
                    intercept_server_message(&message, state, app, &profiles, &mut responses)?
                {
                    socket.send(response).map_err(|error| error.to_string())?;
                }
                if completed {
                    return Ok(());
                }
            }
            Err(tungstenite::Error::ConnectionClosed) => return Ok(()),
            Err(tungstenite::Error::Io(error)) if transient(&error) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn intercept_server_message(
    message: &Message,
    state: &AppState,
    app: &AppHandle,
    profiles: &HashMap<String, PermissionProfile>,
    responses: &mut HashMap<String, String>,
) -> Result<Option<Message>, String> {
    let Message::Text(text) = message else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Ok(None);
    };
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if is_approval(method) && value.get("id").is_some() {
        return approval_response(&value, method, state, app, profiles).map(Some);
    }
    remember_response(&value, method, responses);
    if let Some(event) = notification_event(&value, method, profiles, responses) {
        let _ = event_server::publish_event(state, app, event);
    }
    Ok(None)
}

fn observe_client_message(message: &Message, profiles: &mut HashMap<String, PermissionProfile>) {
    let Message::Text(text) = message else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !matches!(method, "thread/resume" | "turn/start") {
        return;
    }
    let Some(params) = value.get("params") else {
        return;
    };
    let Some(thread_id) = text_at(params, "threadId") else {
        return;
    };
    let current = profiles
        .get(thread_id)
        .cloned()
        .unwrap_or_else(direct_profile);
    profiles.insert(thread_id.into(), profile_from_params(params, current));
}

fn is_approval(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
    )
}

fn approval_response(
    value: &Value,
    method: &str,
    state: &AppState,
    app: &AppHandle,
    profiles: &HashMap<String, PermissionProfile>,
) -> Result<Message, String> {
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    let thread_id = text_at(&params, "threadId").unwrap_or("unknown");
    let item_id = text_at(&params, "itemId").unwrap_or("approval");
    let permission_id = format!("codex:{thread_id}:{item_id}");
    let cwd = text_at(&params, "cwd").map(str::to_string);
    let (kind, summary, resource, risk) = permission_details(method, &params, cwd.as_deref());
    let event = HookEvent {
        event: HookEventKind::PermissionRequest,
        session_id: session_id(thread_id),
        agent: AgentKind::Codex,
        agent_label: Some("Codex".into()),
        project: cwd.as_deref().and_then(project_name),
        source: Some(SessionSource::Cli),
        source_app: None,
        status_label: Some("Aguardando sua permissão".into()),
        started_at: None,
        process_id: None,
        native_session_id: Some(thread_id.into()),
        working_directory: cwd,
        permission_profile: Some(
            profiles
                .get(thread_id)
                .cloned()
                .unwrap_or_else(direct_profile),
        ),
        permission: Some(PermissionRequest {
            id: permission_id.clone(),
            kind,
            summary,
            resource,
            risk,
            requested_at: now_millis().to_string(),
        }),
        last_response: None,
        wait_for_decision: true,
    };
    event_server::publish_event(state, app, event)?;
    let action = state
        .wait_for_decision(&permission_id, Duration::from_secs(15 * 60))?
        .unwrap_or(PermissionAction::Deny);
    let result = decision_result(method, action, &params);
    let response =
        json!({ "id": value.get("id").cloned().unwrap_or(Value::Null), "result": result });
    Ok(Message::Text(response.to_string().into()))
}

fn permission_details(
    method: &str,
    params: &Value,
    cwd: Option<&str>,
) -> (String, String, String, String) {
    let reason = text_at(params, "reason");
    match method {
        "item/commandExecution/requestApproval" => (
            "command".into(),
            reason.unwrap_or("Executar comando").into(),
            text_at(params, "command")
                .unwrap_or("Comando não informado")
                .into(),
            if params
                .get("networkApprovalContext")
                .is_some_and(|value| !value.is_null())
            {
                "high".into()
            } else {
                "medium".into()
            },
        ),
        "item/fileChange/requestApproval" => (
            "file_change".into(),
            reason.unwrap_or("Alterar arquivos").into(),
            text_at(params, "grantRoot")
                .or(cwd)
                .unwrap_or("Arquivos da sessão")
                .into(),
            "medium".into(),
        ),
        _ => (
            "permissions".into(),
            reason.unwrap_or("Ampliar permissões da sessão").into(),
            cwd.unwrap_or("Recursos adicionais").into(),
            "high".into(),
        ),
    }
}

fn decision_result(method: &str, action: PermissionAction, params: &Value) -> Value {
    if method == "item/permissions/requestApproval" {
        let permissions = if action == PermissionAction::Deny {
            json!({})
        } else {
            params
                .get("permissions")
                .cloned()
                .unwrap_or_else(|| json!({}))
        };
        return json!({
            "permissions": permissions,
            "scope": if action == PermissionAction::AllowSession { "session" } else { "turn" }
        });
    }
    json!({
        "decision": match action {
            PermissionAction::AllowOnce => "accept",
            PermissionAction::AllowSession => "acceptForSession",
            PermissionAction::Deny | PermissionAction::OpenSource => "decline",
        }
    })
}

fn notification_event(
    value: &Value,
    method: &str,
    profiles: &HashMap<String, PermissionProfile>,
    responses: &mut HashMap<String, String>,
) -> Option<HookEvent> {
    let params = value.get("params")?;
    let (event, thread_id, status_label, cwd, name, started_at, last_response) = match method {
        "thread/started" => {
            let thread = params.get("thread")?;
            (
                HookEventKind::SessionStarted,
                text_at(thread, "id")?,
                "Sessão iniciada",
                text_at(thread, "cwd"),
                text_at(thread, "name"),
                thread
                    .get("createdAt")
                    .and_then(Value::as_i64)
                    .map(|value| value.to_string()),
                None,
            )
        }
        "turn/started" => {
            let thread_id = text_at(params, "threadId")?;
            responses.remove(thread_id);
            (
                HookEventKind::Running,
                thread_id,
                "Executando",
                None,
                None,
                None,
                None,
            )
        }
        "turn/completed" => {
            let thread_id = text_at(params, "threadId")?;
            let status = params
                .get("turn")
                .and_then(|turn| text_at(turn, "status"))
                .unwrap_or("completed");
            let (event, label) = if status == "failed" {
                (HookEventKind::Failed, "Tarefa encerrada com erro")
            } else {
                (HookEventKind::Completed, "Tarefa finalizada")
            };
            let last_response = responses
                .remove(thread_id)
                .or_else(|| response_from_turn(params));
            (event, thread_id, label, None, None, None, last_response)
        }
        "thread/closed" => (
            HookEventKind::SessionEnded,
            text_at(params, "threadId")?,
            "Sessão encerrada",
            None,
            None,
            None,
            None,
        ),
        _ => return None,
    };
    Some(HookEvent {
        event,
        session_id: session_id(thread_id),
        agent: AgentKind::Codex,
        agent_label: Some("Codex".into()),
        project: name
            .map(str::to_string)
            .or_else(|| cwd.and_then(project_name)),
        source: Some(SessionSource::Cli),
        source_app: None,
        status_label: Some(status_label.into()),
        started_at,
        process_id: None,
        native_session_id: Some(thread_id.into()),
        working_directory: cwd.map(str::to_string),
        permission_profile: Some(
            profiles
                .get(thread_id)
                .cloned()
                .unwrap_or_else(direct_profile),
        ),
        permission: None,
        last_response,
        wait_for_decision: false,
    })
}

fn remember_response(value: &Value, method: &str, responses: &mut HashMap<String, String>) {
    if method != "item/completed" {
        return;
    }
    let Some(params) = value.get("params") else {
        return;
    };
    let Some(thread_id) = text_at(params, "threadId") else {
        return;
    };
    let Some(item) = params.get("item") else {
        return;
    };
    if text_at(item, "type") != Some("agentMessage") {
        return;
    }
    let Some(text) = text_at(item, "text").and_then(response_text) else {
        return;
    };
    responses.insert(thread_id.to_string(), text);
}

fn response_from_turn(params: &Value) -> Option<String> {
    params
        .get("turn")?
        .get("items")?
        .as_array()?
        .iter()
        .rev()
        .find(|item| text_at(item, "type") == Some("agentMessage"))
        .and_then(|item| text_at(item, "text"))
        .and_then(response_text)
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

fn direct_profile() -> PermissionProfile {
    PermissionProfile {
        mode: AccessMode::Custom,
        label: "Permissões desta sessão".into(),
        approval_policy: "Decisões encaminhadas pelo Codex App Server".into(),
        approvals_reviewer: None,
        can_respond_from_lume: true,
        available_actions: vec![
            PermissionAction::AllowOnce,
            PermissionAction::AllowSession,
            PermissionAction::Deny,
        ],
    }
}

fn profile_from_params(params: &Value, mut profile: PermissionProfile) -> PermissionProfile {
    let sandbox = text_at(params, "sandbox").or_else(|| {
        params
            .get("sandboxPolicy")
            .and_then(|policy| text_at(policy, "type"))
    });
    if let Some(sandbox) = sandbox {
        let (mode, label) = match sandbox {
            "danger-full-access" | "dangerFullAccess" => (AccessMode::FullAccess, "Acesso total"),
            "read-only" | "readOnly" => (AccessMode::ReadOnly, "Somente leitura"),
            "workspace-write" | "workspaceWrite" => {
                (AccessMode::WorkspaceWrite, "Acesso ao projeto")
            }
            _ => (AccessMode::Custom, "Permissões personalizadas"),
        };
        profile.mode = mode;
        profile.label = label.into();
    }
    if let Some(policy) = params
        .get("approvalPolicy")
        .filter(|value| !value.is_null())
    {
        profile.approval_policy = policy
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| "Política granular".into());
    }
    if let Some(reviewer) = params
        .get("approvalsReviewer")
        .or_else(|| params.get("approvals_reviewer"))
        .and_then(Value::as_str)
    {
        profile.approvals_reviewer = Some(reviewer.into());
    }
    profile
}

fn text_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn session_id(thread_id: &str) -> String {
    format!("codex-app-server:{thread_id}")
}

fn project_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_command_decisions_to_codex_protocol() {
        let params = json!({});
        assert_eq!(
            decision_result(
                "item/commandExecution/requestApproval",
                PermissionAction::AllowSession,
                &params
            ),
            json!({ "decision": "acceptForSession" })
        );
        assert_eq!(
            decision_result(
                "item/fileChange/requestApproval",
                PermissionAction::Deny,
                &params
            ),
            json!({ "decision": "decline" })
        );
    }

    #[test]
    fn permission_grants_echo_requested_profile_without_extra_data() {
        let params = json!({ "permissions": { "network": { "enabled": true } } });
        assert_eq!(
            decision_result(
                "item/permissions/requestApproval",
                PermissionAction::AllowOnce,
                &params
            ),
            json!({
                "permissions": { "network": { "enabled": true } },
                "scope": "turn"
            })
        );
    }

    #[test]
    fn reads_per_thread_codex_access_configuration() {
        let profile = profile_from_params(
            &json!({
                "threadId": "thread",
                "sandboxPolicy": { "type": "readOnly", "networkAccess": false },
                "approvalPolicy": "on-request",
                "approvalsReviewer": "auto_review"
            }),
            direct_profile(),
        );
        assert_eq!(profile.mode, AccessMode::ReadOnly);
        assert_eq!(profile.label, "Somente leitura");
        assert_eq!(profile.approval_policy, "on-request");
        assert_eq!(profile.approvals_reviewer.as_deref(), Some("auto_review"));
    }

    #[test]
    fn prompt_uses_the_documented_turn_start_shape() {
        assert_eq!(
            prompt_turn_request("thread-1", "Continue os testes"),
            json!({
                "method": "turn/start",
                "id": 3,
                "params": {
                    "threadId": "thread-1",
                    "input": [{ "type": "text", "text": "Continue os testes" }]
                }
            })
        );
    }

    #[test]
    fn completed_turn_carries_the_last_agent_message() {
        let mut responses = HashMap::new();
        remember_response(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "item": { "type": "agentMessage", "text": "Resposta final" }
                }
            }),
            "item/completed",
            &mut responses,
        );
        let event = notification_event(
            &json!({
                "method": "turn/completed",
                "params": { "threadId": "thread-1", "turn": { "status": "completed" } }
            }),
            "turn/completed",
            &HashMap::new(),
            &mut responses,
        )
        .expect("evento concluído");

        assert_eq!(event.last_response.as_deref(), Some("Resposta final"));
        assert!(responses.is_empty());
    }
}
