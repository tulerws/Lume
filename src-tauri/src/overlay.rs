#[cfg(target_os = "linux")]
mod linux {
    use std::{
        collections::HashMap,
        ffi::CString,
        sync::{Mutex, OnceLock},
    };

    use gtk::{
        gdk::prelude::MonitorExt,
        glib::translate::ToGlibPtr,
        prelude::{GtkWindowExt, WidgetExt},
    };
    use libloading::Library;
    use tauri::WebviewWindow;
    use x11_dl::xlib;

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

    struct X11Api {
        functions: xlib::Xlib,
        display: *mut xlib::Display,
    }

    unsafe impl Send for X11Api {}

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

    fn x11_api() -> Option<&'static Mutex<X11Api>> {
        static API: OnceLock<Option<Mutex<X11Api>>> = OnceLock::new();
        API.get_or_init(|| {
            let functions = xlib::Xlib::open().ok()?;
            let display = unsafe { (functions.XOpenDisplay)(std::ptr::null()) };
            if display.is_null() {
                return None;
            }
            Some(Mutex::new(X11Api { functions, display }))
        })
        .as_ref()
    }

    fn move_xwayland_frame(surface: &gtk::gdk::Window, x: i32, y: i32) -> bool {
        let Some(api) = x11_api() else {
            return false;
        };
        let Ok(api) = api.lock() else {
            return false;
        };

        unsafe {
            let surface_pointer: *mut gtk::gdk::ffi::GdkWindow = surface.to_glib_none().0;
            let client = gdkx11::ffi::gdk_x11_window_get_xid(surface_pointer.cast());
            let mut root = 0;
            let mut parent = 0;
            let mut children = std::ptr::null_mut();
            let mut child_count = 0;
            if client == 0
                || (api.functions.XQueryTree)(
                    api.display,
                    client,
                    &mut root,
                    &mut parent,
                    &mut children,
                    &mut child_count,
                ) == 0
            {
                return false;
            }
            if !children.is_null() {
                (api.functions.XFree)(children.cast());
            }

            let frame = if parent != 0 && parent != root {
                parent
            } else {
                client
            };
            let mut attributes: xlib::XSetWindowAttributes = std::mem::zeroed();
            attributes.override_redirect = xlib::True;
            (api.functions.XChangeWindowAttributes)(
                api.display,
                frame,
                xlib::CWOverrideRedirect,
                &mut attributes,
            );
            (api.functions.XMoveWindow)(api.display, frame, x, y);
            attributes.override_redirect = xlib::False;
            (api.functions.XChangeWindowAttributes)(
                api.display,
                frame,
                xlib::CWOverrideRedirect,
                &mut attributes,
            );
            (api.functions.XSync)(api.display, xlib::False);
        }
        true
    }

    pub fn xwayland_drag_target(window: &WebviewWindow) -> Option<super::XwaylandDragTarget> {
        if std::env::var("LUME_LINUX_BACKEND").ok().as_deref() != Some("xwayland-fallback") {
            return None;
        }
        let gtk_window = window.gtk_window().ok()?;
        let surface = gtk_window.window()?;
        let surface_pointer: *mut gtk::gdk::ffi::GdkWindow = surface.to_glib_none().0;
        let client = unsafe { gdkx11::ffi::gdk_x11_window_get_xid(surface_pointer.cast()) };
        (client != 0).then_some(super::XwaylandDragTarget {
            client: client as u64,
        })
    }

    pub fn drag_snapshot_target(target: super::XwaylandDragTarget) -> Option<(bool, i32, i32)> {
        let api = x11_api()?;
        let api = api.lock().ok()?;
        unsafe {
            let client = target.client as xlib::Window;
            let mut root = 0;
            let mut parent = 0;
            let mut children = std::ptr::null_mut();
            let mut child_count = 0;
            if client == 0
                || (api.functions.XQueryTree)(
                    api.display,
                    client,
                    &mut root,
                    &mut parent,
                    &mut children,
                    &mut child_count,
                ) == 0
            {
                return None;
            }
            if !children.is_null() {
                (api.functions.XFree)(children.cast());
            }
            let frame = if parent != 0 && parent != root {
                parent
            } else {
                client
            };
            let mut root_return = 0;
            let mut child_return = 0;
            let mut root_x = 0;
            let mut root_y = 0;
            let mut window_x = 0;
            let mut window_y = 0;
            let mut mask = 0;
            if (api.functions.XQueryPointer)(
                api.display,
                root,
                &mut root_return,
                &mut child_return,
                &mut root_x,
                &mut root_y,
                &mut window_x,
                &mut window_y,
                &mut mask,
            ) == 0
            {
                return None;
            }
            let mut frame_x = 0;
            let mut frame_y = 0;
            let mut translated_child = 0;
            if (api.functions.XTranslateCoordinates)(
                api.display,
                frame,
                root,
                0,
                0,
                &mut frame_x,
                &mut frame_y,
                &mut translated_child,
            ) == 0
            {
                return None;
            }
            Some((mask & xlib::Button1Mask != 0, frame_x, frame_y))
        }
    }

    pub fn drag_snapshot(window: &WebviewWindow) -> Option<(bool, i32, i32)> {
        drag_snapshot_target(xwayland_drag_target(window)?)
    }

    fn layer_monitors() -> &'static Mutex<HashMap<String, (String, f64)>> {
        static MONITORS: OnceLock<Mutex<HashMap<String, (String, f64)>>> = OnceLock::new();
        MONITORS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn layer_positions() -> &'static Mutex<HashMap<String, (i32, i32)>> {
        static POSITIONS: OnceLock<Mutex<HashMap<String, (i32, i32)>>> = OnceLock::new();
        POSITIONS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn remember_layer_position(window: &WebviewWindow, x: i32, y: i32) {
        if let Ok(mut positions) = layer_positions().lock() {
            positions.insert(window.label().to_string(), (x, y));
        }
    }

    pub fn layer_position(window: &WebviewWindow) -> Option<(i32, i32)> {
        layer_positions()
            .lock()
            .ok()
            .and_then(|positions| positions.get(window.label()).copied())
    }

    fn remember_layer_monitor(window: &WebviewWindow, monitor_id: Option<&str>, scale: f64) {
        if let Ok(mut monitors) = layer_monitors().lock() {
            monitors.insert(
                window.label().to_string(),
                (monitor_id.unwrap_or_default().to_string(), scale.max(1.0)),
            );
        }
    }

    fn cached_layer_scale(window: &WebviewWindow, monitor_id: Option<&str>) -> Option<f64> {
        let requested = monitor_id.unwrap_or_default();
        layer_monitors()
            .lock()
            .ok()
            .and_then(|monitors| monitors.get(window.label()).cloned())
            .filter(|(configured, _)| configured == requested)
            .map(|(_, scale)| scale)
    }

    fn rounded_window_region(width: i32, height: i32, radius: i32) -> gtk::cairo::Region {
        let radius = radius.clamp(0, width.min(height) / 2);
        let radius_squared = f64::from(radius * radius);
        let rectangles = (0..height)
            .map(|y| {
                let edge_distance = if y < radius {
                    f64::from(radius - y) - 0.5
                } else if y >= height - radius {
                    f64::from(y - (height - radius)) + 0.5
                } else {
                    0.0
                };
                let inset = if edge_distance > 0.0 {
                    (f64::from(radius) - (radius_squared - edge_distance.powi(2)).sqrt()).ceil()
                        as i32
                } else {
                    0
                };
                gtk::cairo::RectangleInt::new(inset, y, (width - inset * 2).max(1), 1)
            })
            .collect::<Vec<_>>();
        gtk::cairo::Region::create_rectangles(&rectangles)
    }

    fn shape_xwayland_window(
        gtk_window: &gtk::ApplicationWindow,
        width: i32,
        height: i32,
        radius: i32,
    ) {
        if std::env::var("LUME_LINUX_BACKEND").ok().as_deref() != Some("xwayland-fallback") {
            return;
        }
        let Some(surface) = gtk_window.window() else {
            return;
        };
        let region = rounded_window_region(width, height, radius);
        surface.shape_combine_region(Some(&region), 0, 0);
        surface.input_shape_combine_region(&region, 0, 0);
    }

    pub fn forget_window(label: &str) {
        if let Ok(mut monitors) = layer_monitors().lock() {
            monitors.remove(label);
        }
        if let Ok(mut positions) = layer_positions().lock() {
            positions.remove(label);
        }
    }

    pub fn resize_surface(window: &WebviewWindow, width: i32, height: i32) -> bool {
        let width = width.max(1);
        let height = height.max(1);
        let allocation = gtk::Allocation::new(0, 0, width, height);
        let webview_resized = window
            .with_webview(move |webview| {
                let widget = webview.inner();
                widget.set_size_request(width, height);
                widget.size_allocate(&allocation);
                let mut parent = widget.parent();
                while let Some(container) = parent {
                    parent = container.parent();
                    let allocation = gtk::Allocation::new(0, 0, width, height);
                    container.set_size_request(width, height);
                    container.size_allocate(&allocation);
                }
            })
            .is_ok();
        let Ok(gtk_window) = window.gtk_window() else {
            return false;
        };
        if std::env::var("LUME_LINUX_BACKEND").ok().as_deref() == Some("xwayland-fallback") {
            // An unresizable GTK window publishes its current dimensions as
            // fixed X11 size hints. Keep the undecorated fallback resizable so
            // the capsule can shrink and expand frame by frame.
            gtk_window.set_resizable(true);
        }
        gtk_window.resize(width, height);
        let radius = if window.label().starts_with("terminal-") {
            17
        } else {
            let expansion = (f64::from(width - 78) / f64::from(392 - 78)).clamp(0.0, 1.0);
            (22.0 - expansion).round() as i32
        };
        shape_xwayland_window(&gtk_window, width, height, radius);
        webview_resized
    }

    fn monitor_index(window: &WebviewWindow, monitor_id: Option<&str>) -> Option<usize> {
        let id = monitor_id?;
        let monitors = window.available_monitors().ok()?;
        if let Some((index, expected_name)) = id.split_once(':') {
            let index = index.parse::<usize>().ok()?;
            let monitor = monitors.get(index)?;
            let actual_name = monitor.name().map(String::as_str).unwrap_or("");
            if expected_name.is_empty() || expected_name == actual_name {
                return Some(index);
            }
        }
        monitors.iter().position(|monitor| {
            monitor
                .name()
                .as_ref()
                .is_some_and(|name| name.as_str() == id)
        })
    }

    fn configure_layer(
        window: &WebviewWindow,
        show_over_fullscreen: bool,
        monitor_id: Option<&str>,
        position_x: Option<i32>,
        position_y: Option<i32>,
        namespace: &str,
    ) -> bool {
        if std::env::var("LUME_LINUX_BACKEND").ok().as_deref() == Some("xwayland-fallback") {
            if let Ok(gtk_window) = window.gtk_window() {
                gtk_window.set_type_hint(gtk::gdk::WindowTypeHint::Dock);
                gtk_window.set_decorated(false);
                gtk_window.set_keep_above(true);
                gtk_window.set_skip_taskbar_hint(true);
            }
            return false;
        }
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
            let mut layer_scale = window.scale_factor().unwrap_or(1.0).max(1.0);
            if let Some(display) = gtk::gdk::Display::default() {
                let selected_index = monitor_index(window, monitor_id);
                let monitor = selected_index
                    .and_then(|index| display.monitor(index as i32))
                    .or_else(|| display.primary_monitor())
                    .or_else(|| display.monitor(0));
                if let Some(monitor) = monitor {
                    let geometry = monitor.geometry();
                    let workarea = monitor.workarea();
                    let scale = f64::from(monitor.scale_factor()).max(1.0);
                    layer_scale = scale;
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
            if let Ok(namespace) = CString::new(namespace) {
                (api.set_namespace)(pointer, namespace.as_ptr());
            }
            remember_layer_monitor(window, monitor_id, layer_scale);
            remember_layer_position(
                window,
                position_x.unwrap_or_else(|| {
                    (f64::from(left_margin.max(0)) * layer_scale).round() as i32
                }),
                position_y.unwrap_or_else(|| (f64::from(top_margin) * layer_scale).round() as i32),
            );
        }
        true
    }

    pub fn set_terminal_docked_shape(
        window: &WebviewWindow,
        width: i32,
        height: i32,
        docked: bool,
    ) -> bool {
        if std::env::var("LUME_LINUX_BACKEND").ok().as_deref() != Some("xwayland-fallback") {
            return false;
        }
        let Ok(gtk_window) = window.gtk_window() else {
            return false;
        };
        shape_xwayland_window(
            &gtk_window,
            width.max(1),
            height.max(1),
            if docked { 0 } else { 17 },
        );
        true
    }

    pub fn configure(
        window: &WebviewWindow,
        show_over_fullscreen: bool,
        monitor_id: Option<&str>,
        position_x: Option<i32>,
        position_y: Option<i32>,
    ) -> bool {
        configure_layer(
            window,
            show_over_fullscreen,
            monitor_id,
            position_x,
            position_y,
            "lume",
        )
    }

    pub fn configure_terminal(
        window: &WebviewWindow,
        show_over_fullscreen: bool,
        monitor_id: Option<&str>,
        position_x: i32,
        position_y: i32,
    ) -> bool {
        configure_layer(
            window,
            show_over_fullscreen,
            monitor_id,
            Some(position_x),
            Some(position_y),
            &format!("lume-{}", window.label()),
        )
    }

    pub fn move_to(window: &WebviewWindow, x: i32, y: i32, monitor_id: Option<&str>) -> bool {
        if std::env::var("LUME_LINUX_BACKEND").ok().as_deref() == Some("xwayland-fallback") {
            let Ok(gtk_window) = window.gtk_window() else {
                return false;
            };
            let monitors = window.available_monitors().unwrap_or_default();
            let primary = window.primary_monitor().ok().flatten();
            let Some(monitor) = super::select_monitor(&monitors, primary, monitor_id) else {
                return false;
            };
            let target_x = monitor.position().x + x;
            let target_y = monitor.position().y + y;
            if window.label().starts_with("terminal-") {
                if let Some(surface) = gtk_window.window() {
                    if !move_xwayland_frame(&surface, target_x, target_y) {
                        surface.move_(target_x, target_y);
                    }
                } else {
                    gtk_window.move_(target_x, target_y);
                }
            } else {
                gtk_window.move_(target_x, target_y);
            }
            return true;
        }
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
            if let Some(scale) = cached_layer_scale(window, monitor_id) {
                (api.set_margin)(pointer, 0, (f64::from(x) / scale).round() as i32);
                (api.set_margin)(pointer, 2, (f64::from(y) / scale).round() as i32);
                remember_layer_position(window, x, y);
                return true;
            }
            if let Some(display) = gtk::gdk::Display::default() {
                let selected_index = monitor_index(window, monitor_id);
                let monitor = selected_index
                    .and_then(|index| display.monitor(index as i32))
                    .or_else(|| display.primary_monitor())
                    .or_else(|| display.monitor(0));
                if let Some(monitor) = monitor {
                    (api.set_monitor)(pointer, monitor.to_glib_none().0);
                    let scale = f64::from(monitor.scale_factor()).max(1.0);
                    remember_layer_monitor(window, monitor_id, scale);
                    (api.set_margin)(pointer, 0, (f64::from(x) / scale).round() as i32);
                    (api.set_margin)(pointer, 2, (f64::from(y) / scale).round() as i32);
                    remember_layer_position(window, x, y);
                    return true;
                }
            }
            let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
            (api.set_margin)(pointer, 0, (f64::from(x) / scale).round() as i32);
            (api.set_margin)(pointer, 2, (f64::from(y) / scale).round() as i32);
            remember_layer_position(window, x, y);
        }
        true
    }
}

