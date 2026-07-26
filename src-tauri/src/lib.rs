mod adapters;
mod agent_plugins;
mod browser_server;
mod codex_bridge;
mod codex_sessions;
mod desktop_shortcuts;
mod discovery;
mod domain;
mod event_server;
mod executables;
mod integrations;
mod launcher;
mod overlay;
mod pairing;
mod qr_generator;
mod remote_identity;
mod remote_server;
mod session_mirror;
mod state;
mod store;
mod terminal_windows;

use std::{collections::HashSet, io::Read, sync::Mutex};

use domain::{
    AgentSession, HistoryEntry, PermissionAction, Preferences, PromptRefusal, ResultNote,
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
fn resolve_permission(
    state: State<'_, AppState>,
    session_id: String,
    permission_id: String,
    action: PermissionAction,
) -> Result<(), String> {
    // O webview continua recebendo texto: o `Display` de `PermissionDenial`
    // reproduz palavra por palavra o que esta função já devolvia.
    state
        .resolve_permission(&session_id, &permission_id, action)
        .map_err(|denial| denial.to_string())
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
    // Casca fina sobre `send_prompt`, que é a mesma rotina que o servidor remoto
    // chama. `None` na origem porque a ação nasceu aqui mesmo.
    send_prompt(&app, &state, &bridge, &browser, &session_id, &prompt, None)
        .map_err(|refusal| refusal.to_string())
}

/// As recusas que não dependem de nada além do estado das sessões.
///
/// Separada de [`send_prompt`] para poder ser testada: `send_prompt` exige um
/// `AppHandle`, que só existe dentro de um Tauri em execução. O que sobrou aqui
/// é justamente o que o protocolo expõe ao celular — `payload_too_large`,
/// `session_busy`, `session_not_found` — e roda com estado em memória.
fn accept_prompt(
    state: &AppState,
    session_id: &str,
    prompt: &str,
) -> Result<AgentSession, PromptRefusal> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(PromptRefusal::Empty);
    }
    // Em bytes, e não em caracteres: é o tamanho que trafega e o que a linha de
    // comando do agente vai receber.
    if prompt.len() > 16 * 1024 {
        return Err(PromptRefusal::TooLarge);
    }
    let session = state
        .sessions()
        .map_err(PromptRefusal::Internal)?
        .into_iter()
        .find(|session| session.id == session_id)
        .ok_or(PromptRefusal::SessionNotFound)?;
    if matches!(
        session.status,
        domain::SessionStatus::Running | domain::SessionStatus::PermissionRequired
    ) {
        return Err(PromptRefusal::SessionBusy);
    }
    Ok(session)
}

