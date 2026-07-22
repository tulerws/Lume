use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
};

use serde::Serialize;
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
const DOCK_DISTANCE: i32 = 34;
const SCREEN_MARGIN: i32 = 12;

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
}

#[derive(Clone, Debug)]
struct Placement {
    label: String,
    session_id: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    group: Option<String>,
    ready: bool,
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
        }
    }
}

#[derive(Clone, Default)]
pub struct TerminalWindows {
    placements: Arc<Mutex<HashMap<String, Placement>>>,
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
            WindowEvent::Destroyed => registry.remove(&cleanup_label),
            WindowEvent::Resized(size) => {
                let scale = event_window.scale_factor().unwrap_or(1.0);
                registry.resize(
                    &cleanup_label,
                    (f64::from(size.width) / scale).round() as i32,
                    (f64::from(size.height) / scale).round() as i32,
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
        let (states, current) = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível mover o mini terminal".to_string())?;
            let current = placements
                .get(label)
                .cloned()
                .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
            let moving_labels = group_labels(&placements, &current);
            let dx = x - current.x;
            let dy = y - current.y;
            shift(&mut placements, &moving_labels, dx, dy);

            if finalize {
                if let Some((other_label, snap_x, snap_y)) =
                    dock_candidate(&placements, label, &moving_labels)
                {
                    shift(&mut placements, &moving_labels, snap_x, snap_y);
                    merge_groups(&mut placements, &moving_labels, &other_label);
                }
            }
            let current = placements
                .get(label)
                .map(Placement::state)
                .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
            let states = placements
                .values()
                .map(Placement::state)
                .collect::<Vec<_>>();
            (states, current)
        };
        move_native_windows(app, &states, monitor_id);
        Ok(current)
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
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        let monitor_position = window
            .current_monitor()
            .map_err(|error| error.to_string())?
            .map(|monitor| *monitor.position())
            .unwrap_or_default();
        let x = physical_x - monitor_position.x;
        let y = physical_y - monitor_position.y;

        let (states, current, snapped) = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível sincronizar o mini terminal".to_string())?;
            let current = placements
                .get(label)
                .cloned()
                .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
            let moving_labels = group_labels(&placements, &current);
            let dx = x - current.x;
            let dy = y - current.y;
            if dx == 0 && dy == 0 && !finalize {
                return Ok(current.state());
            }
            shift(&mut placements, &moving_labels, dx, dy);

            let mut snapped = false;
            if finalize {
                if let Some((other_label, snap_x, snap_y)) =
                    dock_candidate(&placements, label, &moving_labels)
                {
                    shift(&mut placements, &moving_labels, snap_x, snap_y);
                    merge_groups(&mut placements, &moving_labels, &other_label);
                    snapped = true;
                }
            }
            let current = placements
                .get(label)
                .map(Placement::state)
                .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
            let states = placements
                .values()
                .filter(|placement| {
                    moving_labels.contains(&placement.label)
                        && (placement.label != label || snapped)
                })
                .map(Placement::state)
                .collect::<Vec<_>>();
            (states, current, snapped)
        };
        if !states.is_empty() || snapped {
            move_native_windows(app, &states, monitor_id);
        }
        Ok(current)
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

    fn resize(&self, label: &str, width: i32, height: i32) {
        let Ok(mut placements) = self.placements.lock() else {
            return;
        };
        if let Some(entry) = placements.get_mut(label) {
            entry.width = width.clamp(TERMINAL_MIN_WIDTH, TERMINAL_MAX_WIDTH);
            entry.height = height.clamp(TERMINAL_MIN_HEIGHT, TERMINAL_MAX_HEIGHT);
        }
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

fn dock_candidate(
    placements: &HashMap<String, Placement>,
    label: &str,
    moving_labels: &[String],
) -> Option<(String, i32, i32)> {
    let current = placements.get(label)?;
    placements
        .values()
        .filter(|other| !moving_labels.contains(&other.label))
        .filter_map(|other| snap(current, other).map(|(score, dx, dy)| (score, other, dx, dy)))
        .filter(|(score, _, _, _)| *score <= DOCK_DISTANCE)
        .min_by_key(|(score, _, _, _)| *score)
        .map(|(_, other, dx, dy)| (other.label.clone(), dx, dy))
}

fn snap(current: &Placement, other: &Placement) -> Option<(i32, i32, i32)> {
    let vertical_overlap =
        (current.y + current.height).min(other.y + other.height) - current.y.max(other.y);
    let horizontal_overlap =
        (current.x + current.width).min(other.x + other.width) - current.x.max(other.x);
    let mut candidates = Vec::new();
    if vertical_overlap > 48 {
        let right_gap = (current.x + current.width - other.x).abs();
        candidates.push((
            right_gap,
            other.x - (current.x + current.width),
            other.y - current.y,
        ));
        let left_gap = (current.x - (other.x + other.width)).abs();
        candidates.push((
            left_gap,
            other.x + other.width - current.x,
            other.y - current.y,
        ));
    }
    if horizontal_overlap > 48 {
        let bottom_gap = (current.y + current.height - other.y).abs();
        candidates.push((
            bottom_gap,
            other.x - current.x,
            other.y - (current.y + current.height),
        ));
        let top_gap = (current.y - (other.y + other.height)).abs();
        candidates.push((
            top_gap,
            other.x - current.x,
            other.y + other.height - current.y,
        ));
    }
    candidates.into_iter().min_by_key(|(score, _, _)| *score)
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

fn move_native_windows(app: &AppHandle, states: &[TerminalWindowState], monitor_id: Option<&str>) {
    for state in states {
        let Some(window) = app.get_webview_window(&state.label) else {
            continue;
        };
        let target = state.clone();
        let monitor = monitor_id.map(str::to_string);
        let layer_window = window.clone();
        let _ = window.run_on_main_thread(move || {
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
            group: None,
            ready: true,
        }
    }

    #[test]
    fn nearby_terminals_snap_side_by_side() {
        let left = placement("left", 20, 40);
        let right = placement("right", 350, 48);
        let (_, dx, dy) = snap(&left, &right).expect("encaixe");
        assert_eq!(left.x + dx + left.width, right.x);
        assert_eq!(left.y + dy, right.y);
    }

    #[test]
    fn nearby_terminals_snap_one_above_the_other() {
        let top = placement("top", 40, 20);
        let bottom = placement("bottom", 48, 298);
        let (_, dx, dy) = snap(&top, &bottom).expect("encaixe vertical");
        assert_eq!(top.x + dx, bottom.x);
        assert_eq!(top.y + dy + top.height, bottom.y);
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
