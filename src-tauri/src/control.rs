use std::{fs, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use tauri::{AppHandle, Manager};

use crate::{
    browser_server::BrowserControl,
    codex_bridge::CodexBridge,
    discovery,
    domain::{
        AgentKind, PermissionAction, PromptAttachment, PromptAttachmentInput, SessionSource,
        SessionStatus,
    },
    integrations::{self, IntegrationKind},
    launcher::{self, LaunchRequest},
    protocol,
    state::AppState,
};

const MAX_PROMPT_ATTACHMENTS: usize = 4;
const MAX_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;
const MAX_PREVIEW_LENGTH: usize = 384 * 1024;

struct PreparedPromptAttachment {
    path: String,
    display: PromptAttachment,
}

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
    attachments: Vec<PromptAttachmentInput>,
    allow_local_paths: bool,
) -> Result<(), String> {
    let prompt = prompt.trim();
    if prompt.is_empty() && attachments.is_empty() {
        return Err("Digite um prompt ou anexe uma imagem antes de enviar".into());
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
    let attachments = prepare_prompt_attachments(app, attachments, allow_local_paths)?;
    let attachment_paths = attachments
        .iter()
        .map(|attachment| attachment.path.clone())
        .collect::<Vec<_>>();
    let result = if session.source == SessionSource::Web {
        if !attachments.is_empty() {
            return Err("Esta origem web ainda não aceita imagens pelo Lume".into());
        }
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
        bridge.submit_prompt(
            &thread_id,
            prompt,
            &attachment_paths,
            profile,
            state.clone(),
            app.clone(),
        )
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
        let prompt = prompt_with_attachment_paths(prompt, &attachment_paths);
        launcher::launch(
            LaunchRequest {
                agent,
                working_directory,
                resume: true,
                resume_id: Some(resume_id),
                target,
                initial_prompt: Some(prompt),
                permission_mode: None,
                approval_policy: None,
            },
            &executable,
            &app_data_dir,
            None,
        )
    };
    result?;
    state.record_prompt_activity(
        &session.id,
        prompt,
        attachments
            .into_iter()
            .map(|attachment| attachment.display)
            .collect(),
    )?;
    protocol::emit_sessions_changed(app);
    Ok(())
}

fn prepare_prompt_attachments(
    app: &AppHandle,
    attachments: Vec<PromptAttachmentInput>,
    allow_local_paths: bool,
) -> Result<Vec<PreparedPromptAttachment>, String> {
    if attachments.len() > MAX_PROMPT_ATTACHMENTS {
        return Err(format!(
            "Envie no máximo {MAX_PROMPT_ATTACHMENTS} imagens por prompt"
        ));
    }
    let mut prepared = Vec::with_capacity(attachments.len());
    for (index, attachment) in attachments.into_iter().enumerate() {
        let preview = attachment.preview_data_url.unwrap_or_default();
        if preview.len() > MAX_PREVIEW_LENGTH
            || (!preview.is_empty() && !preview.starts_with("data:image/"))
        {
            return Err("A prévia da imagem é inválida ou muito grande".into());
        }
        let (path, detected_mime) = if let Some(path) = attachment.path {
            if !allow_local_paths {
                return Err("O celular não pode indicar caminhos locais do computador".into());
            }
            let path = fs::canonicalize(path)
                .map_err(|_| "A imagem selecionada não existe".to_string())?;
            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            if bytes.len() > MAX_ATTACHMENT_BYTES {
                return Err("A imagem excede o limite de 5 MB".into());
            }
            let mime = detected_image_mime(&bytes)
                .ok_or_else(|| "O arquivo selecionado não é uma imagem compatível".to_string())?;
            (path.to_string_lossy().to_string(), mime)
        } else if let Some(data) = attachment.data_base64 {
            let bytes = STANDARD
                .decode(data)
                .map_err(|_| "Não foi possível decodificar a imagem".to_string())?;
            if bytes.len() > MAX_ATTACHMENT_BYTES {
                return Err("A imagem excede o limite de 5 MB".into());
            }
            let mime = detected_image_mime(&bytes)
                .ok_or_else(|| "O anexo não é uma imagem compatível".to_string())?;
            let directory = app
                .path()
                .app_cache_dir()
                .map_err(|error| error.to_string())?
                .join("prompt-attachments");
            fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let extension = extension_for_mime(mime);
            let path = directory.join(format!(
                "{}-{index}.{extension}",
                crate::state::now_millis()
            ));
            fs::write(&path, bytes).map_err(|error| error.to_string())?;
            (path.to_string_lossy().to_string(), mime)
        } else {
            return Err("O anexo não contém uma imagem".into());
        };
        if !attachment.mime_type.is_empty() && attachment.mime_type != detected_mime {
            return Err("O tipo informado não corresponde ao conteúdo da imagem".into());
        }
        let name = safe_attachment_name(&attachment.name, index, detected_mime);
        prepared.push(PreparedPromptAttachment {
            path,
            display: PromptAttachment {
                id: format!("attachment:{}:{index}", crate::state::now_millis()),
                name,
                mime_type: detected_mime.into(),
                preview_data_url: preview,
            },
        });
    }
    Ok(prepared)
}

fn prompt_with_attachment_paths(prompt: &str, paths: &[String]) -> String {
    if paths.is_empty() {
        return prompt.to_string();
    }
    let mut value = prompt.to_string();
    if !value.is_empty() {
        value.push_str("\n\n");
    }
    value.push_str("Images attached through Lume. Inspect these local files:\n");
    for path in paths {
        value.push_str("- ");
        value.push_str(path);
        value.push('\n');
    }
    value
}

fn detected_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

fn safe_attachment_name(name: &str, index: usize, mime: &str) -> String {
    let name = Path::new(name.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    name.unwrap_or_else(|| format!("image-{}.{}", index + 1, extension_for_mime(mime)))
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
        protocol::HubCommand::SubmitPrompt {
            session_id,
            prompt,
            attachments,
        } => {
            submit_prompt(
                app,
                state,
                bridge,
                browser,
                &session_id,
                &prompt,
                attachments,
                false,
            )
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
        protocol::HubCommand::RefreshRateLimits { agent } => {
            if agent == AgentKind::Codex {
                bridge.refresh_rate_limits(state, app)
            } else {
                Ok(())
            }
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
