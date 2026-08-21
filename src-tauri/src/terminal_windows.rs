use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{
    webview::PageLoadEvent, AppHandle, Emitter, LogicalSize, Manager, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

use crate::{
    domain::{AgentKind, AgentSession, SessionSource},
    overlay,
};

const TERMINAL_WIDTH: i32 = 368;
const TERMINAL_HEIGHT: i32 = 312;
const TERMINAL_MIN_WIDTH: i32 = 300;
const TERMINAL_MIN_HEIGHT: i32 = 240;
const WORKFLOW_BRIDGE_HORIZONTAL_WIDTH: i32 = 220;
const WORKFLOW_BRIDGE_HORIZONTAL_HEIGHT: i32 = 410;
const WORKFLOW_BRIDGE_VERTICAL_MAX_WIDTH: i32 = 440;
const WORKFLOW_BRIDGE_VERTICAL_HEIGHT: i32 = 410;
const WORKFLOW_BRIDGE_EXPANDED_HEIGHT: i32 = 86;
const WORKFLOW_BRIDGE_MIN_HEIGHT: i32 = 320;
const WORKFLOW_BRIDGE_MARGIN: i32 = 22;
const DOCK_DISTANCE: i32 = 84;
const SCREEN_MARGIN: i32 = 12;
const DOCK_ANIMATION_STEPS: i32 = 9;
const MAX_REASONABLE_COORDINATE: i32 = 65_536;
const TERMINAL_LOAD_TIMEOUT: Duration = Duration::from_secs(15);

fn terminal_webview_path() -> &'static str {
    "terminal/"
}

