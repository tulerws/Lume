use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
};

use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::{
    domain::{
        AccessMode, AgentKind, HookEvent, HookEventKind, PermissionAction, PermissionProfile,
        PermissionRequest, SessionSource,
    },
    state::{now_millis, AppState},
};

const ADDRESS: &str = "127.0.0.1:43120";
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum BrowserState {
    Running,
    PermissionRequired,
    WaitingForInput,
    Completed,
    Failed,
    Closed,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserEvent {
    provider: String,
    session_id: String,
    title: String,
    origin: String,
    browser: Option<String>,
    #[serde(default)]
    protocol_version: u8,
    state: BrowserState,
    #[serde(default)]
    last_response: Option<String>,
}

#[derive(Clone)]
struct BrowserPromptRequest {
    id: String,
    prompt: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserPromptAck {
    provider: String,
    session_id: String,
    prompt_id: String,
    submitted: bool,
}

#[derive(Clone, Default)]
pub struct BrowserControl {
    focus_requests: Arc<Mutex<HashSet<String>>>,
    prompt_requests: Arc<Mutex<HashMap<String, BrowserPromptRequest>>>,
    prompt_sequence: Arc<AtomicU64>,
}

impl BrowserControl {
    pub fn request_focus(&self, session_id: String) -> Result<(), String> {
        self.focus_requests
            .lock()
            .map_err(|_| "Não foi possível acessar o conector web".to_string())?
            .insert(session_id);
        Ok(())
    }

    pub fn request_prompt(&self, session_id: String, prompt: String) -> Result<(), String> {
        let request = BrowserPromptRequest {
            id: format!(
                "{}-{}",
                now_millis(),
                self.prompt_sequence.fetch_add(1, Ordering::Relaxed)
            ),
            prompt,
        };
        self.prompt_requests
            .lock()
            .map_err(|_| "Não foi possível acessar o conector web".to_string())?
            .insert(session_id, request);
        Ok(())
    }

    fn take_focus(&self, session_id: &str) -> bool {
        self.focus_requests
            .lock()
            .map(|mut requests| requests.remove(session_id))
            .unwrap_or(false)
    }

    fn pending_prompt(&self, session_id: &str) -> Option<BrowserPromptRequest> {
        self.prompt_requests
            .lock()
            .ok()
            .and_then(|requests| requests.get(session_id).cloned())
    }

    fn take_prompt(&self, session_id: &str) -> Option<BrowserPromptRequest> {
        self.prompt_requests
            .lock()
            .ok()
            .and_then(|mut requests| requests.remove(session_id))
    }

    fn acknowledge_prompt(
        &self,
        session_id: &str,
        prompt_id: &str,
        submitted: bool,
    ) -> Result<(), String> {
        if !submitted {
            return Ok(());
        }
        let mut requests = self
            .prompt_requests
            .lock()
            .map_err(|_| "Não foi possível acessar o conector web".to_string())?;
        if requests
            .get(session_id)
            .is_some_and(|request| request.id == prompt_id)
        {
            requests.remove(session_id);
        }
        Ok(())
    }
}

pub fn start(state: AppState, app: AppHandle, control: BrowserControl) -> Result<(), String> {
    let listener = TcpListener::bind(ADDRESS)
        .map_err(|error| format!("Não foi possível iniciar o conector web: {error}"))?;
    thread::Builder::new()
        .name("lume-browser-server".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = state.clone();
                let app = app.clone();
                let control = control.clone();
                let _ = thread::Builder::new()
                    .name("lume-browser-client".into())
                    .spawn(move || handle(stream, state, app, control));
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn handle(mut stream: TcpStream, state: AppState, app: AppHandle, control: BrowserControl) {
    let request = read_request(&stream);
    let (status, body, origin) = match request {
        Ok(request) if request.method == "OPTIONS" && allowed_origin(&request.origin) => {
            ("204 No Content", String::new(), request.origin)
        }
        Ok(request)
            if request.method == "GET"
                && request.path == "/health"
                && allowed_origin(&request.origin) =>
        {
            ("200 OK", "{\"ok\":true}".into(), request.origin)
        }
        Ok(request)
            if request.method == "POST"
                && request.path == "/events"
                && allowed_origin(&request.origin) =>
        {
            match serde_json::from_slice::<BrowserEvent>(&request.body)
                .map_err(|error| error.to_string())
                .and_then(|browser_event| {
                    let provider = canonical_provider(&browser_event.provider)?;
                    let session_id = format!("web:{}:{}", provider, browser_event.session_id);
                    let supports_prompt_ack = browser_event.protocol_version >= 2;
                    let previous_status = state
                        .sessions()?
                        .into_iter()
                        .find(|session| session.id == session_id)
                        .map(|session| session.status);
                    let focus = control.take_focus(&session_id);
                    let prompt_request = if supports_prompt_ack {
                        control.pending_prompt(&session_id)
                    } else {
                        control.take_prompt(&session_id)
                    };
                    let event = map_event(browser_event)?;
                    let notification =
                        crate::domain::should_notify(&event.event, previous_status.as_ref());
                    let label = event.agent_label.clone().unwrap_or_else(|| "Agente".into());
                    let project = event.project.clone().unwrap_or_else(|| "sessão web".into());
                    let event_kind = event.event.clone();
                    state.ingest(event)?;
                    crate::protocol::emit_sessions_changed(&app);
                    if notification {
                        let title = match event_kind {
                            HookEventKind::PermissionRequest => "Lume · Ação necessária",
                            HookEventKind::Failed => "Lume · Erro na sessão",
                            _ => "Lume · Tarefa finalizada",
                        };
                        let _ = app
                            .notification()
                            .builder()
                            .title(title)
                            .body(format!("{label} · {project}"))
                            .show();
                    }
                    Ok((focus, prompt_request))
                }) {
                Ok((focus, prompt_request)) => {
                    let prompt = prompt_request
                        .as_ref()
                        .map(|request| request.prompt.as_str());
                    let prompt_id = prompt_request.as_ref().map(|request| request.id.as_str());
                    (
                        "202 Accepted",
                        serde_json::json!({
                            "ok": true,
                            "focus": focus,
                            "prompt": prompt,
                            "promptId": prompt_id,
                        })
                        .to_string(),
                        request.origin,
                    )
                }
                Err(_) => ("400 Bad Request", "{\"ok\":false}".into(), request.origin),
            }
        }
        Ok(request)
            if request.method == "POST"
                && request.path == "/prompt-ack"
                && allowed_origin(&request.origin) =>
        {
            let acknowledged = serde_json::from_slice::<BrowserPromptAck>(&request.body)
                .map_err(|error| error.to_string())
                .and_then(|ack| {
                    let provider = canonical_provider(&ack.provider)?;
                    control.acknowledge_prompt(
                        &format!("web:{provider}:{}", ack.session_id),
                        &ack.prompt_id,
                        ack.submitted,
                    )
                });
            if acknowledged.is_ok() {
                ("200 OK", "{\"ok\":true}".into(), request.origin)
            } else {
                ("400 Bad Request", "{\"ok\":false}".into(), request.origin)
            }
        }
        Ok(request) => ("403 Forbidden", "{\"ok\":false}".into(), request.origin),
        Err(_) => ("400 Bad Request", "{\"ok\":false}".into(), String::new()),
    };
    let cors = if allowed_origin(&origin) {
        format!("Access-Control-Allow-Origin: {origin}\r\n")
    } else {
        String::new()
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{cors}Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

struct HttpRequest {
    method: String,
    path: String,
    origin: String,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> Result<HttpRequest, String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| error.to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("Método ausente")?.to_string();
    let path = parts.next().ok_or("Caminho ausente")?.to_string();
    let mut content_length = 0usize;
    let mut origin = String::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().map_err(|_| "Tamanho inválido")?;
        }
        if let Some(value) = line.strip_prefix("Origin:") {
            origin = value.trim().to_string();
        }
    }
    if content_length > MAX_BODY_BYTES {
        return Err("Evento web excede o limite".into());
    }
    let mut body = vec![0; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    Ok(HttpRequest {
        method,
        path,
        origin,
        body,
    })
}

fn allowed_origin(origin: &str) -> bool {
    origin.starts_with("chrome-extension://") || origin.starts_with("extension://")
}

fn canonical_provider(provider: &str) -> Result<&'static str, String> {
    match provider {
        "codex" | "chatgpt" => Ok("chatgpt"),
        "claude" => Ok("claude"),
        "gemini" => Ok("gemini"),
        _ => Err("Agente web desconhecido".into()),
    }
}

fn map_event(event: BrowserEvent) -> Result<HookEvent, String> {
    if event.session_id.len() > 180 || event.origin.len() > 180 {
        return Err("Evento web inválido".into());
    }
    let provider = canonical_provider(&event.provider)?;
    let (agent, label) = match provider {
        "chatgpt" => (AgentKind::ChatGpt, "ChatGPT"),
        "claude" => (AgentKind::Claude, "Claude"),
        "gemini" => (AgentKind::Gemini, "Gemini"),
        _ => unreachable!(),
    };
    let now = now_millis();
    let source_app = match event.browser.as_deref() {
        Some("chrome" | "edge" | "brave") => event.browser.clone(),
        _ => None,
    };
    let (kind, status_label, permission) = match event.state {
        BrowserState::Running => (HookEventKind::Running, "Executando", None),
        BrowserState::WaitingForInput => (
            HookEventKind::WaitingForInput,
            "Aguardando sua resposta",
            None,
        ),
        BrowserState::Completed => (HookEventKind::Completed, "Finalizado", None),
        BrowserState::Failed => (HookEventKind::Failed, "Erro na página", None),
        BrowserState::Closed => (HookEventKind::SessionEnded, "Sessão fechada", None),
        BrowserState::PermissionRequired => (
            HookEventKind::PermissionRequest,
            "Aguardando confirmação na página",
            Some(PermissionRequest {
                id: format!("web:{}:{now}", event.session_id),
                kind: "tool".into(),
                summary: "A página está aguardando uma confirmação".into(),
                resource: event.origin.clone(),
                risk: "medium".into(),
                requested_at: now.to_string(),
            }),
        ),
    };
    Ok(HookEvent {
        event: kind,
        session_id: format!("web:{provider}:{}", event.session_id),
        agent,
        agent_label: Some(label.into()),
        session_name: Some(truncate(&event.title, 100)),
        project: Some(truncate(&event.origin, 100)),
        source: Some(SessionSource::Web),
        source_app,
        status_label: Some(status_label.into()),
        started_at: None,
        process_id: None,
        native_session_id: Some(event.session_id),
        working_directory: None,
        permission_profile: Some(PermissionProfile {
            mode: AccessMode::Custom,
            label: "Sessão web".into(),
            approval_policy: "Ações permanecem na página original".into(),
            approvals_reviewer: None,
            can_respond_from_lume: false,
            available_actions: vec![PermissionAction::OpenSource],
        }),
        permission,
        question: None,
        last_response: event
            .last_response
            .as_deref()
            .map(str::trim)
            .filter(|response| !response.is_empty())
            .map(|response| truncate(response, 32 * 1024)),
        activity: None,
        activities: Vec::new(),
        wait_for_decision: false,
    })
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_endpoint_rejects_normal_web_pages() {
        assert!(allowed_origin("chrome-extension://local-companion"));
        assert!(allowed_origin("extension://local-companion"));
        assert!(!allowed_origin("https://chatgpt.com"));
        assert!(!allowed_origin("null"));
    }

    #[test]
    fn browser_event_keeps_only_origin_and_hashed_session() {
        let event = map_event(BrowserEvent {
            provider: "chatgpt".into(),
            session_id: "hash-only".into(),
            title: "Projeto".into(),
            origin: "https://chatgpt.com".into(),
            browser: Some("brave".into()),
            protocol_version: 2,
            state: BrowserState::PermissionRequired,
            last_response: None,
        })
        .expect("evento web");
        assert_eq!(event.session_id, "web:chatgpt:hash-only");
        assert_eq!(event.agent, AgentKind::ChatGpt);
        assert_eq!(event.agent_label.as_deref(), Some("ChatGPT"));
        assert_eq!(event.source_app.as_deref(), Some("brave"));
        assert_eq!(
            event.permission.expect("permissão").resource,
            "https://chatgpt.com"
        );
    }

    #[test]
    fn legacy_codex_browser_provider_is_canonicalized_as_chatgpt() {
        let event = map_event(BrowserEvent {
            provider: "codex".into(),
            session_id: "legacy-extension".into(),
            title: "Legacy ChatGPT tab".into(),
            origin: "https://chatgpt.com".into(),
            browser: Some("chrome".into()),
            protocol_version: 0,
            state: BrowserState::WaitingForInput,
            last_response: None,
        })
        .expect("legacy browser event");

        assert_eq!(event.session_id, "web:chatgpt:legacy-extension");
        assert_eq!(event.agent, AgentKind::ChatGpt);
        assert_eq!(event.agent_label.as_deref(), Some("ChatGPT"));
    }

    #[test]
    fn claude_web_is_not_claude_code() {
        let event = map_event(BrowserEvent {
            provider: "claude".into(),
            session_id: "claude-tab".into(),
            title: "Claude".into(),
            origin: "https://claude.ai".into(),
            browser: Some("edge".into()),
            protocol_version: 2,
            state: BrowserState::Running,
            last_response: None,
        })
        .expect("claude browser event");

        assert_eq!(event.agent, AgentKind::Claude);
        assert_eq!(event.agent_label.as_deref(), Some("Claude"));
    }

    #[test]
    fn browser_prompt_waits_for_a_successful_extension_acknowledgement() {
        let control = BrowserControl::default();
        control
            .request_prompt("web:chatgpt:hash-only".into(), "Continue".into())
            .expect("fila local");
        let request = control
            .pending_prompt("web:chatgpt:hash-only")
            .expect("prompt pendente");
        assert_eq!(request.prompt, "Continue");

        control
            .acknowledge_prompt("web:chatgpt:hash-only", &request.id, false)
            .expect("falha mantida");
        assert!(control.pending_prompt("web:chatgpt:hash-only").is_some());

        control
            .acknowledge_prompt("web:chatgpt:hash-only", &request.id, true)
            .expect("entrega confirmada");
        assert!(control.pending_prompt("web:chatgpt:hash-only").is_none());
    }

    #[test]
    fn legacy_browser_prompt_is_consumed_once() {
        let control = BrowserControl::default();
        control
            .request_prompt("web:chatgpt:legacy".into(), "Continue".into())
            .expect("legacy prompt");

        assert_eq!(
            control
                .take_prompt("web:chatgpt:legacy")
                .expect("first delivery")
                .prompt,
            "Continue"
        );
        assert!(control.take_prompt("web:chatgpt:legacy").is_none());
    }

    #[test]
    fn closed_browser_tab_becomes_a_session_end_event() {
        let event = map_event(BrowserEvent {
            provider: "chatgpt".into(),
            session_id: "closed-tab".into(),
            title: "Projeto".into(),
            origin: "https://chatgpt.com".into(),
            browser: Some("chrome".into()),
            protocol_version: 2,
            state: BrowserState::Closed,
            last_response: None,
        })
        .expect("aba fechada");

        assert!(matches!(event.event, HookEventKind::SessionEnded));
    }
}
