mod adapters;
mod agent_plugins;
mod browser_server;
mod codex_bridge;
mod codex_sessions;
mod discovery;
mod domain;
mod event_server;
mod executables;
mod integrations;
mod launcher;
mod overlay;
mod state;
mod store;
mod terminal_windows;

use std::io::Read;

use domain::{AgentSession, HistoryEntry, PermissionAction, Preferences, ResultNote};
use integrations::{CompanionStatus, IntegrationDiagnostic, IntegrationKind, IntegrationStatus};
use launcher::LaunchRequest;
use state::AppState;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;

fn reveal_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Result<Vec<AgentSession>, String> {
    state.sessions()
}

#[tauri::command]
fn resolve_permission(
    state: State<'_, AppState>,
    session_id: String,
    permission_id: String,
    action: PermissionAction,
) -> Result<(), String> {
    state.resolve_permission(&session_id, &permission_id, action)
}

#[tauri::command]
fn open_session_source(
    state: State<'_, AppState>,
    browser: State<'_, browser_server::BrowserControl>,
    session_id: String,
) -> Result<(), String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Sessão não encontrada".to_string())?;
    match session.source {
        domain::SessionSource::Web => browser.request_focus(session.id),
        domain::SessionSource::Vscode => {
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

#[tauri::command]
fn submit_prompt(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, codex_bridge::CodexBridge>,
    browser: State<'_, browser_server::BrowserControl>,
    session_id: String,
    prompt: String,
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
        domain::SessionStatus::Running | domain::SessionStatus::PermissionRequired
    ) {
        return Err("Aguarde o agente terminar antes de enviar outro prompt".into());
    }
    if session.source == domain::SessionSource::Web {
        browser.request_prompt(session.id.clone(), prompt.to_string())?;
        return browser.request_focus(session.id);
    }
    if session.agent == domain::AgentKind::Codex {
        let mut profile = session.permission_profile.clone();
        profile.can_respond_from_lume = true;
        profile.available_actions = vec![
            PermissionAction::AllowOnce,
            PermissionAction::AllowSession,
            PermissionAction::Deny,
        ];
        let thread_id = session
            .native_session_id
            .ok_or_else(|| "A sessão do Codex não informou a thread".to_string())?;
        return bridge.submit_prompt(&thread_id, prompt, profile, state.inner().clone(), app);
    }
    let agent = match session.agent {
        domain::AgentKind::Claude => IntegrationKind::Claude,
        domain::AgentKind::Gemini => IntegrationKind::Gemini,
        domain::AgentKind::Codex => unreachable!(),
        domain::AgentKind::Unknown => {
            return Err("Este agente não oferece retomada direta pelo Lume".into());
        }
    };
    let resume_id = session
        .native_session_id
        .ok_or_else(|| "A sessão não informou um identificador para retomada".to_string())?;
    let working_directory = session
        .working_directory
        .ok_or_else(|| "A sessão não informou a pasta do projeto".to_string())?;
    let preferences = state.preferences()?;
    let target = if session.source == domain::SessionSource::Vscode {
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
}

#[tauri::command]
fn terminate_session(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let session = state
        .sessions()?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or_else(|| "Sessão não encontrada".to_string())?;
    if session.source != domain::SessionSource::Cli {
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
    let _ = app.emit("lume://sessions-changed", ());
    Ok(())
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
fn set_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    preferences: Preferences,
) -> Result<(), String> {
    let previous = state.preferences()?;
    let overlay_configuration_changed = previous.monitor_id != preferences.monitor_id
        || previous.show_over_fullscreen != preferences.show_over_fullscreen;
    if preferences.autostart {
        app.autolaunch()
            .enable()
            .map_err(|error| error.to_string())?;
    } else {
        app.autolaunch()
            .disable()
            .map_err(|error| error.to_string())?;
    }
    state.save_preferences(&preferences)?;
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
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            reveal_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        reveal_main_window(app);
                        let _ = app.emit("lume://open-command-palette", ());
                    }
                })
                .build(),
        )
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
        .setup(|app| {
            let _ = app.global_shortcut().register(Shortcut::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::Space,
            ));
            if let Ok(executable) = integrations::lume_executable() {
                integrations::refresh_connected(&executable.to_string_lossy());
            }
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?
                .join("lume.sqlite3");
            let state = AppState::new(&database_path)?;
            app.manage(state.clone());
            let codex_bridge =
                codex_bridge::CodexBridge::start(state.clone(), app.handle().clone())?;
            app.manage(codex_bridge);
            codex_sessions::start(state.clone(), app.handle().clone())?;
            event_server::start(state.clone(), app.handle().clone())?;
            let browser_control = browser_server::BrowserControl::default();
            browser_server::start(state.clone(), app.handle().clone(), browser_control.clone())?;
            app.manage(browser_control);
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
            resolve_permission,
            open_session_source,
            submit_prompt,
            terminate_session,
            list_history,
            list_result_notes,
            save_result_note,
            delete_result_note,
            get_preferences,
            set_preferences,
            move_overlay,
            open_terminal_window,
            list_terminal_windows,
            get_terminal_window_state,
            close_terminal_window,
            move_terminal_window,
            cancel_terminal_window_move,
            sync_terminal_window_position,
            resize_terminal_window,
            begin_layered_terminal_resize,
            finish_layered_terminal_resize,
            undock_terminal_window,
            restore_terminal_layout,
            integration_statuses,
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
