mod adapters;
mod agent_plugins;
mod browser_server;
mod codex_bridge;
mod codex_sessions;
mod context_builder;
mod control;
mod desktop_shortcuts;
mod discovery;
mod domain;
mod event_server;
mod executables;
mod integrations;
mod launcher;
mod legacy_cli_gateway_cleanup;
mod mobile_gateway;
mod mobile_server;
mod overlay;
mod protocol;
mod session_filters;
mod state;
mod store;
mod terminal_windows;
mod workflow_runtime;

use std::{
    collections::{HashMap, HashSet},
    io::Read,
    sync::Mutex,
};

use domain::{
    AgentKind, AgentSession, HistoryEntry, HookEvent, HookEventKind, PermissionAction, Preferences,
    PromptAttachmentInput, PromptDelivery, QuestionAnswer, ResultNote, SessionActivity,
    SessionControlOrigin, SessionNote, SessionSource, WorkflowRole, WorkflowRoleContract,
};
use integrations::{CompanionStatus, IntegrationDiagnostic, IntegrationKind, IntegrationStatus};
use launcher::LaunchRequest;
use state::AppState;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;

fn reveal_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

struct PendingShortcutAction(Mutex<Option<String>>);

fn shortcut_action_from_args(args: &[String]) -> Option<&str> {
    (args.get(1).map(String::as_str) == Some("shortcut"))
        .then(|| args.get(2).map(String::as_str))
        .flatten()
        .filter(|action| matches!(*action, "open" | "palette" | "new-session" | "whiteboard"))
}

#[tauri::command]
fn take_pending_shortcut_action(
    pending: State<'_, PendingShortcutAction>,
) -> Result<Option<String>, String> {
    pending
        .0
        .lock()
        .map_err(|_| "Não foi possível ler o atalho inicial".to_string())
        .map(|mut action| action.take())
}

fn shortcut_bindings(preferences: &Preferences) -> [(&str, &'static str, &'static str); 4] {
    [
        (&preferences.open_shortcut, "open", "Abrir o Lume"),
        (
            &preferences.global_shortcut,
            "palette",
            "Abrir a paleta de comandos",
        ),
        (
            &preferences.new_session_shortcut,
            "new-session",
            "Abrir uma nova sessão",
        ),
        (
            &preferences.whiteboard_shortcut,
            "whiteboard",
            "Abrir o whiteboard",
        ),
    ]
}

fn parsed_shortcut_bindings(
    preferences: &Preferences,
) -> Result<Vec<(Shortcut, &'static str, &'static str)>, String> {
    let mut ids = HashSet::new();
    shortcut_bindings(preferences)
        .into_iter()
        .map(|(value, action, label)| {
            let shortcut = value
                .parse::<Shortcut>()
                .map_err(|_| format!("Atalho inválido para {label}: {value}"))?;
            if !ids.insert(shortcut.id()) {
                return Err(format!(
                    "O atalho {value} está atribuído a mais de uma ação"
                ));
            }
            Ok((shortcut, action, label))
        })
        .collect()
}

fn register_global_shortcuts(app: &AppHandle, preferences: &Preferences) -> Result<(), String> {
    let bindings = parsed_shortcut_bindings(preferences)?;
    app.global_shortcut()
        .unregister_all()
        .map_err(|error| error.to_string())?;
    for (shortcut, action, label) in bindings {
        if let Err(error) =
            app.global_shortcut()
                .on_shortcut(shortcut, move |app, _shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    reveal_main_window(app);
                    let _ = app.emit("lume://shortcut", action);
                })
        {
            let _ = app.global_shortcut().unregister_all();
            return Err(format!("Não foi possível registrar {label}: {error}"));
        }
    }
    Ok(())
}

fn apply_global_shortcuts(app: &AppHandle, preferences: &Preferences) -> Result<(), String> {
    match desktop_shortcuts::configure(preferences) {
        Ok(true) => app
            .global_shortcut()
            .unregister_all()
            .map_err(|error| error.to_string()),
        Ok(false) => register_global_shortcuts(app, preferences),
        Err(desktop_error) => register_global_shortcuts(app, preferences)
            .map_err(|native_error| format!("{native_error}. {desktop_error}")),
    }
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Result<Vec<AgentSession>, String> {
    state.bounded_sessions(60)
}

#[tauri::command]
fn rename_session(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
    name: String,
) -> Result<String, String> {
    let (session, name) = state.session_rename_plan(&session_id, &name)?;
    if session.agent == AgentKind::Codex {
        let thread_id = session
            .native_session_id
            .as_deref()
            .ok_or_else(|| "Esta sessão do Codex ainda não informou o ID da thread".to_string())?;
        bridge.set_thread_name(thread_id, &name)?;
    }
    let name = state.rename_session(&session_id, &name)?;
    protocol::emit_sessions_changed(&app);
    Ok(name)
}

#[tauri::command]
fn get_hub_snapshot(state: State<'_, AppState>) -> Result<protocol::HubSnapshot, String> {
    Ok(protocol::HubSnapshot::with_activity_limit(
        state.bounded_sessions(60)?,
        60,
    ))
}

#[tauri::command]
fn get_terminal_hub_snapshot(
    state: State<'_, AppState>,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    activity_limit: Option<usize>,
) -> Result<protocol::HubSnapshot, String> {
    let terminal = terminals.state(&label)?;
    let activity_limit = activity_limit.unwrap_or(60).max(1);
    let sessions = state.terminal_sessions(activity_limit, |session| {
        session.id == terminal.session_id
            || (terminal.session_native_id.is_some()
                && terminal.session_native_id == session.native_session_id
                && terminal.session_agent == session.agent)
            || (terminal.session_process_id.is_some()
                && terminal.session_process_id == session.process_id
                && terminal.session_agent == session.agent)
            || (terminal.session_agent == session.agent
                && terminal.session_source == session.source
                && terminal.session_project == session.project
                && terminal.session_working_directory == session.working_directory)
    })?;
    Ok(protocol::HubSnapshot::with_activity_limit(
        sessions,
        activity_limit,
    ))
}

#[tauri::command]
fn begin_mobile_pairing(
    gateway: State<'_, mobile_gateway::MobileGateway>,
    server: State<'_, mobile_server::MobileServer>,
) -> Result<mobile_gateway::PairingOffer, String> {
    if !server.status().network_reachable {
        return Err("Ative o acesso pela rede local antes de parear um dispositivo".into());
    }
    gateway.begin_pairing()
}

#[tauri::command]
fn get_mobile_gateway_status(
    server: State<'_, mobile_server::MobileServer>,
) -> mobile_server::MobileServerStatus {
    server.status()
}

#[tauri::command]
fn enable_mobile_gateway(
    state: State<'_, AppState>,
    server: State<'_, mobile_server::MobileServer>,
) -> Result<mobile_server::MobileServerStatus, String> {
    let status = server.enable_network()?;
    let mut preferences = state.preferences()?;
    preferences.mobile_gateway_enabled = true;
    if let Err(error) = state.save_preferences(&preferences) {
        let _ = server.disable_network();
        return Err(error);
    }
    Ok(status)
}

#[tauri::command]
fn disable_mobile_gateway(
    state: State<'_, AppState>,
    server: State<'_, mobile_server::MobileServer>,
) -> Result<mobile_server::MobileServerStatus, String> {
    let status = server.disable_network()?;
    let mut preferences = state.preferences()?;
    preferences.mobile_gateway_enabled = false;
    if let Err(error) = state.save_preferences(&preferences) {
        let _ = server.enable_network();
        return Err(error);
    }
    Ok(status)
}

#[tauri::command]
fn list_paired_devices(
    state: State<'_, AppState>,
    gateway: State<'_, mobile_gateway::MobileGateway>,
) -> Result<Vec<domain::PairedDevice>, String> {
    gateway.devices(state.inner())
}

#[tauri::command]
fn revoke_paired_device(
    state: State<'_, AppState>,
    gateway: State<'_, mobile_gateway::MobileGateway>,
    id: String,
) -> Result<bool, String> {
    gateway.revoke(state.inner(), &id)
}

#[tauri::command]
fn set_paired_device_scopes(
    state: State<'_, AppState>,
    gateway: State<'_, mobile_gateway::MobileGateway>,
    id: String,
    scopes: Vec<domain::MobileScope>,
) -> Result<bool, String> {
    gateway.set_scopes(state.inner(), &id, scopes)
}

#[tauri::command]
fn execute_hub_command(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    browser: State<'_, browser_server::BrowserControl>,
    workflow_runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    request: protocol::HubCommandRequest,
) -> protocol::HubCommandResponse {
    control::execute_hub_command(
        &app,
        state.inner(),
        bridge.inner(),
        browser.inner(),
        workflow_runtime.inner(),
        request,
    )
}

#[tauri::command]
fn resolve_permission(
    state: State<'_, AppState>,
    session_id: String,
    permission_id: String,
    action: PermissionAction,
) -> Result<(), String> {
    control::resolve_permission(state.inner(), &session_id, &permission_id, action)
}

#[tauri::command]
fn resolve_question(
    state: State<'_, AppState>,
    session_id: String,
    question_id: String,
    answers: Vec<QuestionAnswer>,
) -> Result<(), String> {
    control::resolve_question(state.inner(), &session_id, &question_id, answers)
}

#[tauri::command]
fn open_session_source(
    state: State<'_, AppState>,
    browser: State<'_, browser_server::BrowserControl>,
    session_id: String,
) -> Result<(), String> {
    control::open_session_source(state.inner(), browser.inner(), &session_id)
}

#[tauri::command]
async fn submit_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    browser: State<'_, browser_server::BrowserControl>,
    session_id: String,
    prompt: String,
    attachments: Vec<PromptAttachmentInput>,
    delivery: Option<PromptDelivery>,
) -> Result<(), String> {
    let state = state.inner().clone();
    let bridge = bridge.inner().clone();
    let browser = browser.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        control::submit_prompt(
            &app,
            &state,
            &bridge,
            &browser,
            &session_id,
            &prompt,
            attachments,
            delivery.unwrap_or_default(),
            true,
        )
    })
    .await
    .map_err(|error| format!("Could not complete the prompt submission task: {error}"))?
}

