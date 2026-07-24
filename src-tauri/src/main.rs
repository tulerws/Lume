// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
fn should_use_xwayland_fallback(
    session_type: &str,
    desktop: &str,
    display: Option<&str>,
    force_native_wayland: bool,
) -> bool {
    let desktop_parts = desktop.split([':', ';']).map(str::trim).collect::<Vec<_>>();
    let is_gnome = desktop_parts.iter().any(|part| {
        part.eq_ignore_ascii_case("gnome") || part.to_lowercase().starts_with("gnome-")
    });
    let uses_native_gnome_backend = desktop_parts
        .iter()
        .any(|part| part.eq_ignore_ascii_case("ubuntu") || part.eq_ignore_ascii_case("pop"));
    session_type.eq_ignore_ascii_case("wayland")
        && is_gnome
        && !uses_native_gnome_backend
        && display.is_some_and(|value| !value.trim().is_empty())
        && !force_native_wayland
}

#[cfg(target_os = "linux")]
fn configure_linux_display_backend() {
    let force_native_wayland = std::env::var("LUME_FORCE_NATIVE_WAYLAND")
        .ok()
        .is_some_and(|value| matches!(value.to_lowercase().as_str(), "1" | "true" | "yes"));
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let display = std::env::var("DISPLAY").ok();

    if should_use_xwayland_fallback(
        &session_type,
        &desktop,
        display.as_deref(),
        force_native_wayland,
    ) {
        // GNOME Wayland does not expose the Layer Shell protocol and regular
        // windows cannot be positioned globally. XWayland keeps dragging and
        // persisted overlay coordinates functional on Fedora Workstation.
        std::env::set_var("GDK_BACKEND", "x11");
        std::env::set_var("LUME_LINUX_BACKEND", "xwayland-fallback");
        eprintln!("Lume: usando XWayland para posicionamento compatível com GNOME");
    }
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("hook") {
        let provider = args.get(2).map(String::as_str).unwrap_or("");
        std::process::exit(lume_lib::run_hook_client(provider));
    }
    if args.get(1).map(String::as_str) == Some("terminal-run") {
        let payload = args.get(2).map(String::as_str).unwrap_or("");
        std::process::exit(lume_lib::run_terminal_payload(payload));
    }
    if args.get(1).map(String::as_str) == Some("ingest") {
        std::process::exit(lume_lib::run_ingest_client());
    }
    #[cfg(target_os = "linux")]
    configure_linux_display_backend();
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        && std::env::var("XDG_SESSION_TYPE").ok().as_deref() == Some("wayland")
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    lume_lib::run()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::should_use_xwayland_fallback;

    #[test]
    fn uses_xwayland_on_fedora_gnome_wayland() {
        assert!(should_use_xwayland_fallback(
            "wayland",
            "GNOME",
            Some(":0"),
            false
        ));
    }

    #[test]
    fn keeps_native_backend_on_ubuntu_and_pop_gnome() {
        assert!(!should_use_xwayland_fallback(
            "wayland",
            "ubuntu:GNOME",
            Some(":1"),
            false
        ));
        assert!(!should_use_xwayland_fallback(
            "wayland",
            "pop:GNOME",
            Some(":1"),
            false
        ));
    }

    #[test]
    fn keeps_native_backend_when_layer_shell_can_be_available() {
        assert!(!should_use_xwayland_fallback(
            "wayland",
            "KDE",
            Some(":0"),
            false
        ));
        assert!(!should_use_xwayland_fallback(
            "wayland",
            "COSMIC",
            Some(":0"),
            false
        ));
    }

    #[test]
    fn respects_session_capabilities_and_native_override() {
        assert!(!should_use_xwayland_fallback(
            "x11",
            "GNOME",
            Some(":0"),
            false
        ));
        assert!(!should_use_xwayland_fallback(
            "wayland", "GNOME", None, false
        ));
        assert!(!should_use_xwayland_fallback(
            "wayland",
            "GNOME",
            Some(":0"),
            true
        ));
    }
}
