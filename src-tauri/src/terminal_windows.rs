use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
};

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::{domain::AgentSession, overlay};

const TERMINAL_WIDTH: i32 = 336;
const TERMINAL_HEIGHT: i32 = 286;
const DOCK_DISTANCE: i32 = 34;

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
        show_over_fullscreen: bool,
    ) -> Result<String, String> {
        let label = terminal_label(&session.id);
        if let Some(window) = app.get_webview_window(&label) {
            window.show().map_err(|error| error.to_string())?;
            window.set_focus().map_err(|error| error.to_string())?;
            return Ok(label);
        }

        let offset = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar os mini terminais".to_string())?
            .len() as i32
            * 22;
        let placement = Placement {
            label: label.clone(),
            session_id: session.id.clone(),
            x: (origin_x + 96 + offset).max(12),
            y: (origin_y + 64 + offset).max(12),
            width: TERMINAL_WIDTH,
            height: TERMINAL_HEIGHT,
            group: None,
        };

        self.placements
            .lock()
            .map_err(|_| "Não foi possível guardar o mini terminal".to_string())?
            .insert(label.clone(), placement.clone());

        let window =
            match WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
                .title(format!("Lume · {}", session.agent_label))
                .inner_size(f64::from(TERMINAL_WIDTH), f64::from(TERMINAL_HEIGHT))
                .min_inner_size(f64::from(TERMINAL_WIDTH), f64::from(TERMINAL_HEIGHT))
                .max_inner_size(f64::from(TERMINAL_WIDTH), f64::from(TERMINAL_HEIGHT))
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .resizable(false)
                .visible(false)
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
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                registry.remove(&cleanup_label);
            }
        });

        let layer_window = window.clone();
        let selected_monitor = monitor_id.map(str::to_string);
        window
            .run_on_main_thread(move || {
                let layered = overlay::configure(
                    &layer_window,
                    show_over_fullscreen,
                    selected_monitor.as_deref(),
                    Some(placement.x),
                    Some(placement.y),
                );
                if !layered {
                    let _ = overlay::move_to(
                        &layer_window,
                        placement.x,
                        placement.y,
                        selected_monitor.as_deref(),
                    );
                }
                let _ = layer_window.show();
                let _ = layer_window.set_focus();
            })
            .map_err(|error| error.to_string())?;
        Ok(label)
    }

    pub fn list(&self) -> Result<Vec<TerminalWindowState>, String> {
        let placements = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar os mini terminais".to_string())?;
        Ok(placements.values().map(Placement::state).collect())
    }

    pub fn state(&self, label: &str) -> Result<TerminalWindowState, String> {
        self.placements
            .lock()
            .map_err(|_| "Não foi possível acessar o mini terminal".to_string())?
            .get(label)
            .map(Placement::state)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())
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

    fn remove(&self, label: &str) {
        let Ok(mut placements) = self.placements.lock() else {
            return;
        };
        let group = placements.remove(label).and_then(|entry| entry.group);
        if let Some(group) = group {
            clear_single_member_group(&mut placements, &group);
        }
    }
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
}