#[tauri::command]
fn read_local_image_data_url(path: String) -> Result<String, String> {
    control::local_image_data_url(&path)
}

#[tauri::command]
fn export_local_file(source_path: String, destination_path: String) -> Result<(), String> {
    let source = std::fs::canonicalize(source_path)
        .map_err(|_| "The response file no longer exists".to_string())?;
    if !source.is_file() {
        return Err("The response attachment is not a file".into());
    }
    let destination = std::path::PathBuf::from(destination_path);
    if destination.as_os_str().is_empty() || destination.is_dir() {
        return Err("Choose a valid destination for the file".into());
    }
    std::fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| format!("Could not save the response file: {error}"))
}

#[tauri::command]
fn set_terminal_file_dialog_active(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    active: bool,
) -> Result<(), String> {
    if !label.starts_with("terminal-") {
        return Err("A janela informada não é um terminal do Lume".into());
    }
    app.get_webview_window(&label)
        .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
    let show_over_fullscreen = state.preferences()?.show_over_fullscreen;
    if active {
        overlay::set_native_dialog_active(true);
    }
    let mut updated = 0usize;
    let mut first_error = None;
    for (window_label, window) in app.webview_windows() {
        if !window_label.starts_with("terminal-") {
            continue;
        }
        match overlay::set_file_dialog_active(&window, active, show_over_fullscreen) {
            Ok(()) => updated += 1,
            Err(error) if first_error.is_none() => first_error = Some(error),
            Err(_) => {}
        }
    }
    if !active || updated == 0 {
        overlay::set_native_dialog_active(false);
    }
    if updated > 0 {
        Ok(())
    } else {
        Err(first_error.unwrap_or_else(|| "No Lume terminal window was available".into()))
    }
}

#[tauri::command]
fn refresh_agent_rate_limits(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    agent: AgentKind,
) -> Result<(), String> {
    if agent == AgentKind::Codex {
        bridge.refresh_rate_limits(state.inner(), &app)
    } else {
        Ok(())
    }
}

#[tauri::command]
fn terminate_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    control::terminate_session(&app, state.inner(), &session_id)
}

#[tauri::command]
async fn take_control_session(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    let bridge = bridge.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        control::take_control_session(&app, &state, &bridge, &session_id)
    })
    .await
    .map_err(|error| format!("Could not complete the session takeover task: {error}"))?
}

#[tauri::command]
fn interrupt_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
) -> Result<(), String> {
    control::interrupt_prompt(&app, state.inner(), bridge.inner(), &session_id)
}

#[tauri::command]
fn get_session_collaboration_mode(
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
) -> Result<String, String> {
    control::session_collaboration_mode(state.inner(), bridge.inner(), &session_id)
}

#[tauri::command]
fn set_session_collaboration_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
    mode: String,
) -> Result<String, String> {
    control::set_session_collaboration_mode(&app, state.inner(), bridge.inner(), &session_id, &mode)
}

#[tauri::command]
fn get_session_model_settings(
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
) -> Result<codex_bridge::CodexThreadModelSettings, String> {
    control::session_model_settings(state.inner(), bridge.inner(), &session_id)
}

#[tauri::command]
fn set_session_model_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
    model: String,
    effort: String,
) -> Result<codex_bridge::CodexThreadModelSettings, String> {
    control::set_session_model_settings(
        &app,
        state.inner(),
        bridge.inner(),
        &session_id,
        &model,
        &effort,
    )
}

#[tauri::command]
fn get_claude_session_model_settings(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<domain::SessionModelOverride, String> {
    control::claude_session_model_settings(state.inner(), &session_id)
}

#[tauri::command]
fn set_claude_session_model_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
    effort: Option<String>,
) -> Result<domain::SessionModelOverride, String> {
    control::set_claude_session_model_settings(
        &app,
        state.inner(),
        &session_id,
        model.as_deref(),
        effort.as_deref(),
    )
}

#[tauri::command]
fn steer_queued_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
    activity_id: String,
) -> Result<(), String> {
    control::steer_queued_prompt(
        &app,
        state.inner(),
        bridge.inner(),
        &session_id,
        &activity_id,
    )
}

