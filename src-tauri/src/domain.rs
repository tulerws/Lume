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

/// Por que uma decisão de permissão foi recusada.
///
/// Existe tipada, e não como texto, porque o controle remoto precisa traduzir
/// cada motivo num código que o aplicativo trata de forma **diferente**:
/// `permission_gone` é situação normal — "respondida em outro aparelho" — e
/// `action_not_available` é erro de cliente. Com `String` no lugar, a tradução
/// seria comparação de mensagem, e reescrever uma frase mudaria o comportamento
/// do celular sem erro de compilação e sem aviso.
///
/// O `Display` reproduz **exatamente** o texto que o desktop já mostrava antes
/// desta tipagem existir, e o teste `the_desktop_wording_is_unchanged` prende
/// cada frase.
#[derive(Clone, Debug, PartialEq)]
pub enum PermissionDenial {
    SessionNotFound,
    NoPendingPermission,
    PermissionMismatch,
    ActionNotAllowed,
    SourceIsNotLume,
    /// Abrir a origem não é decisão de permissão: é navegação, e o desktop a
    /// intercepta antes de chegar ao backend.
    OpenSourceIsNotADecision,
    /// Falha de infraestrutura — cadeado envenenado, escrita no banco. A
    /// mensagem serve ao desktop; o celular recebe só o código `internal`.
    Internal(String),
}

impl std::fmt::Display for PermissionDenial {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound => formatter.write_str("Sessão não encontrada"),
            Self::NoPendingPermission => {
                formatter.write_str("A sessão não possui uma permissão pendente")
            }
            Self::PermissionMismatch => {
                formatter.write_str("A permissão não corresponde à sessão")
            }
            Self::ActionNotAllowed => {
                formatter.write_str("Esta ação não é permitida pela configuração da sessão")
            }
            Self::SourceIsNotLume => {
                formatter.write_str("Esta origem deve ser aberta na interface original")
            }
            Self::OpenSourceIsNotADecision => {
                formatter.write_str("Use a origem da sessão para continuar")
            }
            Self::Internal(message) => formatter.write_str(message),
        }
    }
}

/// Por que uma sessão não foi encerrada.
///
/// Mesma razão de [`PermissionDenial`] e [`PromptRefusal`] existirem: o celular
/// precisa distinguir "esta sessão sumiu" de "esta sessão nunca poderá ser
/// encerrada daqui". A segunda diz ao aplicativo para esconder o botão, e não
/// para tentar de novo.
#[derive(Clone, Debug, PartialEq)]
pub enum TerminationRefusal {
    SessionNotFound,
    /// Integração que divide o processo com o editor ou o navegador. Encerrar
    /// fecharia o VS Code ou o Chrome inteiro, não o agente.
    SharedProcess,
    NoProcess,
    Internal(String),
}

impl std::fmt::Display for TerminationRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound => formatter.write_str("Sessão não encontrada"),
            Self::SharedProcess => formatter.write_str(
                "Esta integração não possui um processo isolado; o Lume não fechará o editor ou navegador inteiro",
            ),
            Self::NoProcess => {
                formatter.write_str("A sessão não possui um processo associado")
            }
            Self::Internal(message) => formatter.write_str(message),
        }
    }
}

/// Por que um prompt não foi enviado.
///
/// Mesma razão de [`PermissionDenial`] existir: o celular precisa saber a
/// diferença entre "espere o agente terminar" (`session_busy`, e ele tenta de
/// novo depois) e "esta sessão não tem como retomar" (`action_not_available`, e
/// o botão não devia existir). O `Display` reproduz o texto que o comando do
/// Tauri já devolvia, e `the_prompt_wording_is_unchanged` prende cada frase.
#[derive(Clone, Debug, PartialEq)]
pub enum PromptRefusal {
    Empty,
    TooLarge,
    SessionNotFound,
    /// Agente ocupado. É recusa temporária, e a única aqui que vale repetir.
    SessionBusy,
    CodexThreadMissing,
    AgentWithoutResume,
    ResumeIdMissing,
    WorkingDirectoryMissing,
    Internal(String),
}