#[derive(Clone, Copy, Debug)]
pub struct XwaylandDragTarget {
    client: u64,
}

pub fn position(window: &tauri::WebviewWindow) -> Option<(i32, i32)> {
    #[cfg(target_os = "linux")]
    {
        linux::layer_position(window)
    }
    #[cfg(not(target_os = "linux"))]
    {
        window
            .outer_position()
            .ok()
            .map(|position| (position.x, position.y))
    }
}

pub fn forget_window(label: &str) {
    #[cfg(target_os = "linux")]
    linux::forget_window(label);
    #[cfg(not(target_os = "linux"))]
    let _ = label;
}

pub fn xwayland_drag_target(window: &tauri::WebviewWindow) -> Option<XwaylandDragTarget> {
    #[cfg(target_os = "linux")]
    {
        linux::xwayland_drag_target(window)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        None
    }
}

pub fn drag_snapshot_target(target: XwaylandDragTarget) -> Option<(bool, i32, i32)> {
    #[cfg(target_os = "linux")]
    {
        linux::drag_snapshot_target(target)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = target;
        None
    }
}

pub fn drag_snapshot(window: &tauri::WebviewWindow) -> Option<(bool, i32, i32)> {
    #[cfg(target_os = "linux")]
    {
        linux::drag_snapshot(window)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        None
    }
}

