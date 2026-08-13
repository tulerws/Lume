use std::{fs, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    browser_server::BrowserControl,
    codex_bridge::CodexBridge,
    discovery,
    domain::{
        AgentKind, PendingQuestion, PermissionAction, PromptAttachment, PromptAttachmentInput,
        PromptDelivery, QuestionAnswer, SessionSource, SessionStatus,
    },
    integrations::{self, IntegrationKind},
    launcher::{self, LaunchRequest},
    protocol,
    state::AppState,
};

const MAX_PROMPT_ATTACHMENTS: usize = 4;
const MAX_IMAGE_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;
const MAX_FILE_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
const MAX_PREVIEW_LENGTH: usize = 384 * 1024;

struct PreparedPromptAttachment {
    path: String,
    is_image: bool,
    display: PromptAttachment,
}

pub fn local_image_data_url(path: &str) -> Result<String, String> {
    let path = fs::canonicalize(path).map_err(|_| "A imagem selecionada não existe".to_string())?;
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_IMAGE_ATTACHMENT_BYTES {
        return Err("A imagem excede o limite de 5 MB".into());
    }
    let mime = detected_image_mime(&bytes)
        .ok_or_else(|| "O arquivo selecionado não é uma imagem compatível".to_string())?;
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

pub fn response_file_payload(
    state: &AppState,
    session_id: &str,
    attachment_id: &str,
) -> Result<serde_json::Value, String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    let attachment = session
        .activities
        .iter()
        .filter(|activity| activity.kind == "message")
        .flat_map(|activity| activity.attachments.iter())
        .find(|attachment| attachment.id == attachment_id)
        .ok_or_else(|| "The response file is no longer available".to_string())?;
    let path = attachment
        .path
        .as_deref()
        .ok_or_else(|| "The response file has no local path".to_string())?;
    let path =
        fs::canonicalize(path).map_err(|_| "The response file no longer exists".to_string())?;
    if !path.is_file() {
        return Err("The response attachment is not a file".into());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_FILE_ATTACHMENT_BYTES {
        return Err("The response file exceeds the 25 MB download limit".into());
    }
    Ok(serde_json::json!({
        "attachmentId": attachment.id,
        "name": attachment.name,
        "mimeType": attachment.mime_type,
        "dataBase64": STANDARD.encode(bytes),
    }))
}

pub fn resolve_permission(
    state: &AppState,
    session_id: &str,
    permission_id: &str,
    action: PermissionAction,
) -> Result<(), String> {
    state.resolve_permission(session_id, permission_id, action)
}

pub fn resolve_question(
    state: &AppState,
    session_id: &str,
    question_id: &str,
    answers: Vec<QuestionAnswer>,
) -> Result<(), String> {
    state.resolve_question(session_id, question_id, answers)
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
    delivery: PromptDelivery,
    allow_local_paths: bool,
) -> Result<(), String> {
    let prompt = prompt.trim();
    if prompt.is_empty() && attachments.is_empty() {
        return Err("Digite um prompt ou anexe um arquivo antes de enviar".into());
    }
    if prompt.len() > 16 * 1024 {
        return Err("O prompt excede o limite local de 16 KB".into());
    }
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Sessão não encontrada".to_string())?;
    if let Some(question) = session.pending_question.as_ref() {
        if !attachments.is_empty() {
            return Err("Responda à pergunta antes de anexar um arquivo".into());
        }
        let answers = question_answers_from_prompt(question, prompt)?;
        state.resolve_question(session_id, &question.id, answers)?;
        protocol::emit_sessions_changed(app);
        return Ok(());
    }
    let is_running = matches!(
        session.status,
        SessionStatus::Running | SessionStatus::PermissionRequired
    );
    let attachments = prepare_prompt_attachments(app, attachments, allow_local_paths)?;
    let image_paths = attachments
        .iter()
        .filter(|attachment| attachment.is_image)
        .map(|attachment| attachment.path.clone())
        .collect::<Vec<_>>();
    let file_paths = attachments
        .iter()
        .filter(|attachment| !attachment.is_image)
        .map(|attachment| attachment.path.clone())
        .collect::<Vec<_>>();
    let codex_prompt = prompt_with_attachment_paths(prompt, &file_paths);
    let display_attachments = attachments
        .iter()
        .map(|attachment| attachment.display.clone())
        .collect::<Vec<_>>();
    let queued_for_later = is_running && delivery == PromptDelivery::Queue;
    let result = if is_running {
        if session.agent != AgentKind::Codex {
            return Err(
                "This running agent cannot receive queued or side prompts through Lume yet".into(),
            );
        }
        let thread_id = session
            .native_session_id
            .clone()
            .ok_or_else(|| "The Codex session did not provide its thread id".to_string())?;
        match delivery {
            PromptDelivery::Steer => {
                bridge.steer_prompt(&thread_id, &codex_prompt, &image_paths, state, app)
            }
            PromptDelivery::Queue => {
                let mut profile = session.permission_profile.clone();
                profile.can_respond_from_lume = true;
                let activity_id =
                    format!("local:{}:queued:{}", session.id, crate::state::now_millis());
                bridge.queue_prompt(
                    &session.id,
                    &activity_id,
                    &thread_id,
                    &codex_prompt,
                    &image_paths,
                    profile,
                )?;
                state.record_queued_prompt_activity(
                    &session.id,
                    &activity_id,
                    prompt,
                    display_attachments.clone(),
                )
            }
            PromptDelivery::NewTurn => {
                return Err("Choose Steer now or Queue next while Codex is running".into());
            }
        }
    } else if session.source == SessionSource::Web {
        if !attachments.is_empty() {
            return Err("Esta origem web ainda não aceita arquivos pelo Lume".into());
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
        let first_attempt = bridge.submit_prompt(
            &thread_id,
            &codex_prompt,
            &image_paths,
            profile.clone(),
            state.clone(),
            app.clone(),
        );
        if let Err(error) = first_attempt {
            if is_active_codex_writer(&error) {
                return Err(
                    "Essa CLI foi aberta fora do Lume. Reinicie a sessão pelo Lume para conseguir enviar mensagens pelo terminal."
                        .into(),
                );
            }
            if !is_missing_codex_rollout(&error) {
                return Err(error);
            }
            let working_directory = session.working_directory.as_deref().ok_or_else(|| {
                "The Codex session has no project directory to reconnect".to_string()
            })?;
            bridge.recover_thread_and_submit_prompt(
                &session.id,
                working_directory,
                &codex_prompt,
                &image_paths,
                profile,
                state.clone(),
                app.clone(),
            )
        } else {
            Ok(())
        }
    } else {
        let agent = match session.agent {
            AgentKind::ClaudeCode => IntegrationKind::Claude,
            AgentKind::Gemini => IntegrationKind::Gemini,
            AgentKind::Codex => unreachable!(),
            AgentKind::ChatGpt | AgentKind::Claude | AgentKind::Unknown => {
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
        let attachment_paths = attachments
            .iter()
            .map(|attachment| attachment.path.clone())
            .collect::<Vec<_>>();
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
    if !queued_for_later {
        state.record_prompt_activity(&session.id, prompt, display_attachments)?;
    }
    protocol::emit_sessions_changed(app);
    Ok(())
}

fn is_missing_codex_rollout(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("no rollout found") || normalized.contains("rollout not found")
}

fn is_active_codex_writer(error: &str) -> bool {
    error.to_ascii_lowercase().contains("active writer")
}

pub fn interrupt_prompt(
    app: &AppHandle,
    state: &AppState,
    bridge: &CodexBridge,
    session_id: &str,
) -> Result<(), String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    if !matches!(
        session.status,
        SessionStatus::Running | SessionStatus::PermissionRequired
    ) {
        return Err("This agent does not have a prompt running right now".into());
    }
    if session.agent == AgentKind::Codex {
        let thread_id = session
            .native_session_id
            .as_deref()
            .ok_or_else(|| "The Codex session did not provide its thread id".to_string())?;
        if let Err(error) = bridge.interrupt_prompt(thread_id, state, app) {
            if !is_no_active_prompt(&error) {
                return Err(error);
            }
        }
    } else if session.agent == AgentKind::ClaudeCode {
        let native_session_id = session
            .native_session_id
            .as_deref()
            .ok_or_else(|| "The Claude session did not provide its session id".to_string())?;
        discovery::interrupt_resumed_prompt_process(native_session_id, &session.agent)?;
    } else {
        let process_id = session
            .process_id
            .ok_or_else(|| "This session cannot be interrupted safely".to_string())?;
        discovery::interrupt_agent_process(process_id, &session.agent)?;
    }
    state.mark_prompt_interrupted(session_id)?;
    protocol::emit_sessions_changed(app);
    Ok(())
}

fn is_no_active_prompt(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("does not have a prompt running")
        || error.contains("no active turn")
        || error.contains("turn is not active")
}

pub fn session_collaboration_mode(
    state: &AppState,
    bridge: &CodexBridge,
    session_id: &str,
) -> Result<String, String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    if session.agent != AgentKind::Codex {
        return Err("Collaboration modes are only available for Codex sessions".into());
    }
    let thread_id = session
        .native_session_id
        .as_deref()
        .ok_or_else(|| "The Codex session did not provide its thread id".to_string())?;
    bridge.collaboration_mode(thread_id)
}

pub fn set_session_collaboration_mode(
    app: &AppHandle,
    state: &AppState,
    bridge: &CodexBridge,
    session_id: &str,
    mode: &str,
) -> Result<String, String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    if session.agent != AgentKind::Codex {
        return Err("Collaboration modes are only available for Codex sessions".into());
    }
    let thread_id = session
        .native_session_id
        .as_deref()
        .ok_or_else(|| "The Codex session did not provide its thread id".to_string())?;
    let mode = bridge.set_collaboration_mode(thread_id, mode, state, app)?;
    protocol::emit_sessions_changed(app);
    Ok(mode)
}

pub fn steer_queued_prompt(
    app: &AppHandle,
    state: &AppState,
    bridge: &CodexBridge,
    session_id: &str,
    activity_id: &str,
) -> Result<(), String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    if session.status != SessionStatus::Running || session.agent != AgentKind::Codex {
        return Err("This session cannot steer a queued prompt right now".into());
    }
    let thread_id = session
        .native_session_id
        .as_deref()
        .ok_or_else(|| "The Codex session did not provide its thread id".to_string())?;
    bridge.steer_queued_prompt(session_id, activity_id, thread_id, state, app)?;
    protocol::emit_sessions_changed(app);
    Ok(())
}

fn question_answers_from_prompt(
    request: &PendingQuestion,
    prompt: &str,
) -> Result<Vec<QuestionAnswer>, String> {
    let inputs = if request.questions.len() == 1 {
        vec![prompt.trim()]
    } else {
        prompt.split(',').map(str::trim).collect::<Vec<_>>()
    };
    if inputs.len() != request.questions.len() {
        return Err("Responda cada pergunta separando as opções por vírgula".into());
    }
    request
        .questions
        .iter()
        .zip(inputs)
        .map(|(question, input)| {
            let value = input
                .parse::<usize>()
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| question.options.get(index))
                .map(|option| option.label.clone())
                .or_else(|| {
                    question
                        .options
                        .iter()
                        .find(|option| option.label.eq_ignore_ascii_case(input))
                        .map(|option| option.label.clone())
                })
                .or_else(|| {
                    (question.options.is_empty() || question.is_other).then(|| input.to_string())
                })
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "Digite um número de 1 a {} para responder \"{}\"",
                        question.options.len(),
                        question.header
                    )
                })?;
            Ok(QuestionAnswer {
                question_id: question.id.clone(),
                answers: vec![value],
            })
        })
        .collect()
}