#[tauri::command]
fn list_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<HistoryEntry>, String> {
    state.history(limit.unwrap_or(50))
}

#[tauri::command]
fn list_workflow_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<domain::WorkflowHistoryRecord>, String> {
    state.workflow_history(limit.unwrap_or(50))
}

#[tauri::command]
fn list_result_notes(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<ResultNote>, String> {
    state.result_notes(limit.unwrap_or(100))
}

#[tauri::command]
fn save_result_note(
    state: State<'_, AppState>,
    session_id: String,
    result_id: String,
    title: String,
) -> Result<ResultNote, String> {
    state.save_result_note(&session_id, &result_id, &title)
}

#[tauri::command]
fn delete_result_note(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_result_note(&id)
}

#[tauri::command]
fn list_session_notes(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<SessionNote>, String> {
    state.session_notes(&session_id)
}

#[tauri::command]
fn save_session_note(
    state: State<'_, AppState>,
    session_id: String,
    note_id: Option<String>,
    title: String,
    body: String,
    kind: String,
    pinned: bool,
) -> Result<SessionNote, String> {
    state.save_session_note(
        &session_id,
        note_id.as_deref(),
        &title,
        &body,
        &kind,
        pinned,
    )
}

#[tauri::command]
fn delete_session_note(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_session_note(&id)
}

#[tauri::command]
fn get_preferences(state: State<'_, AppState>) -> Result<Preferences, String> {
    state.preferences()
}

#[tauri::command]
fn get_workflow_role_contract(role: WorkflowRole) -> WorkflowRoleContract {
    role.default_contract()
}

#[tauri::command]
fn preview_workflow_context(
    state: State<'_, AppState>,
    group: domain::WorkflowGroupDefinition,
    connection_id: String,
    objective: String,
    source_result_id: Option<String>,
) -> Result<context_builder::WorkflowContextPackage, String> {
    let sessions = state.bounded_sessions(200)?;
    let settings = state.preferences()?.workflow_settings;
    context_builder::build_context_package_with_limit(
        &group,
        &connection_id,
        &objective,
        source_result_id.as_deref(),
        &sessions,
        settings.max_context_tokens as usize,
    )
}

#[tauri::command]
fn get_workflow_run(
    state: State<'_, AppState>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
) -> Result<Option<domain::WorkflowRun>, String> {
    runtime.get(state.inner(), &workflow_id)
}

#[tauri::command]
fn start_workflow_run(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    browser: State<'_, browser_server::BrowserControl>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    group: domain::WorkflowGroupDefinition,
    objective: String,
) -> Result<domain::WorkflowRun, String> {
    runtime.start(
        &app,
        state.inner(),
        bridge.inner(),
        browser.inner(),
        group,
        &objective,
    )
}

#[tauri::command]
fn approve_workflow_handoff(
    app: AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
) -> Result<domain::WorkflowRun, String> {
    runtime.approve(&app, state.inner(), &workflow_id)
}

#[tauri::command]
fn advance_workflow_run(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    browser: State<'_, browser_server::BrowserControl>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
) -> Result<domain::WorkflowRun, String> {
    runtime.advance(
        &app,
        state.inner(),
        bridge.inner(),
        browser.inner(),
        &workflow_id,
    )
}

#[tauri::command]
fn pause_workflow_run(
    app: AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
) -> Result<domain::WorkflowRun, String> {
    runtime.pause(&app, state.inner(), &workflow_id)
}

#[tauri::command]
fn resume_workflow_run(
    app: AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
) -> Result<domain::WorkflowRun, String> {
    runtime.resume(&app, state.inner(), &workflow_id)
}

#[tauri::command]
fn retry_workflow_step(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    browser: State<'_, browser_server::BrowserControl>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
) -> Result<domain::WorkflowRun, String> {
    runtime.retry(
        &app,
        state.inner(),
        bridge.inner(),
        browser.inner(),
        &workflow_id,
    )
}

#[tauri::command]
fn skip_workflow_step(
    app: AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
) -> Result<domain::WorkflowRun, String> {
    runtime.skip(&app, state.inner(), &workflow_id)
}

#[tauri::command]
fn cancel_workflow_run(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
) -> Result<domain::WorkflowRun, String> {
    let current_session = runtime
        .current_session_id(state.inner(), &workflow_id)
        .ok()
        .flatten();
    let run = runtime.cancel(&app, state.inner(), &workflow_id)?;
    if let Some(session_id) = current_session {
        let _ = control::interrupt_prompt(&app, state.inner(), bridge.inner(), &session_id);
    }
    Ok(run)
}

#[tauri::command]
fn rebind_workflow_session(
    app: AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, workflow_runtime::WorkflowRuntime>,
    workflow_id: String,
    step_id: String,
    session_native_id: String,
) -> Result<Option<domain::WorkflowRun>, String> {
    runtime.rebind_session(
        &app,
        state.inner(),
        &workflow_id,
        &step_id,
        &session_native_id,
    )
}

#[tauri::command]
fn display_backend() -> &'static str {
    #[cfg(target_os = "linux")]
    match std::env::var("LUME_LINUX_BACKEND").ok().as_deref() {
        Some("xwayland-fallback") => return "xwayland-fallback",
        Some("native-gnome") => return "native-gnome",
        Some("gnome-wayland-limited") => return "gnome-wayland-limited",
        _ => {}
    }
    "native"
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPosition {
    x: i32,
    y: i32,
}

#[tauri::command]
fn get_overlay_position(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<OverlayPosition, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Janela do Lume não encontrada".to_string())?;
    if let Some((x, y)) = overlay::position(&window) {
        return Ok(OverlayPosition { x, y });
    }
    let preferences = state.preferences()?;
    let (x, y) = match (preferences.overlay_x, preferences.overlay_y) {
        (Some(x), Some(y)) => (x, y),
        _ => overlay::default_position(&window, preferences.monitor_id.as_deref())?,
    };
    Ok(OverlayPosition { x, y })
}

fn workflow_has_cycle(group: &domain::WorkflowGroupDefinition) -> bool {
    fn visit<'a>(
        step: &'a str,
        edges: &HashMap<&'a str, Vec<&'a str>>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> bool {
        if visiting.contains(step) {
            return true;
        }
        if !visited.insert(step) {
            return false;
        }
        visiting.insert(step);
        let cyclic = edges
            .get(step)
            .into_iter()
            .flatten()
            .any(|next| visit(next, edges, visiting, visited));
        visiting.remove(step);
        cyclic
    }

    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for connection in &group.connections {
        edges
            .entry(connection.from_step_id.as_str())
            .or_default()
            .push(connection.to_step_id.as_str());
    }
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    group
        .steps
        .iter()
        .any(|step| visit(&step.id, &edges, &mut visiting, &mut visited))
}

fn validate_workflow_groups(preferences: &Preferences) -> Result<(), String> {
    let settings = &preferences.workflow_settings;
    if !(1..=100).contains(&settings.max_transitions) {
        return Err("Workflow transitions must be between 1 and 100".into());
    }
    if !(1..=10).contains(&settings.max_attempts_per_step) {
        return Err("Workflow attempts per step must be between 1 and 10".into());
    }
    if settings.step_timeout_minutes > 1_440 {
        return Err("Workflow step timeout cannot exceed 1440 minutes".into());
    }
    if !(1_000..=100_000).contains(&settings.max_context_tokens) {
        return Err("Workflow context limit must be between 1000 and 100000 tokens".into());
    }
    if settings.minimum_rate_limit_remaining_percent > 100 {
        return Err("Workflow rate limit reserve cannot exceed 100 percent".into());
    }
    let mut group_ids = HashSet::new();
    for group in &preferences.workflow_groups {
        if group.id.trim().is_empty() || group.terminal_group_id.trim().is_empty() {
            return Err("Workflow groups must have stable identifiers".into());
        }
        if !group_ids.insert(group.id.as_str()) {
            return Err("Workflow group identifiers must be unique".into());
        }
        let mut step_ids = HashSet::new();
        let mut sessions = HashSet::new();
        for step in &group.steps {
            if step.id.trim().is_empty() || step.session_native_id.trim().is_empty() {
                return Err("Workflow steps must identify their terminal session".into());
            }
            if !step_ids.insert(step.id.as_str())
                || !sessions.insert(step.session_native_id.as_str())
            {
                return Err("A terminal can appear only once in a workflow group".into());
            }
            if [
                &step.custom_role_label,
                &step.instruction,
                &step.expected_input,
                &step.produced_output,
                &step.completion_condition,
            ]
            .iter()
            .any(|value| value.len() > 4_000)
            {
                return Err("Workflow step fields cannot exceed 4000 characters".into());
            }
        }
        let known_steps = group
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<HashSet<_>>();
        let mut connection_ids = HashSet::new();
        let mut connection_pairs = HashSet::new();
        for connection in &group.connections {
            if connection.id.trim().is_empty()
                || !known_steps.contains(connection.from_step_id.as_str())
                || !known_steps.contains(connection.to_step_id.as_str())
            {
                return Err("Workflow connections must reference existing steps".into());
            }
            if connection.from_step_id == connection.to_step_id {
                return Err("A workflow step cannot connect to itself".into());
            }
            if !connection_ids.insert(connection.id.as_str())
                || !connection_pairs.insert((
                    connection.from_step_id.as_str(),
                    connection.to_step_id.as_str(),
                ))
            {
                return Err("Workflow connections must be unique".into());
            }
            if connection.additional_instruction.len() > 4_000 {
                return Err(
                    "Workflow connection instructions cannot exceed 4000 characters".into(),
                );
            }
            let context = context_builder::effective_selection(connection);
            if !context.response
                && !context.files
                && !context.checks
                && !context.plan
                && !context.activity
                && connection.additional_instruction.trim().is_empty()
            {
                return Err("Workflow connections must transfer some context".into());
            }
            if context.diffs && !context.files {
                return Err("Workflow diffs require changed files to be included".into());
            }
        }
        if workflow_has_cycle(group) {
            return Err("Circular workflow connections are disabled by default".into());
        }
    }
    Ok(())
}

#[tauri::command]
fn set_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    preferences: Preferences,
) -> Result<(), String> {
    validate_workflow_groups(&preferences)?;
    let previous = state.preferences()?;
    let overlay_configuration_changed = previous.monitor_id != preferences.monitor_id
        || previous.show_over_fullscreen != preferences.show_over_fullscreen;
    let shortcuts_changed = shortcut_bindings(&previous)
        .iter()
        .map(|binding| binding.0)
        .ne(shortcut_bindings(&preferences)
            .iter()
            .map(|binding| binding.0));
    if shortcuts_changed {
        if let Err(error) = apply_global_shortcuts(&app, &preferences) {
            let _ = apply_global_shortcuts(&app, &previous);
            return Err(error);
        }
    }
    if preferences.autostart {
        app.autolaunch()
            .enable()
            .map_err(|error| error.to_string())?;
    } else {
        app.autolaunch()
            .disable()
            .map_err(|error| error.to_string())?;
    }
    if let Err(error) = state.save_preferences(&preferences) {
        if shortcuts_changed {
            let _ = apply_global_shortcuts(&app, &previous);
        }
        return Err(error);
    }
    if overlay_configuration_changed {
        let Some(window) = app.get_webview_window("main") else {
            return Ok(());
        };
        let show_over_fullscreen = preferences.show_over_fullscreen;
        let monitor_id = preferences.monitor_id.clone();
        let window_for_layer = window.clone();
        let _ = window.run_on_main_thread(move || {
            let _ = overlay::configure(
                &window_for_layer,
                show_over_fullscreen,
                monitor_id.as_deref(),
                preferences.overlay_x,
                preferences.overlay_y,
            );
        });
    }
    Ok(())
}

#[tauri::command]
fn move_overlay(
    app: AppHandle,
    state: State<'_, AppState>,
    x: i32,
    y: i32,
    persist: bool,
    monitor_id: Option<String>,
) -> Result<(), String> {
    let monitor_id = if persist {
        let mut preferences = state.preferences()?;
        preferences.overlay_x = Some(x);
        preferences.overlay_y = Some(y);
        state.save_preferences(&preferences)?;
        preferences.monitor_id
    } else {
        monitor_id
    };
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Janela do Lume não encontrada".to_string())?;
    let window_for_move = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = overlay::move_to(&window_for_move, x, y, monitor_id.as_deref());
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn resize_overlay_surface(app: AppHandle, width: i32, height: i32) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Janela do Lume não encontrada".to_string())?;
    let window_for_resize = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = overlay::resize_surface(&window_for_resize, width, height);
        })
        .map_err(|error| error.to_string())
}

