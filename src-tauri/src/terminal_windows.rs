use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{
    webview::PageLoadEvent, AppHandle, Emitter, LogicalSize, Manager, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

use crate::{domain::AgentSession, overlay};

const TERMINAL_WIDTH: i32 = 336;
const TERMINAL_HEIGHT: i32 = 286;
const TERMINAL_MIN_WIDTH: i32 = 300;
const TERMINAL_MIN_HEIGHT: i32 = 240;
const TERMINAL_MAX_WIDTH: i32 = 760;
const TERMINAL_MAX_HEIGHT: i32 = 640;
const DOCK_DISTANCE: i32 = 46;
const SCREEN_MARGIN: i32 = 12;
const DOCK_ANIMATION_STEPS: i32 = 9;
const MAX_REASONABLE_COORDINATE: i32 = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DockSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockPreview {
    pub target_label: String,
    pub side: DockSide,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DockPreviewEvent {
    moving_label: String,
    preview: Option<DockPreview>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalWindowState {
    pub label: String,
    pub session_id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub docked: bool,
    pub group_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredTerminalPlacement {
    pub session_id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub group_id: Option<String>,
}

#[derive(Clone, Debug)]
struct Placement {
    label: String,
    session_id: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
    group: Option<String>,
    ready: bool,
}

#[derive(Clone, Debug)]
struct DockPlan {
    score: i32,
    other_label: String,
    side: DockSide,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl DockPlan {
    fn preview(&self) -> DockPreview {
        DockPreview {
            target_label: self.other_label.clone(),
            side: self.side,
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Clone, Debug)]
struct WindowTransition {
    from: TerminalWindowState,
    to: TerminalWindowState,
}

#[derive(Debug)]
struct MoveUpdate {
    moving_labels: Vec<String>,
    current: TerminalWindowState,
    preview: Option<DockPreview>,
    snapped: bool,
    transitions: Vec<WindowTransition>,
}

impl Placement {
    fn state(&self) -> TerminalWindowState {
        TerminalWindowState {
            label: self.label.clone(),
            session_id: self.session_id.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            docked: self.group.is_some(),
            group_id: self.group.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub struct TerminalWindows {
    placements: Arc<Mutex<HashMap<String, Placement>>>,
    settling: Arc<Mutex<HashSet<String>>>,
}

impl TerminalWindows {
    pub fn open(
        &self,
        app: &AppHandle,
        session: &AgentSession,
        monitor_id: Option<&str>,
        origin_x: i32,
        origin_y: i32,
        _show_over_fullscreen: bool,
    ) -> Result<String, String> {
        let label = terminal_label(&session.id);
        if let Some(window) = app.get_webview_window(&label) {
            let ready = self
                .placements
                .lock()
                .ok()
                .and_then(|placements| placements.get(&label).map(|placement| placement.ready))
                .unwrap_or(false);
            if !ready {
                let _ = window.close();
                self.remove(&label);
                return Err(
                    "O mini terminal anterior não carregou; clique em Abrir novamente".into(),
                );
            } else {
                let current = self.state(&label)?;
                let (x, y) = initial_position(app, monitor_id, current.x, current.y);
                if let Ok(mut placements) = self.placements.lock() {
                    if let Some(placement) = placements.get_mut(&label) {
                        placement.x = x;
                        placement.y = y;
                    }
                }
                window.show().map_err(|error| error.to_string())?;
                let _ = window.set_focus();
                return Ok(label);
            }
        }

        let offset = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar os mini terminais".to_string())?
            .len() as i32
            * 22;
        let (x, y) = initial_position(
            app,
            monitor_id,
            origin_x + 96 + offset,
            origin_y + 64 + offset,
        );
        let placement = Placement {
            label: label.clone(),
            session_id: session.id.clone(),
            x,
            y,
            width: TERMINAL_WIDTH,
            height: TERMINAL_HEIGHT,
            scale: selected_monitor_scale(app, monitor_id),
            group: None,
            ready: false,
        };

        self.placements
            .lock()
            .map_err(|_| "Não foi possível guardar o mini terminal".to_string())?
            .insert(label.clone(), placement.clone());

        let ready_registry = self.clone();
        let ready_label = label.clone();
        let window =
            match WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
                .title(format!("Lume · {}", session.agent_label))
                .inner_size(f64::from(TERMINAL_WIDTH), f64::from(TERMINAL_HEIGHT))
                .position(f64::from(x), f64::from(y))
                .min_inner_size(
                    f64::from(TERMINAL_MIN_WIDTH),
                    f64::from(TERMINAL_MIN_HEIGHT),
                )
                .max_inner_size(
                    f64::from(TERMINAL_MAX_WIDTH),
                    f64::from(TERMINAL_MAX_HEIGHT),
                )
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .resizable(true)
                .visible(true)
                .on_page_load(move |window, payload| {
                    if matches!(payload.event(), PageLoadEvent::Finished) {
                        ready_registry.mark_ready(&ready_label);
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = window.emit("lume://terminal-windows-changed", ());
                    }
                })
                .build()
            {
                Ok(window) => window,
                Err(error) => {
                    self.remove(&label);
                    return Err(error.to_string());
                }
            };

        let registry = self.clone();
        let cleanup_label = label.clone();
        let event_window = window.clone();
        window.on_window_event(move |event| match event {
            WindowEvent::Destroyed => {
                emit_dock_preview(event_window.app_handle(), &cleanup_label, None);
                registry.remove(&cleanup_label);
            }
            WindowEvent::Resized(size) => {
                if registry.is_settling(&cleanup_label) {
                    return;
                }
                let scale = event_window.scale_factor().unwrap_or(1.0);
                registry.resize(
                    &cleanup_label,
                    (f64::from(size.width) / scale).round() as i32,
                    (f64::from(size.height) / scale).round() as i32,
                    scale,
                );
            }
            _ => {}
        });

        window.show().map_err(|error| {
            self.remove(&label);
            let _ = window.close();
            error.to_string()
        })?;
        let _ = window.set_focus();
        Ok(label)
    }

    pub fn list(&self, app: &AppHandle) -> Result<Vec<TerminalWindowState>, String> {
        let placements = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar os mini terminais".to_string())?;
        Ok(placements
            .values()
            .filter(|placement| {
                placement.ready
                    && app
                        .get_webview_window(&placement.label)
                        .and_then(|window| window.is_visible().ok())
                        .unwrap_or(false)
            })
            .map(Placement::state)
            .collect())
    }

    pub fn state(&self, label: &str) -> Result<TerminalWindowState, String> {
        self.placements
            .lock()
            .map_err(|_| "Não foi possível acessar o mini terminal".to_string())?
            .get(label)
            .map(Placement::state)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())
    }

    pub fn close(&self, app: &AppHandle, label: &str) -> Result<(), String> {
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        emit_dock_preview(app, label, None);
        window.close().map_err(|error| error.to_string())?;
        self.remove(label);
        Ok(())
    }

    pub fn move_window(
        &self,
        app: &AppHandle,
        label: &str,
        x: i32,
        y: i32,
        finalize: bool,
        monitor_id: Option<&str>,
    ) -> Result<TerminalWindowState, String> {
        validate_coordinates(x, y)?;
        let update = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível mover o mini terminal".to_string())?;
            update_placements(&mut placements, label, x, y, finalize)?
        };
        emit_dock_preview(app, label, update.preview.clone());
        if update.snapped {
            self.animate_native_windows(app, update.transitions, monitor_id);
        } else {
            let states = self.states_for_labels(&update.moving_labels)?;
            move_native_windows(app, &states, monitor_id, true);
        }
        Ok(update.current)
    }

    pub fn sync_native_position(
        &self,
        app: &AppHandle,
        label: &str,
        physical_x: i32,
        physical_y: i32,
        finalize: bool,
        monitor_id: Option<&str>,
    ) -> Result<TerminalWindowState, String> {
        validate_coordinates(physical_x, physical_y)?;
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        let monitor = window
            .current_monitor()
            .map_err(|error| error.to_string())?;
        let monitor_position = monitor
            .as_ref()
            .map(|monitor| *monitor.position())
            .unwrap_or_default();
        let monitor_size = monitor.as_ref().map(|monitor| *monitor.size());
        let monitor_scale = monitor
            .as_ref()
            .map(|monitor| monitor.scale_factor())
            .unwrap_or(1.0);
        let mut x = physical_x - monitor_position.x;
        let mut y = physical_y - monitor_position.y;
        validate_coordinates(x, y)?;

        let update = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível sincronizar o mini terminal".to_string())?;
            if let (Some(size), Some(current)) = (monitor_size, placements.get(label)) {
                (x, y) = clamp_drag_to_monitor(
                    x,
                    y,
                    current.width,
                    current.height,
                    size.width as i32,
                    size.height as i32,
                    monitor_scale,
                );
            }
            if let Some(current) = placements.get_mut(label) {
                current.scale = monitor_scale;
            }
            update_placements(&mut placements, label, x, y, finalize)?
        };
        emit_dock_preview(app, label, update.preview.clone());
        if update.snapped {
            self.animate_native_windows(app, update.transitions, monitor_id);
        } else if finalize || update.moving_labels.len() > 1 {
            let states = self
                .states_for_labels(&update.moving_labels)?
                .into_iter()
                .filter(|state| finalize || state.label != label)
                .collect::<Vec<_>>();
            move_native_windows(app, &states, monitor_id, false);
        }
        Ok(update.current)
    }

    fn states_for_labels(&self, labels: &[String]) -> Result<Vec<TerminalWindowState>, String> {
        let placements = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar os mini terminais".to_string())?;
        Ok(placements
            .values()
            .filter(|placement| labels.contains(&placement.label))
            .map(Placement::state)
            .collect())
    }

    fn animate_native_windows(
        &self,
        app: &AppHandle,
        transitions: Vec<WindowTransition>,
        monitor_id: Option<&str>,
    ) {
        if transitions.is_empty() {
            return;
        }
        if let Ok(mut settling) = self.settling.lock() {
            settling.extend(
                transitions
                    .iter()
                    .map(|transition| transition.to.label.clone()),
            );
        }
        let settling = self.settling.clone();
        let app = app.clone();
        let monitor_id = monitor_id.map(str::to_string);
        let _ = std::thread::Builder::new()
            .name("lume-terminal-dock".into())
            .spawn(move || {
                for step in 1..=DOCK_ANIMATION_STEPS {
                    let progress = f64::from(step) / f64::from(DOCK_ANIMATION_STEPS);
                    let eased = 1.0 - (1.0 - progress).powi(3);
                    let states = transitions
                        .iter()
                        .map(|transition| interpolate_state(transition, eased))
                        .collect::<Vec<_>>();
                    move_native_windows(&app, &states, monitor_id.as_deref(), true);
                    std::thread::sleep(Duration::from_millis(16));
                }
                std::thread::sleep(Duration::from_millis(48));
                if let Ok(mut settling) = settling.lock() {
                    for transition in &transitions {
                        settling.remove(&transition.to.label);
                    }
                }
            });
    }

    pub fn undock(&self, label: &str) -> Result<TerminalWindowState, String> {
        let mut placements = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível desacoplar o mini terminal".to_string())?;
        let old_group = placements.get(label).and_then(|entry| entry.group.clone());
        let entry = placements
            .get_mut(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        entry.group = None;
        if let Some(group) = old_group {
            clear_single_member_group(&mut placements, &group);
        }
        placements
            .get(label)
            .map(Placement::state)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())
    }

    pub fn restore_layout(
        &self,
        app: &AppHandle,
        entries: Vec<RestoredTerminalPlacement>,
        monitor_id: Option<&str>,
    ) -> Result<Vec<TerminalWindowState>, String> {
        let states = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível restaurar o layout".to_string())?;
            for entry in entries {
                validate_coordinates(entry.x, entry.y)?;
                let Some(placement) = placements
                    .values_mut()
                    .find(|placement| placement.session_id == entry.session_id)
                else {
                    continue;
                };
                placement.x = entry.x;
                placement.y = entry.y;
                placement.width = entry.width.clamp(TERMINAL_MIN_WIDTH, TERMINAL_MAX_WIDTH);
                placement.height = entry.height.clamp(TERMINAL_MIN_HEIGHT, TERMINAL_MAX_HEIGHT);
                placement.group = entry.group_id;
            }
            placements
                .values()
                .map(Placement::state)
                .collect::<Vec<_>>()
        };

        for state in &states {
            let Some(window) = app.get_webview_window(&state.label) else {
                continue;
            };
            let target = state.clone();
            let monitor = monitor_id.map(str::to_string);
            let layout_window = window.clone();
            let _ = window.run_on_main_thread(move || {
                let _ = layout_window.set_size(LogicalSize::new(
                    f64::from(target.width),
                    f64::from(target.height),
                ));
                let _ = overlay::move_to(&layout_window, target.x, target.y, monitor.as_deref());
            });
        }
        Ok(states)
    }

    pub fn resize_window(
        &self,
        app: &AppHandle,
        label: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        monitor_id: Option<&str>,
    ) -> Result<TerminalWindowState, String> {
        let state = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível redimensionar o mini terminal".to_string())?;
            let old_group = placements.get(label).and_then(|entry| entry.group.clone());
            let entry = placements
                .get_mut(label)
                .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
            entry.x = x;
            entry.y = y;
            entry.width = width.clamp(TERMINAL_MIN_WIDTH, TERMINAL_MAX_WIDTH);
            entry.height = height.clamp(TERMINAL_MIN_HEIGHT, TERMINAL_MAX_HEIGHT);
            entry.group = None;
            let state = entry.state();
            if let Some(group) = old_group {
                clear_single_member_group(&mut placements, &group);
            }
            state
        };
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        let target = state.clone();
        let monitor = monitor_id.map(str::to_string);
        let resize_window = window.clone();
        window
            .run_on_main_thread(move || {
                let _ = resize_window.set_size(LogicalSize::new(
                    f64::from(target.width),
                    f64::from(target.height),
                ));
                let _ = overlay::move_to(&resize_window, target.x, target.y, monitor.as_deref());
            })
            .map_err(|error| error.to_string())?;
        Ok(state)
    }

    fn remove(&self, label: &str) {
        let Ok(mut placements) = self.placements.lock() else {
            return;
        };
        let group = placements.remove(label).and_then(|entry| entry.group);
        if let Some(group) = group {
            clear_single_member_group(&mut placements, &group);
        }
    }

    fn resize(&self, label: &str, width: i32, height: i32, scale: f64) {
        let Ok(mut placements) = self.placements.lock() else {
            return;
        };
        if let Some(entry) = placements.get_mut(label) {
            entry.width = width.clamp(TERMINAL_MIN_WIDTH, TERMINAL_MAX_WIDTH);
            entry.height = height.clamp(TERMINAL_MIN_HEIGHT, TERMINAL_MAX_HEIGHT);
            entry.scale = scale.max(1.0);
        }
    }

    fn is_settling(&self, label: &str) -> bool {
        self.settling
            .lock()
            .map(|settling| settling.contains(label))
            .unwrap_or(false)
    }

    fn mark_ready(&self, label: &str) {
        if let Ok(mut placements) = self.placements.lock() {
            if let Some(placement) = placements.get_mut(label) {
                placement.ready = true;
            }
        }
    }
}

fn initial_position(
    app: &AppHandle,
    monitor_id: Option<&str>,
    desired_x: i32,
    desired_y: i32,
) -> (i32, i32) {
    let Some(main) = app.get_webview_window("main") else {
        return (desired_x.max(SCREEN_MARGIN), desired_y.max(SCREEN_MARGIN));
    };
    let Ok(monitors) = main.available_monitors() else {
        return (desired_x.max(SCREEN_MARGIN), desired_y.max(SCREEN_MARGIN));
    };
    let monitor = monitor_id
        .and_then(|id| {
            monitors
                .iter()
                .find(|monitor| monitor.name().is_some_and(|name| name == id))
        })
        .or_else(|| monitors.iter().find(|monitor| monitor.position().x == 0))
        .or_else(|| monitors.first());
    let Some(monitor) = monitor else {
        return (desired_x.max(SCREEN_MARGIN), desired_y.max(SCREEN_MARGIN));
    };
    clamp_to_monitor(
        desired_x,
        desired_y,
        monitor.size().width as i32,
        monitor.size().height as i32,
        monitor.scale_factor(),
    )
}

fn selected_monitor_scale(app: &AppHandle, monitor_id: Option<&str>) -> f64 {
    let Some(main) = app.get_webview_window("main") else {
        return 1.0;
    };
    let Ok(monitors) = main.available_monitors() else {
        return 1.0;
    };
    monitor_id
        .and_then(|id| {
            monitors
                .iter()
                .find(|monitor| monitor.name().is_some_and(|name| name == id))
        })
        .or_else(|| monitors.iter().find(|monitor| monitor.position().x == 0))
        .or_else(|| monitors.first())
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1.0)
}

fn clamp_to_monitor(
    x: i32,
    y: i32,
    monitor_width: i32,
    monitor_height: i32,
    scale: f64,
) -> (i32, i32) {
    let physical_width = (f64::from(TERMINAL_WIDTH) * scale).round() as i32;
    let physical_height = (f64::from(TERMINAL_HEIGHT) * scale).round() as i32;
    let max_x = (monitor_width - physical_width - SCREEN_MARGIN).max(SCREEN_MARGIN);
    let max_y = (monitor_height - physical_height - SCREEN_MARGIN).max(SCREEN_MARGIN);
    (x.clamp(SCREEN_MARGIN, max_x), y.clamp(SCREEN_MARGIN, max_y))
}

fn clamp_drag_to_monitor(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    monitor_width: i32,
    monitor_height: i32,
    scale: f64,
) -> (i32, i32) {
    let physical_width = (f64::from(width) * scale).round() as i32;
    let physical_height = (f64::from(height) * scale).round() as i32;
    let max_x = (monitor_width - physical_width - SCREEN_MARGIN).max(SCREEN_MARGIN);
    let max_y = (monitor_height - physical_height - SCREEN_MARGIN).max(SCREEN_MARGIN);
    (x.clamp(SCREEN_MARGIN, max_x), y.clamp(SCREEN_MARGIN, max_y))
}

fn terminal_label(session_id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    format!("terminal-{:x}", hasher.finish())
}

fn group_labels(placements: &HashMap<String, Placement>, current: &Placement) -> Vec<String> {
    match &current.group {
        Some(group) => placements
            .values()
            .filter(|entry| entry.group.as_ref() == Some(group))
            .map(|entry| entry.label.clone())
            .collect(),
        None => vec![current.label.clone()],
    }
}

fn shift(placements: &mut HashMap<String, Placement>, labels: &[String], dx: i32, dy: i32) {
    for label in labels {
        if let Some(entry) = placements.get_mut(label) {
            entry.x += dx;
            entry.y += dy;
        }
    }
}

fn validate_coordinates(x: i32, y: i32) -> Result<(), String> {
    if x.abs() > MAX_REASONABLE_COORDINATE || y.abs() > MAX_REASONABLE_COORDINATE {
        return Err(
            "O compositor enviou uma posição inválida; o terminal foi mantido visível".into(),
        );
    }
    Ok(())
}

fn update_placements(
    placements: &mut HashMap<String, Placement>,
    label: &str,
    x: i32,
    y: i32,
    finalize: bool,
) -> Result<MoveUpdate, String> {
    let current = placements
        .get(label)
        .cloned()
        .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
    let moving_labels = group_labels(placements, &current);
    shift(placements, &moving_labels, x - current.x, y - current.y);

    let plan = dock_candidate(placements, label, &moving_labels);
    let mut transitions = Vec::new();
    let mut snapped = false;
    if finalize {
        if let Some(plan) = plan.as_ref() {
            let from = states_by_label(placements, &moving_labels);
            apply_dock_plan(placements, label, &moving_labels, plan);
            merge_groups(placements, &moving_labels, &plan.other_label);
            let to = states_by_label(placements, &moving_labels);
            transitions = transitions_between(from, to);
            snapped = true;
        }
    }

    let current = placements
        .get(label)
        .map(Placement::state)
        .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
    Ok(MoveUpdate {
        moving_labels,
        current,
        preview: if finalize {
            None
        } else {
            plan.map(|candidate| candidate.preview())
        },
        snapped,
        transitions,
    })
}

fn states_by_label(
    placements: &HashMap<String, Placement>,
    labels: &[String],
) -> HashMap<String, TerminalWindowState> {
    placements
        .values()
        .filter(|placement| labels.contains(&placement.label))
        .map(|placement| (placement.label.clone(), placement.state()))
        .collect()
}

fn transitions_between(
    from: HashMap<String, TerminalWindowState>,
    to: HashMap<String, TerminalWindowState>,
) -> Vec<WindowTransition> {
    to.into_iter()
        .filter_map(|(label, to)| {
            from.get(&label)
                .cloned()
                .map(|from| WindowTransition { from, to })
        })
        .collect()
}

fn apply_dock_plan(
    placements: &mut HashMap<String, Placement>,
    label: &str,
    moving_labels: &[String],
    plan: &DockPlan,
) {
    let Some(current) = placements.get(label).cloned() else {
        return;
    };
    shift(
        placements,
        moving_labels,
        plan.x - current.x,
        plan.y - current.y,
    );
    if moving_labels.len() == 1 {
        if let Some(entry) = placements.get_mut(label) {
            entry.width = plan.width.clamp(TERMINAL_MIN_WIDTH, TERMINAL_MAX_WIDTH);
            entry.height = plan.height.clamp(TERMINAL_MIN_HEIGHT, TERMINAL_MAX_HEIGHT);
        }
    }
}

fn dock_candidate(
    placements: &HashMap<String, Placement>,
    label: &str,
    moving_labels: &[String],
) -> Option<DockPlan> {
    let current = placements.get(label)?;
    placements
        .values()
        .filter(|other| !moving_labels.contains(&other.label))
        .filter_map(|other| snap(current, other))
        .filter(|plan| plan.score <= DOCK_DISTANCE)
        .min_by_key(|plan| plan.score)
}

fn snap(current: &Placement, other: &Placement) -> Option<DockPlan> {
    let current_width = physical_width(current);
    let current_height = physical_height(current);
    let other_width = physical_width(other);
    let other_height = physical_height(other);
    let vertical_overlap =
        (current.y + current_height).min(other.y + other_height) - current.y.max(other.y);
    let horizontal_overlap =
        (current.x + current_width).min(other.x + other_width) - current.x.max(other.x);
    let mut candidates = Vec::new();
    if vertical_overlap > 48 {
        candidates.push(DockPlan {
            score: (current.x + current_width - other.x).abs(),
            other_label: other.label.clone(),
            side: DockSide::Left,
            x: other.x - current_width,
            y: other.y,
            width: current.width,
            height: other.height,
        });
        candidates.push(DockPlan {
            score: (current.x - (other.x + other_width)).abs(),
            other_label: other.label.clone(),
            side: DockSide::Right,
            x: other.x + other_width,
            y: other.y,
            width: current.width,
            height: other.height,
        });
    }
    if horizontal_overlap > 48 {
        candidates.push(DockPlan {
            score: (current.y + current_height - other.y).abs(),
            other_label: other.label.clone(),
            side: DockSide::Top,
            x: other.x,
            y: other.y - current_height,
            width: other.width,
            height: current.height,
        });
        candidates.push(DockPlan {
            score: (current.y - (other.y + other_height)).abs(),
            other_label: other.label.clone(),
            side: DockSide::Bottom,
            x: other.x,
            y: other.y + other_height,
            width: other.width,
            height: current.height,
        });
    }
    candidates.into_iter().min_by_key(|plan| plan.score)
}

fn physical_width(placement: &Placement) -> i32 {
    (f64::from(placement.width) * placement.scale).round() as i32
}

fn physical_height(placement: &Placement) -> i32 {
    (f64::from(placement.height) * placement.scale).round() as i32
}

fn merge_groups(
    placements: &mut HashMap<String, Placement>,
    moving_labels: &[String],
    other_label: &str,
) {
    let other_group = placements
        .get(other_label)
        .and_then(|entry| entry.group.clone())
        .unwrap_or_else(|| other_label.to_string());
    let previous_group = placements
        .get(other_label)
        .and_then(|entry| entry.group.clone());
    for entry in placements.values_mut() {
        if moving_labels.contains(&entry.label)
            || entry.label == other_label
            || previous_group
                .as_ref()
                .is_some_and(|group| entry.group.as_ref() == Some(group))
        {
            entry.group = Some(other_group.clone());
        }
    }
}

fn clear_single_member_group(placements: &mut HashMap<String, Placement>, group: &str) {
    let members = placements
        .values()
        .filter(|entry| entry.group.as_deref() == Some(group))
        .map(|entry| entry.label.clone())
        .collect::<Vec<_>>();
    if members.len() == 1 {
        if let Some(entry) = placements.get_mut(&members[0]) {
            entry.group = None;
        }
    }
}

fn emit_dock_preview(app: &AppHandle, moving_label: &str, preview: Option<DockPreview>) {
    let _ = app.emit(
        "lume://terminal-dock-preview",
        DockPreviewEvent {
            moving_label: moving_label.to_string(),
            preview,
        },
    );
}

fn interpolate_state(transition: &WindowTransition, progress: f64) -> TerminalWindowState {
    let interpolate =
        |from: i32, to: i32| (f64::from(from) + f64::from(to - from) * progress).round() as i32;
    TerminalWindowState {
        label: transition.to.label.clone(),
        session_id: transition.to.session_id.clone(),
        x: interpolate(transition.from.x, transition.to.x),
        y: interpolate(transition.from.y, transition.to.y),
        width: interpolate(transition.from.width, transition.to.width),
        height: interpolate(transition.from.height, transition.to.height),
        docked: transition.to.docked,
        group_id: transition.to.group_id.clone(),
    }
}

fn move_native_windows(
    app: &AppHandle,
    states: &[TerminalWindowState],
    monitor_id: Option<&str>,
    resize: bool,
) {
    for state in states {
        let Some(window) = app.get_webview_window(&state.label) else {
            continue;
        };
        let target = state.clone();
        let monitor = monitor_id.map(str::to_string);
        let layer_window = window.clone();
        let _ = window.run_on_main_thread(move || {
            if resize {
                let _ = layer_window.set_size(LogicalSize::new(
                    f64::from(target.width),
                    f64::from(target.height),
                ));
            }
            let _ = overlay::move_to(&layer_window, target.x, target.y, monitor.as_deref());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(label: &str, x: i32, y: i32) -> Placement {
        Placement {
            label: label.into(),
            session_id: label.into(),
            x,
            y,
            width: TERMINAL_WIDTH,
            height: TERMINAL_HEIGHT,
            scale: 1.0,
            group: None,
            ready: true,
        }
    }

    #[test]
    fn nearby_terminals_snap_side_by_side() {
        let left = placement("left", 20, 40);
        let right = placement("right", 350, 48);
        let plan = snap(&left, &right).expect("encaixe");
        assert_eq!(plan.side, DockSide::Left);
        assert_eq!(plan.x + plan.width, right.x);
        assert_eq!(plan.y, right.y);
        assert_eq!(plan.height, right.height);
    }

    #[test]
    fn nearby_terminals_snap_one_above_the_other() {
        let top = placement("top", 40, 20);
        let bottom = placement("bottom", 48, 298);
        let plan = snap(&top, &bottom).expect("encaixe vertical");
        assert_eq!(plan.side, DockSide::Top);
        assert_eq!(plan.x, bottom.x);
        assert_eq!(plan.y + plan.height, bottom.y);
        assert_eq!(plan.width, bottom.width);
    }

    #[test]
    fn horizontal_dock_matches_the_target_height() {
        let mut left = placement("left", 20, 40);
        left.height = 420;
        let right = placement("right", 350, 48);
        let mut placements = HashMap::from([
            (left.label.clone(), left),
            (right.label.clone(), right.clone()),
        ]);

        let update = update_placements(&mut placements, "left", 20, 40, true).expect("encaixe");

        assert!(update.snapped);
        assert_eq!(update.current.height, right.height);
        assert_eq!(update.current.y, right.y);
        assert_eq!(update.current.x + update.current.width, right.x);
    }

    #[test]
    fn vertical_dock_matches_the_target_width() {
        let mut top = placement("top", 40, 20);
        top.width = 520;
        let bottom = placement("bottom", 48, 298);
        let mut placements = HashMap::from([
            (top.label.clone(), top),
            (bottom.label.clone(), bottom.clone()),
        ]);

        let update = update_placements(&mut placements, "top", 40, 20, true).expect("encaixe");

        assert!(update.snapped);
        assert_eq!(update.current.width, bottom.width);
        assert_eq!(update.current.x, bottom.x);
        assert_eq!(update.current.y + update.current.height, bottom.y);
    }

    #[test]
    fn scaled_terminals_detect_the_visual_gap_and_offer_a_preview() {
        let mut left = placement("left", 45, 100);
        left.scale = 1.25;
        let mut right = placement("right", 472, 106);
        right.scale = 1.25;
        let mut placements =
            HashMap::from([(left.label.clone(), left), (right.label.clone(), right)]);

        let update = update_placements(&mut placements, "right", 472, 106, false).expect("prévia");
        let preview = update.preview.expect("highlight de acoplamento");

        assert_eq!(preview.target_label, "left");
        assert_eq!(preview.side, DockSide::Right);
        assert_eq!(preview.x, 465);
    }

    #[test]
    fn rejects_compositor_outlier_coordinates() {
        assert!(validate_coordinates(100_000, 20).is_err());
    }

    #[test]
    fn terminal_opened_near_the_edge_stays_inside_the_monitor() {
        let (x, y) = clamp_to_monitor(1_920, 1_080, 1_920, 1_080, 1.0);
        assert_eq!(x, 1_572);
        assert_eq!(y, 782);
    }

    #[test]
    fn terminal_clamp_accounts_for_windows_display_scaling() {
        let (x, y) = clamp_to_monitor(1_900, 1_050, 1_920, 1_080, 1.25);
        assert_eq!(x, 1_488);
        assert_eq!(y, 710);
    }
}
