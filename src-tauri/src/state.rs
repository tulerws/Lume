use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    discovery::DiscoveredProcess,
    domain::{
        AccessMode, AgentKind, AgentSession, HistoryEntry, HookEvent, HookEventKind,
        PermissionAction, PermissionProfile, Preferences, SessionSource, SessionStatus,
    },
    store::Store,
};

#[derive(Clone)]
pub struct AppState {
    sessions: Arc<Mutex<Vec<AgentSession>>>,
    store: Arc<Mutex<Store>>,
    decisions: Arc<(Mutex<HashMap<String, PermissionAction>>, Condvar)>,
}

impl AppState {
    pub fn new(database_path: &Path) -> Result<Self, String> {
        let store = Store::open(database_path)?;
        let mut sessions = store.load_sessions()?;
        for session in &mut sessions {
            session.pending_permission = None;
            session.working_directory = None;
            store.save_session(session)?;
        }
        // O banco é pequeno; a limpeza física na inicialização também remove
        // vestígios deixados em WAL/páginas livres por versões anteriores.
        store.scrub_deleted_content()?;
        let preferences = store.load_preferences()?;
        let cutoff =
            now_millis() - i64::from(preferences.history_retention_days) * 24 * 60 * 60 * 1_000;
        store.purge_history(cutoff)?;
        Ok(Self {
            sessions: Arc::new(Mutex::new(sessions)),
            store: Arc::new(Mutex::new(store)),
            decisions: Arc::new((Mutex::new(HashMap::new()), Condvar::new())),
        })
    }