fn open_terminal_window_impl(
    app: AppHandle,
    state: State<'_, AppState>,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    session_id: String,
) -> Result<String, String> {
    let session = state.connected_session(&session_id)?;
    let preferences = state.preferences()?;
    terminals.open(
        &app,
        &session,
        preferences.monitor_id.as_deref(),
        preferences.overlay_x.unwrap_or(40),
        preferences.overlay_y.unwrap_or(44),
        preferences.show_over_fullscreen,
        preferences.workflow_enabled,
    )
}

#[cfg(target_os = "linux")]
#[tauri::command]
// GTK window creation must stay on the command's main-thread path on Linux.
fn open_terminal_window(
    app: AppHandle,
    state: State<'_, AppState>,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    session_id: String,
) -> Result<String, String> {
    open_terminal_window_impl(app, state, terminals, session_id)
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
async fn open_terminal_window(
    app: AppHandle,
    state: State<'_, AppState>,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    session_id: String,
) -> Result<String, String> {
    // Keeping the command asynchronous prevents WebView2 creation from blocking
    // the overlay command dispatcher on Windows.
    open_terminal_window_impl(app, state, terminals, session_id)
}

#[tauri::command]
fn terminal_frontend_ready(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<(), String> {
    terminals.frontend_ready(&app, &label)
}

#[tauri::command]
fn toggle_terminal_group_fullscreen(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<Option<bool>, String> {
    terminals.toggle_group_fullscreen(&app, &label)
}

#[tauri::command]
fn terminal_group_fullscreen_active(
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> bool {
    terminals.group_fullscreen_active(&label)
}

#[tauri::command]
fn list_terminal_windows(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
) -> Result<Vec<terminal_windows::TerminalWindowState>, String> {
    terminals.list(&app)
}

#[tauri::command]
fn set_terminal_windows_visible(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    visible: bool,
) -> Result<(), String> {
    terminals.set_visible(&app, visible)
}

#[tauri::command]
fn get_terminal_window_state(
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.state(&label)
}

#[tauri::command]
fn close_terminal_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<(), String> {
    terminals.close(&app, &label)?;
    let _ = app.emit("lume://terminal-windows-changed", ());
    Ok(())
}

#[tauri::command]
fn minimize_terminal_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<(), String> {
    terminals.hide(&app, &label)
}

#[tauri::command]
fn move_terminal_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    x: i32,
    y: i32,
    finalize: bool,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.move_window(&app, &label, x, y, finalize)
}

#[tauri::command]
fn cancel_terminal_window_move(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.cancel_move(&app, &label)
}

#[tauri::command]
fn sync_terminal_window_position(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    x: i32,
    y: i32,
    finalize: bool,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.sync_native_position(&app, &label, x, y, finalize)
}

#[tauri::command]
fn terminal_drag_snapshot(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<terminal_windows::TerminalDragSnapshot, String> {
    terminals.drag_snapshot(&app, &label)
}

#[tauri::command]
fn begin_terminal_native_drag(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<(), String> {
    terminals.begin_native_drag(&app, &label)
}

#[tauri::command]
fn resize_terminal_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    from_left: bool,
    from_top: bool,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.resize_window(&app, &label, x, y, width, height, from_left, from_top)
}

#[tauri::command]
fn begin_layered_terminal_resize(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.begin_layered_resize(&app, &label)
}

#[tauri::command]
fn finish_layered_terminal_resize(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.finish_layered_resize(&app, &label)
}

#[cfg(target_os = "linux")]
#[tauri::command]
fn open_workflow_bridge_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    side: terminal_windows::DockSide,
) -> Result<String, String> {
    terminals.open_workflow_bridge(&app, &label, side)
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
async fn open_workflow_bridge_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    side: terminal_windows::DockSide,
) -> Result<String, String> {
    terminals.open_workflow_bridge(&app, &label, side)
}

#[tauri::command]
fn set_workflow_connection_hover(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    side: terminal_windows::DockSide,
    hovered: bool,
) -> Result<(), String> {
    terminals.set_workflow_connection_hover(&app, &label, side, hovered)
}

#[cfg(target_os = "linux")]
#[tauri::command]
fn prepare_workflow_bridge_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    side: terminal_windows::DockSide,
) -> Result<String, String> {
    terminals.prepare_workflow_bridge(&app, &label, side)
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
async fn prepare_workflow_bridge_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    side: terminal_windows::DockSide,
) -> Result<String, String> {
    terminals.prepare_workflow_bridge(&app, &label, side)
}

#[tauri::command]
fn discard_prepared_workflow_bridge_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<(), String> {
    terminals.discard_prepared_workflow_bridge(&app, &label)
}

#[tauri::command]
fn get_workflow_bridge_context(
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<terminal_windows::WorkflowBridgeContext, String> {
    terminals.workflow_bridge_context(&label)
}

#[tauri::command]
fn set_workflow_bridge_expanded(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
    expanded: bool,
    content_height: Option<i32>,
) -> Result<(), String> {
    terminals.set_workflow_bridge_expanded(&app, &label, expanded, content_height)
}

#[tauri::command]
fn undock_terminal_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.undock(&app, &label)
}

#[tauri::command]
fn set_terminal_workflow_enabled(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    enabled: bool,
) -> Result<Vec<terminal_windows::TerminalWindowState>, String> {
    terminals.set_workflow_enabled(&app, enabled)
}

#[tauri::command]
fn restore_terminal_layout(
    app: AppHandle,
    state: State<'_, AppState>,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    entries: Vec<terminal_windows::RestoredTerminalPlacement>,
) -> Result<Vec<terminal_windows::TerminalWindowState>, String> {
    let preferences = state.preferences()?;
    terminals.restore_layout(
        &app,
        entries,
        preferences.monitor_id.as_deref(),
        preferences.workflow_enabled,
    )
}

#[tauri::command]
fn integration_statuses() -> Result<Vec<IntegrationStatus>, String> {
    let executable = integrations::lume_executable()?;
    Ok(integrations::statuses(&executable.to_string_lossy()))
}

#[tauri::command]
fn list_resumable_sessions(
    kind: IntegrationKind,
    state: State<'_, AppState>,
) -> Result<Vec<integrations::ResumableSession>, String> {
    let open_sessions = state
        .connected_sessions()?
        .into_iter()
        .filter_map(|session| session.native_session_id)
        .collect::<HashSet<_>>();
    Ok(integrations::resumable_sessions(&kind)?
        .into_iter()
        .filter(|session| !open_sessions.contains(&session.id))
        .collect())
}

#[tauri::command]
fn diagnose_integration(
    kind: IntegrationKind,
    state: State<'_, AppState>,
) -> Result<IntegrationDiagnostic, String> {
    let executable = integrations::lume_executable()?;
    let last_event_at = state
        .connected_sessions()?
        .into_iter()
        .filter(|session| {
            matches!(
                (&kind, &session.agent),
                (IntegrationKind::Codex, domain::AgentKind::Codex)
                    | (IntegrationKind::Claude, domain::AgentKind::ClaudeCode)
                    | (IntegrationKind::Antigravity, domain::AgentKind::Antigravity)
                    | (IntegrationKind::DeepSeek, domain::AgentKind::DeepSeek)
                    | (IntegrationKind::Gemini, domain::AgentKind::Gemini)
            )
        })
        .map(|session| session.updated_at)
        .max();
    integrations::diagnose(&kind, &executable.to_string_lossy(), last_event_at)
}

#[tauri::command]
fn configure_integration(kind: IntegrationKind, enabled: bool) -> Result<(), String> {
    let executable = integrations::lume_executable()?;
    integrations::configure(&kind, &executable.to_string_lossy(), enabled)
}

#[tauri::command]
fn vscode_status() -> CompanionStatus {
    integrations::vscode_status()
}

#[tauri::command]
fn configure_vscode(app: AppHandle, enabled: bool) -> Result<(), String> {
    let bundled = app
        .path()
        .resolve("lume-vscode.vsix", tauri::path::BaseDirectory::Resource)
        .map_err(|error| error.to_string())?;
    let development =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/lume-vscode.vsix");
    let vsix = if bundled.exists() {
        bundled
    } else {
        development
    };
    integrations::configure_vscode(enabled, &vsix)
}

#[tauri::command]
fn reveal_browser_companion(app: AppHandle) -> Result<String, String> {
    let bundled = app
        .path()
        .resolve("chromium", tauri::path::BaseDirectory::Resource)
        .map_err(|error| error.to_string())?;
    let development =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extensions/chromium");
    let directory = if bundled.exists() {
        bundled
    } else {
        development
    };
    app.opener()
        .open_path(directory.to_string_lossy(), None::<String>)
        .map_err(|error| error.to_string())?;
    Ok(directory.to_string_lossy().to_string())
}

#[tauri::command]
fn list_external_plugins(
    app: AppHandle,
) -> Result<Vec<agent_plugins::ExternalAgentPlugin>, String> {
    Ok(agent_plugins::external_catalog(&app))
}

#[tauri::command]
fn install_external_plugin(
    app: AppHandle,
    path: String,
) -> Result<agent_plugins::ExternalAgentPlugin, String> {
    agent_plugins::install_external(&app, std::path::Path::new(&path))
}

#[tauri::command]
fn remove_external_plugin(app: AppHandle, id: String) -> Result<(), String> {
    agent_plugins::remove_external(&app, &id)
}

#[tauri::command]
fn reveal_plugin_directory(app: AppHandle) -> Result<String, String> {
    let directory = agent_plugins::plugin_directory(&app)?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let template = directory.join("plugin-template.json.example");
    if !template.exists() {
        std::fs::write(
            &template,
            include_str!("../../docs/external-plugin.example.json"),
        )
        .map_err(|error| error.to_string())?;
    }
    app.opener()
        .open_path(directory.to_string_lossy(), None::<String>)
        .map_err(|error| error.to_string())?;
    Ok(directory.to_string_lossy().to_string())
}

#[tauri::command]
async fn launch_session(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    request: LaunchRequest,
) -> Result<(), String> {
    let state = state.inner().clone();
    let bridge = bridge.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        launch_session_impl(&app, &state, &bridge, request)
    })
    .await
    .map_err(|error| format!("Could not complete the session launch task: {error}"))?
}

