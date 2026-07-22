use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    Claude,
    Gemini,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    Cli,
    Vscode,
    Web,
    Desktop,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    PermissionRequired,
    WaitingForInput,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    FullAccess,
    WorkspaceWrite,
    ReadOnly,
    Plan,
    Custom,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    AllowOnce,
    AllowSession,
    Deny,
    OpenSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionProfile {
    pub mode: AccessMode,
    pub label: String,
    pub approval_policy: String,
    pub can_respond_from_lume: bool,
    pub available_actions: Vec<PermissionAction>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub resource: String,
    pub risk: String,
    pub requested_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResult {
    pub id: String,
    pub response: String,
    pub created_at: i64,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub tests: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultNote {
    pub id: String,
    pub title: String,
    pub body: String,
    pub agent_label: String,
    pub project: String,
    pub files: Vec<String>,
    pub tests: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: String,
    pub agent: AgentKind,
    pub agent_label: String,
    pub project: String,
    pub source: SessionSource,
    #[serde(default)]
    pub source_app: Option<String>,
    pub status: SessionStatus,
    pub status_label: String,
    pub started_at: String,
    pub updated_at: i64,
    pub process_id: Option<u32>,
    pub native_session_id: Option<String>,
    pub working_directory: Option<String>,
    pub permission_profile: PermissionProfile,
    pub pending_permission: Option<PermissionRequest>,
    #[serde(default)]
    pub last_response: Option<String>,
    #[serde(default)]
    pub results: Vec<SessionResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub session_id: String,
    pub agent_label: String,
    pub project: String,
    pub event: String,
    pub summary: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectProfile {
    pub label: String,
    pub sound_enabled: bool,
    pub launch_target: Option<String>,
    pub monitor_id: Option<String>,
    pub overlay_x: Option<i32>,
    pub overlay_y: Option<i32>,
    pub permission_mode: Option<AccessMode>,
    pub approval_policy: Option<String>,
    pub whiteboard_layout_id: Option<String>,
    pub preferred_agents: Vec<String>,
}

impl Default for ProjectProfile {
    fn default() -> Self {
        Self {
            label: String::new(),
            sound_enabled: true,
            launch_target: None,
            monitor_id: None,
            overlay_x: None,
            overlay_y: None,
            permission_mode: None,
            approval_policy: None,
            whiteboard_layout_id: None,
            preferred_agents: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhiteboardLayoutTerminal {
    pub agent: AgentKind,
    pub agent_label: String,
    pub project: String,
    pub source: SessionSource,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub group_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhiteboardLayout {
    pub id: String,
    pub name: String,
    pub terminals: Vec<WhiteboardLayoutTerminal>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub language: String,
    pub dark_mode: Option<bool>,
    pub sound_enabled: bool,
    pub autostart: bool,
    pub monitor_id: Option<String>,
    pub overlay_x: Option<i32>,
    pub overlay_y: Option<i32>,
    pub show_over_fullscreen: bool,
    pub history_retention_days: u16,
    pub launch_target: String,
    pub project_profiles: HashMap<String, ProjectProfile>,
    pub whiteboard_layouts: Vec<WhiteboardLayout>,
    pub global_shortcut: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: "en".into(),
            dark_mode: None,
            sound_enabled: true,
            autostart: true,
            monitor_id: None,
            overlay_x: None,
            overlay_y: None,
            show_over_fullscreen: false,
            history_retention_days: 30,
            launch_target: "auto".into(),
            project_profiles: HashMap::new(),
            whiteboard_layouts: Vec::new(),
            global_shortcut: "Ctrl+Shift+Space".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKind {
    SessionStarted,
    Running,
    PermissionRequest,
    WaitingForInput,
    Completed,
    Failed,
    SessionEnded,
}

pub fn should_notify(event: &HookEventKind, previous: Option<&SessionStatus>) -> bool {
    match event {
        HookEventKind::PermissionRequest => previous != Some(&SessionStatus::PermissionRequired),
        HookEventKind::Completed => matches!(
            previous,
            Some(SessionStatus::Running | SessionStatus::PermissionRequired)
        ),
        HookEventKind::Failed => previous.is_some() && previous != Some(&SessionStatus::Failed),
        _ => false,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEvent {
    pub event: HookEventKind,
    pub session_id: String,
    pub agent: AgentKind,
    pub agent_label: Option<String>,
    pub project: Option<String>,
    pub source: Option<SessionSource>,
    #[serde(default)]
    pub source_app: Option<String>,
    pub status_label: Option<String>,
    pub started_at: Option<String>,
    pub process_id: Option<u32>,
    pub native_session_id: Option<String>,
    pub working_directory: Option<String>,
    pub permission_profile: Option<PermissionProfile>,
    pub permission: Option<PermissionRequest>,
    #[serde(default)]
    pub last_response: Option<String>,
    #[serde(default)]
    pub wait_for_decision: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_only_fire_on_meaningful_task_transitions() {
        assert!(should_notify(
            &HookEventKind::Completed,
            Some(&SessionStatus::Running)
        ));
        assert!(!should_notify(
            &HookEventKind::Completed,
            Some(&SessionStatus::Completed)
        ));
        assert!(should_notify(
            &HookEventKind::PermissionRequest,
            Some(&SessionStatus::Running)
        ));
        assert!(!should_notify(
            &HookEventKind::PermissionRequest,
            Some(&SessionStatus::PermissionRequired)
        ));
        assert!(!should_notify(&HookEventKind::SessionEnded, None));
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookResponse {
    pub ok: bool,
    pub action: Option<PermissionAction>,
    pub message: Option<String>,
}
