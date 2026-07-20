mod domain;
mod state;

use domain::{AgentSession, PermissionAction};
use state::AppState;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
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
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Inicialização automática é o padrão e poderá ser desativada nas preferências.
            let _ = app.autolaunch().enable();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![list_sessions, resolve_permission])
        .run(tauri::generate_context!())
        .expect("erro ao executar o Lume");
}