pub fn resize_surface(
    window: &tauri::WebviewWindow,
    width: i32,
    height: i32,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if linux::resize_surface(window, width, height) {
            return Ok(());
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, width, height);
    Ok(())
}

pub fn set_terminal_docked_shape(
    window: &tauri::WebviewWindow,
    width: i32,
    height: i32,
    docked: bool,
) {
    #[cfg(target_os = "linux")]
    {
        let _ = linux::set_terminal_docked_shape(window, width, height, docked);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, width, height, docked);
}

pub(crate) fn monitor_identifier(monitors: &[tauri::Monitor], monitor: &tauri::Monitor) -> String {
    let index = monitors
        .iter()
        .position(|candidate| {
            candidate.position() == monitor.position()
                && candidate.size() == monitor.size()
                && candidate.name() == monitor.name()
        })
        .unwrap_or(0);
    format!(
        "{index}:{}",
        monitor.name().map(String::as_str).unwrap_or_default()
    )
}

pub(crate) fn select_monitor(
    monitors: &[tauri::Monitor],
    primary: Option<tauri::Monitor>,
    monitor_id: Option<&str>,
) -> Option<tauri::Monitor> {
    if let Some(id) = monitor_id {
        if let Some((index, expected_name)) = id.split_once(':') {
            if let Ok(index) = index.parse::<usize>() {
                if let Some(monitor) = monitors.get(index) {
                    let actual_name = monitor.name().map(String::as_str).unwrap_or("");
                    if expected_name.is_empty() || expected_name == actual_name {
                        return Some(monitor.clone());
                    }
                }
            }
        }
        if let Some(monitor) = monitors
            .iter()
            .find(|monitor| monitor.name().is_some_and(|name| name == id))
        {
            return Some(monitor.clone());
        }
    }
    primary
        .or_else(|| {
            monitors
                .iter()
                .find(|monitor| monitor.position().x == 0)
                .cloned()
        })
        .or_else(|| monitors.first().cloned())
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

pub fn configure_terminal(
    window: &tauri::WebviewWindow,
    show_over_fullscreen: bool,
    monitor_id: Option<&str>,
    position_x: i32,
    position_y: i32,
) -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::configure_terminal(
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
    let monitor = select_monitor(&monitors, primary, monitor_id)
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
    let primary = window
        .primary_monitor()
        .map_err(|error| error.to_string())?;
    let monitor = select_monitor(&monitors, primary, monitor_id);
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
