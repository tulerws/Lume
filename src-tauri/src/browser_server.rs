use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

use serde::Deserialize;
use tauri::{AppHandle, Emitter};
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserEvent {
    provider: String,
    session_id: String,
    title: String,
    origin: String,
    browser: Option<String>,
    state: BrowserState,
    #[serde(default)]
    last_response: Option<String>,
}

#[derive(Clone, Default)]
pub struct BrowserControl {
    focus_requests: Arc<Mutex<HashSet<String>>>,
    prompt_requests: Arc<Mutex<HashMap<String, String>>>,
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
        self.prompt_requests
            .lock()
            .map_err(|_| "Não foi possível acessar o conector web".to_string())?
            .insert(session_id, prompt);
        Ok(())
    }

    fn take_focus(&self, session_id: &str) -> bool {
        self.focus_requests
            .lock()
            .map(|mut requests| requests.remove(session_id))
            .unwrap_or(false)
    }

    fn take_prompt(&self, session_id: &str) -> Option<String> {
        self.prompt_requests
            .lock()
            .ok()
            .and_then(|mut requests| requests.remove(session_id))
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
                    let session_id = format!(
                        "web:{}:{}",
                        browser_event.provider, browser_event.session_id
                    );
                    let previous_status = state
                        .sessions()?
                        .into_iter()
                        .find(|session| session.id == session_id)
                        .map(|session| session.status);
                    let focus = control.take_focus(&session_id);
                    let prompt = control.take_prompt(&session_id);
                    let event = map_event(browser_event)?;
                    let notification =
                        crate::domain::should_notify(&event.event, previous_status.as_ref());
                    let label = event.agent_label.clone().unwrap_or_else(|| "Agente".into());
                    let project = event.project.clone().unwrap_or_else(|| "sessão web".into());
                    let event_kind = event.event.clone();
                    state.ingest(event)?;
                    let _ = app.emit("lume://sessions-changed", ());
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
                    Ok((focus, prompt))
                }) {
                Ok((focus, prompt)) => (
                    "202 Accepted",
                    serde_json::json!({ "ok": true, "focus": focus, "prompt": prompt }).to_string(),
                    request.origin,
                ),
                Err(_) => ("400 Bad Request", "{\"ok\":false}".into(), request.origin),
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

fn map_event(event: BrowserEvent) -> Result<HookEvent, String> {
    if event.session_id.len() > 180 || event.origin.len() > 180 {
        return Err("Evento web inválido".into());
    }
    let (agent, label) = match event.provider.as_str() {
        "codex" => (AgentKind::Codex, "Codex"),
        "claude" => (AgentKind::Claude, "Claude"),
        "gemini" => (AgentKind::Gemini, "Gemini"),
        _ => return Err("Agente web desconhecido".into()),
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
        session_id: format!("web:{}:{}", event.provider, event.session_id),
        agent,
        agent_label: Some(label.into()),
        project: Some(truncate(&event.title, 100)),
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
            can_respond_from_lume: false,
            available_actions: vec![PermissionAction::OpenSource],
        }),
        permission,
        last_response: event
            .last_response
            .as_deref()
            .map(str::trim)
            .filter(|response| !response.is_empty())
            .map(|response| truncate(response, 32 * 1024)),
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
            provider: "codex".into(),
            session_id: "hash-only".into(),
            title: "Projeto".into(),
            origin: "https://chatgpt.com".into(),
            browser: Some("brave".into()),
            state: BrowserState::PermissionRequired,
            last_response: None,
        })
        .expect("evento web");
        assert_eq!(event.session_id, "web:codex:hash-only");
        assert_eq!(event.source_app.as_deref(), Some("brave"));
        assert_eq!(
            event.permission.expect("permissão").resource,
            "https://chatgpt.com"
        );
    }

    #[test]
    fn browser_prompt_is_kept_only_until_the_next_extension_poll() {
        let control = BrowserControl::default();
        control
            .request_prompt("web:codex:hash-only".into(), "Continue".into())
            .expect("fila local");
        assert_eq!(
            control.take_prompt("web:codex:hash-only").as_deref(),
            Some("Continue")
        );
        assert!(control.take_prompt("web:codex:hash-only").is_none());
    }
}