fn launch_session_impl(
    app: &AppHandle,
    state: &AppState,
    bridge: &codex_bridge::CodexBridge,
    mut request: LaunchRequest,
) -> Result<(), String> {
    if request.target == "vscode" && !integrations::vscode_status().configured {
        return Err("Conecte o Lume Companion ao VS Code nos Ajustes".into());
    }
    let executable = integrations::lume_executable()?;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let resume_preview = request
        .resume
        .then(|| request.resume_id.as_deref())
        .flatten()
        .and_then(|session_id| integrations::resume_preview(&request.agent, session_id));
    let prepared_event = if request.agent == IntegrationKind::Codex {
        let resume_id = request.resume_id.as_deref().filter(|_| request.resume);
        let prepared = if let Some(thread_id) = resume_id {
            let thread_name = integrations::indexed_session_names(&IntegrationKind::Codex)
                .ok()
                .and_then(|names| names.get(thread_id).cloned());
            bridge.prepare_existing_thread_launch(
                thread_id,
                thread_name,
                request.permission_mode.as_ref(),
                request.approval_policy.as_deref(),
            )
        } else {
            bridge.prepare_thread(
                &request.working_directory,
                None,
                request.permission_mode.as_ref(),
                request.approval_policy.as_deref(),
            )
        }
        .map_err(codex_resume_error)?;
        request.resume = true;
        request.resume_id = Some(prepared.thread_id.clone());
        Some(prepared_codex_session_event(
            &request,
            prepared,
            resume_preview.as_ref(),
        ))
    } else {
        resume_preview
            .as_ref()
            .and_then(|preview| prepared_resume_preview_event(&request, preview))
    };
    let codex_remote = if request.agent == IntegrationKind::Codex {
        Some(codex_bridge::PROXY_URL)
    } else {
        None
    };
    let codex_thread_id = (request.agent == IntegrationKind::Codex)
        .then(|| request.resume_id.clone())
        .flatten();
    launcher::launch(request, &executable, &app_data_dir, codex_remote)?;
    if let Some(thread_id) = codex_thread_id.as_deref() {
        bridge.wait_for_proxy_thread(thread_id, std::time::Duration::from_secs(12))?;
    }
    if let Some(event) = prepared_event {
        event_server::publish_event(&state, &app, event)?;
    }
    Ok(())
}