impl std::fmt::Display for PromptRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Digite um prompt antes de enviar"),
            Self::TooLarge => formatter.write_str("O prompt excede o limite local de 16 KB"),
            Self::SessionNotFound => formatter.write_str("Sessão não encontrada"),
            Self::SessionBusy => {
                formatter.write_str("Aguarde o agente terminar antes de enviar outro prompt")
            }
            Self::CodexThreadMissing => {
                formatter.write_str("A sessão do Codex não informou a thread")
            }
            Self::AgentWithoutResume => {
                formatter.write_str("Este agente não oferece retomada direta pelo Lume")
            }
            Self::ResumeIdMissing => {
                formatter.write_str("A sessão não informou um identificador para retomada")
            }
            Self::WorkingDirectoryMissing => {
                formatter.write_str("A sessão não informou a pasta do projeto")
            }
            Self::Internal(message) => formatter.write_str(message),
        }
    }
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
    #[serde(default)]
    pub approvals_reviewer: Option<String>,
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
    #[serde(default)]
    pub activities: Vec<SessionActivity>,
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

/// Aparelho autorizado a controlar este Lume remotamente.
///
/// **O hash do token não está aqui, e é de propósito.** Este tipo é serializado
/// para a interface; credencial, mesmo em forma de hash, não pode viajar junto
/// por descuido. Quem precisa do hash pede explicitamente ao `Store`.
/// Espelho de `RemoteStatus` em `src/lib/domain.ts`. Campo que mudar aqui muda
/// lá, ou a linha em Ajustes passa a mentir sem ninguém perceber.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub available: bool,
    pub enabled: bool,
    pub port: u16,
    pub paired_devices: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
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
    pub activity: Option<SessionActivity>,
    #[serde(default)]
    pub wait_for_decision: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tipar o erro do `resolve_permission` foi mudança para o controle remoto,
    /// e ela não tinha permissão de mexer no que o desktop mostra. Estas são as
    /// frases exatas que `AppState::resolve_permission` devolvia como `String`
    /// antes de `PermissionDenial` existir.
    #[test]
    fn the_desktop_wording_is_unchanged() {
        let cases = [
            (PermissionDenial::SessionNotFound, "Sessão não encontrada"),
            (
                PermissionDenial::NoPendingPermission,
                "A sessão não possui uma permissão pendente",
            ),
            (
                PermissionDenial::PermissionMismatch,
                "A permissão não corresponde à sessão",
            ),
            (
                PermissionDenial::ActionNotAllowed,
                "Esta ação não é permitida pela configuração da sessão",
            ),
            (
                PermissionDenial::SourceIsNotLume,
                "Esta origem deve ser aberta na interface original",
            ),
            (
                PermissionDenial::OpenSourceIsNotADecision,
                "Use a origem da sessão para continuar",
            ),
        ];
        for (denial, expected) in cases {
            assert_eq!(denial.to_string(), expected);
        }

        // A falha de infraestrutura carrega a mensagem de quem falhou, sem
        // moldura em volta: era o que o `?` produzia antes.
        assert_eq!(
            PermissionDenial::Internal("Não foi possível salvar a decisão".into()).to_string(),
            "Não foi possível salvar a decisão"
        );
    }

    /// E para o encerramento. `stop_session` foi extraído do comando do Tauri
    /// pelo mesmo motivo, e também sem licença para mudar o texto do desktop.
    #[test]
    fn the_termination_wording_is_unchanged() {
        assert_eq!(
            TerminationRefusal::SessionNotFound.to_string(),
            "Sessão não encontrada"
        );
        assert_eq!(
            TerminationRefusal::SharedProcess.to_string(),
            "Esta integração não possui um processo isolado; o Lume não fechará o editor ou navegador inteiro"
        );
        assert_eq!(
            TerminationRefusal::NoProcess.to_string(),
            "A sessão não possui um processo associado"
        );
    }

    /// Mesma coisa para o prompt: extrair `send_prompt` do comando do Tauri não
    /// tinha permissão de mudar o que o usuário do desktop lê.
    #[test]
    fn the_prompt_wording_is_unchanged() {
        let cases = [
            (PromptRefusal::Empty, "Digite um prompt antes de enviar"),
            (
                PromptRefusal::TooLarge,
                "O prompt excede o limite local de 16 KB",
            ),
            (PromptRefusal::SessionNotFound, "Sessão não encontrada"),
            (
                PromptRefusal::SessionBusy,
                "Aguarde o agente terminar antes de enviar outro prompt",
            ),
            (
                PromptRefusal::CodexThreadMissing,
                "A sessão do Codex não informou a thread",
            ),
            (
                PromptRefusal::AgentWithoutResume,
                "Este agente não oferece retomada direta pelo Lume",
            ),
            (
                PromptRefusal::ResumeIdMissing,
                "A sessão não informou um identificador para retomada",
            ),
            (
                PromptRefusal::WorkingDirectoryMissing,
                "A sessão não informou a pasta do projeto",
            ),
        ];
        for (refusal, expected) in cases {
            assert_eq!(refusal.to_string(), expected);
        }
    }

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
