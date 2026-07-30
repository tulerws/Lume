use std::{
    io::{BufRead, BufReader, Write},
    net::{Shutdown, TcpListener, TcpStream},
    thread,
    time::Duration,
};

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::{
    domain::{HookEvent, HookEventKind, HookResponse, PermissionAction},
    state::AppState,
};

pub const EVENT_SERVER_ADDRESS: &str = "127.0.0.1:43119";

pub fn start(state: AppState, app: AppHandle) -> Result<(), String> {
    let listener = TcpListener::bind(EVENT_SERVER_ADDRESS)
        .map_err(|error| format!("Não foi possível iniciar a entrada local de eventos: {error}"))?;
    thread::Builder::new()
        .name("lume-event-server".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = state.clone();
                let app = app.clone();
                let _ = thread::Builder::new()
                    .name("lume-event-client".into())
                    .spawn(move || handle_connection(stream, state, app));
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn handle_connection(mut stream: TcpStream, state: AppState, app: AppHandle) {
    let response = read_event(&stream).and_then(|mut event| {
        let automatically_approved = matches!(event.event, HookEventKind::PermissionRequest)
            && event
                .permission_profile
                .as_ref()
                .is_some_and(|profile| profile.automatically_approves());
        let wait_for_decision = event.wait_for_decision;
        let question_id = event.question.as_ref().map(|question| question.id.clone());
        if automatically_approved {
            event.event = HookEventKind::Running;
            event.status_label = Some("Executando".into());
            event.permission = None;
            event.wait_for_decision = false;
        }
        let permission_id = publish_event(&state, &app, event)?;

        if automatically_approved {
            return Ok(HookResponse {
                ok: true,
                action: wait_for_decision.then_some(PermissionAction::AllowOnce),
                question_answers: None,
                message: None,
            });
        }
        if wait_for_decision {
            if let Some(question_id) = question_id {
                let answers =
                    state.wait_for_question_answer(&question_id, Duration::from_secs(15 * 60))?;
                if answers.is_none() {
                    state.expire_question(&question_id)?;
                    crate::protocol::emit_sessions_changed(&app);
                }
                return Ok(HookResponse {
                    ok: answers.is_some(),
                    action: None,
                    question_answers: answers,
                    message: None,
                });
            }
            let permission_id = permission_id.ok_or_else(|| {
                "O evento aguardava uma decisão, mas não continha permissão".to_string()
            })?;
            let action = state.wait_for_decision(&permission_id, Duration::from_secs(15 * 60))?;
            return Ok(HookResponse {
                ok: action.is_some(),
                action,
                question_answers: None,
                message: None,
            });
        }

        Ok(HookResponse {
            ok: true,
            action: None,
            question_answers: None,
            message: None,
        })
    });

    let response = response.unwrap_or_else(|message| HookResponse {
        ok: false,
        action: None,
        question_answers: None,
        message: Some(message),
    });
    if let Ok(payload) = serde_json::to_string(&response) {
        let _ = writeln!(stream, "{payload}");
    }
}

pub fn publish_event(
    state: &AppState,
    app: &AppHandle,
    event: HookEvent,
) -> Result<Option<String>, String> {
    let previous_status = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == event.session_id)
        .map(|session| session.status);
    let notification = notification_for(&event, previous_status.as_ref());
    let permission_id = state.ingest(event)?;
    crate::protocol::emit_sessions_changed(app);
    if let Some((title, body)) = notification {
        let _ = app.notification().builder().title(title).body(body).show();
    }
    Ok(permission_id)
}

fn notification_for(
    event: &HookEvent,
    previous_status: Option<&crate::domain::SessionStatus>,
) -> Option<(String, String)> {
    if !crate::domain::should_notify(&event.event, previous_status) {
        return None;
    }
    let agent = event
        .agent_label
        .clone()
        .unwrap_or_else(|| match event.agent {
            crate::domain::AgentKind::Codex => "Codex".into(),
            crate::domain::AgentKind::ChatGpt => "ChatGPT".into(),
            crate::domain::AgentKind::Claude => "Claude".into(),
            crate::domain::AgentKind::ClaudeCode => "Claude Code".into(),
            crate::domain::AgentKind::Gemini => "Gemini".into(),
            crate::domain::AgentKind::Unknown => "Agente".into(),
        });
    let project = event.project.as_deref().unwrap_or("sessão local");
    let title = match event.event {
        crate::domain::HookEventKind::PermissionRequest => "Lume · Permissão necessária",
        crate::domain::HookEventKind::QuestionRequest => "Lume · Resposta necessária",
        crate::domain::HookEventKind::Completed => "Lume · Tarefa finalizada",
        crate::domain::HookEventKind::Failed => "Lume · Erro na sessão",
        _ => return None,
    };
    Some((title.into(), format!("{agent} · {project}")))
}

fn read_event(stream: &TcpStream) -> Result<HookEvent, String> {
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&line).map_err(|error| format!("Evento local inválido: {error}"))
}

pub fn send_event(event_json: &str) -> Result<HookResponse, String> {
    let _: HookEvent = serde_json::from_str(event_json)
        .map_err(|error| format!("Evento local inválido: {error}"))?;
    let mut stream = TcpStream::connect_timeout(
        &EVENT_SERVER_ADDRESS
            .parse()
            .map_err(|error| format!("Endereço local inválido: {error}"))?,
        Duration::from_secs(2),
    )
    .map_err(|_| "O Lume não está em execução".to_string())?;
    stream
        .write_all(event_json.trim().as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|error| error.to_string())?;
    let _ = stream.shutdown(Shutdown::Write);

    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&response).map_err(|error| error.to_string())
}
