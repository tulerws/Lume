use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    #[serde(rename = "chatgpt")]
    ChatGpt,
    Claude,
    ClaudeCode,
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

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptDelivery {
    #[default]
    NewTurn,
    Steer,
    Queue,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileScope {
    Monitor,
    Prompt,
    Approve,
    Terminate,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub scopes: Vec<MobileScope>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionProfile {
    pub mode: AccessMode,
    pub label: String,
    pub approval_policy: String,
    #[serde(default)]
    pub approvals_reviewer: Option<String>,
    pub can_respond_from_lume: bool,
    pub available_actions: Vec<PermissionAction>,
}

impl PermissionProfile {
    pub fn automatically_approves(&self) -> bool {
        if self.mode == AccessMode::FullAccess {
            return true;
        }
        self.approvals_reviewer.as_deref().is_some_and(|reviewer| {
            matches!(
                reviewer.trim().to_ascii_lowercase().as_str(),
                "auto_review" | "auto-review" | "approve_for_me" | "approve-for-me"
            )
        })
    }
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
pub struct PromptAttachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub preview_data_url: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptAttachmentInput {
    pub name: String,
    pub mime_type: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub data_base64: Option<String>,
    #[serde(default)]
    pub preview_data_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractiveQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub is_other: bool,
    pub is_secret: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestion {
    pub id: String,
    pub questions: Vec<InteractiveQuestion>,
    pub requested_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub question_id: String,
    pub answers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRateLimit {
    pub id: String,
    pub label: String,
    pub used_percent: u8,
    pub resets_at: Option<i64>,
    pub window_minutes: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActivity {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    pub status: String,
    pub created_at: i64,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
    #[serde(default, skip_serializing)]
    pub append_detail: bool,
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
    #[serde(default)]
    pub session_name: String,
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
    pub pending_question: Option<PendingQuestion>,
    #[serde(default)]
    pub last_response: Option<String>,
    #[serde(default)]
    pub results: Vec<SessionResult>,
    #[serde(default)]
    pub activities: Vec<SessionActivity>,
    #[serde(default)]
    pub rate_limits: Vec<AgentRateLimit>,
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRole {
    Planner,
    Implementer,
    Reviewer,
    Tester,
    Researcher,
    #[default]
    Custom,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowRoleContract {
    pub instruction: String,
    pub expected_input: String,
    pub produced_output: String,
    pub completion_condition: String,
}

impl WorkflowRole {
    pub fn default_contract(self) -> WorkflowRoleContract {
        let fields = match self {
            Self::Planner => (
                "Analyze the objective, constraints, dependencies, and risks. Produce a clear, ordered, actionable plan. Resolve important ambiguities before handing work off, and do not implement the solution unless explicitly requested.",
                "The objective, relevant context, constraints, and any existing decisions.",
                "An ordered implementation plan with key decisions, dependencies, risks, and validation steps.",
                "The plan is actionable, covers the full requested scope, and clearly identifies any remaining blocker or decision.",
            ),
            Self::Implementer => (
                "Implement the approved objective while preserving the requested scope and the project's existing conventions. Inspect the relevant code before changing it, keep changes focused, and validate the result.",
                "An approved plan or a precise objective, plus the relevant project context and constraints.",
                "Working changes, a concise list of affected files, and the checks or tests performed.",
                "The requested behavior is implemented, relevant validation passes, and no known blocking issue remains.",
            ),
            Self::Reviewer => (
                "Review the proposed work against its objective. Look for correctness issues, regressions, security or privacy risks, missing edge cases, and maintainability problems. Prioritize concrete findings and do not modify the work unless explicitly requested.",
                "The objective, implementation result, changed files, and available validation evidence.",
                "Prioritized findings with evidence and recommended corrections, or an explicit approval when no actionable issue is found.",
                "Every relevant area has been reviewed and each actionable finding has clear evidence and severity.",
            ),
            Self::Tester => (
                "Validate the requested behavior with focused, reproducible checks. Cover the main path, relevant edge cases, and likely regressions. Report failures precisely and avoid changing product behavior unless explicitly requested.",
                "The objective, expected behavior, implementation result, and available test environment.",
                "Executed checks with pass or fail results, reproducible failure details, and remaining coverage gaps.",
                "All planned checks have run and the final status, failures, and untested risks are clearly documented.",
            ),
            Self::Researcher => (
                "Investigate the question using relevant project evidence and authoritative sources when needed. Compare viable alternatives, separate verified facts from inference, and recommend the best-supported direction.",
                "The research question, decision context, constraints, and preferred evidence sources.",
                "Concise findings, compared alternatives, supporting evidence, uncertainties, and a recommendation.",
                "The research question is answered with sufficient evidence and remaining uncertainty is explicitly stated.",
            ),
            Self::Custom => ("", "", "", ""),
        };
        WorkflowRoleContract {
            instruction: fields.0.into(),
            expected_input: fields.1.into(),
            produced_output: fields.2.into(),
            completion_condition: fields.3.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowStepDefinition {
    pub id: String,
    pub session_native_id: String,
    pub role: WorkflowRole,
    pub custom_role_label: String,
    pub instruction: String,
    pub expected_input: String,
    pub produced_output: String,
    pub completion_condition: String,
    pub attempt: u16,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowAdvanceMode {
    #[default]
    Manual,
    Automatic,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowContextPolicy {
    Minimal,
    #[default]
    Standard,
    Detailed,
    Custom,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowContextSelection {
    pub response: bool,
    pub files: bool,
    pub checks: bool,
    pub plan: bool,
    pub activity: bool,
    pub diffs: bool,
}

impl Default for WorkflowContextSelection {
    fn default() -> Self {
        Self {
            response: true,
            files: true,
            checks: true,
            plan: false,
            activity: false,
            diffs: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowConnectionDefinition {
    pub id: String,
    pub from_step_id: String,
    pub to_step_id: String,
    pub include_response: bool,
    pub include_files: bool,
    pub include_tests: bool,
    pub context_policy: WorkflowContextPolicy,
    pub context_selection: WorkflowContextSelection,
    pub additional_instruction: String,
    pub requires_approval: bool,
    pub advance_mode: WorkflowAdvanceMode,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowGroupDefinition {
    pub id: String,
    pub terminal_group_id: String,
    pub steps: Vec<WorkflowStepDefinition>,
    pub connections: Vec<WorkflowConnectionDefinition>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    #[default]
    Draft,
    Ready,
    Running,
    WaitingForApproval,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepRunStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowStepRun {
    pub step_id: String,
    pub status: WorkflowStepRunStatus,
    pub attempt: u16,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub result_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub objective: String,
    pub status: WorkflowRunStatus,
    pub current_step_id: Option<String>,
    pub pending_connection_id: Option<String>,
    pub handoff_approved: bool,
    pub recovering: bool,
    pub steps: Vec<WorkflowStepRun>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub language: String,
    pub dark_mode: Option<bool>,
    pub sound_enabled: bool,
    pub sound_volume: u8,
    pub popup_notifications_enabled: bool,
    pub autostart: bool,
    pub monitor_id: Option<String>,
    pub overlay_x: Option<i32>,
    pub overlay_y: Option<i32>,
    pub show_over_fullscreen: bool,
    pub history_retention_days: u16,
    pub launch_target: String,
    pub project_profiles: HashMap<String, ProjectProfile>,
    pub session_aliases: HashMap<String, String>,
    pub whiteboard_layouts: Vec<WhiteboardLayout>,
    pub workflow_enabled: bool,
    pub workflow_groups: Vec<WorkflowGroupDefinition>,
    pub global_shortcut: String,
    pub open_shortcut: String,
    pub new_session_shortcut: String,
    pub whiteboard_shortcut: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: "en".into(),
            dark_mode: None,
            sound_enabled: true,
            sound_volume: 55,
            popup_notifications_enabled: true,
            autostart: true,
            monitor_id: None,
            overlay_x: None,
            overlay_y: None,
            show_over_fullscreen: false,
            history_retention_days: 30,
            launch_target: "auto".into(),
            project_profiles: HashMap::new(),
            session_aliases: HashMap::new(),
            whiteboard_layouts: Vec::new(),
            workflow_enabled: false,
            workflow_groups: Vec::new(),
            global_shortcut: "Ctrl+Shift+Space".into(),
            open_shortcut: "Ctrl+Alt+Shift+L".into(),
            new_session_shortcut: "Ctrl+Alt+Shift+N".into(),
            whiteboard_shortcut: "Ctrl+Alt+Shift+B".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKind {
    SessionStarted,
    Running,
    Activity,
    PermissionRequest,
    QuestionRequest,
    WaitingForInput,
    Completed,
    Failed,
    SessionEnded,
}

pub fn should_notify(event: &HookEventKind, previous: Option<&SessionStatus>) -> bool {
    match event {
        HookEventKind::PermissionRequest => previous != Some(&SessionStatus::PermissionRequired),
        HookEventKind::QuestionRequest => true,
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
    #[serde(default)]
    pub session_name: Option<String>,
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
    pub question: Option<PendingQuestion>,
    #[serde(default)]
    pub last_response: Option<String>,
    #[serde(default)]
    pub activity: Option<SessionActivity>,
    #[serde(default)]
    pub activities: Vec<SessionActivity>,
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

    #[test]
    fn full_access_and_auto_review_profiles_approve_automatically() {
        let mut profile = PermissionProfile {
            mode: AccessMode::FullAccess,
            label: "Acesso total".into(),
            approval_policy: "never".into(),
            approvals_reviewer: None,
            can_respond_from_lume: true,
            available_actions: Vec::new(),
        };
        assert!(profile.automatically_approves());

        profile.mode = AccessMode::WorkspaceWrite;
        profile.approvals_reviewer = Some("auto_review".into());
        assert!(profile.automatically_approves());

        profile.approvals_reviewer = None;
        assert!(!profile.automatically_approves());
    }

    #[test]
    fn standard_workflow_roles_have_contracts_but_custom_starts_empty() {
        for role in [
            WorkflowRole::Planner,
            WorkflowRole::Implementer,
            WorkflowRole::Reviewer,
            WorkflowRole::Tester,
            WorkflowRole::Researcher,
        ] {
            let contract = role.default_contract();
            assert!(!contract.instruction.is_empty());
            assert!(!contract.expected_input.is_empty());
            assert!(!contract.produced_output.is_empty());
            assert!(!contract.completion_condition.is_empty());
        }

        let custom = WorkflowRole::Custom.default_contract();
        assert!(custom.instruction.is_empty());
        assert!(custom.expected_input.is_empty());
        assert!(custom.produced_output.is_empty());
        assert!(custom.completion_condition.is_empty());
    }

    #[test]
    fn legacy_workflow_connections_gain_the_standard_context_policy() {
        let connection: WorkflowConnectionDefinition = serde_json::from_value(serde_json::json!({
            "id": "edge-1",
            "fromStepId": "source",
            "toStepId": "target",
            "includeResponse": true,
            "includeFiles": true,
            "includeTests": true
        }))
        .expect("legacy connection");

        assert_eq!(connection.context_policy, WorkflowContextPolicy::Standard);
        assert_eq!(
            connection.context_selection,
            WorkflowContextSelection::default()
        );
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookResponse {
    pub ok: bool,
    pub action: Option<PermissionAction>,
    #[serde(default)]
    pub question_answers: Option<Vec<QuestionAnswer>>,
    pub message: Option<String>,
}