    pub fn sessions(&self) -> Result<Vec<AgentSession>, String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?
            .iter()
            .filter(|session| {
                matches!(
                    session.status,
                    SessionStatus::Running
                        | SessionStatus::PermissionRequired
                        | SessionStatus::WaitingForInput
                ) || now_millis() - session.updated_at < 10 * 60 * 1_000
            })
            .cloned()
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| (status_priority(&session.status), -session.updated_at));
        Ok(sessions)
    }

    pub fn history(&self, limit: usize) -> Result<Vec<HistoryEntry>, String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível acessar o histórico".to_string())?
            .history(limit.min(200))
    }

    pub fn preferences(&self) -> Result<Preferences, String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível acessar as preferências".to_string())?
            .load_preferences()
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<(), String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "Não foi possível salvar as preferências".to_string())?;
        store.save_preferences(preferences)?;
        let cutoff =
            now_millis() - i64::from(preferences.history_retention_days) * 24 * 60 * 60 * 1_000;
        store.purge_history(cutoff)
    }

    pub fn ingest(&self, event: HookEvent) -> Result<Option<String>, String> {
        if event.session_id.trim().is_empty() {
            return Err("O evento não informou uma sessão".into());
        }

        let now = now_millis();
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível atualizar as sessões".to_string())?;
        let index = sessions
            .iter()
            .position(|session| session.id == event.session_id);

        if index.is_none() {
            sessions.push(session_from_event(&event, now));
        }
        let session = sessions
            .iter_mut()
            .find(|session| session.id == event.session_id)
            .expect("a sessão acabou de ser inserida");

        apply_metadata(session, &event);
        session.updated_at = now;

        let permission_id = match event.event {
            HookEventKind::SessionStarted | HookEventKind::Running => {
                session.status = SessionStatus::Running;
                session.status_label = event.status_label.unwrap_or_else(|| "Executando".into());
                session.pending_permission = None;
                None
            }
            HookEventKind::PermissionRequest => {
                let permission = event
                    .permission
                    .ok_or_else(|| "A solicitação não contém a permissão".to_string())?;
                let id = permission.id.clone();
                session.status = SessionStatus::PermissionRequired;
                session.status_label = event
                    .status_label
                    .unwrap_or_else(|| "Aguardando permissão".into());
                session.pending_permission = Some(permission);
                Some(id)
            }
            HookEventKind::WaitingForInput => {
                session.status = SessionStatus::WaitingForInput;
                session.status_label = event
                    .status_label
                    .unwrap_or_else(|| "Aguardando sua resposta".into());
                session.pending_permission = None;
                None
            }
            HookEventKind::Completed | HookEventKind::SessionEnded => {
                session.status = SessionStatus::Completed;
                session.status_label = event.status_label.unwrap_or_else(|| "Finalizado".into());
                session.pending_permission = None;
                None
            }
            HookEventKind::Failed => {
                session.status = SessionStatus::Failed;
                session.status_label = event.status_label.unwrap_or_else(|| "Falhou".into());
                session.pending_permission = None;
                None
            }
        };

        let snapshot = session.clone();
        let history = history_for_event(&snapshot, &event.event, now);
        drop(sessions);

        let store = self
            .store
            .lock()
            .map_err(|_| "Não foi possível persistir a sessão".to_string())?;
        store.save_session(&snapshot)?;
        if let Some(entry) = history {
            store.add_history(&entry)?;
        }
        Ok(permission_id)
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

        let (event, summary) = match action {
            PermissionAction::Deny => {
                session.status = SessionStatus::Running;
                session.status_label = "Permissão recusada".into();
                ("permission_denied", "Permissão recusada")
            }
            PermissionAction::AllowOnce | PermissionAction::AllowSession => {
                session.status = SessionStatus::Running;
                session.status_label = "Continuando a tarefa".into();
                ("permission_allowed", "Permissão concedida")
            }
            PermissionAction::OpenSource => {
                return Err("Use a origem da sessão para continuar".into());
            }
        };

        // O comando, caminho e payload deixam de existir assim que a decisão é tomada.
        session.pending_permission = None;
        session.updated_at = now_millis();
        let snapshot = session.clone();
        let history = HistoryEntry {
            id: format!("{}-{}", session.updated_at, permission_id),
            session_id: session.id.clone(),
            agent_label: session.agent_label.clone(),
            project: session.project.clone(),
            event: event.into(),
            summary: summary.into(),
            created_at: session.updated_at,
        };
        drop(sessions);

        let store = self
            .store
            .lock()
            .map_err(|_| "Não foi possível salvar a decisão".to_string())?;
        store.save_session(&snapshot)?;
        store.add_history(&history)?;
        drop(store);

        let (decisions, changed) = &*self.decisions;
        let mut values = decisions
            .lock()
            .map_err(|_| "Não foi possível entregar a decisão".to_string())?;
        values.insert(permission_id.into(), action);
        changed.notify_all();
        Ok(())
    }

    pub fn wait_for_decision(
        &self,
        permission_id: &str,
        timeout: Duration,
    ) -> Result<Option<PermissionAction>, String> {
        let (decisions, changed) = &*self.decisions;
        let values = decisions
            .lock()
            .map_err(|_| "Não foi possível aguardar a decisão".to_string())?;
        let (mut values, _) = changed
            .wait_timeout_while(values, timeout, |values| {
                !values.contains_key(permission_id)
            })
            .map_err(|_| "Não foi possível aguardar a decisão".to_string())?;
        Ok(values.remove(permission_id))
    }

    pub fn reconcile_processes(&self, discovered: Vec<DiscoveredProcess>) -> Result<bool, String> {
        let now = now_millis();
        let active_pids = discovered
            .iter()
            .map(|process| process.process_id)
            .collect::<std::collections::HashSet<_>>();
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível atualizar os processos".to_string())?;
        let mut changed = false;
        let mut snapshots = Vec::new();
        let mut history = Vec::new();

        for process in discovered {
            if let Some(session) = sessions
                .iter_mut()
                .find(|session| session.process_id == Some(process.process_id))
            {
                let mut refreshed = false;
                if session.working_directory != process.working_directory {
                    session.working_directory = process.working_directory;
                    refreshed = true;
                }
                if session.source != process.source {
                    session.source = process.source;
                    refreshed = true;
                }
                if session.status == SessionStatus::Completed {
                    session.status = SessionStatus::Running;
                    session.status_label = "Processo detectado".into();
                    refreshed = true;
                }
                if refreshed {
                    session.updated_at = now;
                    snapshots.push(session.clone());
                    changed = true;
                }
                continue;
            }
            let project = process
                .working_directory
                .as_deref()
                .and_then(|path| Path::new(path).file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("Sessão local")
                .to_string();
            let agent_name = agent_label(&process.agent).to_string();
            let session = AgentSession {
                id: format!(
                    "process:{}:{}",
                    agent_name.to_lowercase(),
                    process.process_id
                ),
                agent: process.agent.clone(),
                agent_label: agent_name,
                project,
                source: process.source,
                status: SessionStatus::Running,
                status_label: "Processo detectado".into(),
                started_at: now.to_string(),
                updated_at: now,
                process_id: Some(process.process_id),
                native_session_id: None,
                working_directory: process.working_directory,
                permission_profile: default_profile(&process.agent),
                pending_permission: None,
            };
            snapshots.push(session.clone());
            sessions.push(session);
            changed = true;
        }

        for session in sessions.iter_mut().filter(|session| {
            session.id.starts_with("process:")
                && matches!(session.status, SessionStatus::Running)
                && session
                    .process_id
                    .is_some_and(|pid| !active_pids.contains(&pid))
        }) {
            session.status = SessionStatus::Completed;
            session.status_label = "Processo encerrado".into();
            session.updated_at = now;
            snapshots.push(session.clone());
            history.push(HistoryEntry {
                id: format!("{}-{}-completed", now, session.id),
                session_id: session.id.clone(),
                agent_label: session.agent_label.clone(),
                project: session.project.clone(),
                event: "completed".into(),
                summary: "Sessão encerrada".into(),
                created_at: now,
            });
            changed = true;
        }
        drop(sessions);

        if changed {
            let store = self
                .store
                .lock()
                .map_err(|_| "Não foi possível persistir os processos".to_string())?;
            for session in snapshots {
                store.save_session(&session)?;
            }
            for entry in history {
                store.add_history(&entry)?;
            }
        }
        Ok(changed)
    }
}

