#[cfg(target_os = "linux")]
mod linux {
    use std::{ffi::CString, sync::OnceLock};

    use gtk::{gdk::prelude::MonitorExt, glib::translate::ToGlibPtr};
    use libloading::Library;
    use tauri::WebviewWindow;

    type GtkWindow = *mut gtk::ffi::GtkWindow;

    struct LayerApi {
        _library: Library,
        is_supported: unsafe extern "C" fn() -> i32,
        is_layer_window: unsafe extern "C" fn(GtkWindow) -> i32,
        init: unsafe extern "C" fn(GtkWindow),
        set_layer: unsafe extern "C" fn(GtkWindow, i32),
        set_anchor: unsafe extern "C" fn(GtkWindow, i32, i32),
        set_margin: unsafe extern "C" fn(GtkWindow, i32, i32),
        set_exclusive_zone: unsafe extern "C" fn(GtkWindow, i32),
        set_keyboard_mode: unsafe extern "C" fn(GtkWindow, i32),
        set_monitor: unsafe extern "C" fn(GtkWindow, *mut gtk::gdk::ffi::GdkMonitor),
        set_namespace: unsafe extern "C" fn(GtkWindow, *const std::ffi::c_char),
    }

    unsafe impl Send for LayerApi {}
    unsafe impl Sync for LayerApi {}

    impl LayerApi {
        unsafe fn load() -> Option<Self> {
            let library = unsafe {
                Library::new("libgtk-layer-shell.so.0")
                    .or_else(|_| Library::new("libgtk-layer-shell.so"))
                    .ok()?
            };
            macro_rules! symbol {
                ($name:literal, $kind:ty) => {{
                    let symbol = unsafe { library.get::<$kind>($name).ok()? };
                    *symbol
                }};
            }
            Some(Self {
                is_supported: symbol!(b"gtk_layer_is_supported\0", unsafe extern "C" fn() -> i32),
                is_layer_window: symbol!(
                    b"gtk_layer_is_layer_window\0",
                    unsafe extern "C" fn(GtkWindow) -> i32
                ),
                init: symbol!(
                    b"gtk_layer_init_for_window\0",
                    unsafe extern "C" fn(GtkWindow)
                ),
                set_layer: symbol!(
                    b"gtk_layer_set_layer\0",
                    unsafe extern "C" fn(GtkWindow, i32)
                ),
                set_anchor: symbol!(
                    b"gtk_layer_set_anchor\0",
                    unsafe extern "C" fn(GtkWindow, i32, i32)
                ),
                set_margin: symbol!(
                    b"gtk_layer_set_margin\0",
                    unsafe extern "C" fn(GtkWindow, i32, i32)
                ),
                set_exclusive_zone: symbol!(
                    b"gtk_layer_set_exclusive_zone\0",
                    unsafe extern "C" fn(GtkWindow, i32)
                ),
                set_keyboard_mode: symbol!(
                    b"gtk_layer_set_keyboard_mode\0",
                    unsafe extern "C" fn(GtkWindow, i32)
                ),
                set_monitor: symbol!(
                    b"gtk_layer_set_monitor\0",
                    unsafe extern "C" fn(GtkWindow, *mut gtk::gdk::ffi::GdkMonitor)
                ),
                set_namespace: symbol!(
                    b"gtk_layer_set_namespace\0",
                    unsafe extern "C" fn(GtkWindow, *const std::ffi::c_char)
                ),
                _library: library,
            })
        }
    }

