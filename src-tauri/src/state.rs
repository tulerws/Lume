use std::sync::Mutex;

use crate::domain::{AgentSession, PermissionAction, SessionStatus};

pub struct AppState {
    sessions: Mutex<Vec<AgentSession>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(AgentSession::demo_sessions()),
        }
    }
}

impl AppState {
    pub fn sessions(&self) -> Result<Vec<AgentSession>, String> {
        self.sessions
            .lock()
            .map(|sessions| sessions.clone())
            .map_err(|_| "Não foi possível acessar as sessões".to_string())
    }

    pub fn resolve_permission(
        &self,
        session_id: &str,
        permission_id: &str,
        action: PermissionAction,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?;

        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Sessão não encontrada".to_string())?;

        let pending = session
            .pending_permission
            .as_ref()
            .ok_or_else(|| "A sessão não possui uma permissão pendente".to_string())?;

        if pending.id != permission_id {
            return Err("A permissão não corresponde à sessão".into());
        }

        if !session
            .permission_profile
            .available_actions
            .contains(&action)
        {
            return Err("Esta ação não é permitida pela configuração da sessão".into());
        }

        if !session.permission_profile.can_respond_from_lume {
            return Err("Esta origem deve ser aberta na interface original".into());
        }

        match action {
            PermissionAction::Deny => {
                session.status = SessionStatus::Failed;
                session.status_label = "Permissão recusada".into();
            }
            PermissionAction::AllowOnce | PermissionAction::AllowSession => {
                session.status = SessionStatus::Running;
                session.status_label = "Continuando a tarefa".into();
            }
            PermissionAction::OpenSource => {
                return Err("Use a origem da sessão para continuar".into());
            }
        }

        // O conteúdo sensível existe apenas enquanto a decisão está pendente.
        session.pending_permission = None;
        Ok(())
    }
}
