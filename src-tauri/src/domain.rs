use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    Claude,
    Gemini,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSource {
    Cli,
    Vscode,
    Web,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    PermissionRequired,
    Completed,
    Failed,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    FullAccess,
    WorkspaceWrite,
    ReadOnly,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    AllowOnce,
    AllowSession,
    Deny,
    OpenSource,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionProfile {
    pub mode: AccessMode,
    pub label: String,
    pub approval_policy: String,
    pub can_respond_from_lume: bool,
    pub available_actions: Vec<PermissionAction>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub resource: String,
    pub risk: String,
    pub requested_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: String,
    pub agent: AgentKind,
    pub agent_label: String,
    pub project: String,
    pub source: SessionSource,
    pub status: SessionStatus,
    pub status_label: String,
    pub started_at: String,
    pub permission_profile: PermissionProfile,
    pub pending_permission: Option<PermissionRequest>,
}

impl AgentSession {
    pub fn demo_sessions() -> Vec<Self> {
        vec![
            Self {
                id: "codex-lume".into(),
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
                project: "Lume".into(),
                source: SessionSource::Vscode,
                status: SessionStatus::Running,
                status_label: "Implementando a interface".into(),
                started_at: "2026-07-20T15:32:00-03:00".into(),
                permission_profile: PermissionProfile {
                    mode: AccessMode::WorkspaceWrite,
                    label: "Acesso ao projeto".into(),
                    approval_policy: "Pede confirmação fora do workspace".into(),
                    can_respond_from_lume: true,
                    available_actions: vec![PermissionAction::AllowOnce, PermissionAction::Deny],
                },
                pending_permission: None,
            },
            Self {
                id: "claude-api".into(),
                agent: AgentKind::Claude,
                agent_label: "Claude".into(),
                project: "vibeservice-api".into(),
                source: SessionSource::Cli,
                status: SessionStatus::PermissionRequired,
                status_label: "Aguardando permissão".into(),
                started_at: "2026-07-20T15:28:00-03:00".into(),
                permission_profile: PermissionProfile {
                    mode: AccessMode::ReadOnly,
                    label: "Somente leitura".into(),
                    approval_policy: "Confirma alterações e comandos".into(),
                    can_respond_from_lume: true,
                    available_actions: vec![
                        PermissionAction::AllowOnce,
                        PermissionAction::AllowSession,
                        PermissionAction::Deny,
                    ],
                },
                pending_permission: Some(PermissionRequest {
                    id: "permission-claude-1".into(),
                    kind: "command".into(),
                    summary: "Executar a suíte de testes do projeto".into(),
                    resource: "npm test".into(),
                    risk: "low".into(),
                    requested_at: "2026-07-20T15:35:00-03:00".into(),
                }),
            },
            Self {
                id: "gemini-web".into(),
                agent: AgentKind::Gemini,
                agent_label: "Gemini".into(),
                project: "Pesquisa de referências".into(),
                source: SessionSource::Web,
                status: SessionStatus::Completed,
                status_label: "Finalizado há 2 min".into(),
                started_at: "2026-07-20T15:18:00-03:00".into(),
                permission_profile: PermissionProfile {
                    mode: AccessMode::FullAccess,
                    label: "Sem acesso local".into(),
                    approval_policy: "Monitoramento da aba".into(),
                    can_respond_from_lume: false,
                    available_actions: vec![PermissionAction::OpenSource],
                },
                pending_permission: None,
            },
        ]
    }
}