fn codex_resume_error(error: String) -> String {
    if error.to_ascii_lowercase().contains("active writer") {
        "This thread is still open in another Codex CLI or client. Close that origin and wait a moment, or transfer the detected session to Lume from its terminal.".into()
    } else {
        error
    }
}

fn prepared_codex_session_event(
    request: &LaunchRequest,
    prepared: codex_bridge::PreparedThread,
    preview: Option<&integrations::ResumePreview>,
) -> HookEvent {
    let project = std::path::Path::new(&request.working_directory)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string);
    HookEvent {
        event: HookEventKind::SessionStarted,
        session_id: format!("codex-app-server:{}", prepared.thread_id),
        agent: AgentKind::Codex,
        agent_label: Some("Codex".into()),
        session_name: prepared.thread_name.clone(),
        project,
        source: Some(if request.target == "vscode" {
            SessionSource::Vscode
        } else {
            SessionSource::Cli
        }),
        source_app: None,
        control_origin: SessionControlOrigin::Lume,
        status_label: Some("Esperando ação".into()),
        started_at: None,
        process_id: None,
        native_session_id: Some(prepared.thread_id.clone()),
        working_directory: Some(request.working_directory.clone()),
        permission_profile: Some(prepared.permission_profile),
        permission: None,
        question: None,
        last_response: preview.map(|preview| preview.response.clone()),
        activity: None,
        activities: preview
            .map(|preview| vec![resume_preview_activity(&prepared.thread_id, preview)])
            .unwrap_or_default(),
        wait_for_decision: false,
    }
}

fn prepared_resume_preview_event(
    request: &LaunchRequest,
    preview: &integrations::ResumePreview,
) -> Option<HookEvent> {
    let (agent, agent_label, prefix) = match request.agent {
        IntegrationKind::Claude => (AgentKind::ClaudeCode, "Claude Code", "claude"),
        IntegrationKind::Codex => (AgentKind::Codex, "Codex", "codex"),
        IntegrationKind::Antigravity => return None,
        IntegrationKind::DeepSeek => return None,
        IntegrationKind::Gemini => return None,
    };
    let native_session_id = request.resume_id.clone()?;
    let project = std::path::Path::new(&request.working_directory)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string);
    Some(HookEvent {
        event: HookEventKind::Activity,
        session_id: format!("{prefix}-resume:{native_session_id}"),
        agent,
        agent_label: Some(agent_label.into()),
        session_name: None,
        project,
        source: Some(if request.target == "vscode" {
            SessionSource::Vscode
        } else {
            SessionSource::Cli
        }),
        source_app: None,
        control_origin: SessionControlOrigin::Lume,
        status_label: Some("Esperando ação".into()),
        started_at: None,
        process_id: None,
        native_session_id: Some(native_session_id.clone()),
        working_directory: Some(request.working_directory.clone()),
        permission_profile: None,
        permission: None,
        question: None,
        last_response: Some(preview.response.clone()),
        activity: None,
        activities: vec![resume_preview_activity(&native_session_id, preview)],
        wait_for_decision: false,
    })
}