/// Envia um prompt para uma sessão. **Único caminho**, usado pela interface do
/// desktop e pelo controle remoto.
///
/// `origin` é o nome do aparelho quando a ação veio do celular, e entra no
/// rastro. Ausente significa que ela nasceu no próprio desktop.
///
/// Devolve `PromptRefusal` e não `String` porque o controle remoto traduz cada
/// motivo num código de protocolo que o aplicativo trata de forma diferente —
/// `session_busy` vale nova tentativa, `action_not_available` não. O `Display`
/// reproduz o texto de antes, então o webview não vê diferença.
#[allow(clippy::too_many_arguments)]
fn send_prompt(
    app: &AppHandle,
    state: &AppState,
    bridge: &codex_bridge::CodexBridge,
    browser: &browser_server::BrowserControl,
    session_id: &str,
    prompt: &str,
    origin: Option<&str>,
) -> Result<(), PromptRefusal> {
    let session = accept_prompt(state, session_id, prompt)?;
    let prompt = prompt.trim();
    let result = if session.source == domain::SessionSource::Web {
        browser
            .request_prompt(session.id.clone(), prompt.to_string())
            .map_err(PromptRefusal::Internal)?;
        browser.request_focus(session.id.clone())
    } else if session.agent == domain::AgentKind::Codex {
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
            .ok_or(PromptRefusal::CodexThreadMissing)?;
        bridge.submit_prompt(
            &thread_id,
            prompt,
            profile,
            state.clone(),
            app.clone(),
        )
    } else {
        let agent = match session.agent {
            domain::AgentKind::Claude => IntegrationKind::Claude,
            domain::AgentKind::Gemini => IntegrationKind::Gemini,
            domain::AgentKind::Codex => unreachable!(),
            domain::AgentKind::Unknown => {
                return Err(PromptRefusal::AgentWithoutResume);
            }
        };
        let resume_id = session
            .native_session_id
            .clone()
            .ok_or(PromptRefusal::ResumeIdMissing)?;
        let working_directory = session
            .working_directory
            .clone()
            .ok_or(PromptRefusal::WorkingDirectoryMissing)?;
        let preferences = state.preferences().map_err(PromptRefusal::Internal)?;
        let target = if session.source == domain::SessionSource::Vscode {
            "vscode".to_string()
        } else {
            preferences.launch_target
        };
        let executable = integrations::lume_executable().map_err(PromptRefusal::Internal)?;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| PromptRefusal::Internal(error.to_string()))?;
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
    result.map_err(PromptRefusal::Internal)?;
    // O rastro nomeia o aparelho quando a ação veio de fora. Sem isso, com dois
    // celulares pareados, "prompt enviado" não diz de onde.
    let title = match origin {
        Some(device) => format!("Prompt enviado pelo Lume ({device})"),
        None => "Prompt enviado pelo Lume".to_string(),
    };
    state
        .record_activity(
            &session.id,
            "prompt",
            &title,
            Some(prompt.to_string()),
            "completed",
            Vec::new(),
        )
        .map_err(PromptRefusal::Internal)?;
    // Serve o webview e o contador de revisão de uma vez.
    let _ = app.emit(remote_server::SESSIONS_CHANGED, ());
    Ok(())
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
fn remote_status(
    state: State<'_, AppState>,
    server: State<'_, remote_server::RemoteServer>,
) -> Result<domain::RemoteStatus, String> {
    Ok(domain::RemoteStatus {
        available: true,
        enabled: server.is_running(),
        port: remote_server::REMOTE_CONTROL_PORT,
        paired_devices: state.remote_device_count()?,
    })
}

/// Abre a janela de pareamento: garante o listener, gera o código e devolve o
/// QR já desenhado.
///
/// O listener sobe **antes** do código ser gerado. Na ordem inversa, existiria
/// um instante em que o QR na tela aponta para uma porta fechada.
/// A ponte entre uma conexão remota e o resto do Lume.
///
/// Guarda o `AppHandle` e busca o estado gerenciado na hora do uso, em vez de
/// clonar cada dependência na criação: o `AppHandle` já é o caminho oficial para
/// alcançá-las, e cópias antecipadas seriam mais uma coisa a manter em sincronia.
struct RemoteDesktop {
    app: AppHandle,
}

impl RemoteDesktop {
    fn boxed(app: AppHandle) -> Box<dyn remote_server::Desktop> {
        Box::new(Self { app })
    }
}

impl remote_server::Desktop for RemoteDesktop {
    fn announce(&self) {
        let _ = self.app.emit(remote_server::SESSIONS_CHANGED, ());
    }

    fn submit_prompt(
        &self,
        session_id: &str,
        prompt: &str,
        device: &str,
    ) -> Result<(), PromptRefusal> {
        // A mesma função que o comando do Tauri chama. Não há caminho remoto
        // paralelo: o que muda é só a atribuição no rastro.
        send_prompt(
            &self.app,
            &self.app.state::<AppState>(),
            &self.app.state::<codex_bridge::CodexBridge>(),
            &self.app.state::<browser_server::BrowserControl>(),
            session_id,
            prompt,
            Some(device),
        )
    }
}

#[tauri::command]
fn remote_pairing_start(
    app: AppHandle,
    state: State<'_, AppState>,
    server: State<'_, remote_server::RemoteServer>,
) -> Result<pairing::Invitation, String> {
    server.ensure_started(&app, &state, RemoteDesktop::boxed(app.clone()))?;

    let identity = remote_identity::RemoteIdentity::load_or_create(
        &remote_server::identity_directory(&app)?,
    )?;
    let code = server.pairing().begin()?;
    let hosts = remote_identity::local_addresses();
    let hostname = sysinfo::System::host_name().unwrap_or_else(|| "Lume".to_string());

    // A lista devolvida é a que sobreviveu ao orçamento de densidade, e é ela
    // que a tela exibe: oferecer para digitação um endereço que o QR não
    // carrega seria pior que não oferecer nenhum.
    let (uri, hosts) = pairing::invite_uri_within_budget(&pairing::Invite {
        code,
        fingerprint: identity.fingerprint(),
        port: remote_server::REMOTE_CONTROL_PORT,
        hosts,
        hostname: hostname.clone(),
    })?;

    Ok(pairing::Invitation {
        qr_svg: qr_generator::to_svg(&qr_generator::encode(&uri)?),
        hostname,
        hosts: hosts.iter().map(ToString::to_string).collect(),
        port: remote_server::REMOTE_CONTROL_PORT,
        expires_in_seconds: server
            .pairing()
            .remaining()
            .map(|remaining| remaining.as_secs())
            .unwrap_or(0),
    })
}

#[tauri::command]
fn remote_pairing_status(
    state: State<'_, AppState>,
    server: State<'_, remote_server::RemoteServer>,
) -> Result<pairing::PairingProgress, String> {
    let remaining = server.pairing().remaining();
    Ok(pairing::PairingProgress {
        active: remaining.is_some(),
        expires_in_seconds: remaining.map(|left| left.as_secs()).unwrap_or(0),
        paired_devices: state.remote_device_count()?,
    })
}

/// Fecha a janela do QR e, se não sobrou nada para servir, derruba a porta.
#[tauri::command]
fn remote_pairing_cancel(
    state: State<'_, AppState>,
    server: State<'_, remote_server::RemoteServer>,
) -> Result<(), String> {
    server.pairing().cancel();
    server.stop_if_idle(&state)
}

#[tauri::command]
fn remote_devices(state: State<'_, AppState>) -> Result<Vec<domain::RemoteDevice>, String> {
    state.remote_devices()
}

/// Revoga o aparelho. A linha some e, com ela, a única forma de o token voltar
/// a valer.
///
/// A conexão viva daquele aparelho cai sozinha em até um ciclo de ping, quando
/// o `keepalive` reconsulta a tabela e não se encontra mais nela. Revogado o
/// último, a porta é fechada.
#[tauri::command]
fn remote_revoke_device(
    state: State<'_, AppState>,
    server: State<'_, remote_server::RemoteServer>,
    id: String,
) -> Result<(), String> {
    state.revoke_remote_device(&id)?;
    server.stop_if_idle(&state)
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
            app.manage(terminal_windows::TerminalWindows::default());
            discovery::start(state.clone(), app.handle().clone())?;
            overlay::start_fullscreen_guard(state.clone(), app.handle().clone())?;
            // Gerenciado pelo Tauri porque os comandos do QR e o handshake
            // precisam da mesma instância: uma cópia teria outra sessão de
            // pareamento, e o código na tela nunca conferiria.
            app.manage(remote_server::RemoteServer::default());
            // Registrado no arranque e nunca removido, mesmo com o servidor
            // desligado: um contador que ninguém lê custa um incremento por
            // mudança de sessão, e amarrar seu ciclo de vida ao do listener
            // criaria a janela em que o servidor sobe antes de haver ouvinte.
            remote_server::watch_sessions(
                app.handle(),
                app.state::<remote_server::RemoteServer>().revision(),
            );
            // Sem aparelho pareado isto não abre porta alguma. Falha ao subir o
            // servidor remoto não pode impedir o Lume de abrir.
            if let Err(error) = remote_server::start_if_paired(
                app.handle(),
                &state,
                &app.state::<remote_server::RemoteServer>(),
                RemoteDesktop::boxed(app.handle().clone()),
            ) {
                eprintln!("Servidor remoto não iniciou: {error}");
            }

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
            take_pending_shortcut_action,
            display_backend,
            get_overlay_position,
            set_preferences,
            move_overlay,
            resize_overlay_surface,
            open_terminal_window,
            list_terminal_windows,
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
            diagnose_integration,
            configure_integration,
            vscode_status,
            configure_vscode,
            remote_status,
            remote_pairing_start,
            remote_pairing_status,
            remote_pairing_cancel,
            remote_devices,
            remote_revoke_device,
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
    use domain::{AgentKind, HookEvent, HookEventKind, SessionSource, SessionStatus};

    fn state_with_session(status: HookEventKind) -> AppState {
        let state = AppState::new(std::path::Path::new(":memory:")).expect("estado em memória");
        state
            .ingest(HookEvent {
                event: status,
                session_id: "s-1".into(),
                agent: AgentKind::Claude,
                agent_label: Some("Claude".into()),
                project: Some("Lume".into()),
                source: Some(SessionSource::Cli),
                source_app: None,
                status_label: Some("Sessão detectada".into()),
                started_at: None,
                process_id: None,
                native_session_id: Some("nativa-1".into()),
                working_directory: Some("/home/lume/projetos/Lume".into()),
                permission_profile: None,
                permission: None,
                last_response: None,
                activity: None,
                wait_for_decision: false,
            })
            .expect("sessão");
        state
    }

    #[test]
    fn a_prompt_needs_content() {
        let state = state_with_session(HookEventKind::WaitingForInput);

        assert_eq!(
            accept_prompt(&state, "s-1", "   \n  ").unwrap_err(),
            PromptRefusal::Empty
        );
    }

    #[test]
    fn a_prompt_above_sixteen_kilobytes_is_refused() {
        let state = state_with_session(HookEventKind::WaitingForInput);
        let limit = 16 * 1024;

        // Na borda exata ele passa; um byte além, não. O limite é em bytes
        // porque é o tamanho que trafega e o que a linha de comando recebe.
        assert!(accept_prompt(&state, "s-1", &"a".repeat(limit)).is_ok());
        assert_eq!(
            accept_prompt(&state, "s-1", &"a".repeat(limit + 1)).unwrap_err(),
            PromptRefusal::TooLarge
        );
    }

    #[test]
    fn a_prompt_for_an_unknown_session_is_refused() {
        let state = state_with_session(HookEventKind::WaitingForInput);

        assert_eq!(
            accept_prompt(&state, "fantasma", "rode os testes").unwrap_err(),
            PromptRefusal::SessionNotFound
        );
    }

    #[test]
    fn a_busy_agent_does_not_take_another_prompt() {
        let state = state_with_session(HookEventKind::Running);
        assert_eq!(
            state.sessions().expect("sessões")[0].status,
            SessionStatus::Running
        );

        // A regra já existia no desktop, e vale igual no remoto: dois prompts em
        // voo na mesma sessão embaralham a conversa do agente.
        assert_eq!(
            accept_prompt(&state, "s-1", "rode os testes").unwrap_err(),
            PromptRefusal::SessionBusy
        );
    }

    #[test]
    fn an_idle_session_accepts_the_prompt() {
        let state = state_with_session(HookEventKind::WaitingForInput);

        let accepted = accept_prompt(&state, "s-1", "  rode os testes  ").expect("aceito");
        assert_eq!(accepted.id, "s-1");
    }

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