fn prepare_prompt_attachments(
    app: &AppHandle,
    attachments: Vec<PromptAttachmentInput>,
    allow_local_paths: bool,
) -> Result<Vec<PreparedPromptAttachment>, String> {
    if attachments.len() > MAX_PROMPT_ATTACHMENTS {
        return Err(format!(
            "Envie no máximo {MAX_PROMPT_ATTACHMENTS} arquivos por prompt"
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
        let (path, detected_mime, is_image) = if let Some(path) = attachment.path {
            if !allow_local_paths {
                return Err("O celular não pode indicar caminhos locais do computador".into());
            }
            let path = fs::canonicalize(path)
                .map_err(|_| "O arquivo selecionado não existe".to_string())?;
            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            if bytes.len() > MAX_FILE_ATTACHMENT_BYTES {
                return Err("O arquivo excede o limite de 25 MB".into());
            }
            let image_mime = detected_image_mime(&bytes);
            if image_mime.is_some() && bytes.len() > MAX_IMAGE_ATTACHMENT_BYTES {
                return Err("A imagem excede o limite de 5 MB".into());
            }
            (
                path.to_string_lossy().to_string(),
                image_mime
                    .or_else(|| {
                        (!attachment.mime_type.is_empty()).then_some(attachment.mime_type.as_str())
                    })
                    .unwrap_or("application/octet-stream")
                    .to_string(),
                image_mime.is_some(),
            )
        } else if let Some(data) = attachment.data_base64 {
            let bytes = STANDARD
                .decode(data)
                .map_err(|_| "Não foi possível decodificar o arquivo".to_string())?;
            if bytes.len() > MAX_FILE_ATTACHMENT_BYTES {
                return Err("O arquivo excede o limite de 25 MB".into());
            }
            let image_mime = detected_image_mime(&bytes);
            if image_mime.is_some() && bytes.len() > MAX_IMAGE_ATTACHMENT_BYTES {
                return Err("A imagem excede o limite de 5 MB".into());
            }
            let mime = image_mime
                .or_else(|| {
                    (!attachment.mime_type.is_empty()).then_some(attachment.mime_type.as_str())
                })
                .unwrap_or("application/octet-stream")
                .to_string();
            let directory = app
                .path()
                .app_cache_dir()
                .map_err(|error| error.to_string())?
                .join("prompt-attachments");
            fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let name = safe_attachment_name(&attachment.name, index, &mime);
            let path = directory.join(format!(
                "{}-{index}-{}",
                crate::state::now_millis(),
                cache_attachment_name(&name)
            ));
            fs::write(&path, bytes).map_err(|error| error.to_string())?;
            (
                path.to_string_lossy().to_string(),
                mime,
                image_mime.is_some(),
            )
        } else {
            return Err("O anexo não contém um arquivo".into());
        };
        if is_image && !attachment.mime_type.is_empty() && attachment.mime_type != detected_mime {
            return Err("O tipo informado não corresponde ao conteúdo da imagem".into());
        }
        let name = safe_attachment_name(&attachment.name, index, &detected_mime);
        prepared.push(PreparedPromptAttachment {
            path: path.clone(),
            is_image,
            display: PromptAttachment {
                id: format!("attachment:{}:{index}", crate::state::now_millis()),
                name,
                mime_type: detected_mime,
                preview_data_url: if is_image { preview } else { String::new() },
                path: Some(path),
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
    value.push_str("Files attached through Lume. Inspect these local paths:\n");
    for path in paths {
        value.push_str(&format!("- {path:?}\n"));
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
    name.unwrap_or_else(|| {
        if mime.starts_with("image/") {
            format!("image-{}.{}", index + 1, extension_for_mime(mime))
        } else {
            format!("file-{}", index + 1)
        }
    })
}

fn cache_attachment_name(name: &str) -> String {
    let value = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "attachment".into()
    } else {
        value
    }
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
    let process_ids = state
        .sessions()?
        .into_iter()
        .filter(|candidate| {
            candidate.source == SessionSource::Cli
                && candidate.agent == session.agent
                && match session.native_session_id.as_deref() {
                    Some(native_id) => candidate.native_session_id.as_deref() == Some(native_id),
                    None => candidate.id == session.id,
                }
        })
        .filter_map(|candidate| candidate.process_id)
        .collect::<std::collections::BTreeSet<_>>();
    if process_ids.is_empty() {
        return Err("A sessão não possui um processo associado".into());
    }
    let mut terminated = 0usize;
    let mut errors = Vec::new();
    for process_id in process_ids {
        match discovery::terminate_agent_process(process_id, &session.agent) {
            Ok(()) => {
                state.mark_process_terminated(process_id)?;
                terminated += 1;
            }
            Err(error) => errors.push(error),
        }
    }
    if terminated == 0 {
        return Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| "O sistema recusou o encerramento do agente".into()));
    }
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
    let result: Result<Option<serde_json::Value>, String> = match request.command {
        protocol::HubCommand::SubmitPrompt {
            session_id,
            prompt,
            attachments,
            delivery,
        } => submit_prompt(
            app,
            state,
            bridge,
            browser,
            &session_id,
            &prompt,
            attachments,
            delivery,
            false,
        )
        .map(|_| None),
        protocol::HubCommand::ResolvePermission {
            session_id,
            permission_id,
            action,
        } => resolve_permission(state, &session_id, &permission_id, action).map(|_| None),
        protocol::HubCommand::ResolveQuestion {
            session_id,
            question_id,
            answers,
        } => resolve_question(state, &session_id, &question_id, answers).map(|_| None),
        protocol::HubCommand::TerminateSession { session_id } => {
            terminate_session(app, state, &session_id).map(|_| None)
        }
        protocol::HubCommand::InterruptPrompt { session_id } => {
            interrupt_prompt(app, state, bridge, &session_id).map(|_| None)
        }
        protocol::HubCommand::DownloadResponseFile {
            session_id,
            attachment_id,
        } => response_file_payload(state, &session_id, &attachment_id).map(Some),
        protocol::HubCommand::OpenSessionSource { session_id } => {
            open_session_source(state, browser, &session_id).map(|_| None)
        }
        protocol::HubCommand::RefreshRateLimits { agent } => {
            if agent == AgentKind::Codex {
                bridge.refresh_rate_limits(state, app).map(|_| None)
            } else {
                Ok(None)
            }
        }
        protocol::HubCommand::ReportMobileVersion { version } => {
            if protocol::is_version_newer(&version, env!("CARGO_PKG_VERSION")) {
                crate::reveal_main_window(app);
                app.emit(
                    "lume://companion-update-check",
                    serde_json::json!({ "mobileVersion": version }),
                )
                .map(|_| None)
                .map_err(|error| error.to_string())
            } else {
                Ok(None)
            }
        }
    };
    match result {
        Ok(Some(data)) => protocol::HubCommandResponse::success_with_data(request_id, data),
        Ok(None) => protocol::HubCommandResponse::success(request_id),
        Err(message) => protocol::HubCommandResponse::failure(
            request_id,
            protocol::ProtocolError::from_control(message),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_image_preview_is_returned_as_a_valid_data_url() {
        let path = std::env::temp_dir().join(format!(
            "lume-preview-{}-{}.png",
            std::process::id(),
            crate::state::now_millis()
        ));
        fs::write(&path, b"\x89PNG\r\n\x1a\npreview").expect("imagem temporária");

        let preview = local_image_data_url(path.to_str().expect("caminho")).expect("prévia");

        assert!(preview.starts_with("data:image/png;base64,"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn generic_attachments_are_added_as_quoted_local_paths() {
        let prompt = prompt_with_attachment_paths(
            "Inspect this workbook",
            &["/tmp/report \"final\".xlsx".into()],
        );

        assert!(prompt.starts_with("Inspect this workbook\n\nFiles attached through Lume."));
        assert!(prompt.contains(r#""/tmp/report \"final\".xlsx""#));
    }

    #[test]
    fn missing_codex_rollout_errors_are_recoverable() {
        assert!(is_missing_codex_rollout(
            "no rollout found for thread id thread-1"
        ));
        assert!(is_missing_codex_rollout("Rollout not found"));
        assert!(!is_missing_codex_rollout("The Codex server is offline"));
    }

    #[test]
    fn active_codex_writer_errors_are_detected() {
        assert!(is_active_codex_writer(
            "thread thread-1 already has an active writer"
        ));
        assert!(!is_active_codex_writer("Rollout not found"));
    }

    #[test]
    fn cached_attachment_names_cannot_create_nested_paths() {
        assert_eq!(
            cache_attachment_name("../monthly report.xlsx"),
            ".._monthly_report.xlsx"
        );
    }

    #[test]
    fn paired_mobile_can_download_only_a_reported_response_file() {
        let path = std::env::temp_dir().join(format!(
            "lume-response-download-{}-{}.pdf",
            std::process::id(),
            crate::state::now_millis()
        ));
        fs::write(&path, b"response file").expect("arquivo temporário");
        let canonical = fs::canonicalize(&path)
            .expect("caminho")
            .to_string_lossy()
            .to_string();
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(crate::domain::HookEvent {
                event: crate::domain::HookEventKind::Activity,
                session_id: "codex:response-download".into(),
                agent: AgentKind::Codex,
                agent_label: Some("Codex".into()),
                session_name: None,
                project: Some("lume".into()),
                source: Some(SessionSource::Cli),
                source_app: None,
                status_label: None,
                started_at: None,
                process_id: Some(4242),
                native_session_id: Some("response-download".into()),
                working_directory: path
                    .parent()
                    .map(|value| value.to_string_lossy().to_string()),
                permission_profile: None,
                permission: None,
                question: None,
                last_response: None,
                activity: Some(crate::domain::SessionActivity {
                    id: "response-message".into(),
                    kind: "message".into(),
                    title: "Agent response".into(),
                    detail: Some(format!("[Download]({canonical})")),
                    status: "completed".into(),
                    created_at: crate::state::now_millis(),
                    files: Vec::new(),
                    attachments: Vec::new(),
                    append_detail: false,
                }),
                activities: Vec::new(),
                wait_for_decision: false,
            })
            .expect("resposta");
        let session = state.sessions().expect("sessões").remove(0);
        let attachment_id = session.activities[0].attachments[0].id.clone();

        let payload = response_file_payload(&state, &session.id, &attachment_id)
            .expect("arquivo da resposta");
        assert_eq!(
            payload["name"],
            path.file_name().unwrap().to_string_lossy().as_ref()
        );
        assert_eq!(
            STANDARD
                .decode(payload["dataBase64"].as_str().expect("base64"))
                .expect("conteúdo"),
            b"response file"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn option_number_becomes_the_agent_answer() {
        let request = PendingQuestion {
            id: "request".into(),
            questions: vec![crate::domain::InteractiveQuestion {
                id: "approach".into(),
                header: "Approach".into(),
                question: "Which approach?".into(),
                is_other: true,
                is_secret: false,
                options: vec![
                    crate::domain::QuestionOption {
                        label: "First".into(),
                        description: String::new(),
                    },
                    crate::domain::QuestionOption {
                        label: "Second".into(),
                        description: String::new(),
                    },
                ],
            }],
            requested_at: "0".into(),
        };
        let answers = question_answers_from_prompt(&request, "2").expect("resposta");
        assert_eq!(answers[0].answers, vec!["Second"]);
    }

    #[test]
    fn missing_active_turn_is_an_idempotent_interruption() {
        assert!(is_no_active_prompt(
            "This agent does not have a prompt running right now"
        ));
        assert!(is_no_active_prompt("No active turn for this thread"));
        assert!(!is_no_active_prompt("App Server connection failed"));
    }
}