fn session_from_event(event: &HookEvent, now: i64) -> AgentSession {
    let project = event.project.clone().unwrap_or_else(|| {
        event
            .working_directory
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Sessão sem projeto")
            .to_string()
    });
    AgentSession {
        id: event.session_id.clone(),
        agent: event.agent.clone(),
        agent_label: event
            .agent_label
            .clone()
            .unwrap_or_else(|| agent_label(&event.agent).into()),
        project,
        source: event.source.clone().unwrap_or(SessionSource::Cli),
        status: SessionStatus::Running,
        status_label: "Detectado".into(),
        started_at: event.started_at.clone().unwrap_or_else(|| now.to_string()),
        updated_at: now,
        process_id: event.process_id,
        native_session_id: event.native_session_id.clone(),
        working_directory: event.working_directory.clone(),
        permission_profile: event
            .permission_profile
            .clone()
            .unwrap_or_else(|| default_profile(&event.agent)),
        pending_permission: None,
    }
}

fn apply_metadata(session: &mut AgentSession, event: &HookEvent) {
    if let Some(label) = &event.agent_label {
        session.agent_label = label.clone();
    }
    if let Some(project) = &event.project {
        session.project = project.clone();
    }
    if let Some(source) = &event.source {
        session.source = source.clone();
    }
    if let Some(process_id) = event.process_id {
        session.process_id = Some(process_id);
    }
    if let Some(native_session_id) = &event.native_session_id {
        session.native_session_id = Some(native_session_id.clone());
    }
    if let Some(working_directory) = &event.working_directory {
        session.working_directory = Some(working_directory.clone());
    }
    if let Some(profile) = &event.permission_profile {
        session.permission_profile = profile.clone();
    }
}

fn default_profile(agent: &AgentKind) -> PermissionProfile {
    match agent {
        AgentKind::Claude => PermissionProfile {
            mode: AccessMode::Custom,
            label: "Permissões da sessão".into(),
            approval_policy: "Ações disponíveis conforme o hook".into(),
            can_respond_from_lume: true,
            available_actions: vec![
                PermissionAction::AllowOnce,
                PermissionAction::AllowSession,
                PermissionAction::Deny,
            ],
        },
        _ => PermissionProfile {
            mode: AccessMode::Custom,
            label: "Monitoramento local".into(),
            approval_policy: "A resposta depende da origem".into(),
            can_respond_from_lume: false,
            available_actions: vec![PermissionAction::OpenSource],
        },
    }
}

fn history_for_event(
    session: &AgentSession,
    event: &HookEventKind,
    now: i64,
) -> Option<HistoryEntry> {
    let (event, summary) = match event {
        HookEventKind::Completed | HookEventKind::SessionEnded => {
            ("completed", "Tarefa finalizada")
        }
        HookEventKind::Failed => ("failed", "Tarefa encerrada com erro"),
        _ => return None,
    };
    Some(HistoryEntry {
        id: format!("{}-{}-{}", now, session.id, event),
        session_id: session.id.clone(),
        agent_label: session.agent_label.clone(),
        project: session.project.clone(),
        event: event.into(),
        summary: summary.into(),
        created_at: now,
    })
}

fn status_priority(status: &SessionStatus) -> u8 {
    match status {
        SessionStatus::PermissionRequired => 0,
        SessionStatus::Failed => 1,
        SessionStatus::Running | SessionStatus::WaitingForInput => 2,
        SessionStatus::Completed => 3,
    }
}

fn agent_label(agent: &AgentKind) -> &'static str {
    match agent {
        AgentKind::Codex => "Codex",
        AgentKind::Claude => "Claude",
        AgentKind::Gemini => "Gemini",
        AgentKind::Unknown => "Agente",
    }
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