    fn api() -> Option<&'static LayerApi> {
        static API: OnceLock<Option<LayerApi>> = OnceLock::new();
        API.get_or_init(|| unsafe { LayerApi::load() }).as_ref()
    }

    pub fn configure(
        window: &WebviewWindow,
        show_over_fullscreen: bool,
        monitor_id: Option<&str>,
        position_x: Option<i32>,
        position_y: Option<i32>,
    ) -> bool {
        if std::env::var("XDG_SESSION_TYPE").ok().as_deref() != Some("wayland") {
            return false;
        }
        let Some(api) = api() else {
            return false;
        };
        let Ok(gtk_window) = window.gtk_window() else {
            return false;
        };
        unsafe {
            if (api.is_supported)() == 0 {
                return false;
            }
            let application_pointer: *mut gtk::ffi::GtkApplicationWindow =
                gtk_window.to_glib_none().0;
            let pointer = application_pointer.cast::<gtk::ffi::GtkWindow>();
            if (api.is_layer_window)(pointer) == 0 {
                (api.init)(pointer);
            }
            let mut left_margin = 0;
            let mut top_margin = 12;
            if let Some(display) = gtk::gdk::Display::default() {
                let selected_index = monitor_id.and_then(|id| {
                    window.available_monitors().ok().and_then(|monitors| {
                        monitors.iter().position(|monitor| {
                            monitor
                                .name()
                                .as_ref()
                                .is_some_and(|name| name.as_str() == id)
                        })
                    })
                });
                let monitor = selected_index
                    .and_then(|index| display.monitor(index as i32))
                    .or_else(|| display.primary_monitor());
                if let Some(monitor) = monitor {
                    let geometry = monitor.geometry();
                    let workarea = monitor.workarea();
                    let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
                    let window_width = window
                        .outer_size()
                        .map(|size| (f64::from(size.width) / scale).round() as i32)
                        .unwrap_or(78);
                    left_margin = position_x
                        .map(|value| (f64::from(value) / scale).round() as i32)
                        .unwrap_or_else(|| ((geometry.width() - window_width) / 2).max(0));
                    top_margin = (workarea.y() - geometry.y() + 12).max(12);
                    if let Some(value) = position_y {
                        top_margin = (f64::from(value) / scale).round() as i32;
                    }
                    (api.set_monitor)(pointer, monitor.to_glib_none().0);
                }
            }
            let desktop = std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .to_lowercase();
            if position_y.is_none() && (desktop.contains("gnome") || desktop.contains("cosmic")) {
                top_margin = top_margin.max(44);
            }
            // Top stays below fullscreen surfaces; Overlay is an explicit opt-in.
            (api.set_layer)(pointer, if show_over_fullscreen { 3 } else { 2 });
            (api.set_anchor)(pointer, 0, 1);
            (api.set_anchor)(pointer, 1, 0);
            (api.set_anchor)(pointer, 2, 1);
            (api.set_anchor)(pointer, 3, 0);
            (api.set_margin)(pointer, 0, left_margin.max(0));
            (api.set_margin)(pointer, 2, top_margin);
            (api.set_exclusive_zone)(pointer, -1);
            (api.set_keyboard_mode)(pointer, 2);
            if let Ok(namespace) = CString::new("lume") {
                (api.set_namespace)(pointer, namespace.as_ptr());
            }
        }
        true
    }

    pub fn move_to(window: &WebviewWindow, x: i32, y: i32, monitor_id: Option<&str>) -> bool {
        if std::env::var("XDG_SESSION_TYPE").ok().as_deref() != Some("wayland") {
            return false;
        }
        let Some(api) = api() else {
            return false;
        };
        let Ok(gtk_window) = window.gtk_window() else {
            return false;
        };
        unsafe {
            let application_pointer: *mut gtk::ffi::GtkApplicationWindow =
                gtk_window.to_glib_none().0;
            let pointer = application_pointer.cast::<gtk::ffi::GtkWindow>();
            if (api.is_layer_window)(pointer) == 0 {
                return false;
            }
            if let Some(display) = gtk::gdk::Display::default() {
                let selected_index = monitor_id.and_then(|id| {
                    window.available_monitors().ok().and_then(|monitors| {
                        monitors.iter().position(|monitor| {
                            monitor
                                .name()
                                .as_ref()
                                .is_some_and(|name| name.as_str() == id)
                        })
                    })
                });
                let monitor = selected_index
                    .and_then(|index| display.monitor(index as i32))
                    .or_else(|| display.primary_monitor());
                if let Some(monitor) = monitor {
                    (api.set_monitor)(pointer, monitor.to_glib_none().0);
                }
            }
            let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
            (api.set_anchor)(pointer, 0, 1);
            (api.set_anchor)(pointer, 1, 0);
            (api.set_anchor)(pointer, 2, 1);
            (api.set_anchor)(pointer, 3, 0);
            (api.set_margin)(pointer, 0, (f64::from(x) / scale).round() as i32);
            (api.set_margin)(pointer, 2, (f64::from(y) / scale).round() as i32);
        }
        true
    }
}

