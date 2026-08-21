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
        let automatically_approved = permission_is_automatically_approved(&state, &event)?;
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

fn permission_is_automatically_approved(
    state: &AppState,
    event: &HookEvent,
) -> Result<bool, String> {
    if !matches!(event.event, HookEventKind::PermissionRequest) {
        return Ok(false);
    }
    if event
        .permission_profile
        .as_ref()
        .is_some_and(|profile| profile.automatically_approves())
    {
        return Ok(true);
    }

    state.session_automatically_approves(&event.session_id, event.native_session_id.as_deref())
}

pub fn publish_event(
    state: &AppState,
    app: &AppHandle,
    event: HookEvent,
) -> Result<Option<String>, String> {
    let session_id = event.session_id.clone();
    let native_session_id = event.native_session_id.clone();
    let previous_status = state.session_status(&session_id, native_session_id.as_deref())?;
    let notification = notification_for(&event, previous_status.as_ref());
    let permission_id = state.ingest(event)?;
    crate::protocol::emit_session_changed(app, &session_id, native_session_id.as_deref());
    if state.preferences()?.popup_notifications_enabled {
        if let Some((title, body)) = notification {
            let _ = app.notification().builder().title(title).body(body).show();
        }
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
            crate::domain::AgentKind::Antigravity => "Antigravity".into(),
            crate::domain::AgentKind::DeepSeek => "DeepSeek".into(),
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::domain::{
        AccessMode, AgentKind, PermissionProfile, PermissionRequest, SessionSource,
    };

    fn event(kind: HookEventKind) -> HookEvent {
        HookEvent {
            event: kind,
            session_id: "codex:thread-1".into(),
            agent: AgentKind::Codex,
            agent_label: Some("Codex".into()),
            session_name: None,
            project: Some("Lume".into()),
            source: Some(SessionSource::Cli),
            source_app: None,
            control_origin: crate::domain::SessionControlOrigin::External,
            status_label: None,
            started_at: None,
            process_id: Some(4242),
            native_session_id: Some("thread-1".into()),
            working_directory: Some("/work/lume".into()),
            permission_profile: None,
            permission: None,
            question: None,
            last_response: None,
            activity: None,
            activities: Vec::new(),
            wait_for_decision: false,
        }
    }

    #[test]
    fn permission_uses_the_stored_automatic_profile_when_the_hook_omits_it() {
        let state = AppState::new(Path::new(":memory:")).expect("state");
        let mut started = event(HookEventKind::SessionStarted);
        started.permission_profile = Some(PermissionProfile {
            mode: AccessMode::WorkspaceWrite,
            label: "Approve for me".into(),
            approval_policy: "on-request".into(),
            approvals_reviewer: Some("auto_review".into()),
            can_respond_from_lume: false,
            available_actions: vec![PermissionAction::OpenSource],
        });
        state.ingest(started).expect("stored profile");

        let mut permission = event(HookEventKind::PermissionRequest);
        permission.permission = Some(PermissionRequest {
            id: "permission-1".into(),
            kind: "command".into(),
            summary: "Run command".into(),
            resource: "cargo test".into(),
            risk: "medium".into(),
            requested_at: "1".into(),
        });

        assert!(
            permission_is_automatically_approved(&state, &permission).expect("automatic decision")
        );
    }
}