fn resume_preview_activity(
    native_session_id: &str,
    preview: &integrations::ResumePreview,
) -> SessionActivity {
    SessionActivity {
        id: format!("resume-preview:{native_session_id}"),
        kind: "message".into(),
        title: "Resposta anterior".into(),
        detail: Some(preview.response.clone()),
        status: "completed".into(),
        created_at: preview.updated_at,
        files: Vec::new(),
        attachments: Vec::new(),
        append_detail: false,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let startup_args = std::env::args().collect::<Vec<_>>();
    let startup_shortcut_action = shortcut_action_from_args(&startup_args).map(str::to_string);
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            reveal_main_window(app);
            let action = shortcut_action_from_args(&args).unwrap_or("open");
            let _ = app.emit("lume://shortcut", action);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            if let Ok(executable) = integrations::lume_executable() {
                integrations::refresh_connected(&executable.to_string_lossy());
            }
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?
                .join("lume.sqlite3");
            if let Err(error) = legacy_cli_gateway_cleanup::cleanup(
                database_path.parent().unwrap_or(&database_path),
            ) {
                eprintln!("Could not finish the legacy CLI Gateway cleanup: {error}");
            }
            let state = AppState::new(&database_path)?;
            app.manage(PendingShortcutAction(Mutex::new(
                startup_shortcut_action.clone(),
            )));
            let _ = apply_global_shortcuts(app.handle(), &state.preferences()?);
            app.manage(state.clone());
            let codex_bridge =
                codex_bridge::CodexBridge::start(state.clone(), app.handle().clone())?;
            app.manage(codex_bridge.clone());
            codex_sessions::start(state.clone(), app.handle().clone())?;
            event_server::start(state.clone(), app.handle().clone())?;
            let browser_control = browser_server::BrowserControl::default();
            browser_server::start(state.clone(), app.handle().clone(), browser_control.clone())?;
            app.manage(browser_control.clone());
            let mobile_gateway = mobile_gateway::MobileGateway::default();
            let mobile_server = mobile_server::MobileServer::start_loopback(
                state.clone(),
                mobile_gateway.clone(),
                app.handle().clone(),
                database_path.parent().unwrap_or(&database_path),
            )
            .unwrap_or_else(|error| {
                eprintln!("{error}");
                mobile_server::MobileServer::default()
            });
            if state.preferences()?.mobile_gateway_enabled {
                if let Err(error) = mobile_server.enable_network() {
                    eprintln!("Could not restore mobile network access: {error}");
                }
            }
            app.manage(mobile_gateway);
            app.manage(mobile_server);
            app.manage(terminal_windows::TerminalWindows::default());
            let workflow_runtime = workflow_runtime::WorkflowRuntime::default();
            if let Err(error) = workflow_runtime.restore(&state) {
                eprintln!("Could not restore workflow runs: {error}");
            }
            workflow_runtime.start_monitor(
                state.clone(),
                app.handle().clone(),
                codex_bridge,
                browser_control,
            );
            app.manage(workflow_runtime);
            discovery::start(state.clone(), app.handle().clone())?;
            overlay::start_fullscreen_guard(state.clone(), app.handle().clone())?;

            if let Some(window) = app.get_webview_window("main") {
                let preferences = state.preferences()?;
                let configured = overlay::configure(
                    &window,
                    preferences.show_over_fullscreen,
                    preferences.monitor_id.as_deref(),
                    preferences.overlay_x,
                    preferences.overlay_y,
                );
                if !configured {
                    if let Ok((default_x, default_y)) =
                        overlay::default_position(&window, preferences.monitor_id.as_deref())
                    {
                        let _ = overlay::move_to(
                            &window,
                            preferences.overlay_x.unwrap_or(default_x),
                            preferences.overlay_y.unwrap_or(default_y),
                            preferences.monitor_id.as_deref(),
                        );
                    }
                }
                window.show()?;
            }

            let show = MenuItem::with_id(app, "show", "Mostrar Lume", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .expect("ícone padrão ausente")
                        .clone(),
                )
                .tooltip("Lume — monitor de agentes")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        reveal_main_window(tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => reveal_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            if state.preferences()?.autostart {
                let _ = app.autolaunch().enable();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            rename_session,
            get_hub_snapshot,
            get_terminal_hub_snapshot,
            execute_hub_command,
            begin_mobile_pairing,
            get_mobile_gateway_status,
            enable_mobile_gateway,
            disable_mobile_gateway,
            list_paired_devices,
            revoke_paired_device,
            set_paired_device_scopes,
            resolve_permission,
            resolve_question,
            open_session_source,
            submit_prompt,
            read_local_image_data_url,
            export_local_file,
            set_terminal_file_dialog_active,
            refresh_agent_rate_limits,
            interrupt_prompt,
            get_session_collaboration_mode,
            set_session_collaboration_mode,
            get_session_model_settings,
            set_session_model_settings,
            get_claude_session_model_settings,
            set_claude_session_model_settings,
            steer_queued_prompt,
            terminate_session,
            take_control_session,
            list_history,
            list_workflow_history,
            list_result_notes,
            save_result_note,
            delete_result_note,
            list_session_notes,
            save_session_note,
            delete_session_note,
            get_preferences,
            get_workflow_role_contract,
            preview_workflow_context,
            get_workflow_run,
            start_workflow_run,
            approve_workflow_handoff,
            advance_workflow_run,
            pause_workflow_run,
            resume_workflow_run,
            retry_workflow_step,
            skip_workflow_step,
            cancel_workflow_run,
            rebind_workflow_session,
            take_pending_shortcut_action,
            display_backend,
            get_overlay_position,
            set_preferences,
            move_overlay,
            resize_overlay_surface,
            open_terminal_window,
            terminal_frontend_ready,
            toggle_terminal_group_fullscreen,
            terminal_group_fullscreen_active,
            list_terminal_windows,
            set_terminal_windows_visible,
            get_terminal_window_state,
            minimize_terminal_window,
            close_terminal_window,
            move_terminal_window,
            cancel_terminal_window_move,
            sync_terminal_window_position,
            terminal_drag_snapshot,
            begin_terminal_native_drag,
            resize_terminal_window,
            begin_layered_terminal_resize,
            finish_layered_terminal_resize,
            open_workflow_bridge_window,
            set_workflow_connection_hover,
            prepare_workflow_bridge_window,
            discard_prepared_workflow_bridge_window,
            get_workflow_bridge_context,
            set_workflow_bridge_expanded,
            undock_terminal_window,
            set_terminal_workflow_enabled,
            restore_terminal_layout,
            integration_statuses,
            list_resumable_sessions,
            diagnose_integration,
            configure_integration,
            vscode_status,
            configure_vscode,
            reveal_browser_companion,
            list_external_plugins,
            install_external_plugin,
            remove_external_plugin,
            reveal_plugin_directory,
            launch_session
        ])
        .run(tauri::generate_context!())
        .expect("erro ao executar o Lume");
}

pub fn run_ingest_client() -> i32 {
    let mut payload = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut payload) {
        eprintln!("Não foi possível ler o evento: {error}");
        return 2;
    }
    match event_server::send_event(&payload) {
        Ok(response) => match serde_json::to_string(&response) {
            Ok(json) => {
                println!("{json}");
                if response.ok {
                    0
                } else {
                    1
                }
            }
            Err(error) => {
                eprintln!("Não foi possível responder ao hook: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub fn run_hook_client(provider: &str) -> i32 {
    adapters::run_hook(provider)
}

pub fn run_terminal_payload(path: &str) -> i32 {
    launcher::run_terminal_payload(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_global_shortcuts_are_valid_and_unique() {
        let bindings = parsed_shortcut_bindings(&Preferences::default()).expect("atalhos padrão");

        assert_eq!(bindings.len(), 4);
        assert_eq!(
            bindings
                .iter()
                .map(|(_, action, _)| *action)
                .collect::<Vec<_>>(),
            vec!["open", "palette", "new-session", "whiteboard"]
        );
    }

    #[test]
    fn repeated_global_shortcuts_are_rejected() {
        let mut preferences = Preferences::default();
        preferences.open_shortcut = preferences.global_shortcut.clone();

        assert!(parsed_shortcut_bindings(&preferences).is_err());
    }

    #[test]
    fn workflow_roles_and_contracts_are_validated_as_a_group() {
        let mut preferences = Preferences::default();
        preferences.workflow_groups = vec![domain::WorkflowGroupDefinition {
            id: "workflow-group-1".into(),
            terminal_group_id: "terminal-group-1".into(),
            steps: vec![domain::WorkflowStepDefinition {
                id: "step-1".into(),
                session_native_id: "thread-1".into(),
                role: domain::WorkflowRole::Planner,
                instruction: "Create the implementation plan".into(),
                expected_input: "Objective".into(),
                produced_output: "Approved plan".into(),
                completion_condition: "Plan is complete".into(),
                ..Default::default()
            }],
            ..Default::default()
        }];

        assert!(validate_workflow_groups(&preferences).is_ok());
    }

    #[test]
    fn workflow_rejects_the_same_terminal_twice() {
        let mut preferences = Preferences::default();
        let step = domain::WorkflowStepDefinition {
            id: "step-1".into(),
            session_native_id: "thread-1".into(),
            role: domain::WorkflowRole::Implementer,
            ..Default::default()
        };
        preferences.workflow_groups = vec![domain::WorkflowGroupDefinition {
            id: "workflow-group-1".into(),
            terminal_group_id: "terminal-group-1".into(),
            steps: vec![
                step.clone(),
                domain::WorkflowStepDefinition {
                    id: "step-2".into(),
                    ..step
                },
            ],
            ..Default::default()
        }];

        assert!(validate_workflow_groups(&preferences).is_err());
    }

    #[test]
    fn workflow_connections_reference_unique_existing_steps() {
        let mut preferences = Preferences::default();
        let steps = ["planner", "implementer"]
            .into_iter()
            .map(|id| domain::WorkflowStepDefinition {
                id: id.into(),
                session_native_id: format!("thread-{id}"),
                ..Default::default()
            })
            .collect();
        preferences.workflow_groups = vec![domain::WorkflowGroupDefinition {
            id: "workflow-group-1".into(),
            terminal_group_id: "terminal-group-1".into(),
            steps,
            connections: vec![domain::WorkflowConnectionDefinition {
                id: "connection-1".into(),
                from_step_id: "planner".into(),
                to_step_id: "implementer".into(),
                include_response: true,
                ..Default::default()
            }],
        }];

        assert!(validate_workflow_groups(&preferences).is_ok());
        preferences.workflow_groups[0].connections[0].to_step_id = "missing".into();
        assert!(validate_workflow_groups(&preferences).is_err());
    }

    #[test]
    fn workflow_connections_reject_cycles_by_default() {
        let mut preferences = Preferences::default();
        preferences.workflow_groups = vec![domain::WorkflowGroupDefinition {
            id: "workflow-group-1".into(),
            terminal_group_id: "terminal-group-1".into(),
            steps: ["one", "two"]
                .into_iter()
                .map(|id| domain::WorkflowStepDefinition {
                    id: id.into(),
                    session_native_id: format!("thread-{id}"),
                    ..Default::default()
                })
                .collect(),
            connections: vec![
                domain::WorkflowConnectionDefinition {
                    id: "forward".into(),
                    from_step_id: "one".into(),
                    to_step_id: "two".into(),
                    include_response: true,
                    ..Default::default()
                },
                domain::WorkflowConnectionDefinition {
                    id: "back".into(),
                    from_step_id: "two".into(),
                    to_step_id: "one".into(),
                    include_response: true,
                    ..Default::default()
                },
            ],
        }];

        assert!(validate_workflow_groups(&preferences).is_err());
    }

    #[test]
    fn reads_shortcut_action_from_secondary_instance_arguments() {
        let args = vec!["lume".into(), "shortcut".into(), "palette".into()];

        assert_eq!(shortcut_action_from_args(&args), Some("palette"));
    }

    #[test]
    fn prepared_codex_launch_is_visible_before_its_first_prompt() {
        let request = LaunchRequest {
            agent: IntegrationKind::Codex,
            working_directory: "/work/lume".into(),
            resume: true,
            resume_id: Some("thread-1".into()),
            target: "terminal".into(),
            initial_prompt: None,
            permission_mode: None,
            approval_policy: None,
            model: None,
            reasoning_effort: None,
        };
        let event = prepared_codex_session_event(
            &request,
            codex_bridge::PreparedThread {
                thread_id: "thread-1".into(),
                thread_name: Some("Lume principal".into()),
                permission_profile: domain::PermissionProfile {
                    mode: domain::AccessMode::WorkspaceWrite,
                    label: "Acesso ao projeto".into(),
                    approval_policy: "on-request".into(),
                    approvals_reviewer: None,
                    can_respond_from_lume: true,
                    available_actions: vec![PermissionAction::AllowOnce],
                },
            },
            None,
        );

        assert_eq!(event.native_session_id.as_deref(), Some("thread-1"));
        assert_eq!(event.session_name.as_deref(), Some("Lume principal"));
        assert_eq!(event.working_directory.as_deref(), Some("/work/lume"));
        assert_eq!(event.source, Some(SessionSource::Cli));
        assert!(event
            .permission_profile
            .is_some_and(|profile| profile.can_respond_from_lume));
    }

    #[test]
    fn active_codex_writer_has_an_actionable_resume_error() {
        let message =
            codex_resume_error("thread thread-1 already has an active writer (code -32600)".into());

        assert!(message.contains("still open in another Codex CLI or client"));
        assert!(!message.contains("-32600"));
    }

    #[test]
    fn resumed_session_starts_with_its_previous_agent_response() {
        let request = LaunchRequest {
            agent: IntegrationKind::Claude,
            working_directory: "/work/lume".into(),
            resume: true,
            resume_id: Some("claude-thread-1".into()),
            target: "terminal".into(),
            initial_prompt: None,
            permission_mode: None,
            approval_policy: None,
            model: None,
            reasoning_effort: None,
        };
        let event = prepared_resume_preview_event(
            &request,
            &integrations::ResumePreview {
                response: "Resposta anterior.".into(),
                updated_at: 42,
            },
        )
        .expect("prévia retomada");

        assert!(matches!(event.event, HookEventKind::Activity));
        assert_eq!(event.last_response.as_deref(), Some("Resposta anterior."));
        assert_eq!(event.activities.len(), 1);
        assert_eq!(event.activities[0].created_at, 42);
        assert_eq!(
            event.activities[0].detail.as_deref(),
            Some("Resposta anterior.")
        );
    }
}