pub fn configure(
    window: &tauri::WebviewWindow,
    show_over_fullscreen: bool,
    monitor_id: Option<&str>,
    position_x: Option<i32>,
    position_y: Option<i32>,
) -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::configure(
            window,
            show_over_fullscreen,
            monitor_id,
            position_x,
            position_y,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            window,
            show_over_fullscreen,
            monitor_id,
            position_x,
            position_y,
        );
        false
    }
}

pub fn default_position(
    window: &tauri::WebviewWindow,
    monitor_id: Option<&str>,
) -> Result<(i32, i32), String> {
    let monitors = window
        .available_monitors()
        .map_err(|error| error.to_string())?;
    let primary = window
        .primary_monitor()
        .map_err(|error| error.to_string())?;
    let monitor = monitor_id
        .and_then(|id| {
            monitors
                .iter()
                .find(|monitor| monitor.name().is_some_and(|name| name == id))
        })
        .or(primary.as_ref())
        .or_else(|| monitors.iter().find(|monitor| monitor.position().x == 0))
        .or_else(|| monitors.first())
        .ok_or_else(|| "Nenhum monitor disponível".to_string())?;
    let window_size = window.outer_size().map_err(|error| error.to_string())?;
    let scale = monitor.scale_factor().max(1.0);
    let x = (i64::from(monitor.size().width) - i64::from(window_size.width))
        .div_euclid(2)
        .max(0) as i32;
    let top_inset = if cfg!(target_os = "linux") { 44 } else { 12 };
    let y = (f64::from(top_inset) * scale).round() as i32;
    Ok((x, y))
}

pub fn move_to(
    window: &tauri::WebviewWindow,
    x: i32,
    y: i32,
    monitor_id: Option<&str>,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if linux::move_to(window, x, y, monitor_id) {
        return Ok(());
    }

    let monitors = window
        .available_monitors()
        .map_err(|error| error.to_string())?;
    let monitor = monitor_id
        .and_then(|id| {
            monitors
                .iter()
                .find(|monitor| monitor.name().is_some_and(|name| name == id))
        })
        .or_else(|| monitors.iter().find(|monitor| monitor.position().x == 0))
        .or_else(|| monitors.first());
    let Some(monitor) = monitor else {
        return Err("Nenhum monitor disponível".into());
    };
    window
        .set_position(tauri::PhysicalPosition::new(
            monitor.position().x + x,
            monitor.position().y + y,
        ))
        .map_err(|error| error.to_string())
}

pub fn start_fullscreen_guard(
    state: crate::state::AppState,
    app: tauri::AppHandle,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("lume-fullscreen-guard".into())
        .spawn(move || {
            let mut last_topmost = None;
            loop {
                let show_over_fullscreen = state
                    .preferences()
                    .map(|preferences| preferences.show_over_fullscreen)
                    .unwrap_or(false);
                if let Some(fullscreen) = foreground_is_fullscreen() {
                    let topmost = show_over_fullscreen || !fullscreen;
                    if last_topmost != Some(topmost) {
                        if let Some(window) = tauri::Manager::get_webview_window(&app, "main") {
                            let _ = window.set_always_on_top(topmost);
                        }
                        last_topmost = Some(topmost);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(900));
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn foreground_is_fullscreen() -> Option<bool> {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect},
    };
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return None;
        }
        let mut window_rect = RECT::default();
        if GetWindowRect(window, &mut window_rect) == 0 {
            return None;
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT::default(),
            rcWork: RECT::default(),
            dwFlags: 0,
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        Some(
            window_rect.left <= info.rcMonitor.left
                && window_rect.top <= info.rcMonitor.top
                && window_rect.right >= info.rcMonitor.right
                && window_rect.bottom >= info.rcMonitor.bottom,
        )
    }
}

#[cfg(target_os = "linux")]
fn foreground_is_fullscreen() -> Option<bool> {
    if std::env::var("XDG_SESSION_TYPE").ok().as_deref() != Some("x11") {
        return None;
    }
    let root = std::process::Command::new("xprop")
        .args(["-root", "_NET_ACTIVE_WINDOW"])
        .output()
        .ok()?;
    let root = String::from_utf8_lossy(&root.stdout);
    let window_id = root.split_whitespace().last()?;
    let state = std::process::Command::new("xprop")
        .args(["-id", window_id, "_NET_WM_STATE"])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&state.stdout).contains("_NET_WM_STATE_FULLSCREEN"))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn foreground_is_fullscreen() -> Option<bool> {
    None
}
