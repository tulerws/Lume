use tauri::{AppHandle, Manager};

use crate::{
    browser_server::BrowserControl,
    codex_bridge::CodexBridge,
    discovery,
    domain::{AgentKind, PermissionAction, SessionSource, SessionStatus},
    integrations::{self, IntegrationKind},
    launcher::{self, LaunchRequest},
    protocol,
    state::AppState,
};

pub fn resolve_permission(
    state: &AppState,
    session_id: &str,
    permission_id: &str,
    action: PermissionAction,
) -> Result<(), String> {
    state.resolve_permission(session_id, permission_id, action)
}

pub fn open_session_source(
    state: &AppState,
    browser: &BrowserControl,
    session_id: &str,
) -> Result<(), String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Sessão não encontrada".to_string())?;
    match session.source {
        SessionSource::Web => browser.request_focus(session.id),
        SessionSource::Vscode => {
            let directory = session
                .working_directory
                .ok_or_else(|| "A sessão não informou a pasta do projeto".to_string())?;
            integrations::code_command()
                .args(["--reuse-window", &directory])
                .spawn()
                .map_err(|error| format!("Não foi possível abrir o VS Code: {error}"))?;
            Ok(())
        }
        _ => Err("O sistema não permite focar com segurança esta janela de terminal".into()),
    }
}

pub fn submit_prompt(
    app: &AppHandle,
    state: &AppState,
    bridge: &CodexBridge,
    browser: &BrowserControl,
    session_id: &str,
    prompt: &str,
) -> Result<(), String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Digite um prompt antes de enviar".into());
    }
    if prompt.len() > 16 * 1024 {
        return Err("O prompt excede o limite local de 16 KB".into());
    }
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Sessão não encontrada".to_string())?;
    if matches!(
        session.status,
        SessionStatus::Running | SessionStatus::PermissionRequired
    ) {
        return Err("Aguarde o agente terminar antes de enviar outro prompt".into());
    }
    let result = if session.source == SessionSource::Web {
        browser.request_prompt(session.id.clone(), prompt.to_string())?;
        browser.request_focus(session.id.clone())
    } else if session.agent == AgentKind::Codex {
        let mut profile = session.permission_profile.clone();
        profile.can_respond_from_lume = true;
        profile.available_actions = vec![
            PermissionAction::AllowOnce,
            PermissionAction::AllowSession,
            PermissionAction::Deny,
        ];
        let thread_id = session
            .native_session_id
            .clone()
            .ok_or_else(|| "A sessão do Codex não informou a thread".to_string())?;
        bridge.submit_prompt(&thread_id, prompt, profile, state.clone(), app.clone())
    } else {
        let agent = match session.agent {
            AgentKind::Claude => IntegrationKind::Claude,
            AgentKind::Gemini => IntegrationKind::Gemini,
            AgentKind::Codex => unreachable!(),
            AgentKind::Unknown => {
                return Err("Este agente não oferece retomada direta pelo Lume".into());
            }
        };
        let resume_id = session
            .native_session_id
            .clone()
            .ok_or_else(|| "A sessão não informou um identificador para retomada".to_string())?;
        let working_directory = session
            .working_directory
            .clone()
            .ok_or_else(|| "A sessão não informou a pasta do projeto".to_string())?;
        let preferences = state.preferences()?;
        let target = if session.source == SessionSource::Vscode {
            "vscode".to_string()
        } else {
            preferences.launch_target
        };
        let executable = integrations::lume_executable()?;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?;
        launcher::launch(
            LaunchRequest {
                agent,
                working_directory,
                resume: true,
                resume_id: Some(resume_id),
                target,
                initial_prompt: Some(prompt.to_string()),
                permission_mode: None,
                approval_policy: None,
            },
            &executable,
            &app_data_dir,
            None,
        )
    };
    result?;
    state.record_activity(
        &session.id,
        "prompt",
        "Prompt enviado pelo Lume",
        Some(prompt.to_string()),
        "completed",
        Vec::new(),
    )?;
    protocol::emit_sessions_changed(app);
    Ok(())
}

pub fn terminate_session(
    app: &AppHandle,
    state: &AppState,
    session_id: &str,
) -> Result<(), String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Sessão não encontrada".to_string())?;
    if session.source != SessionSource::Cli {
        return Err(
            "Esta integração não possui um processo isolado; o Lume não fechará o editor ou navegador inteiro"
                .into(),
        );
    }
    let process_id = session
        .process_id
        .ok_or_else(|| "A sessão não possui um processo associado".to_string())?;
    discovery::terminate_agent_process(process_id, &session.agent)?;
    state.mark_process_terminated(process_id)?;
    protocol::emit_sessions_changed(app);
    Ok(())
}

pub fn execute_hub_command(
    app: &AppHandle,
    state: &AppState,
    bridge: &CodexBridge,
    browser: &BrowserControl,
    request: protocol::HubCommandRequest,
) -> protocol::HubCommandResponse {
    let request_id = request.request_id.clone();
    if let Err(error) = request.validate() {
        return protocol::HubCommandResponse::failure(request_id, error);
    }
    let result = match request.command {
        protocol::HubCommand::SubmitPrompt { session_id, prompt } => {
            submit_prompt(app, state, bridge, browser, &session_id, &prompt)
        }
        protocol::HubCommand::ResolvePermission {
            session_id,
            permission_id,
            action,
        } => resolve_permission(state, &session_id, &permission_id, action),
        protocol::HubCommand::TerminateSession { session_id } => {
            terminate_session(app, state, &session_id)
        }
        protocol::HubCommand::OpenSessionSource { session_id } => {
            open_session_source(state, browser, &session_id)
        }
    };
    match result {
        Ok(()) => protocol::HubCommandResponse::success(request_id),
        Err(message) => protocol::HubCommandResponse::failure(
            request_id,
            protocol::ProtocolError::from_control(message),
        ),
    }
}