fn workflow_bridge_webview_path() -> &'static str {
    "workflow-bridge/"
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
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
    pub proximity: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DockPreviewEvent {
    moving_label: String,
    preview: Option<DockPreview>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeDragEndedEvent {
    label: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalWindowState {
    pub label: String,
    pub session_id: String,
    pub session_native_id: Option<String>,
    pub session_process_id: Option<u32>,
    pub session_agent: AgentKind,
    pub session_source: SessionSource,
    pub session_project: String,
    pub session_working_directory: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub docked: bool,
    pub group_id: Option<String>,
    pub connected_sides: Vec<DockSide>,
    pub bridge_sides: Vec<DockSide>,
    pub workflow_bridge_open: bool,
    pub workflow_enabled: bool,
    pub monitor_id: String,
    pub layered: bool,
    pub scale: f64,
}

#[derive(Clone, Debug)]
struct WorkflowBridgePlacement {
    group_id: String,
    source_label: String,
    target_label: String,
    source_session_native_id: String,
    target_session_native_id: String,
    side: DockSide,
    native_connectors: bool,
    monitor_id: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    rendered_height: i32,
    compact_positions: HashMap<String, (i32, i32)>,
    expanded: bool,
    active: bool,
    prepared_at: Instant,
    original_positions: HashMap<String, (i32, i32)>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowBridgeContext {
    pub group_id: String,
    pub source_session_native_id: String,
    pub target_session_native_id: String,
    pub side: DockSide,
    pub native_connectors: bool,
    pub height: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowConnectionHoverEvent {
    pub connection_label: String,
    pub source_label: String,
    pub target_label: String,
    pub side: DockSide,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalDragSnapshot {
    pub pressed: bool,
    pub x: i32,
    pub y: i32,
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
    pub monitor_id: Option<String>,
}

#[derive(Clone, Debug)]
struct Placement {
    label: String,
    session_id: String,
    session_native_id: Option<String>,
    session_process_id: Option<u32>,
    session_agent: AgentKind,
    session_source: SessionSource,
    session_project: String,
    session_working_directory: Option<String>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
    group: Option<String>,
    workflow_enabled: bool,
    monitor_id: String,
    layered: bool,
    ready: bool,
    configured: bool,
    presented: bool,
    created_at: Instant,
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
            proximity: (1.0 - f64::from(self.score) / f64::from(DOCK_DISTANCE)).clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug)]
struct WindowTransition {
    from: TerminalWindowState,
    to: TerminalWindowState,
}

#[derive(Clone, Debug)]
struct MonitorBounds {
    id: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
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
            session_native_id: self.session_native_id.clone(),
            session_process_id: self.session_process_id,
            session_agent: self.session_agent.clone(),
            session_source: self.session_source.clone(),
            session_project: self.session_project.clone(),
            session_working_directory: self.session_working_directory.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            docked: self.group.is_some(),
            group_id: self.group.clone(),
            connected_sides: Vec::new(),
            bridge_sides: Vec::new(),
            workflow_bridge_open: false,
            workflow_enabled: self.workflow_enabled,
            monitor_id: self.monitor_id.clone(),
            layered: self.layered,
            scale: self.scale,
        }
    }
}

#[derive(Clone)]
pub struct TerminalWindows {
    placements: Arc<Mutex<HashMap<String, Placement>>>,
    fullscreen_groups: Arc<Mutex<HashMap<String, Vec<Placement>>>>,
    settling: Arc<Mutex<HashSet<String>>>,
    native_drags: Arc<Mutex<HashSet<String>>>,
    native_move_sequences: Arc<Mutex<HashMap<String, u64>>>,
    native_resize_directions: Arc<Mutex<HashMap<String, (bool, bool, u64)>>>,
    native_resize_watchers: Arc<Mutex<HashSet<String>>>,
    resize_snapshots: Arc<Mutex<HashMap<String, Vec<Placement>>>>,
    workflow_bridges: Arc<Mutex<HashMap<String, WorkflowBridgePlacement>>>,
    workflow_connection_hovers: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    visible: Arc<Mutex<bool>>,
}

impl Default for TerminalWindows {
    fn default() -> Self {
        Self {
            placements: Arc::default(),
            fullscreen_groups: Arc::default(),
            settling: Arc::default(),
            native_drags: Arc::default(),
            native_move_sequences: Arc::default(),
            native_resize_directions: Arc::default(),
            native_resize_watchers: Arc::default(),
            resize_snapshots: Arc::default(),
            workflow_bridges: Arc::default(),
            workflow_connection_hovers: Arc::default(),
            visible: Arc::new(Mutex::new(true)),
        }
    }
}

impl TerminalWindows {
    pub fn set_workflow_connection_hover(
        &self,
        app: &AppHandle,
        label: &str,
        side: DockSide,
        hovered: bool,
    ) -> Result<(), String> {
        let (connection_label, source_label, target_label, canonical_side) = {
            let placements = self
                .placements
                .lock()
                .map_err(|_| "Could not access terminal positions".to_string())?;
            let current = placements
                .get(label)
                .ok_or_else(|| "Source terminal not found".to_string())?;
            let neighbor = connected_neighbor(&placements, current, side)
                .ok_or_else(|| "The shared terminal connection was not found".to_string())?;
            let group_id = current
                .group
                .as_deref()
                .ok_or_else(|| "This terminal is no longer connected".to_string())?;
            let (source_label, target_label, canonical_side) = match side {
                DockSide::Right | DockSide::Bottom => (current.label.clone(), neighbor.label, side),
                DockSide::Left | DockSide::Top => {
                    (neighbor.label, current.label.clone(), opposite_side(side))
                }
            };
            let connection_label = terminal_label(&format!(
                "workflow-bridge:{group_id}:{}:{}",
                source_label, target_label
            ))
            .replacen("terminal-", "workflow-bridge-", 1);
            (connection_label, source_label, target_label, canonical_side)
        };
        let visible = {
            let mut hovers = self
                .workflow_connection_hovers
                .lock()
                .map_err(|_| "Could not update the workflow connection".to_string())?;
            if hovered {
                hovers
                    .entry(connection_label.clone())
                    .or_default()
                    .insert(label.to_string());
            } else if let Some(labels) = hovers.get_mut(&connection_label) {
                labels.remove(label);
                if labels.is_empty() {
                    hovers.remove(&connection_label);
                }
            }
            hovers.contains_key(&connection_label)
        };
        app.emit(
            "lume://workflow-connection-hover",
            WorkflowConnectionHoverEvent {
                connection_label,
                source_label,
                target_label,
                side: canonical_side,
                visible,
            },
        )
        .map_err(|error| error.to_string())
    }

    pub fn open_workflow_bridge(
        &self,
        app: &AppHandle,
        source_label: &str,
        side: DockSide,
    ) -> Result<String, String> {
        self.ensure_workflow_bridge(app, source_label, side, true)
    }

    pub fn prepare_workflow_bridge(
        &self,
        app: &AppHandle,
        source_label: &str,
        side: DockSide,
    ) -> Result<String, String> {
        let label = self.ensure_workflow_bridge(app, source_label, side, false)?;
        let prepared_at = self
            .workflow_bridges
            .lock()
            .ok()
            .and_then(|bridges| bridges.get(&label).map(|bridge| bridge.prepared_at));
        let registry = self.clone();
        let cleanup_app = app.clone();
        let cleanup_label = label.clone();
        let _ = std::thread::Builder::new()
            .name("lume-workflow-prewarm".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(8));
                let should_close = registry
                    .workflow_bridges
                    .lock()
                    .ok()
                    .and_then(|bridges| {
                        bridges
                            .get(&cleanup_label)
                            .map(|bridge| !bridge.active && Some(bridge.prepared_at) == prepared_at)
                    })
                    .unwrap_or(false);
                if should_close {
                    if let Some(window) = cleanup_app.get_webview_window(&cleanup_label) {
                        let window_to_close = window.clone();
                        let _ = window.run_on_main_thread(move || {
                            let _ = window_to_close.close();
                        });
                    }
                }
            });
        Ok(label)
    }

    fn ensure_workflow_bridge(
        &self,
        app: &AppHandle,
        source_label: &str,
        side: DockSide,
        activate: bool,
    ) -> Result<String, String> {
        let mut source_label = source_label.to_string();
        let mut side = side;
        if matches!(side, DockSide::Left | DockSide::Top) {
            let placements = self
                .placements
                .lock()
                .map_err(|_| "Could not access terminal positions".to_string())?;
            let current = placements
                .get(&source_label)
                .ok_or_else(|| "Source terminal not found".to_string())?;
            let neighbor = connected_neighbor(&placements, current, side)
                .ok_or_else(|| "The shared terminal connection was not found".to_string())?;
            source_label = neighbor.label.clone();
            side = opposite_side(side);
        }
        if !matches!(side, DockSide::Right | DockSide::Bottom) {
            return Err("Open the workflow bridge from the shared connection control".into());
        }
        let (
            label,
            bridge,
            monitor_id,
            bridge_x,
            bridge_y,
            bridge_width,
            bridge_height,
            next_positions,
        ) = {
            let placements = self
                .placements
                .lock()
                .map_err(|_| "Could not access terminal positions".to_string())?;
            let source = placements
                .get(&source_label)
                .cloned()
                .ok_or_else(|| "Source terminal not found".to_string())?;
            let group_id = source
                .group
                .clone()
                .ok_or_else(|| "This terminal is no longer connected".to_string())?;
            let target = connected_neighbor(&placements, &source, side)
                .ok_or_else(|| "The shared terminal connection was not found".to_string())?;
            let members = placements
                .values()
                .filter(|entry| entry.group.as_deref() == Some(&group_id))
                .cloned()
                .collect::<Vec<_>>();
            if members.len() < 2 {
                return Err("This workflow group is no longer connected".into());
            }
            let (_, monitor) = selected_monitor(app, Some(&source.monitor_id))
                .ok_or_else(|| "The terminal monitor is unavailable".to_string())?;
            let scale = monitor.scale_factor().max(1.0);
            let screen_width = monitor.size().width as i32;
            let screen_height = monitor.size().height as i32;
            let screen_margin = (f64::from(SCREEN_MARGIN) * scale).round() as i32;
            let original_left = members
                .iter()
                .map(|entry| entry.x)
                .min()
                .unwrap_or(source.x);
            let original_top = members
                .iter()
                .map(|entry| entry.y)
                .min()
                .unwrap_or(source.y);
            let original_right = members
                .iter()
                .map(|entry| entry.x + physical_width(entry))
                .max()
                .unwrap_or(source.x + physical_width(&source));
            let original_bottom = members
                .iter()
                .map(|entry| entry.y + physical_height(entry))
                .max()
                .unwrap_or(source.y + physical_height(&source));
            let original_positions = members
                .iter()
                .map(|entry| (entry.label.clone(), (entry.x, entry.y)))
                .collect::<HashMap<_, _>>();
            let source_key = source
                .session_native_id
                .clone()
                .unwrap_or_else(|| source.session_id.clone());
            let target_key = target
                .session_native_id
                .clone()
                .unwrap_or_else(|| target.session_id.clone());
            let bridge_label = terminal_label(&format!(
                "workflow-bridge:{group_id}:{}:{}",
                source.label, target.label
            ))
            .replacen("terminal-", "workflow-bridge-", 1);
            let native_connectors = cfg!(target_os = "linux");
            let (bridge_width, bridge_height, desired_gap, boundary, cross_start, cross_end) =
                match side {
                    DockSide::Right => {
                        let overlap_top = source.y.max(target.y);
                        let overlap_bottom = (source.y + physical_height(&source))
                            .min(target.y + physical_height(&target));
                        (
                            WORKFLOW_BRIDGE_HORIZONTAL_WIDTH
                                + if native_connectors {
                                    0
                                } else {
                                    WORKFLOW_BRIDGE_MARGIN * 2
                                },
                            WORKFLOW_BRIDGE_HORIZONTAL_HEIGHT,
                            ((WORKFLOW_BRIDGE_HORIZONTAL_WIDTH + WORKFLOW_BRIDGE_MARGIN * 2) as f64
                                * scale)
                                .round() as i32,
                            source.x + physical_width(&source),
                            overlap_top,
                            overlap_bottom,
                        )
                    }
                    DockSide::Bottom => {
                        let overlap_left = source.x.max(target.x);
                        let overlap_right = (source.x + physical_width(&source))
                            .min(target.x + physical_width(&target));
                        let available_width = ((overlap_right - overlap_left) as f64 / scale)
                            .round() as i32
                            - WORKFLOW_BRIDGE_MARGIN * 2;
                        let width = available_width.clamp(280, WORKFLOW_BRIDGE_VERTICAL_MAX_WIDTH);
                        (
                            width,
                            WORKFLOW_BRIDGE_VERTICAL_HEIGHT
                                + if native_connectors {
                                    0
                                } else {
                                    WORKFLOW_BRIDGE_MARGIN * 2
                                },
                            ((WORKFLOW_BRIDGE_VERTICAL_HEIGHT + WORKFLOW_BRIDGE_MARGIN * 2) as f64
                                * scale)
                                .round() as i32,
                            source.y + physical_height(&source),
                            overlap_left,
                            overlap_right,
                        )
                    }
                    DockSide::Left | DockSide::Top => unreachable!(),
                };
            let available_gap = match side {
                DockSide::Right => {
                    screen_width - screen_margin * 2 - (original_right - original_left)
                }
                DockSide::Bottom => {
                    screen_height - screen_margin * 2 - (original_bottom - original_top)
                }
                DockSide::Left | DockSide::Top => 0,
            };
            if available_gap < desired_gap {
                return Err("There is not enough screen space to open this connection".into());
            }
            let gap = desired_gap;
            let first_shift = -(gap / 2);
            let second_shift = gap + first_shift;
            let mut next_positions = members
                .iter()
                .map(|entry| {
                    let first_side = match side {
                        DockSide::Right => entry.x + physical_width(entry) / 2 <= boundary,
                        DockSide::Bottom => entry.y + physical_height(entry) / 2 <= boundary,
                        DockSide::Left | DockSide::Top => false,
                    };
                    let (dx, dy) = match side {
                        DockSide::Right => (
                            if first_side {
                                first_shift
                            } else {
                                second_shift
                            },
                            0,
                        ),
                        DockSide::Bottom => (
                            0,
                            if first_side {
                                first_shift
                            } else {
                                second_shift
                            },
                        ),
                        DockSide::Left | DockSide::Top => (0, 0),
                    };
                    (entry.label.clone(), (entry.x + dx, entry.y + dy))
                })
                .collect::<HashMap<_, _>>();
            let (next_left, next_top, next_right, next_bottom) = members.iter().fold(
                (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
                |(left, top, right, bottom), entry| {
                    let (x, y) = next_positions[&entry.label];
                    (
                        left.min(x),
                        top.min(y),
                        right.max(x + physical_width(entry)),
                        bottom.max(y + physical_height(entry)),
                    )
                },
            );
            let global_dx = if next_left < screen_margin {
                screen_margin - next_left
            } else if next_right > screen_width - screen_margin {
                screen_width - screen_margin - next_right
            } else {
                0
            };
            let global_dy = if next_top < screen_margin {
                screen_margin - next_top
            } else if next_bottom > screen_height - screen_margin {
                screen_height - screen_margin - next_bottom
            } else {
                0
            };
            for position in next_positions.values_mut() {
                position.0 += global_dx;
                position.1 += global_dy;
            }
            let bridge_physical_width = (f64::from(bridge_width) * scale).round() as i32;
            let bridge_physical_height = (f64::from(bridge_height) * scale).round() as i32;
            let bridge_margin = if native_connectors {
                (f64::from(WORKFLOW_BRIDGE_MARGIN) * scale).round() as i32
            } else {
                0
            };
            let (bridge_x, bridge_y) = match side {
                DockSide::Right => (
                    boundary + first_shift + bridge_margin + global_dx,
                    cross_start
                        + ((cross_end - cross_start - bridge_physical_height) / 2).max(0)
                        + global_dy,
                ),
                DockSide::Bottom => (
                    cross_start
                        + ((cross_end - cross_start - bridge_physical_width) / 2).max(0)
                        + global_dx,
                    boundary + first_shift + bridge_margin + global_dy,
                ),
                DockSide::Left | DockSide::Top => unreachable!(),
            };
            (
                bridge_label,
                WorkflowBridgePlacement {
                    group_id,
                    source_label: source.label,
                    target_label: target.label,
                    source_session_native_id: source_key,
                    target_session_native_id: target_key,
                    side,
                    native_connectors,
                    monitor_id: source.monitor_id.clone(),
                    x: bridge_x,
                    y: bridge_y,
                    width: bridge_width,
                    height: bridge_height,
                    rendered_height: bridge_height,
                    compact_positions: next_positions.clone(),
                    expanded: false,
                    active: activate,
                    prepared_at: Instant::now(),
                    original_positions,
                },
                source.monitor_id,
                bridge_x,
                bridge_y,
                bridge_width,
                bridge_height,
                next_positions,
            )
        };
        let previous_layout = self.workflow_bridges.lock().ok().and_then(|bridges| {
            bridges
                .get(&label)
                .map(|entry| (entry.expanded, entry.rendered_height))
        });
        let (previously_expanded, previous_height) =
            previous_layout.unwrap_or((false, bridge_height));
        let mut bridge = bridge;
        bridge.expanded = if activate { false } else { previously_expanded };
        bridge.rendered_height = if activate {
            bridge_height
        } else {
            previous_height
        };
        let window = if let Some(window) = app.get_webview_window(&label) {
            self.workflow_bridges
                .lock()
                .map_err(|_| "Could not prepare the workflow bridge".to_string())?
                .insert(label.clone(), bridge);
            window
        } else {
            self.workflow_bridges
                .lock()
                .map_err(|_| "Could not prepare the workflow bridge".to_string())?
                .insert(label.clone(), bridge);
            let registry = self.clone();
            let cleanup_label = label.clone();
            let cleanup_app = app.clone();
            let build_label = label.clone();
            let window = WebviewWindowBuilder::new(
                app,
                &label,
                WebviewUrl::App(workflow_bridge_webview_path().into()),
            )
            .title("Lume · Connection")
            .inner_size(f64::from(bridge_width), f64::from(bridge_height))
            .min_inner_size(
                f64::from(bridge_width),
                f64::from(WORKFLOW_BRIDGE_MIN_HEIGHT),
            )
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .resizable(false)
            .visible(false)
            .build()
            .map_err(|error| {
                if let Ok(mut bridges) = self.workflow_bridges.lock() {
                    bridges.remove(&build_label);
                }
                error.to_string()
            })?;
            window.on_window_event(move |event| {
                if matches!(event, WindowEvent::Destroyed) {
                    overlay::forget_window(&cleanup_label);
                    registry.restore_workflow_bridge(&cleanup_app, &cleanup_label);
                }
            });
            window
        };
        if !activate {
            return Ok(label);
        }
        if let Some(active_bridge) = self
            .workflow_bridges
            .lock()
            .ok()
            .and_then(|bridges| bridges.get(&label).cloned())
        {
            if let Ok(mut hovers) = self.workflow_connection_hovers.lock() {
                hovers.remove(&label);
            }
            let _ = app.emit(
                "lume://workflow-connection-hover",
                WorkflowConnectionHoverEvent {
                    connection_label: label.clone(),
                    source_label: active_bridge.source_label,
                    target_label: active_bridge.target_label,
                    side: active_bridge.side,
                    visible: false,
                },
            );
        }
        let transitions = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Could not open the workflow connection".to_string())?;
            let labels = next_positions.keys().cloned().collect::<Vec<_>>();
            let before = states_by_label(&placements, &labels);
            for (label, (x, y)) in &next_positions {
                if let Some(entry) = placements.get_mut(label) {
                    entry.x = *x;
                    entry.y = *y;
                }
            }
            let after = states_by_label(&placements, &labels);
            transitions_between(before, after)
        };
        self.invalidate_pending_native_moves(&next_positions.keys().cloned().collect::<Vec<_>>());
        emit_windows_changed(app);
        let layered =
            overlay::configure_workflow_editor(&window, Some(&monitor_id), bridge_x, bridge_y);
        if layered {
            let _ = overlay::resize_surface(&window, bridge_width, bridge_height);
        } else {
            if let Err(error) = overlay::move_to(&window, bridge_x, bridge_y, Some(&monitor_id)) {
                let _ = window.close();
                return Err(error);
            }
        }
        if cfg!(target_os = "linux") {
            let _ = overlay::show_workflow_connectors(
                &window,
                &label,
                Some(&monitor_id),
                bridge_x,
                bridge_y,
                bridge_width,
                bridge_height,
                side == DockSide::Bottom,
                WORKFLOW_BRIDGE_MARGIN,
            );
        }
        if let Err(error) = window.show() {
            let _ = window.close();
            return Err(error.to_string());
        }
        let _ = window.set_focus();
        let _ = window.emit("lume://workflow-bridge-reveal", ());
        overlay::reveal_workflow_connectors(&label);
        self.animate_native_windows(app, transitions);
        if previously_expanded || previous_height != bridge_height {
            self.set_workflow_bridge_expanded(
                app,
                &label,
                previously_expanded,
                Some(previous_height),
            )?;
        }
        Ok(label)
    }

    pub fn discard_prepared_workflow_bridge(
        &self,
        app: &AppHandle,
        label: &str,
    ) -> Result<(), String> {
        let prepared = self
            .workflow_bridges
            .lock()
            .map_err(|_| "Could not access the workflow connection".to_string())?
            .get(label)
            .is_some_and(|bridge| !bridge.active);
        if prepared {
            if let Some(window) = app.get_webview_window(label) {
                window.close().map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    pub fn workflow_bridge_context(&self, label: &str) -> Result<WorkflowBridgeContext, String> {
        self.workflow_bridges
            .lock()
            .map_err(|_| "Could not read the workflow bridge".to_string())?
            .get(label)
            .map(|bridge| WorkflowBridgeContext {
                group_id: bridge.group_id.clone(),
                source_session_native_id: bridge.source_session_native_id.clone(),
                target_session_native_id: bridge.target_session_native_id.clone(),
                side: bridge.side,
                native_connectors: bridge.native_connectors,
                height: bridge.rendered_height,
            })
            .ok_or_else(|| "Workflow bridge not found".to_string())
    }

    pub fn set_workflow_bridge_expanded(
        &self,
        app: &AppHandle,
        label: &str,
        expanded: bool,
        content_height: Option<i32>,
    ) -> Result<(), String> {
        let bridge = self
            .workflow_bridges
            .lock()
            .map_err(|_| "Could not access the workflow connection".to_string())?
            .get(label)
            .cloned()
            .ok_or_else(|| "Workflow connection not found".to_string())?;
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Workflow connection window not found".to_string())?;
        let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
        let extra_height = if expanded {
            WORKFLOW_BRIDGE_EXPANDED_HEIGHT
        } else {
            0
        };
        let target_height = content_height
            .unwrap_or(bridge.height + extra_height)
            .clamp(WORKFLOW_BRIDGE_MIN_HEIGHT, bridge.height + 180);
        if bridge.expanded == expanded && bridge.rendered_height == target_height {
            return Ok(());
        }
        let height_delta = target_height - bridge.height;
        let physical_extra = (f64::from(height_delta) * scale).round() as i32;
        let leading_shift = -(physical_extra / 2);
        let trailing_shift = physical_extra + leading_shift;
        let target_y = bridge.y + leading_shift;

        let transitions = if bridge.active && bridge.side == DockSide::Bottom {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Could not resize the workflow connection".to_string())?;
            let labels = bridge.compact_positions.keys().cloned().collect::<Vec<_>>();
            let before = states_by_label(&placements, &labels);
            for (member_label, (base_x, base_y)) in &bridge.compact_positions {
                let Some(entry) = placements.get_mut(member_label) else {
                    continue;
                };
                let shift = if expanded {
                    if base_y + physical_height(entry) / 2 <= bridge.y {
                        leading_shift
                    } else {
                        trailing_shift
                    }
                } else {
                    0
                };
                entry.x = *base_x;
                entry.y = *base_y + shift;
            }
            let after = states_by_label(&placements, &labels);
            transitions_between(before, after)
        } else {
            Vec::new()
        };

        let mut bridges = self
            .workflow_bridges
            .lock()
            .map_err(|_| "Could not update the workflow connection".to_string())?;
        let current_bridge = bridges
            .get_mut(label)
            .ok_or_else(|| "Workflow connection not found".to_string())?;
        current_bridge.expanded = expanded;
        current_bridge.rendered_height = target_height;
        drop(bridges);

        let window_for_resize = window.clone();
        let connector_label = label.to_string();
        let monitor_id = bridge.monitor_id.clone();
        let _ = window.run_on_main_thread(move || {
            let _ = window_for_resize.set_size(LogicalSize::new(
                f64::from(bridge.width),
                f64::from(target_height),
            ));
            let _ = overlay::resize_surface(&window_for_resize, bridge.width, target_height);
            let _ = overlay::move_to(&window_for_resize, bridge.x, target_y, Some(&monitor_id));
            if bridge.active && bridge.native_connectors {
                let _ = overlay::show_workflow_connectors(
                    &window_for_resize,
                    &connector_label,
                    Some(&monitor_id),
                    bridge.x,
                    target_y,
                    bridge.width,
                    target_height,
                    bridge.side == DockSide::Bottom,
                    WORKFLOW_BRIDGE_MARGIN,
                );
                overlay::reveal_workflow_connectors(&connector_label);
            }
        });
        if !transitions.is_empty() {
            self.animate_native_windows(app, transitions);
            emit_windows_changed(app);
        }
        Ok(())
    }

    fn restore_workflow_bridge(&self, app: &AppHandle, label: &str) {
        let bridge = self
            .workflow_bridges
            .lock()
            .ok()
            .and_then(|mut bridges| bridges.remove(label));
        let Some(bridge) = bridge else {
            return;
        };
        if !bridge.active {
            return;
        }
        let transitions = self.placements.lock().ok().map(|mut placements| {
            let labels = bridge
                .original_positions
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            let before = states_by_label(&placements, &labels);
            for (terminal_label, (x, y)) in bridge.original_positions {
                if let Some(entry) = placements.get_mut(&terminal_label) {
                    entry.x = x;
                    entry.y = y;
                }
            }
            let after = states_by_label(&placements, &labels);
            transitions_between(before, after)
        });
        if let Some(transitions) = transitions {
            self.animate_native_windows(app, transitions);
            emit_windows_changed(app);
        }
    }

    pub fn open(
        &self,
        app: &AppHandle,
        session: &AgentSession,
        monitor_id: Option<&str>,
        origin_x: i32,
        origin_y: i32,
        show_over_fullscreen: bool,
        workflow_enabled: bool,
    ) -> Result<String, String> {
        let label = terminal_label(&session.id);
        if let Some(window) = app.get_webview_window(&label) {
            let loading = self.placements.lock().ok().and_then(|placements| {
                placements
                    .get(&label)
                    .map(|placement| (!placement.ready, placement.created_at.elapsed()))
            });
            if loading.is_some_and(|(loading, elapsed)| loading && elapsed < TERMINAL_LOAD_TIMEOUT)
            {
                return Ok(label);
            }
            if loading.is_none_or(|(loading, _)| loading) {
                let _ = window.close();
                self.remove(&label);
                return Err(
                    "O mini terminal anterior não carregou; clique em Abrir novamente".into(),
                );
            } else {
                let current = self.state(&label)?;
                if current.docked {
                    window.show().map_err(|error| error.to_string())?;
                    let _ = window.set_focus();
                    return Ok(label);
                }
                let effective_monitor = monitor_id.or(Some(current.monitor_id.as_str()));
                let (resolved_monitor_id, monitor) = selected_monitor(app, effective_monitor)
                    .ok_or_else(|| "Nenhum monitor disponível".to_string())?;
                let (x, y) = clamp_drag_to_monitor(
                    current.x,
                    current.y,
                    current.width,
                    current.height,
                    monitor.size().width as i32,
                    monitor.size().height as i32,
                    monitor.scale_factor(),
                );
                if let Ok(mut placements) = self.placements.lock() {
                    if let Some(placement) = placements.get_mut(&label) {
                        placement.x = x;
                        placement.y = y;
                        placement.monitor_id = resolved_monitor_id.clone();
                        placement.scale = monitor.scale_factor();
                    }
                }
                overlay::move_to(&window, x, y, Some(&resolved_monitor_id))?;
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
            TERMINAL_WIDTH,
            TERMINAL_HEIGHT,
        );
        let (resolved_monitor_id, monitor_scale) = selected_monitor(app, monitor_id)
            .map(|(id, monitor)| (id, monitor.scale_factor()))
            .unwrap_or_else(|| (monitor_id.unwrap_or_default().to_string(), 1.0));
        let placement = Placement {
            label: label.clone(),
            session_id: session.id.clone(),
            session_native_id: session.native_session_id.clone(),
            session_process_id: session.process_id,
            session_agent: session.agent.clone(),
            session_source: session.source.clone(),
            session_project: session.project.clone(),
            session_working_directory: session.working_directory.clone(),
            x,
            y,
            width: TERMINAL_WIDTH,
            height: TERMINAL_HEIGHT,
            scale: monitor_scale,
            group: None,
            workflow_enabled,
            monitor_id: resolved_monitor_id.clone(),
            layered: false,
            ready: false,
            configured: false,
            presented: false,
            created_at: Instant::now(),
        };

        self.placements
            .lock()
            .map_err(|_| "Não foi possível guardar o mini terminal".to_string())?
            .insert(label.clone(), placement.clone());

        let ready_registry = self.clone();
        let ready_label = label.clone();
        let window = match WebviewWindowBuilder::new(
            app,
            &label,
            WebviewUrl::App(terminal_webview_path().into()),
        )
        .title(format!("Lume · {}", session.agent_label))
        .inner_size(f64::from(TERMINAL_WIDTH), f64::from(TERMINAL_HEIGHT))
        .min_inner_size(
            f64::from(TERMINAL_MIN_WIDTH),
            f64::from(TERMINAL_MIN_HEIGHT),
        )
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(true)
        .visible(false)
        .on_page_load(move |window, payload| {
            let ready_event = matches!(payload.event(), PageLoadEvent::Finished)
                || (cfg!(target_os = "windows")
                    && matches!(payload.event(), PageLoadEvent::Started));
            if ready_event && ready_registry.mark_ready(&ready_label) {
                ready_registry.present_if_ready(&window, &ready_label);
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
                overlay::forget_window(&cleanup_label);
                registry
                    .restore_fullscreen_group_for_member(event_window.app_handle(), &cleanup_label);
                registry.remove(&cleanup_label);
            }
            WindowEvent::Resized(size) => {
                if registry.is_settling(&cleanup_label) {
                    return;
                }
                let scale = event_window.scale_factor().unwrap_or(1.0);
                let position = relative_window_position(&event_window);
                registry.resize(
                    event_window.app_handle(),
                    &cleanup_label,
                    position,
                    (f64::from(size.width) / scale).round() as i32,
                    (f64::from(size.height) / scale).round() as i32,
                    scale,
                );
            }
            _ => {}
        });

        let layered = overlay::configure_terminal(
            &window,
            show_over_fullscreen,
            Some(&resolved_monitor_id),
            x,
            y,
        );
        if !layered {
            overlay::move_to(&window, x, y, Some(&resolved_monitor_id))?;
        } else {
            let _ = overlay::resize_surface(&window, TERMINAL_WIDTH, TERMINAL_HEIGHT);
        }
        self.mark_configured(&label, layered);
        self.present_if_ready(&window, &label);
        self.start_load_watchdog(app, &label);
        Ok(label)
    }

    pub fn frontend_ready(&self, app: &AppHandle, label: &str) -> Result<(), String> {
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        self.mark_ready(label);
        self.present_if_ready(&window, label);
        Ok(())
    }

    pub fn list(&self, app: &AppHandle) -> Result<Vec<TerminalWindowState>, String> {
        let placements = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar os mini terminais".to_string())?;
        let (bridge_sides, bridge_groups) = self
            .workflow_bridges
            .lock()
            .map(|bridges| {
                (
                    bridge_sides_by_label(&bridges),
                    workflow_bridge_groups(&bridges),
                )
            })
            .unwrap_or_default();
        Ok(placements
            .values()
            .filter(|placement| {
                placement.ready
                    && app
                        .get_webview_window(&placement.label)
                        .and_then(|window| window.is_visible().ok())
                        .unwrap_or(false)
            })
            .map(|placement| {
                let mut state = state_with_connections(&placements, placement);
                state.bridge_sides = bridge_sides
                    .get(&placement.label)
                    .cloned()
                    .unwrap_or_default();
                state.workflow_bridge_open = state
                    .group_id
                    .as_ref()
                    .is_some_and(|group| bridge_groups.contains(group));
                state
            })
            .collect())
    }

    pub fn state(&self, label: &str) -> Result<TerminalWindowState, String> {
        let placements = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar o mini terminal".to_string())?;
        let (bridge_sides, bridge_groups) = self
            .workflow_bridges
            .lock()
            .map(|bridges| {
                (
                    bridge_sides_by_label(&bridges),
                    workflow_bridge_groups(&bridges),
                )
            })
            .unwrap_or_default();
        placements
            .get(label)
            .map(|placement| {
                let mut state = state_with_connections(&placements, placement);
                state.bridge_sides = bridge_sides.get(label).cloned().unwrap_or_default();
                state.workflow_bridge_open = state
                    .group_id
                    .as_ref()
                    .is_some_and(|group| bridge_groups.contains(group));
                state
            })
            .ok_or_else(|| "Mini terminal não encontrado".to_string())
    }

    pub fn set_visible(&self, app: &AppHandle, visible: bool) -> Result<(), String> {
        *self
            .visible
            .lock()
            .map_err(|_| "Não foi possível alterar os mini terminais".to_string())? = visible;
        let labels = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar os mini terminais".to_string())?
            .values()
            .filter(|placement| placement.ready)
            .map(|placement| placement.label.clone())
            .collect::<Vec<_>>();
        let bridge_labels = self
            .workflow_bridges
            .lock()
            .map_err(|_| "Não foi possível acessar as conexões do workflow".to_string())?
            .iter()
            .filter(|(_, bridge)| bridge.active)
            .map(|(label, _)| label.clone())
            .collect::<Vec<_>>();

        for label in labels {
            let Some(window) = app.get_webview_window(&label) else {
                continue;
            };
            if visible {
                window.show().map_err(|error| error.to_string())?;
            } else {
                emit_dock_preview(app, &label, None);
                window.hide().map_err(|error| error.to_string())?;
            }
        }
        for label in bridge_labels {
            let Some(window) = app.get_webview_window(&label) else {
                continue;
            };
            if visible {
                window.show().map_err(|error| error.to_string())?;
                overlay::reveal_workflow_connectors(&label);
            } else {
                overlay::hide_workflow_connectors(&label);
                window.hide().map_err(|error| error.to_string())?;
            }
        }
        emit_windows_changed(app);
        Ok(())
    }

    pub fn hide(&self, app: &AppHandle, label: &str) -> Result<(), String> {
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        self.restore_fullscreen_group_for_member(app, label);
        emit_dock_preview(app, label, None);
        window.hide().map_err(|error| error.to_string())?;
        emit_windows_changed(app);
        Ok(())
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

    pub fn cancel_move(&self, app: &AppHandle, label: &str) -> Result<TerminalWindowState, String> {
        emit_dock_preview(app, label, None);
        self.state(label)
    }

    pub fn drag_snapshot(
        &self,
        app: &AppHandle,
        label: &str,
    ) -> Result<TerminalDragSnapshot, String> {
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        let (pressed, x, y) = overlay::drag_snapshot(&window)
            .ok_or_else(|| "Estado do arraste indisponível".to_string())?;
        Ok(TerminalDragSnapshot { pressed, x, y })
    }

    pub fn begin_native_drag(&self, app: &AppHandle, label: &str) -> Result<(), String> {
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        let mut target = overlay::xwayland_drag_target(&window);
        for _ in 0..4 {
            if target.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(8));
            target = overlay::xwayland_drag_target(&window);
        }
        let target = target.ok_or_else(|| {
            "XWayland window is not ready for dragging. Try again in a moment.".to_string()
        })?;
        let monitors = monitor_bounds(&window)?;
        let (_, pointer_x, pointer_y, frame_x, frame_y) =
            overlay::xwayland_pointer_snapshot_target(target)
                .ok_or_else(|| "Could not read the XWayland pointer position".to_string())?;
        let grab_offset_x = pointer_x - frame_x;
        let grab_offset_y = pointer_y - frame_y;

        {
            let mut active = self
                .native_drags
                .lock()
                .map_err(|_| "Não foi possível iniciar o arraste".to_string())?;
            if !active.insert(label.to_string()) {
                return Ok(());
            }
        }

        let registry = self.clone();
        let app = app.clone();
        let label = label.to_string();
        let thread_label = label.clone();
        std::thread::Builder::new()
            .name("lume-xwayland-drag".into())
            .spawn(move || {
                let started = Instant::now();
                let mut last_position = Some((frame_x, frame_y));
                let mut saw_pressed = false;
                let mut saw_movement = false;
                let mut failed_reads = 0;

                loop {
                    if started.elapsed() > Duration::from_secs(120) {
                        break;
                    }
                    let Some((pressed, pointer_x, pointer_y, _, _)) =
                        overlay::xwayland_pointer_snapshot_target(target)
                    else {
                        failed_reads += 1;
                        if failed_reads >= 8 {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(16));
                        continue;
                    };
                    failed_reads = 0;
                    saw_pressed |= pressed;
                    let x = pointer_x - grab_offset_x;
                    let y = pointer_y - grab_offset_y;
                    let moved = last_position.is_some_and(|position| position != (x, y));
                    if moved && (pressed || saw_pressed) {
                        saw_movement = true;
                        let _ = registry.sync_native_position_on_monitors(
                            &app,
                            &thread_label,
                            x,
                            y,
                            false,
                            &monitors,
                            false,
                        );
                    }
                    last_position = Some((x, y));

                    if !pressed && (saw_pressed || started.elapsed() > Duration::from_millis(180)) {
                        if saw_movement {
                            let _ = registry.sync_native_position_on_monitors(
                                &app,
                                &thread_label,
                                x,
                                y,
                                true,
                                &monitors,
                                false,
                            );
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(16));
                }

                emit_dock_preview(&app, &thread_label, None);
                if let Ok(mut active) = registry.native_drags.lock() {
                    active.remove(&thread_label);
                }
                let _ = app.emit(
                    "lume://terminal-native-drag-ended",
                    NativeDragEndedEvent {
                        label: thread_label,
                    },
                );
            })
            .map_err(|error| {
                if let Ok(mut active) = self.native_drags.lock() {
                    active.remove(&label);
                }
                error.to_string()
            })?;
        window.start_dragging().map_err(|error| {
            if let Ok(mut active) = self.native_drags.lock() {
                active.remove(&label);
            }
            error.to_string()
        })?;
        Ok(())
    }

    pub fn move_window(
        &self,
        app: &AppHandle,
        label: &str,
        x: i32,
        y: i32,
        finalize: bool,
    ) -> Result<TerminalWindowState, String> {
        if self.is_settling(label) {
            return self.state(label);
        }
        validate_coordinates(x, y)?;
        let current = self.state(label)?;
        let (x, y) = selected_monitor(app, Some(&current.monitor_id))
            .map(|(_, monitor)| {
                clamp_drag_to_monitor(
                    x,
                    y,
                    current.width,
                    current.height,
                    monitor.size().width as i32,
                    monitor.size().height as i32,
                    monitor.scale_factor(),
                )
            })
            .unwrap_or((x, y));
        let result = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível mover o mini terminal".to_string())?;
            update_placements(&mut placements, label, x, y, finalize)
        };
        let update = match result {
            Ok(update) => update,
            Err(error) => {
                emit_dock_preview(app, label, None);
                return Err(error);
            }
        };
        emit_dock_preview(
            app,
            label,
            if finalize {
                None
            } else {
                update.preview.clone()
            },
        );
        if update.snapped {
            self.invalidate_pending_native_moves(&update.moving_labels);
            self.animate_native_windows(app, update.transitions);
            emit_windows_changed(app);
        } else {
            let states = self.states_for_labels(&update.moving_labels)?;
            self.move_latest_native_windows(app, &states, false);
            if finalize {
                emit_windows_changed(app);
            }
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
    ) -> Result<TerminalWindowState, String> {
        if self.is_settling(label) {
            return self.state(label);
        }
        validate_coordinates(physical_x, physical_y)?;
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        let monitors = monitor_bounds(&window)?;
        self.sync_native_position_on_monitors(
            app, label, physical_x, physical_y, finalize, &monitors, false,
        )
    }

    fn sync_native_position_on_monitors(
        &self,
        app: &AppHandle,
        label: &str,
        physical_x: i32,
        physical_y: i32,
        finalize: bool,
        monitors: &[MonitorBounds],
        move_current_window: bool,
    ) -> Result<TerminalWindowState, String> {
        if self.is_settling(label) {
            return self.state(label);
        }
        let current = self.state(label)?;
        let monitor = drag_monitor_for_point(monitors, physical_x, physical_y, &current.monitor_id)
            .ok_or_else(|| "Nenhum monitor disponível".to_string())?;
        let mut x = physical_x - monitor.x;
        let mut y = physical_y - monitor.y;
        validate_coordinates(x, y)?;

        let update = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível sincronizar o mini terminal".to_string())?;
            if let Some(current) = placements.get(label) {
                (x, y) = clamp_drag_to_monitor(
                    x,
                    y,
                    current.width,
                    current.height,
                    monitor.width as i32,
                    monitor.height as i32,
                    monitor.scale,
                );
            }
            let crossed_monitor = current.monitor_id != monitor.id;
            if crossed_monitor {
                let old_monitor = monitors
                    .iter()
                    .find(|candidate| candidate.id == current.monitor_id)
                    .unwrap_or(monitor);
                let delta_x = monitor.x + x - (old_monitor.x + current.x);
                let delta_y = monitor.y + y - (old_monitor.y + current.y);
                let moving_labels = current
                    .group_id
                    .as_ref()
                    .map(|group| {
                        placements
                            .values()
                            .filter(|entry| entry.group.as_ref() == Some(group))
                            .map(|entry| entry.label.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|| vec![label.to_string()]);
                for moving_label in moving_labels {
                    let Some(entry) = placements.get_mut(&moving_label) else {
                        continue;
                    };
                    let source_monitor = monitors
                        .iter()
                        .find(|candidate| candidate.id == entry.monitor_id)
                        .unwrap_or(old_monitor);
                    let global_x = source_monitor.x + entry.x + delta_x;
                    let global_y = source_monitor.y + entry.y + delta_y;
                    let destination =
                        drag_monitor_for_point(monitors, global_x, global_y, &entry.monitor_id)
                            .unwrap_or(monitor);
                    let physical_width = physical_width(entry);
                    let physical_height = physical_height(entry);
                    entry.width = (f64::from(physical_width) / destination.scale)
                        .round()
                        .max(1.0) as i32;
                    entry.height = (f64::from(physical_height) / destination.scale)
                        .round()
                        .max(1.0) as i32;
                    (entry.x, entry.y) = clamp_drag_to_monitor(
                        global_x - destination.x,
                        global_y - destination.y,
                        entry.width,
                        entry.height,
                        destination.width as i32,
                        destination.height as i32,
                        destination.scale,
                    );
                    entry.scale = destination.scale;
                    entry.monitor_id = destination.id.clone();
                }
                if let Some(current) = placements.get(label) {
                    x = current.x;
                    y = current.y;
                }
            } else if let Some(current) = placements.get_mut(label) {
                current.scale = monitor.scale;
            }
            update_placements(&mut placements, label, x, y, finalize)?
        };
        emit_dock_preview(
            app,
            label,
            if finalize {
                None
            } else {
                update.preview.clone()
            },
        );
        if update.snapped {
            self.invalidate_pending_native_moves(&update.moving_labels);
            self.animate_native_windows(app, update.transitions);
            emit_windows_changed(app);
        } else {
            let states = self
                .states_for_labels(&update.moving_labels)?
                .into_iter()
                .filter(|state| move_current_window || state.label != label)
                .collect::<Vec<_>>();
            self.move_latest_native_windows(app, &states, false);
            if finalize {
                emit_windows_changed(app);
            }
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

    fn invalidate_pending_native_moves(&self, labels: &[String]) {
        if let Ok(mut sequences) = self.native_move_sequences.lock() {
            for label in labels {
                let sequence = sequences.entry(label.clone()).or_default();
                *sequence = sequence.wrapping_add(1);
            }
        }
    }

    fn move_latest_native_windows(
        &self,
        app: &AppHandle,
        states: &[TerminalWindowState],
        resize: bool,
    ) {
        for state in states {
            let Some(window) = app.get_webview_window(&state.label) else {
                continue;
            };
            let sequence = {
                let Ok(mut sequences) = self.native_move_sequences.lock() else {
                    continue;
                };
                let sequence = sequences.entry(state.label.clone()).or_default();
                *sequence = sequence.wrapping_add(1);
                *sequence
            };
            let sequences = self.native_move_sequences.clone();
            let target = state.clone();
            let layer_window = window.clone();
            let _ = window.run_on_main_thread(move || {
                let is_latest = sequences
                    .lock()
                    .ok()
                    .and_then(|current| current.get(&target.label).copied())
                    == Some(sequence);
                if !is_latest {
                    return;
                }
                if resize {
                    let _ = layer_window.set_size(LogicalSize::new(
                        f64::from(target.width),
                        f64::from(target.height),
                    ));
                    let _ = overlay::resize_surface(&layer_window, target.width, target.height);
                }
                overlay::set_terminal_docked_shape(
                    &layer_window,
                    target.width,
                    target.height,
                    target.docked,
                );
                let _ =
                    overlay::move_to(&layer_window, target.x, target.y, Some(&target.monitor_id));
            });
        }
    }

    fn animate_native_windows(&self, app: &AppHandle, transitions: Vec<WindowTransition>) {
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
                    move_native_windows(&app, &states, true);
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

    fn settle_while(&self, app: &AppHandle, states: Vec<TerminalWindowState>, resize: bool) {
        if let Ok(mut settling) = self.settling.lock() {
            settling.extend(states.iter().map(|state| state.label.clone()));
        }
        let settling = self.settling.clone();
        let app = app.clone();
        let _ = std::thread::Builder::new()
            .name("lume-terminal-layout".into())
            .spawn(move || {
                move_native_windows(&app, &states, resize);
                std::thread::sleep(Duration::from_millis(220));
                if let Ok(mut settling) = settling.lock() {
                    for state in &states {
                        settling.remove(&state.label);
                    }
                }
            });
    }

    pub fn toggle_group_fullscreen(
        &self,
        app: &AppHandle,
        label: &str,
    ) -> Result<Option<bool>, String> {
        let current = self.state(label)?;
        let fullscreen_id = current
            .group_id
            .clone()
            .unwrap_or_else(|| format!("window:{label}"));

        if let Some(saved) = self
            .fullscreen_groups
            .lock()
            .map_err(|_| "Não foi possível acessar o fullscreen do grupo".to_string())?
            .remove(&fullscreen_id)
        {
            let transitions = {
                let mut placements = self
                    .placements
                    .lock()
                    .map_err(|_| "Não foi possível restaurar o grupo".to_string())?;
                saved
                    .into_iter()
                    .filter_map(|saved_entry| {
                        let entry = placements.get_mut(&saved_entry.label)?;
                        let from = entry.state();
                        *entry = saved_entry;
                        Some(WindowTransition {
                            from,
                            to: entry.state(),
                        })
                    })
                    .collect::<Vec<_>>()
            };
            self.animate_native_windows(app, transitions);
            emit_windows_changed(app);
            return Ok(Some(false));
        }

        let window = app
            .get_webview_window(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        let monitors = monitor_bounds(&window)?;
        let target_monitor = monitors
            .iter()
            .find(|monitor| monitor.id == current.monitor_id)
            .or_else(|| monitors.first())
            .ok_or_else(|| "Nenhum monitor disponível".to_string())?
            .clone();
        let (work_x, work_y, work_width, work_height) = overlay::monitor_work_area(
            &window,
            Some(&target_monitor.id),
        )
        .unwrap_or((0, 0, target_monitor.width, target_monitor.height));

        let (saved, transitions) = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível expandir o grupo".to_string())?;
            let group = if let Some(group_id) = current.group_id.as_deref() {
                placements
                    .values()
                    .filter(|entry| entry.group.as_deref() == Some(group_id))
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                placements
                    .get(label)
                    .cloned()
                    .into_iter()
                    .collect::<Vec<_>>()
            };
            if group.is_empty() {
                return Ok(None);
            }

            let physical_rect = |entry: &Placement| {
                let monitor = monitors
                    .iter()
                    .find(|monitor| monitor.id == entry.monitor_id)
                    .unwrap_or(&target_monitor);
                let x = monitor.x + entry.x;
                let y = monitor.y + entry.y;
                let width = physical_width(entry);
                let height = physical_height(entry);
                (x, y, width, height)
            };
            let min_x = group
                .iter()
                .map(|entry| physical_rect(entry).0)
                .min()
                .unwrap_or(0);
            let min_y = group
                .iter()
                .map(|entry| physical_rect(entry).1)
                .min()
                .unwrap_or(0);
            let max_x = group
                .iter()
                .map(|entry| {
                    let (x, _, width, _) = physical_rect(entry);
                    x + width
                })
                .max()
                .unwrap_or(min_x + 1);
            let max_y = group
                .iter()
                .map(|entry| {
                    let (_, y, _, height) = physical_rect(entry);
                    y + height
                })
                .max()
                .unwrap_or(min_y + 1);
            let layout_width = (max_x - min_x).max(1);
            let layout_height = (max_y - min_y).max(1);
            let scale_x = f64::from(work_width) / f64::from(layout_width);
            let scale_y = f64::from(work_height) / f64::from(layout_height);

            let mut transitions = Vec::with_capacity(group.len());
            for original in &group {
                let (global_x, global_y, physical_width, physical_height) = physical_rect(original);
                let Some(entry) = placements.get_mut(&original.label) else {
                    continue;
                };
                let from = entry.state();
                entry.x =
                    (f64::from(work_x) + f64::from(global_x - min_x) * scale_x).round() as i32;
                entry.y =
                    (f64::from(work_y) + f64::from(global_y - min_y) * scale_y).round() as i32;
                entry.width = (f64::from(physical_width) * scale_x / target_monitor.scale)
                    .round()
                    .max(1.0) as i32;
                entry.height = (f64::from(physical_height) * scale_y / target_monitor.scale)
                    .round()
                    .max(1.0) as i32;
                entry.monitor_id = target_monitor.id.clone();
                entry.scale = target_monitor.scale;
                transitions.push(WindowTransition {
                    from,
                    to: entry.state(),
                });
            }
            (group, transitions)
        };

        self.fullscreen_groups
            .lock()
            .map_err(|_| "Não foi possível guardar o fullscreen do grupo".to_string())?
            .insert(fullscreen_id, saved);
        self.animate_native_windows(app, transitions);
        emit_windows_changed(app);
        Ok(Some(true))
    }

    pub fn group_fullscreen_active(&self, label: &str) -> bool {
        self.fullscreen_groups
            .lock()
            .map(|groups| {
                groups
                    .values()
                    .any(|entries| entries.iter().any(|entry| entry.label == label))
            })
            .unwrap_or(false)
    }

    fn restore_fullscreen_group_for_member(&self, app: &AppHandle, label: &str) {
        let saved = self.fullscreen_groups.lock().ok().and_then(|mut groups| {
            let group_id = groups.iter().find_map(|(group_id, entries)| {
                entries
                    .iter()
                    .any(|entry| entry.label == label)
                    .then(|| group_id.clone())
            })?;
            groups.remove(&group_id)
        });
        let Some(saved) = saved else {
            return;
        };
        let transitions = {
            let Ok(mut placements) = self.placements.lock() else {
                return;
            };
            saved
                .into_iter()
                .filter(|saved_entry| saved_entry.label != label)
                .filter_map(|saved_entry| {
                    let entry = placements.get_mut(&saved_entry.label)?;
                    let from = entry.state();
                    *entry = saved_entry;
                    Some(WindowTransition {
                        from,
                        to: entry.state(),
                    })
                })
                .collect::<Vec<_>>()
        };
        self.animate_native_windows(app, transitions);
        emit_windows_changed(app);
    }

    pub fn undock(&self, app: &AppHandle, label: &str) -> Result<TerminalWindowState, String> {
        let mut placements = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível desacoplar o mini terminal".to_string())?;
        let old_group = placements.get(label).and_then(|entry| entry.group.clone());
        let affected_labels = old_group
            .as_ref()
            .map(|group| {
                placements
                    .values()
                    .filter(|entry| entry.group.as_ref() == Some(group))
                    .map(|entry| entry.label.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![label.to_string()]);
        let entry = placements
            .get_mut(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        entry.group = None;
        if let Some(group) = old_group {
            clear_single_member_group(&mut placements, &group);
        }
        let states = affected_labels
            .iter()
            .filter_map(|affected_label| {
                placements
                    .get(affected_label)
                    .map(|placement| state_with_connections(&placements, placement))
            })
            .collect::<Vec<_>>();
        let state = states
            .iter()
            .find(|state| state.label == label)
            .cloned()
            .or_else(|| placements.get(label).map(Placement::state))
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        drop(placements);
        move_native_windows(app, &states, false);
        emit_windows_changed(app);
        Ok(state)
    }

    pub fn set_workflow_enabled(
        &self,
        app: &AppHandle,
        enabled: bool,
    ) -> Result<Vec<TerminalWindowState>, String> {
        let mut placements = self
            .placements
            .lock()
            .map_err(|_| "Não foi possível acessar os terminais".to_string())?;
        let states = apply_workflow_mode(&mut placements, enabled);
        drop(placements);
        emit_windows_changed(app);
        Ok(states)
    }

    pub fn restore_layout(
        &self,
        app: &AppHandle,
        entries: Vec<RestoredTerminalPlacement>,
        monitor_id: Option<&str>,
        workflow_enabled: bool,
    ) -> Result<Vec<TerminalWindowState>, String> {
        for entry in &entries {
            validate_coordinates(entry.x, entry.y)?;
        }

        let updates = entries
            .into_iter()
            .filter_map(|entry| {
                let selected = selected_monitor(app, entry.monitor_id.as_deref().or(monitor_id));
                let (resolved_monitor_id, monitor) = selected?;
                let width = entry.width.max(TERMINAL_MIN_WIDTH);
                let height = entry.height.max(TERMINAL_MIN_HEIGHT);
                let (x, y) = clamp_drag_to_monitor(
                    entry.x,
                    entry.y,
                    width,
                    height,
                    monitor.size().width as i32,
                    monitor.size().height as i32,
                    monitor.scale_factor(),
                );
                Some((
                    entry.session_id,
                    x,
                    y,
                    width,
                    height,
                    entry.group_id,
                    resolved_monitor_id,
                    monitor.scale_factor(),
                ))
            })
            .collect::<Vec<_>>();

        let states = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível restaurar o layout".to_string())?;
            for (session_id, x, y, width, height, group, monitor_id, scale) in updates {
                let Some(placement) = placements
                    .values_mut()
                    .find(|placement| placement.session_id == session_id)
                else {
                    continue;
                };
                placement.x = x;
                placement.y = y;
                placement.width = width;
                placement.height = height;
                placement.group = group;
                placement.workflow_enabled = workflow_enabled;
                placement.monitor_id = monitor_id;
                placement.scale = scale;
            }
            let groups = placements
                .values()
                .filter_map(|placement| placement.group.clone())
                .collect::<HashSet<_>>();
            for group in groups {
                clear_single_member_group(&mut placements, &group);
            }
            placements
                .values()
                .map(Placement::state)
                .collect::<Vec<_>>()
        };

        self.settle_while(app, states.clone(), true);
        emit_windows_changed(app);
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
        from_left: bool,
        from_top: bool,
    ) -> Result<TerminalWindowState, String> {
        validate_coordinates(x, y)?;
        let current = self.state(label)?;
        if current.workflow_bridge_open {
            return Err("Close the workflow connection before resizing these terminals".into());
        }
        let width = width.max(TERMINAL_MIN_WIDTH);
        let height = height.max(TERMINAL_MIN_HEIGHT);
        let (x, y) = if current.docked {
            (x, y)
        } else {
            selected_monitor(app, Some(&current.monitor_id))
                .map(|(_, monitor)| {
                    clamp_drag_to_monitor(
                        x,
                        y,
                        width,
                        height,
                        monitor.size().width as i32,
                        monitor.size().height as i32,
                        monitor.scale_factor(),
                    )
                })
                .unwrap_or((x, y))
        };
        let baseline = self
            .resize_snapshots
            .lock()
            .ok()
            .and_then(|snapshots| snapshots.get(label).cloned());
        let states = {
            let mut placements = self
                .placements
                .lock()
                .map_err(|_| "Não foi possível redimensionar o mini terminal".to_string())?;
            resize_group_placements(
                &mut placements,
                label,
                x,
                y,
                width,
                height,
                current.scale,
                from_left,
                from_top,
                baseline.as_deref(),
            )?
        };
        self.move_latest_native_windows(app, &states, true);
        states
            .into_iter()
            .find(|state| state.label == label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())
    }

    pub fn begin_layered_resize(
        &self,
        app: &AppHandle,
        label: &str,
    ) -> Result<TerminalWindowState, String> {
        let state = self.state(label)?;
        if state.workflow_bridge_open {
            return Err("Close the workflow connection before resizing these terminals".into());
        }
        if let Ok(placements) = self.placements.lock() {
            if let Some(current) = placements.get(label) {
                let labels = group_labels(&placements, current);
                let snapshot = labels
                    .iter()
                    .filter_map(|member_label| placements.get(member_label).cloned())
                    .collect::<Vec<_>>();
                if let Ok(mut snapshots) = self.resize_snapshots.lock() {
                    snapshots.insert(label.to_string(), snapshot);
                }
                if let Ok(mut settling) = self.settling.lock() {
                    settling.extend(labels);
                }
            }
        }
        emit_windows_changed(app);
        Ok(state)
    }

    pub fn finish_layered_resize(
        &self,
        app: &AppHandle,
        label: &str,
    ) -> Result<TerminalWindowState, String> {
        let state = self.state(label)?;
        let settling = self.settling.clone();
        if let Ok(mut snapshots) = self.resize_snapshots.lock() {
            snapshots.remove(label);
        }
        let labels = self
            .placements
            .lock()
            .ok()
            .and_then(|placements| {
                placements
                    .get(label)
                    .map(|current| group_labels(&placements, current))
            })
            .unwrap_or_else(|| vec![label.to_string()]);
        let _ = std::thread::Builder::new()
            .name("lume-terminal-resize".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(140));
                if let Ok(mut settling) = settling.lock() {
                    for label in &labels {
                        settling.remove(label);
                    }
                }
            });
        emit_windows_changed(app);
        Ok(state)
    }

    fn remove(&self, label: &str) {
        if let Ok(mut fullscreen_groups) = self.fullscreen_groups.lock() {
            fullscreen_groups
                .retain(|_, entries| !entries.iter().any(|entry| entry.label == label));
        }
        let Ok(mut placements) = self.placements.lock() else {
            return;
        };
        let group = placements.remove(label).and_then(|entry| entry.group);
        if let Some(group) = group {
            clear_single_member_group(&mut placements, &group);
        }
    }

    fn resize(
        &self,
        app: &AppHandle,
        label: &str,
        position: Option<(i32, i32)>,
        width: i32,
        height: i32,
        scale: f64,
    ) {
        if self
            .state(label)
            .is_ok_and(|state| state.workflow_bridge_open)
        {
            return;
        }
        self.ensure_resize_snapshot(label);
        let (from_left, from_top) =
            self.native_resize_direction(app, label, position, width, height, scale);
        let states =
            self.resize_placement(label, position, width, height, scale, from_left, from_top);
        if states.is_empty() {
            return;
        }
        let other_states = states
            .iter()
            .filter(|state| state.label != label)
            .cloned()
            .collect::<Vec<_>>();
        if !other_states.is_empty() {
            if let Ok(mut settling) = self.settling.lock() {
                settling.extend(other_states.iter().map(|state| state.label.clone()));
            }
            self.move_latest_native_windows(app, &other_states, true);
        }
    }

    fn resize_placement(
        &self,
        label: &str,
        position: Option<(i32, i32)>,
        width: i32,
        height: i32,
        scale: f64,
        from_left: bool,
        from_top: bool,
    ) -> Vec<TerminalWindowState> {
        let baseline = self
            .resize_snapshots
            .lock()
            .ok()
            .and_then(|snapshots| snapshots.get(label).cloned());
        let Ok(mut placements) = self.placements.lock() else {
            return Vec::new();
        };
        let width = width.max(TERMINAL_MIN_WIDTH);
        let height = height.max(TERMINAL_MIN_HEIGHT);
        let Some(current) = placements.get(label) else {
            return Vec::new();
        };
        let (x, y) = position.unwrap_or((current.x, current.y));
        resize_group_placements(
            &mut placements,
            label,
            x,
            y,
            width,
            height,
            scale,
            from_left,
            from_top,
            baseline.as_deref(),
        )
        .unwrap_or_default()
    }

    fn ensure_resize_snapshot(&self, label: &str) {
        let already_present = self
            .resize_snapshots
            .lock()
            .map(|snapshots| snapshots.contains_key(label))
            .unwrap_or(true);
        if already_present {
            return;
        }
        let snapshot = self.placements.lock().ok().and_then(|placements| {
            let current = placements.get(label)?;
            Some(
                group_labels(&placements, current)
                    .iter()
                    .filter_map(|member_label| placements.get(member_label).cloned())
                    .collect::<Vec<_>>(),
            )
        });
        if let Some(snapshot) = snapshot {
            if let Ok(mut snapshots) = self.resize_snapshots.lock() {
                snapshots.entry(label.to_string()).or_insert(snapshot);
            }
        }
    }

    fn native_resize_direction(
        &self,
        app: &AppHandle,
        label: &str,
        position: Option<(i32, i32)>,
        width: i32,
        height: i32,
        scale: f64,
    ) -> (bool, bool) {
        let inferred = self
            .placements
            .lock()
            .ok()
            .and_then(|placements| placements.get(label).cloned())
            .map(|current| {
                let (x, y) = position.unwrap_or((current.x, current.y));
                let new_width = (f64::from(width) * scale.max(1.0)).round() as i32;
                let new_height = (f64::from(height) * scale.max(1.0)).round() as i32;
                let left_delta = (x - current.x).abs();
                let right_delta = (x + new_width - current.x - physical_width(&current)).abs();
                let top_delta = (y - current.y).abs();
                let bottom_delta = (y + new_height - current.y - physical_height(&current)).abs();
                (right_delta < left_delta, bottom_delta < top_delta)
            })
            .unwrap_or((false, false));
        let sequence = {
            let Ok(mut directions) = self.native_resize_directions.lock() else {
                return inferred;
            };
            let entry = directions
                .entry(label.to_string())
                .or_insert((inferred.0, inferred.1, 0));
            entry.2 = entry.2.wrapping_add(1);
            *entry
        };
        let should_watch = self
            .native_resize_watchers
            .lock()
            .map(|mut watchers| watchers.insert(label.to_string()))
            .unwrap_or(false);
        if !should_watch {
            return (sequence.0, sequence.1);
        }
        let directions = self.native_resize_directions.clone();
        let watchers = self.native_resize_watchers.clone();
        let snapshots = self.resize_snapshots.clone();
        let placements = self.placements.clone();
        let settling = self.settling.clone();
        let app = app.clone();
        let label = label.to_string();
        let _ = std::thread::Builder::new()
            .name("lume-terminal-resize-direction".into())
            .spawn(move || {
                let mut observed_sequence = sequence.2;
                loop {
                    std::thread::sleep(Duration::from_millis(180));
                    let finished = if let Ok(mut directions) = directions.lock() {
                        match directions.get(&label).map(|entry| entry.2) {
                            Some(current) if current == observed_sequence => {
                                directions.remove(&label);
                                if let Ok(mut watchers) = watchers.lock() {
                                    watchers.remove(&label);
                                }
                                true
                            }
                            Some(current) => {
                                observed_sequence = current;
                                false
                            }
                            None => {
                                if let Ok(mut watchers) = watchers.lock() {
                                    watchers.remove(&label);
                                }
                                return;
                            }
                        }
                    } else {
                        if let Ok(mut watchers) = watchers.lock() {
                            watchers.remove(&label);
                        }
                        return;
                    };
                    if !finished {
                        continue;
                    }
                    if let Ok(mut snapshots) = snapshots.lock() {
                        snapshots.remove(&label);
                    }
                    let labels = placements
                        .lock()
                        .ok()
                        .and_then(|placements| {
                            placements
                                .get(&label)
                                .map(|current| group_labels(&placements, current))
                        })
                        .unwrap_or_else(|| vec![label.clone()]);
                    if let Ok(mut settling) = settling.lock() {
                        for member in labels {
                            settling.remove(&member);
                        }
                    }
                    emit_windows_changed(&app);
                    return;
                }
            });
        (sequence.0, sequence.1)
    }

    fn is_settling(&self, label: &str) -> bool {
        self.settling
            .lock()
            .map(|settling| settling.contains(label))
            .unwrap_or(false)
    }

    fn start_load_watchdog(&self, app: &AppHandle, label: &str) {
        let registry = self.clone();
        let app = app.clone();
        let label = label.to_string();
        let _ = std::thread::Builder::new()
            .name("lume-terminal-load-watchdog".into())
            .spawn(move || {
                std::thread::sleep(TERMINAL_LOAD_TIMEOUT);
                let timed_out = registry
                    .placements
                    .lock()
                    .ok()
                    .and_then(|placements| placements.get(&label).map(|entry| !entry.ready))
                    .unwrap_or(false);
                if !timed_out {
                    return;
                }
                registry.remove(&label);
                if let Some(window) = app.get_webview_window(&label) {
                    let window_to_close = window.clone();
                    let _ = window.run_on_main_thread(move || {
                        let _ = window_to_close.close();
                    });
                }
                emit_windows_changed(&app);
            });
    }

    fn mark_ready(&self, label: &str) -> bool {
        if let Ok(mut placements) = self.placements.lock() {
            if let Some(placement) = placements.get_mut(label) {
                if placement.ready {
                    return false;
                }
                placement.ready = true;
                return true;
            }
        }
        false
    }

    fn mark_configured(&self, label: &str, layered: bool) {
        if let Ok(mut placements) = self.placements.lock() {
            if let Some(placement) = placements.get_mut(label) {
                placement.layered = layered;
                placement.configured = true;
            }
        }
    }

    fn present_if_ready(&self, window: &tauri::WebviewWindow, label: &str) {
        let should_present = self
            .placements
            .lock()
            .ok()
            .and_then(|mut placements| {
                let placement = placements.get_mut(label)?;
                if !placement.ready || !placement.configured || placement.presented {
                    return Some(false);
                }
                placement.presented = true;
                Some(true)
            })
            .unwrap_or(false);
        if !should_present {
            return;
        }
        if !self.visible.lock().map(|visible| *visible).unwrap_or(true) {
            return;
        }
        if window.show().is_err() {
            if let Ok(mut placements) = self.placements.lock() {
                if let Some(placement) = placements.get_mut(label) {
                    placement.presented = false;
                }
            }
            return;
        }
        let _ = window.set_focus();
        emit_windows_changed(window.app_handle());
    }
}

fn relative_window_position(window: &tauri::WebviewWindow) -> Option<(i32, i32)> {
    let position = window.outer_position().ok()?;
    let monitor_position = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|monitor| *monitor.position())
        .unwrap_or_default();
    Some((
        position.x - monitor_position.x,
        position.y - monitor_position.y,
    ))
}

fn initial_position(
    app: &AppHandle,
    monitor_id: Option<&str>,
    desired_x: i32,
    desired_y: i32,
    width: i32,
    height: i32,
) -> (i32, i32) {
    let Some((_, monitor)) = selected_monitor(app, monitor_id) else {
        return (desired_x.max(SCREEN_MARGIN), desired_y.max(SCREEN_MARGIN));
    };
    clamp_drag_to_monitor(
        desired_x,
        desired_y,
        width,
        height,
        monitor.size().width as i32,
        monitor.size().height as i32,
        monitor.scale_factor(),
    )
}

fn selected_monitor(app: &AppHandle, monitor_id: Option<&str>) -> Option<(String, tauri::Monitor)> {
    let Some(main) = app.get_webview_window("main") else {
        return None;
    };
    let Ok(monitors) = main.available_monitors() else {
        return None;
    };
    let primary = main.primary_monitor().ok().flatten();
    let monitor = overlay::select_monitor(&monitors, primary, monitor_id)?;
    let id = overlay::monitor_identifier(&monitors, &monitor);
    Some((id, monitor))
}

fn monitor_bounds(window: &tauri::WebviewWindow) -> Result<Vec<MonitorBounds>, String> {
    let monitors = window
        .available_monitors()
        .map_err(|error| error.to_string())?;
    Ok(monitors
        .iter()
        .map(|monitor| MonitorBounds {
            id: overlay::monitor_identifier(&monitors, monitor),
            x: monitor.position().x,
            y: monitor.position().y,
            width: monitor.size().width,
            height: monitor.size().height,
            scale: monitor.scale_factor(),
        })
        .collect())
}

fn drag_monitor_for_point<'a>(
    monitors: &'a [MonitorBounds],
    x: i32,
    y: i32,
    preferred_id: &str,
) -> Option<&'a MonitorBounds> {
    monitors
        .iter()
        .find(|monitor| {
            x >= monitor.x
                && y >= monitor.y
                && x < monitor.x + monitor.width as i32
                && y < monitor.y + monitor.height as i32
        })
        .or_else(|| monitors.iter().find(|monitor| monitor.id == preferred_id))
        .or_else(|| monitors.first())
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

fn resize_group_placements(
    placements: &mut HashMap<String, Placement>,
    label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
    from_left: bool,
    from_top: bool,
    baseline: Option<&[Placement]>,
) -> Result<Vec<TerminalWindowState>, String> {
    let live_current = placements
        .get(label)
        .cloned()
        .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
    let live_labels = group_labels(placements, &live_current);
    let members = baseline
        .filter(|members| {
            members.iter().any(|entry| entry.label == label) && members.len() == live_labels.len()
        })
        .map(|members| members.to_vec())
        .unwrap_or_else(|| {
            live_labels
                .iter()
                .filter_map(|member_label| placements.get(member_label).cloned())
                .collect()
        });
    let current = members
        .iter()
        .find(|entry| entry.label == label)
        .cloned()
        .unwrap_or(live_current);
    let labels = members
        .iter()
        .map(|entry| entry.label.clone())
        .collect::<Vec<_>>();
    let width = width.max(TERMINAL_MIN_WIDTH);
    let height = height.max(TERMINAL_MIN_HEIGHT);
    let scale = scale.max(1.0);

    if labels.len() == 1 {
        {
            let entry = placements
                .get_mut(label)
                .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
            entry.x = x;
            entry.y = y;
            entry.width = width;
            entry.height = height;
            entry.scale = scale;
        }
        let entry = placements
            .get(label)
            .ok_or_else(|| "Mini terminal não encontrado".to_string())?;
        return Ok(vec![state_with_connections(placements, entry)]);
    }

    let group_left = members
        .iter()
        .map(|entry| entry.x)
        .min()
        .unwrap_or(current.x);
    let group_top = members
        .iter()
        .map(|entry| entry.y)
        .min()
        .unwrap_or(current.y);
    let group_right = members
        .iter()
        .map(|entry| entry.x + physical_width(entry))
        .max()
        .unwrap_or(current.x + physical_width(&current));
    let group_bottom = members
        .iter()
        .map(|entry| entry.y + physical_height(entry))
        .max()
        .unwrap_or(current.y + physical_height(&current));
    let group_width = (group_right - group_left).max(1);
    let group_height = (group_bottom - group_top).max(1);
    let requested_physical_width = (f64::from(width) * scale).round() as i32;
    let requested_physical_height = (f64::from(height) * scale).round() as i32;
    let width_delta = requested_physical_width - physical_width(&current);
    let height_delta = requested_physical_height - physical_height(&current);
    let mut width_ratio = f64::from((group_width + width_delta).max(1)) / f64::from(group_width);
    let mut height_ratio =
        f64::from((group_height + height_delta).max(1)) / f64::from(group_height);
    for member in &members {
        width_ratio = width_ratio.max(
            f64::from(TERMINAL_MIN_WIDTH) * member.scale / f64::from(physical_width(member).max(1)),
        );
        height_ratio = height_ratio.max(
            f64::from(TERMINAL_MIN_HEIGHT) * member.scale
                / f64::from(physical_height(member).max(1)),
        );
    }
    let horizontal_anchor = if from_left { group_right } else { group_left };
    let vertical_anchor = if from_top { group_bottom } else { group_top };

    for member in &members {
        let old_right = member.x + physical_width(member);
        let old_bottom = member.y + physical_height(member);
        let new_left =
            f64::from(horizontal_anchor) + f64::from(member.x - horizontal_anchor) * width_ratio;
        let new_right =
            f64::from(horizontal_anchor) + f64::from(old_right - horizontal_anchor) * width_ratio;
        let new_top =
            f64::from(vertical_anchor) + f64::from(member.y - vertical_anchor) * height_ratio;
        let new_bottom =
            f64::from(vertical_anchor) + f64::from(old_bottom - vertical_anchor) * height_ratio;
        if let Some(entry) = placements.get_mut(&member.label) {
            entry.x = new_left.round() as i32;
            entry.y = new_top.round() as i32;
            entry.width = ((new_right - new_left) / member.scale)
                .round()
                .max(f64::from(TERMINAL_MIN_WIDTH)) as i32;
            entry.height = ((new_bottom - new_top) / member.scale)
                .round()
                .max(f64::from(TERMINAL_MIN_HEIGHT)) as i32;
            if member.label == label {
                entry.scale = scale;
            }
        }
    }

    Ok(labels
        .iter()
        .filter_map(|member_label| {
            placements
                .get(member_label)
                .map(|entry| state_with_connections(placements, entry))
        })
        .collect())
}

fn state_with_connections(
    placements: &HashMap<String, Placement>,
    current: &Placement,
) -> TerminalWindowState {
    let mut state = current.state();
    let Some(group) = current.group.as_ref() else {
        return state;
    };
    let current_width = physical_width(current);
    let current_height = physical_height(current);
    for other in placements
        .values()
        .filter(|other| other.label != current.label && other.group.as_ref() == Some(group))
    {
        let other_width = physical_width(other);
        let other_height = physical_height(other);
        let vertical_overlap =
            (current.y + current_height).min(other.y + other_height) - current.y.max(other.y);
        let horizontal_overlap =
            (current.x + current_width).min(other.x + other_width) - current.x.max(other.x);
        if vertical_overlap > 0 {
            if (current.x + current_width - other.x).abs() <= 2 {
                state.connected_sides.push(DockSide::Right);
            }
            if (other.x + other_width - current.x).abs() <= 2 {
                state.connected_sides.push(DockSide::Left);
            }
        }
        if horizontal_overlap > 0 {
            if (current.y + current_height - other.y).abs() <= 2 {
                state.connected_sides.push(DockSide::Bottom);
            }
            if (other.y + other_height - current.y).abs() <= 2 {
                state.connected_sides.push(DockSide::Top);
            }
        }
    }
    state.connected_sides.sort_by_key(|side| *side as u8);
    state.connected_sides.dedup();
    state
}

fn bridge_sides_by_label(
    bridges: &HashMap<String, WorkflowBridgePlacement>,
) -> HashMap<String, Vec<DockSide>> {
    let mut sides = HashMap::<String, Vec<DockSide>>::new();
    for bridge in bridges.values() {
        if !bridge.active {
            continue;
        }
        sides
            .entry(bridge.source_label.clone())
            .or_default()
            .push(bridge.side);
        sides
            .entry(bridge.target_label.clone())
            .or_default()
            .push(opposite_side(bridge.side));
    }
    for entry in sides.values_mut() {
        entry.sort_by_key(|side| *side as u8);
        entry.dedup();
    }
    sides
}

fn workflow_bridge_groups(bridges: &HashMap<String, WorkflowBridgePlacement>) -> HashSet<String> {
    bridges
        .values()
        .filter(|bridge| bridge.active)
        .map(|bridge| bridge.group_id.clone())
        .collect()
}

fn connected_neighbor(
    placements: &HashMap<String, Placement>,
    current: &Placement,
    side: DockSide,
) -> Option<Placement> {
    let group = current.group.as_ref()?;
    let current_width = physical_width(current);
    let current_height = physical_height(current);
    placements
        .values()
        .filter(|other| other.label != current.label && other.group.as_ref() == Some(group))
        .filter_map(|other| {
            let other_width = physical_width(other);
            let other_height = physical_height(other);
            let vertical_overlap =
                (current.y + current_height).min(other.y + other_height) - current.y.max(other.y);
            let horizontal_overlap =
                (current.x + current_width).min(other.x + other_width) - current.x.max(other.x);
            let distance = match side {
                DockSide::Right if vertical_overlap > 0 => {
                    (current.x + current_width - other.x).abs()
                }
                DockSide::Left if vertical_overlap > 0 => (other.x + other_width - current.x).abs(),
                DockSide::Bottom if horizontal_overlap > 0 => {
                    (current.y + current_height - other.y).abs()
                }
                DockSide::Top if horizontal_overlap > 0 => {
                    (other.y + other_height - current.y).abs()
                }
                _ => return None,
            };
            (distance <= 2).then(|| (distance, other.clone()))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, neighbor)| neighbor)
}

fn shift(placements: &mut HashMap<String, Placement>, labels: &[String], dx: i32, dy: i32) {
    for label in labels {
        if let Some(entry) = placements.get_mut(label) {
            entry.x += dx;
            entry.y += dy;
        }
    }
}

fn apply_workflow_mode(
    placements: &mut HashMap<String, Placement>,
    enabled: bool,
) -> Vec<TerminalWindowState> {
    for placement in placements.values_mut() {
        placement.workflow_enabled = enabled;
    }
    placements
        .values()
        .map(|placement| state_with_connections(placements, placement))
        .collect()
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
            merge_touching_groups(placements, label);
            let to = states_by_label(placements, &moving_labels);
            transitions = transitions_between(from, to);
            snapped = true;
        }
    }

    let current = placements
        .get(label)
        .map(|placement| state_with_connections(placements, placement))
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
            entry.width = plan.width.max(TERMINAL_MIN_WIDTH);
            entry.height = plan.height.max(TERMINAL_MIN_HEIGHT);
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
        .filter(|other| other.monitor_id == current.monitor_id)
        .filter_map(|other| {
            snap(current, other).filter(|plan| {
                !side_is_connected(placements, other, plan.side)
                    && !side_is_connected(placements, current, opposite_side(plan.side))
            })
        })
        .filter(|plan| plan.score <= DOCK_DISTANCE)
        .min_by_key(|plan| plan.score)
}

fn side_is_connected(
    placements: &HashMap<String, Placement>,
    placement: &Placement,
    side: DockSide,
) -> bool {
    state_with_connections(placements, placement)
        .connected_sides
        .contains(&side)
}

fn opposite_side(side: DockSide) -> DockSide {
    match side {
        DockSide::Left => DockSide::Right,
        DockSide::Right => DockSide::Left,
        DockSide::Top => DockSide::Bottom,
        DockSide::Bottom => DockSide::Top,
    }
}

fn snap(current: &Placement, other: &Placement) -> Option<DockPlan> {
    let current_width = physical_width(current);
    let current_height = physical_height(current);
    let other_width = physical_width(other);
    let other_height = physical_height(other);
    let horizontal_direction = (current.x * 2 + current_width) - (other.x * 2 + other_width);
    let vertical_direction = (current.y * 2 + current_height) - (other.y * 2 + other_height);
    let vertical_overlap =
        (current.y + current_height).min(other.y + other_height) - current.y.max(other.y);
    let horizontal_overlap =
        (current.x + current_width).min(other.x + other_width) - current.x.max(other.x);
    let horizontal = || {
        (vertical_overlap > 48).then(|| {
            if horizontal_direction < 0 {
                DockPlan {
                    score: (current.x + current_width - other.x).abs(),
                    other_label: other.label.clone(),
                    side: DockSide::Left,
                    x: other.x - current_width,
                    y: other.y,
                    width: current.width,
                    height: other.height,
                }
            } else {
                DockPlan {
                    score: (current.x - (other.x + other_width)).abs(),
                    other_label: other.label.clone(),
                    side: DockSide::Right,
                    x: other.x + other_width,
                    y: other.y,
                    width: current.width,
                    height: other.height,
                }
            }
        })
    };
    let vertical = || {
        (horizontal_overlap > 48).then(|| {
            if vertical_direction < 0 {
                DockPlan {
                    score: (current.y + current_height - other.y).abs(),
                    other_label: other.label.clone(),
                    side: DockSide::Top,
                    x: other.x,
                    y: other.y - current_height,
                    width: other.width,
                    height: current.height,
                }
            } else {
                DockPlan {
                    score: (current.y - (other.y + other_height)).abs(),
                    other_label: other.label.clone(),
                    side: DockSide::Bottom,
                    x: other.x,
                    y: other.y + other_height,
                    width: other.width,
                    height: current.height,
                }
            }
        })
    };

    if horizontal_direction.abs() >= vertical_direction.abs() {
        horizontal().or_else(vertical)
    } else {
        vertical().or_else(horizontal)
    }
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
    let workflow_enabled = placements.values().any(|entry| {
        entry.workflow_enabled
            && (moving_labels.contains(&entry.label)
                || entry.label == other_label
                || placements
                    .get(other_label)
                    .and_then(|other| other.group.as_ref())
                    .is_some_and(|group| entry.group.as_ref() == Some(group)))
    });
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
            entry.workflow_enabled = workflow_enabled;
        }
    }
}

fn merge_touching_groups(placements: &mut HashMap<String, Placement>, seed_label: &str) {
    loop {
        let Some(seed) = placements.get(seed_label).cloned() else {
            return;
        };
        let Some(group_id) = seed.group.clone() else {
            return;
        };
        let members = placements
            .values()
            .filter(|entry| entry.group.as_ref() == Some(&group_id))
            .cloned()
            .collect::<Vec<_>>();
        let touching = placements
            .values()
            .filter(|entry| entry.group.as_ref() != Some(&group_id))
            .filter(|entry| {
                members.iter().any(|member| {
                    member.monitor_id == entry.monitor_id && placements_are_connected(member, entry)
                })
            })
            .map(|entry| (entry.label.clone(), entry.group.clone()))
            .collect::<Vec<_>>();
        if touching.is_empty() {
            return;
        }
        let workflow_enabled = members.iter().any(|entry| entry.workflow_enabled)
            || touching.iter().any(|(label, group)| {
                placements.values().any(|entry| {
                    entry.workflow_enabled
                        && (entry.label == *label
                            || group
                                .as_ref()
                                .is_some_and(|group| entry.group.as_ref() == Some(group)))
                })
            });
        for entry in placements.values_mut() {
            let joins = touching.iter().any(|(label, group)| {
                entry.label == *label
                    || group
                        .as_ref()
                        .is_some_and(|group| entry.group.as_ref() == Some(group))
            });
            if joins || entry.group.as_ref() == Some(&group_id) {
                entry.group = Some(group_id.clone());
                entry.workflow_enabled = workflow_enabled;
            }
        }
    }
}

fn clear_single_member_group(placements: &mut HashMap<String, Placement>, group: &str) {
    let mut members = placements
        .values()
        .filter(|entry| entry.group.as_deref() == Some(group))
        .map(|entry| entry.label.clone())
        .collect::<Vec<_>>();
    members.sort();
    let mut remaining = members.into_iter().collect::<HashSet<_>>();
    let mut components = Vec::new();

    while let Some(seed) = remaining.iter().next().cloned() {
        remaining.remove(&seed);
        let mut component = vec![seed.clone()];
        let mut pending = vec![seed];
        while let Some(label) = pending.pop() {
            let Some(current) = placements.get(&label) else {
                continue;
            };
            let connected = remaining
                .iter()
                .filter(|other_label| {
                    placements
                        .get(*other_label)
                        .is_some_and(|other| placements_are_connected(current, other))
                })
                .cloned()
                .collect::<Vec<_>>();
            for other_label in connected {
                remaining.remove(&other_label);
                pending.push(other_label.clone());
                component.push(other_label);
            }
        }
        component.sort();
        components.push(component);
    }
    components.sort_by(|left, right| left[0].cmp(&right[0]));

    for (index, component) in components.into_iter().enumerate() {
        let component_group = (component.len() > 1).then(|| {
            if index == 0 {
                group.to_string()
            } else {
                format!("{group}:{index}")
            }
        });
        for label in component {
            if let Some(entry) = placements.get_mut(&label) {
                entry.group = component_group.clone();
            }
        }
    }
}

fn placements_are_connected(left: &Placement, right: &Placement) -> bool {
    let left_width = physical_width(left);
    let left_height = physical_height(left);
    let right_width = physical_width(right);
    let right_height = physical_height(right);
    let vertical_overlap = (left.y + left_height).min(right.y + right_height) - left.y.max(right.y);
    let horizontal_overlap = (left.x + left_width).min(right.x + right_width) - left.x.max(right.x);
    (vertical_overlap > 0
        && ((left.x + left_width - right.x).abs() <= 2
            || (right.x + right_width - left.x).abs() <= 2))
        || (horizontal_overlap > 0
            && ((left.y + left_height - right.y).abs() <= 2
                || (right.y + right_height - left.y).abs() <= 2))
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

fn emit_windows_changed(app: &AppHandle) {
    let _ = app.emit("lume://terminal-windows-changed", ());
}

fn interpolate_state(transition: &WindowTransition, progress: f64) -> TerminalWindowState {
    let interpolate =
        |from: i32, to: i32| (f64::from(from) + f64::from(to - from) * progress).round() as i32;
    TerminalWindowState {
        label: transition.to.label.clone(),
        session_id: transition.to.session_id.clone(),
        session_native_id: transition.to.session_native_id.clone(),
        session_process_id: transition.to.session_process_id,
        session_agent: transition.to.session_agent.clone(),
        session_source: transition.to.session_source.clone(),
        session_project: transition.to.session_project.clone(),
        session_working_directory: transition.to.session_working_directory.clone(),
        x: interpolate(transition.from.x, transition.to.x),
        y: interpolate(transition.from.y, transition.to.y),
        width: interpolate(transition.from.width, transition.to.width),
        height: interpolate(transition.from.height, transition.to.height),
        docked: transition.to.docked,
        group_id: transition.to.group_id.clone(),
        connected_sides: transition.to.connected_sides.clone(),
        bridge_sides: transition.to.bridge_sides.clone(),
        workflow_bridge_open: transition.to.workflow_bridge_open,
        workflow_enabled: transition.to.workflow_enabled,
        monitor_id: transition.to.monitor_id.clone(),
        layered: transition.to.layered,
        scale: transition.to.scale,
    }
}

fn move_native_windows(app: &AppHandle, states: &[TerminalWindowState], resize: bool) {
    for state in states {
        let Some(window) = app.get_webview_window(&state.label) else {
            continue;
        };
        let target = state.clone();
        let layer_window = window.clone();
        let _ = window.run_on_main_thread(move || {
            if resize {
                let _ = layer_window.set_size(LogicalSize::new(
                    f64::from(target.width),
                    f64::from(target.height),
                ));
                let _ = overlay::resize_surface(&layer_window, target.width, target.height);
            }
            overlay::set_terminal_docked_shape(
                &layer_window,
                target.width,
                target.height,
                target.docked,
            );
            let _ = overlay::move_to(&layer_window, target.x, target.y, Some(&target.monitor_id));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webviews_use_canonical_routes_without_index_html() {
        assert_eq!(terminal_webview_path(), "terminal/");
        assert_eq!(workflow_bridge_webview_path(), "workflow-bridge/");
    }

    fn placement(label: &str, x: i32, y: i32) -> Placement {
        Placement {
            label: label.into(),
            session_id: label.into(),
            session_native_id: Some(label.into()),
            session_process_id: None,
            session_agent: AgentKind::Codex,
            session_source: SessionSource::Cli,
            session_project: "Lume".into(),
            session_working_directory: Some("/work/lume".into()),
            x,
            y,
            width: TERMINAL_WIDTH,
            height: TERMINAL_HEIGHT,
            scale: 1.0,
            group: None,
            workflow_enabled: false,
            monitor_id: "0:primary".into(),
            layered: false,
            ready: true,
            configured: true,
            presented: true,
            created_at: Instant::now(),
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
    fn corner_snap_connects_both_touching_edges_and_groups() {
        let mut top = placement("top", 368, 0);
        top.group = Some("top-group".into());
        let mut left = placement("left", 0, 312);
        left.group = Some("left-group".into());
        let moving = placement("moving", 380, 324);
        let mut placements = HashMap::from([
            (top.label.clone(), top),
            (left.label.clone(), left),
            (moving.label.clone(), moving),
        ]);

        let update =
            update_placements(&mut placements, "moving", 380, 324, true).expect("encaixe no canto");
        let moving = state_with_connections(&placements, &placements["moving"]);

        assert!(update.snapped);
        assert!(moving.connected_sides.contains(&DockSide::Left));
        assert!(moving.connected_sides.contains(&DockSide::Top));
        assert_eq!(placements["moving"].group, placements["top"].group);
        assert_eq!(placements["moving"].group, placements["left"].group);
    }

    #[test]
    fn docking_preview_follows_the_moving_terminal_side() {
        let target = placement("target", 500, 500);
        let cases = [
            (164, 500, DockSide::Left),
            (836, 500, DockSide::Right),
            (500, 214, DockSide::Top),
            (500, 786, DockSide::Bottom),
        ];

        for (x, y, expected) in cases {
            let moving = placement("moving", x, y);
            let plan = snap(&moving, &target).expect("prévia direcional");
            assert_eq!(plan.side, expected);
        }
    }

    #[test]
    fn docking_preview_proximity_increases_as_terminals_approach() {
        let target = placement("target", 500, 500);
        let far = snap(&placement("moving", 52, 500), &target)
            .expect("prévia distante")
            .preview();
        let close = snap(&placement("moving", 122, 500), &target)
            .expect("prévia próxima")
            .preview();

        assert!(far.proximity < close.proximity);
        assert!(close.proximity > 0.8);
    }

    #[test]
    fn docking_rejects_a_side_that_already_has_a_terminal() {
        let mut target = placement("target", 500, 500);
        let mut existing_left = placement("existing-left", 132, 500);
        target.group = Some("group".into());
        existing_left.group = Some("group".into());
        let moving = placement("moving", 116, 500);
        let placements = HashMap::from([
            (target.label.clone(), target),
            (existing_left.label.clone(), existing_left),
            (moving.label.clone(), moving),
        ]);

        assert!(dock_candidate(&placements, "moving", &["moving".into()]).is_none());
    }

    #[test]
    fn undocking_the_middle_terminal_separates_both_ends() {
        let mut left = placement("left", 100, 500);
        let mut middle = placement("middle", 468, 500);
        let mut right = placement("right", 836, 500);
        left.group = Some("row".into());
        middle.group = Some("row".into());
        right.group = Some("row".into());
        let mut placements = HashMap::from([
            (left.label.clone(), left),
            (middle.label.clone(), middle),
            (right.label.clone(), right),
        ]);

        placements
            .get_mut("middle")
            .expect("terminal central")
            .group = None;
        clear_single_member_group(&mut placements, "row");

        assert_eq!(placements["left"].group, None);
        assert_eq!(placements["middle"].group, None);
        assert_eq!(placements["right"].group, None);
    }

    #[test]
    fn preview_does_not_create_a_group_until_drop() {
        let left = placement("left", 164, 500);
        let right = placement("right", 500, 500);
        let mut placements =
            HashMap::from([(left.label.clone(), left), (right.label.clone(), right)]);

        let preview = update_placements(&mut placements, "left", 164, 500, false).expect("prévia");
        assert!(preview.preview.is_some());
        assert!(!preview.snapped);
        assert!(placements
            .values()
            .all(|placement| placement.group.is_none()));

        let dropped = update_placements(&mut placements, "left", 164, 500, true).expect("drop");
        assert!(dropped.snapped);
        assert!(placements
            .values()
            .all(|placement| placement.group.is_some()));
        assert_eq!(
            state_with_connections(&placements, &placements["left"]).connected_sides,
            vec![DockSide::Right]
        );
        assert_eq!(
            state_with_connections(&placements, &placements["right"]).connected_sides,
            vec![DockSide::Left]
        );
    }

    #[test]
    fn horizontal_dock_matches_the_target_height() {
        let mut left = placement("left", 20, 40);
        left.height = 420;
        let right = placement("right", 350, 48);
        let mut placements: HashMap<String, Placement> = HashMap::from([
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
        assert_eq!(preview.x, 505);
    }

    #[test]
    fn moving_a_docked_terminal_preserves_the_group_delta() {
        let mut left = placement("left", 20, 40);
        let mut right = placement("right", 356, 40);
        left.group = Some("pair".into());
        right.group = Some("pair".into());
        let mut placements =
            HashMap::from([(left.label.clone(), left), (right.label.clone(), right)]);

        update_placements(&mut placements, "left", 92, 64, false).expect("movimento");

        assert_eq!((placements["left"].x, placements["left"].y), (92, 64));
        assert_eq!((placements["right"].x, placements["right"].y), (428, 64));
    }

    #[test]
    fn resizing_a_docked_terminal_scales_and_keeps_the_group_connected() {
        let mut left = placement("left", 100, 80);
        let mut right = placement("right", 100 + TERMINAL_WIDTH, 80);
        left.group = Some("pair".into());
        right.group = Some("pair".into());
        let mut placements =
            HashMap::from([(left.label.clone(), left), (right.label.clone(), right)]);

        let states = resize_group_placements(
            &mut placements,
            "right",
            100 + TERMINAL_WIDTH,
            80,
            TERMINAL_WIDTH + 100,
            TERMINAL_HEIGHT + 60,
            1.0,
            false,
            false,
            None,
        )
        .expect("redimensionamento do grupo");

        assert_eq!(states.len(), 2);
        assert_eq!(placements["left"].group.as_deref(), Some("pair"));
        assert_eq!(placements["right"].group.as_deref(), Some("pair"));
        assert_eq!(placements["left"].width, placements["right"].width);
        assert_eq!(placements["left"].height, placements["right"].height);
        assert_eq!(
            placements["left"].x + physical_width(&placements["left"]),
            placements["right"].x
        );
        assert!(placements_are_connected(
            &placements["left"],
            &placements["right"]
        ));
    }

    #[test]
    fn resizing_from_the_left_keeps_the_opposite_group_edge_anchored() {
        let mut left = placement("left", 100, 80);
        let mut right = placement("right", 100 + TERMINAL_WIDTH, 80);
        left.group = Some("pair".into());
        right.group = Some("pair".into());
        let original_right_edge = right.x + physical_width(&right);
        let mut placements =
            HashMap::from([(left.label.clone(), left), (right.label.clone(), right)]);

        resize_group_placements(
            &mut placements,
            "left",
            0,
            80,
            TERMINAL_WIDTH + 100,
            TERMINAL_HEIGHT,
            1.0,
            true,
            false,
            None,
        )
        .expect("redimensionamento pela esquerda");

        assert_eq!(
            placements["right"].x + physical_width(&placements["right"]),
            original_right_edge
        );
        assert!(placements_are_connected(
            &placements["left"],
            &placements["right"]
        ));
    }

    #[test]
    fn repeated_top_right_resize_frames_keep_the_group_bottom_left_anchored() {
        let mut left = placement("left", 100, 80);
        let mut right = placement("right", 100 + TERMINAL_WIDTH, 80);
        left.group = Some("pair".into());
        right.group = Some("pair".into());
        let original_left = left.x;
        let original_bottom = left.y + physical_height(&left);
        let original_right = right.x + physical_width(&right);
        let original_right_x = right.x;
        let mut placements =
            HashMap::from([(left.label.clone(), left), (right.label.clone(), right)]);
        let baseline = placements.values().cloned().collect::<Vec<_>>();

        for (width_delta, height_delta) in [(40, 20), (100, 60)] {
            resize_group_placements(
                &mut placements,
                "right",
                original_right_x,
                80 - height_delta,
                TERMINAL_WIDTH + width_delta,
                TERMINAL_HEIGHT + height_delta,
                1.0,
                false,
                true,
                Some(&baseline),
            )
            .expect("frame superior direito");
        }

        assert_eq!(placements["left"].x, original_left);
        assert_eq!(
            placements["left"].y + physical_height(&placements["left"]),
            original_bottom
        );
        assert_eq!(
            placements["right"].x + physical_width(&placements["right"]),
            original_right + 100
        );
        assert_eq!(placements["left"].y, 80 - 60);
        assert!(placements_are_connected(
            &placements["left"],
            &placements["right"]
        ));
    }

    #[test]
    fn workflow_mode_changes_all_terminals_without_changing_their_geometry() {
        let mut placements: HashMap<String, Placement> = HashMap::from([
            ("left".into(), placement("left", 20, 30)),
            ("right".into(), placement("right", 20 + TERMINAL_WIDTH, 30)),
        ]);
        for placement in placements.values_mut() {
            placement.group = Some("group-1".into());
        }
        let before = placements
            .iter()
            .map(|(label, placement)| {
                (
                    label.clone(),
                    (placement.x, placement.y, placement.width, placement.height),
                )
            })
            .collect::<HashMap<_, _>>();

        let states = apply_workflow_mode(&mut placements, true);

        assert_eq!(states.len(), 2);
        assert!(states.iter().all(|state| state.workflow_enabled));
        for (label, placement) in &placements {
            assert_eq!(
                before[label],
                (placement.x, placement.y, placement.width, placement.height)
            );
        }
    }

    #[test]
    fn terminals_on_different_monitors_never_dock() {
        let left = placement("left", 20, 40);
        let mut right = placement("right", 350, 48);
        right.monitor_id = "1:test".into();
        let placements = HashMap::from([(left.label.clone(), left), (right.label.clone(), right)]);

        assert!(dock_candidate(&placements, "left", &["left".into()]).is_none());
    }

    #[test]
    fn resized_terminal_keeps_its_new_native_position() {
        let registry = TerminalWindows::default();
        let terminal = placement("terminal", 20, 40);
        registry
            .placements
            .lock()
            .expect("posições")
            .insert(terminal.label.clone(), terminal);

        registry.resize_placement("terminal", Some((130, 150)), 420, 310, 1.0, true, true);
        let state = registry.state("terminal").expect("terminal");

        assert_eq!((state.x, state.y), (130, 150));
        assert_eq!((state.width, state.height), (420, 310));
    }

    #[test]
    fn docking_preview_appears_with_a_comfortable_visual_gap() {
        let left = placement("left", 20, 40);
        let right = placement("right", 430, 48);
        let plan = snap(&left, &right).expect("candidato");
        assert!(plan.score <= DOCK_DISTANCE);
    }

    #[test]
    fn rejects_compositor_outlier_coordinates() {
        assert!(validate_coordinates(100_000, 20).is_err());
    }

    #[test]
    fn terminal_opened_near_the_edge_stays_inside_the_monitor() {
        let (x, y) = clamp_drag_to_monitor(
            1_920,
            1_080,
            TERMINAL_WIDTH,
            TERMINAL_HEIGHT,
            1_920,
            1_080,
            1.0,
        );
        assert_eq!(x, 1_540);
        assert_eq!(y, 756);
    }

    #[test]
    fn terminal_clamp_accounts_for_windows_display_scaling() {
        let (x, y) = clamp_drag_to_monitor(
            1_900,
            1_050,
            TERMINAL_WIDTH,
            TERMINAL_HEIGHT,
            1_920,
            1_080,
            1.25,
        );
        assert_eq!(x, 1_448);
        assert_eq!(y, 678);
    }

    #[test]
    fn xwayland_drag_selects_the_monitor_under_the_window() {
        let monitors = [
            MonitorBounds {
                id: "left".into(),
                x: 0,
                y: 0,
                width: 1_920,
                height: 1_080,
                scale: 1.0,
            },
            MonitorBounds {
                id: "right".into(),
                x: 1_920,
                y: 0,
                width: 2_560,
                height: 1_440,
                scale: 1.25,
            },
        ];

        let selected =
            drag_monitor_for_point(&monitors, 2_100, 200, "left").expect("monitor direito");
        assert_eq!(selected.id, "right");
        assert_eq!(selected.scale, 1.25);
    }

    #[test]
    fn open_bridge_keeps_the_visual_connection_on_both_terminal_edges() {
        let bridges = HashMap::from([(
            "bridge".into(),
            WorkflowBridgePlacement {
                group_id: "group".into(),
                source_label: "left".into(),
                target_label: "right".into(),
                source_session_native_id: "source".into(),
                target_session_native_id: "target".into(),
                side: DockSide::Right,
                native_connectors: true,
                monitor_id: "monitor".into(),
                x: 0,
                y: 0,
                width: WORKFLOW_BRIDGE_HORIZONTAL_WIDTH,
                height: WORKFLOW_BRIDGE_HORIZONTAL_HEIGHT,
                rendered_height: WORKFLOW_BRIDGE_HORIZONTAL_HEIGHT,
                compact_positions: HashMap::new(),
                expanded: false,
                active: true,
                prepared_at: Instant::now(),
                original_positions: HashMap::new(),
            },
        )]);

        let sides = bridge_sides_by_label(&bridges);

        assert_eq!(sides["left"], vec![DockSide::Right]);
        assert_eq!(sides["right"], vec![DockSide::Left]);
    }
}
