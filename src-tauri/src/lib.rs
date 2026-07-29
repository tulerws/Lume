mod adapters;
mod agent_plugins;
mod browser_server;
mod codex_bridge;
mod codex_sessions;
mod control;
mod desktop_shortcuts;
mod discovery;
mod domain;
mod event_server;
mod executables;
mod integrations;
mod launcher;
mod mobile_gateway;
mod mobile_server;
mod overlay;
mod protocol;
mod state;
mod store;
mod terminal_windows;

use std::{collections::HashSet, io::Read, sync::Mutex};

use domain::{
    AgentKind, AgentSession, HistoryEntry, PermissionAction, Preferences, PromptAttachmentInput,
    PromptDelivery, QuestionAnswer, ResultNote,
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
    let native = register_global_shortcuts(app, preferences);
    match desktop_shortcuts::configure(preferences) {
        Ok(true) => Ok(()),
        Ok(false) => native,
        Err(desktop_error) => match native {
            Ok(()) => Err(desktop_error),
            Err(native_error) => Err(format!("{native_error}. {desktop_error}")),
        },
    }
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Result<Vec<AgentSession>, String> {
    state.sessions()
}

#[tauri::command]
fn rename_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    name: String,
) -> Result<String, String> {
    let name = state.rename_session(&session_id, &name)?;
    protocol::emit_sessions_changed(&app);
    Ok(name)
}

#[tauri::command]
fn get_hub_snapshot(state: State<'_, AppState>) -> Result<protocol::HubSnapshot, String> {
    Ok(protocol::HubSnapshot::new(state.sessions()?))
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
    server: State<'_, mobile_server::MobileServer>,
) -> Result<mobile_server::MobileServerStatus, String> {
    server.enable_network()
}

#[tauri::command]
fn disable_mobile_gateway(
    server: State<'_, mobile_server::MobileServer>,
) -> Result<mobile_server::MobileServerStatus, String> {
    server.disable_network()
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
    request: protocol::HubCommandRequest,
) -> protocol::HubCommandResponse {
    control::execute_hub_command(
        &app,
        state.inner(),
        bridge.inner(),
        browser.inner(),
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
fn submit_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    browser: State<'_, browser_server::BrowserControl>,
    session_id: String,
    prompt: String,
    attachments: Vec<PromptAttachmentInput>,
    delivery: Option<PromptDelivery>,
) -> Result<(), String> {
    control::submit_prompt(
        &app,
        state.inner(),
        bridge.inner(),
        browser.inner(),
        &session_id,
        &prompt,
        attachments,
        delivery.unwrap_or_default(),
        true,
    )
}

#[tauri::command]
fn read_local_image_data_url(path: String) -> Result<String, String> {
    control::local_image_data_url(&path)
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
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
    overlay::set_file_dialog_active(&window, active, state.preferences()?.show_over_fullscreen)
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
fn interrupt_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    session_id: String,
) -> Result<(), String> {
    control::interrupt_prompt(&app, state.inner(), bridge.inner(), &session_id)
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
fn get_preferences(state: State<'_, AppState>) -> Result<Preferences, String> {
    state.preferences()
}

#[tauri::command]
fn display_backend() -> &'static str {
    #[cfg(target_os = "linux")]
    match std::env::var("LUME_LINUX_BACKEND").ok().as_deref() {
        Some("xwayland-fallback") => return "xwayland-fallback",
        Some("native-gnome") => return "native-gnome",
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

#[tauri::command]
fn set_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    preferences: Preferences,
) -> Result<(), String> {
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

#[tauri::command]
// Keep this command synchronous: terminal creation configures GTK and must run on its main thread.
fn open_terminal_window(
    app: AppHandle,
    state: State<'_, AppState>,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    session_id: String,
) -> Result<String, String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Sessão não encontrada".to_string())?;
    let preferences = state.preferences()?;
    terminals.open(
        &app,
        &session,
        preferences.monitor_id.as_deref(),
        preferences.overlay_x.unwrap_or(40),
        preferences.overlay_y.unwrap_or(44),
        preferences.show_over_fullscreen,
    )
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
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.resize_window(&app, &label, x, y, width, height)
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

#[tauri::command]
fn undock_terminal_window(
    app: AppHandle,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    label: String,
) -> Result<terminal_windows::TerminalWindowState, String> {
    terminals.undock(&app, &label)
}

#[tauri::command]
fn restore_terminal_layout(
    app: AppHandle,
    state: State<'_, AppState>,
    terminals: State<'_, terminal_windows::TerminalWindows>,
    entries: Vec<terminal_windows::RestoredTerminalPlacement>,
) -> Result<Vec<terminal_windows::TerminalWindowState>, String> {
    let monitor_id = state.preferences()?.monitor_id;
    terminals.restore_layout(&app, entries, monitor_id.as_deref())
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
        .sessions()?
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
        .sessions()?
        .into_iter()
        .filter(|session| {
            matches!(
                (&kind, &session.agent),
                (IntegrationKind::Codex, domain::AgentKind::Codex)
                    | (IntegrationKind::Claude, domain::AgentKind::Claude)
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
fn launch_session(
    app: AppHandle,
    bridge: State<'_, codex_bridge::CodexBridge>,
    request: LaunchRequest,
) -> Result<(), String> {
    let executable = integrations::lume_executable()?;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let codex_remote = if request.agent == IntegrationKind::Codex {
        bridge.ensure_server()?;
        Some(codex_bridge::PROXY_URL)
    } else {
        None
    };
    launcher::launch(request, &executable, &app_data_dir, codex_remote)
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
            let state = AppState::new(&database_path)?;
            app.manage(PendingShortcutAction(Mutex::new(
                startup_shortcut_action.clone(),
            )));
            let _ = apply_global_shortcuts(app.handle(), &state.preferences()?);
            app.manage(state.clone());
            let codex_bridge =
                codex_bridge::CodexBridge::start(state.clone(), app.handle().clone())?;
            app.manage(codex_bridge);
            codex_sessions::start(state.clone(), app.handle().clone())?;
            event_server::start(state.clone(), app.handle().clone())?;
            let browser_control = browser_server::BrowserControl::default();
            browser_server::start(state.clone(), app.handle().clone(), browser_control.clone())?;
            app.manage(browser_control);
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
            app.manage(mobile_gateway);
            app.manage(mobile_server);
            app.manage(terminal_windows::TerminalWindows::default());
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
            set_terminal_file_dialog_active,
            refresh_agent_rate_limits,
            interrupt_prompt,
            steer_queued_prompt,
            terminate_session,
            list_history,
            list_result_notes,
            save_result_note,
            delete_result_note,
            get_preferences,
            take_pending_shortcut_action,
            display_backend,
            get_overlay_position,
            set_preferences,
            move_overlay,
            resize_overlay_surface,
            open_terminal_window,
            list_terminal_windows,
            set_terminal_windows_visible,
            get_terminal_window_state,
            close_terminal_window,
            move_terminal_window,
            cancel_terminal_window_move,
            sync_terminal_window_position,
            terminal_drag_snapshot,
            begin_terminal_native_drag,
            resize_terminal_window,
            begin_layered_terminal_resize,
            finish_layered_terminal_resize,
            undock_terminal_window,
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
    fn reads_shortcut_action_from_secondary_instance_arguments() {
        let args = vec!["lume".into(), "shortcut".into(), "palette".into()];

        assert_eq!(shortcut_action_from_args(&args), Some("palette"));
    }
}
