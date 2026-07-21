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
            if matches!(
                session.status,
                SessionStatus::Running
                    | SessionStatus::PermissionRequired
                    | SessionStatus::WaitingForInput
            ) {
                session.status = SessionStatus::Completed;
                session.status_label = "Aguardando redetecção".into();
            }
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
        let sessions = self
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
        let mut deduplicated = Vec::<AgentSession>::new();
        for session in sessions {
            let duplicate = session.native_session_id.as_ref().and_then(|native_id| {
                deduplicated.iter().position(|existing| {
                    existing.agent == session.agent
                        && existing.native_session_id.as_ref() == Some(native_id)
                })
            });
            if let Some(index) = duplicate {
                if prefer_session(&session, &deduplicated[index]) {
                    deduplicated[index] = session;
                }
            } else {
                deduplicated.push(session);
            }
        }
        let native_processes = deduplicated
            .iter()
            .filter(|session| !is_provisional_process(session))
            .filter_map(|session| session.process_id.map(|pid| (session.agent.clone(), pid)))
            .collect::<Vec<_>>();
        let native_contexts = deduplicated
            .iter()
            .filter(|session| !is_provisional_process(session))
            .filter(|session| {
                matches!(
                    session.status,
                    SessionStatus::Running
                        | SessionStatus::PermissionRequired
                        | SessionStatus::WaitingForInput
                )
            })
            .filter_map(|session| {
                session.working_directory.as_ref().map(|directory| {
                    (
                        session.agent.clone(),
                        session.source.clone(),
                        directory.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let native_vscode_agents = deduplicated
            .iter()
            .filter(|session| !is_provisional_process(session))
            .filter(|session| session.source == SessionSource::Vscode)
            .map(|session| session.agent.clone())
            .collect::<Vec<_>>();
        deduplicated.retain(|session| {
            !is_provisional_process(session)
                || (!session.process_id.is_some_and(|pid| {
                    native_processes
                        .iter()
                        .any(|(agent, native_pid)| agent == &session.agent && *native_pid == pid)
                }) && !session.working_directory.as_ref().is_some_and(|directory| {
                    native_contexts
                        .iter()
                        .any(|(agent, source, native_directory)| {
                            agent == &session.agent
                                && source == &session.source
                                && native_directory == directory
                        })
                }) && !(session.source == SessionSource::Vscode
                    && native_vscode_agents.contains(&session.agent)))
        });
        let mut sessions = deduplicated;
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
        let native_ids = event
            .native_session_id
            .as_ref()
            .map(|native_id| {
                sessions
                    .iter()
                    .filter(|session| {
                        session.agent == event.agent
                            && session.native_session_id.as_ref() == Some(native_id)
                    })
                    .map(|session| session.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let existing_session_id = native_ids
            .iter()
            .filter_map(|id| sessions.iter().find(|session| &session.id == id))
            .max_by_key(|session| {
                (
                    session.permission_profile.can_respond_from_lume,
                    session.id == event.session_id,
                    session.updated_at,
                )
            })
            .map(|session| session.id.clone())
            .or_else(|| {
                sessions
                    .iter()
                    .find(|session| session.id == event.session_id)
                    .map(|session| session.id.clone())
            });
        let provisional_ids = sessions
            .iter()
            .filter(|session| {
                is_provisional_process(session)
                    && session.agent == event.agent
                    && (event
                        .process_id
                        .is_some_and(|process_id| session.process_id == Some(process_id))
                        || (event.source == Some(SessionSource::Vscode)
                            && session.source == SessionSource::Vscode))
            })
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();

        let target_session_id = if let Some(existing_id) = existing_session_id {
            sessions.retain(|session| {
                session.id == existing_id
                    || (!provisional_ids.contains(&session.id) && !native_ids.contains(&session.id))
            });
            existing_id
        } else if let Some(provisional_id) = provisional_ids.first() {
            if let Some(session) = sessions
                .iter_mut()
                .find(|session| session.id == *provisional_id)
            {
                session.id = event.session_id.clone();
            }
            sessions.retain(|session| {
                session.id == event.session_id || !provisional_ids.contains(&session.id)
            });
            event.session_id.clone()
        } else {
            sessions.push(session_from_event(&event, now));
            event.session_id.clone()
        };
        let session = sessions
            .iter_mut()
            .find(|session| session.id == target_session_id)
            .expect("a sessão acabou de ser inserida");

        apply_metadata(session, &event);
        session.updated_at = now;

        let permission_id = match event.event {
            HookEventKind::SessionStarted => {
                session.status = SessionStatus::WaitingForInput;
                session.status_label = "Esperando ação".into();
                session.pending_permission = None;
                None
            }
            HookEventKind::Running => {
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
        for provisional_id in provisional_ids {
            if provisional_id != snapshot.id {
                store.delete_session(&provisional_id)?;
            }
        }
        for native_id in native_ids {
            if native_id != snapshot.id {
                store.delete_session(&native_id)?;
            }
        }
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
        let discovered = coalesce_discovered_processes(discovered);
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível atualizar os processos".to_string())?;
        let mut changed = false;
        let mut snapshots = Vec::new();
        let mut history = Vec::new();
        let mut cancelled_permissions = Vec::new();
        let mut removed_sessions = Vec::new();

        let duplicate_provisional_ids = duplicate_provisional_ids(&sessions, &active_pids);
        if !duplicate_provisional_ids.is_empty() {
            sessions.retain(|session| !duplicate_provisional_ids.contains(&session.id));
            removed_sessions.extend(duplicate_provisional_ids);
            changed = true;
        }

        for process in discovered {
            let has_recent_native_vscode_chat = process.source == SessionSource::Vscode
                && sessions.iter().any(|session| {
                    !is_provisional_process(session)
                        && session.agent == process.agent
                        && session.source == SessionSource::Vscode
                        && (matches!(
                            session.status,
                            SessionStatus::Running
                                | SessionStatus::PermissionRequired
                                | SessionStatus::WaitingForInput
                        ) || now - session.updated_at < 10 * 60 * 1_000)
                });
            if has_recent_native_vscode_chat {
                let provisional_ids = sessions
                    .iter()
                    .filter(|session| {
                        is_provisional_process(session)
                            && session.agent == process.agent
                            && session.source == SessionSource::Vscode
                    })
                    .map(|session| session.id.clone())
                    .collect::<Vec<_>>();
                if !provisional_ids.is_empty() {
                    sessions.retain(|session| !provisional_ids.contains(&session.id));
                    removed_sessions.extend(provisional_ids);
                    changed = true;
                }
                continue;
            }
            let contextual_chat_ids = process
                .working_directory
                .as_ref()
                .map(|directory| {
                    sessions
                        .iter()
                        .filter(|session| !is_provisional_process(session))
                        .filter(|session| {
                            session.agent == process.agent
                                && session.source == process.source
                                && session.working_directory.as_ref() == Some(directory)
                                && matches!(
                                    session.status,
                                    SessionStatus::Running
                                        | SessionStatus::PermissionRequired
                                        | SessionStatus::WaitingForInput
                                )
                                && session.process_id.is_none_or(|pid| {
                                    pid == process.process_id || !active_pids.contains(&pid)
                                })
                        })
                        .map(|session| session.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !contextual_chat_ids.is_empty() {
                let provisional_ids = sessions
                    .iter()
                    .filter(|session| {
                        is_provisional_process(session)
                            && session.process_id == Some(process.process_id)
                    })
                    .map(|session| session.id.clone())
                    .collect::<Vec<_>>();
                if !provisional_ids.is_empty() {
                    sessions.retain(|session| !provisional_ids.contains(&session.id));
                    removed_sessions.extend(provisional_ids);
                    changed = true;
                }
                for session in sessions
                    .iter_mut()
                    .filter(|session| contextual_chat_ids.contains(&session.id))
                {
                    if session.process_id != Some(process.process_id) {
                        session.process_id = Some(process.process_id);
                        session.updated_at = now;
                        snapshots.push(session.clone());
                        changed = true;
                    }
                }
                continue;
            }
            let mut matched_process = false;
            for session in sessions
                .iter_mut()
                .filter(|session| session.process_id == Some(process.process_id))
            {
                matched_process = true;
                let mut refreshed = false;
                if session.working_directory != process.working_directory {
                    session.working_directory = process.working_directory.clone();
                    refreshed = true;
                }
                if session.source != process.source {
                    session.source = process.source.clone();
                    refreshed = true;
                }
                if is_provisional_process(session) && session.status == SessionStatus::Completed {
                    session.status = SessionStatus::WaitingForInput;
                    session.status_label = "Esperando ação".into();
                    refreshed = true;
                }
                if refreshed {
                    session.updated_at = now;
                    snapshots.push(session.clone());
                    changed = true;
                }
            }
            if matched_process {
                continue;
            }
            if let Some(session) = sessions.iter_mut().find(|session| {
                is_provisional_process(session)
                    && session.agent == process.agent
                    && session.source == process.source
                    && session.working_directory == process.working_directory
            }) {
                session.process_id = Some(process.process_id);
                session.status = SessionStatus::WaitingForInput;
                session.status_label = "Esperando ação".into();
                session.updated_at = now;
                snapshots.push(session.clone());
                changed = true;
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
                source_app: None,
                status: SessionStatus::WaitingForInput,
                status_label: "Esperando ação".into(),
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
            matches!(
                session.status,
                SessionStatus::Running
                    | SessionStatus::PermissionRequired
                    | SessionStatus::WaitingForInput
            ) && session
                .process_id
                .is_some_and(|pid| !active_pids.contains(&pid))
        }) {
            session.status = SessionStatus::Completed;
            session.status_label = "Processo encerrado".into();
            if let Some(permission) = session.pending_permission.take() {
                cancelled_permissions.push(permission.id);
            }
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

        if !cancelled_permissions.is_empty() {
            let (decisions, decision_changed) = &*self.decisions;
            let mut values = decisions
                .lock()
                .map_err(|_| "Não foi possível cancelar as permissões pendentes".to_string())?;
            for permission_id in cancelled_permissions {
                values.insert(permission_id, PermissionAction::Deny);
            }
            decision_changed.notify_all();
        }

        if changed {
            let store = self
                .store
                .lock()
                .map_err(|_| "Não foi possível persistir os processos".to_string())?;
            for session in snapshots {
                store.save_session(&session)?;
            }
            for session_id in removed_sessions {
                store.delete_session(&session_id)?;
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
        source_app: event.source_app.clone(),
        status: SessionStatus::WaitingForInput,
        status_label: "Esperando ação".into(),
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
    if let Some(source_app) = &event.source_app {
        session.source_app = Some(source_app.clone());
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
        if profile.can_respond_from_lume || !session.permission_profile.can_respond_from_lume {
            session.permission_profile = profile.clone();
        }
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

fn is_provisional_process(session: &AgentSession) -> bool {
    session.id.starts_with("process:") && session.native_session_id.is_none()
}

fn same_provisional_context(left: &AgentSession, right: &AgentSession) -> bool {
    is_provisional_process(left)
        && is_provisional_process(right)
        && left.agent == right.agent
        && left.source == right.source
        && left.working_directory == right.working_directory
}

fn duplicate_provisional_ids(
    sessions: &[AgentSession],
    active_pids: &std::collections::HashSet<u32>,
) -> std::collections::HashSet<String> {
    let mut survivors = Vec::<&AgentSession>::new();
    let mut duplicates = std::collections::HashSet::new();
    for session in sessions
        .iter()
        .filter(|session| is_provisional_process(session))
    {
        let Some(index) = survivors
            .iter()
            .position(|survivor| same_provisional_context(survivor, session))
        else {
            survivors.push(session);
            continue;
        };
        let survivor = survivors[index];
        let survivor_rank = (
            survivor
                .process_id
                .is_some_and(|pid| active_pids.contains(&pid)),
            survivor.status != SessionStatus::Completed,
        );
        let candidate_rank = (
            session
                .process_id
                .is_some_and(|pid| active_pids.contains(&pid)),
            session.status != SessionStatus::Completed,
        );
        if candidate_rank > survivor_rank {
            duplicates.insert(survivor.id.clone());
            survivors[index] = session;
        } else {
            duplicates.insert(session.id.clone());
        }
    }
    duplicates
}

fn coalesce_discovered_processes(discovered: Vec<DiscoveredProcess>) -> Vec<DiscoveredProcess> {
    let mut unique = Vec::<DiscoveredProcess>::new();
    for process in discovered {
        if let Some(existing) = unique.iter_mut().find(|existing| {
            existing.agent == process.agent
                && existing.source == process.source
                && existing.working_directory == process.working_directory
        }) {
            if process.process_id < existing.process_id {
                *existing = process;
            }
        } else {
            unique.push(process);
        }
    }
    unique
}

fn prefer_session(candidate: &AgentSession, current: &AgentSession) -> bool {
    (
        candidate.permission_profile.can_respond_from_lume,
        !is_provisional_process(candidate),
        candidate.updated_at,
    ) > (
        current.permission_profile.can_respond_from_lume,
        !is_provisional_process(current),
        current.updated_at,
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PermissionRequest;

    fn discovered(process_id: u32) -> DiscoveredProcess {
        DiscoveredProcess {
            agent: AgentKind::Claude,
            process_id,
            working_directory: Some("/work/lume".into()),
            source: SessionSource::Cli,
        }
    }

    fn started_event(session_id: &str, process_id: u32) -> HookEvent {
        HookEvent {
            event: HookEventKind::SessionStarted,
            session_id: session_id.into(),
            agent: AgentKind::Claude,
            agent_label: None,
            project: Some("lume".into()),
            source: Some(SessionSource::Cli),
            source_app: None,
            status_label: Some("Sessão detectada".into()),
            started_at: None,
            process_id: Some(process_id),
            native_session_id: Some("native-session".into()),
            working_directory: Some("/work/lume".into()),
            permission_profile: None,
            permission: None,
            wait_for_decision: false,
        }
    }

    #[test]
    fn hook_event_reuses_the_provisional_process_session() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![discovered(4242)])
            .expect("descoberta");

        state
            .ingest(started_event("claude:session-1", 4242))
            .expect("hook");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "claude:session-1");
        assert_eq!(
            sessions[0].native_session_id.as_deref(),
            Some("native-session")
        );

        let persisted = state
            .store
            .lock()
            .expect("store")
            .load_sessions()
            .expect("persistência");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].id, "claude:session-1");
    }

    #[test]
    fn discovered_process_waits_for_action_instead_of_appearing_to_run() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![discovered(4242)])
            .expect("descoberta");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::WaitingForInput);
        assert_eq!(sessions[0].status_label, "Esperando ação");
    }

    #[test]
    fn sibling_processes_in_the_same_context_create_one_provisional_session() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![discovered(4242), discovered(4343)])
            .expect("descoberta");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].process_id, Some(4242));
    }

    #[test]
    fn provisional_session_survives_a_process_id_change_without_duplication() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![discovered(4242)])
            .expect("primeira descoberta");
        let original_id = state.sessions().expect("sessões")[0].id.clone();

        state
            .reconcile_processes(vec![discovered(4343)])
            .expect("redetecção");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, original_id);
        assert_eq!(sessions[0].process_id, Some(4343));
        assert_eq!(sessions[0].status, SessionStatus::WaitingForInput);
    }

    #[test]
    fn reconciliation_removes_provisional_duplicates_already_in_the_store() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![discovered(4242)])
            .expect("descoberta");
        let mut duplicate = state.sessions().expect("sessões")[0].clone();
        duplicate.id = "process:claude:4343".into();
        duplicate.process_id = Some(4343);
        state
            .sessions
            .lock()
            .expect("estado em memória")
            .push(duplicate.clone());
        state
            .store
            .lock()
            .expect("store")
            .save_session(&duplicate)
            .expect("persistência");

        state
            .reconcile_processes(vec![discovered(4343)])
            .expect("limpeza");

        assert_eq!(state.sessions().expect("sessões").len(), 1);
        assert_eq!(
            state
                .store
                .lock()
                .expect("store")
                .load_sessions()
                .expect("persistência")
                .len(),
            1
        );
    }

    #[test]
    fn integrations_with_the_same_native_chat_become_one_session() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut direct = started_event("claude-direct:chat-1", 4242);
        direct.permission_profile = Some(PermissionProfile {
            mode: AccessMode::Custom,
            label: "Integração direta".into(),
            approval_policy: "Perguntar".into(),
            can_respond_from_lume: true,
            available_actions: vec![PermissionAction::AllowOnce],
        });
        state.ingest(direct).expect("integração direta");

        let mut hook = started_event("claude-hook:chat-1", 4242);
        hook.event = HookEventKind::Completed;
        hook.status_label = Some("Finalizado pelo hook".into());
        hook.permission_profile = Some(PermissionProfile {
            mode: AccessMode::Custom,
            label: "Somente observação".into(),
            approval_policy: "Abrir origem".into(),
            can_respond_from_lume: false,
            available_actions: vec![PermissionAction::OpenSource],
        });
        state.ingest(hook).expect("hook");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "claude-direct:chat-1");
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert!(sessions[0].permission_profile.can_respond_from_lume);
    }

    #[test]
    fn one_agent_process_can_keep_multiple_native_chats() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut first = started_event("claude:chat-1", 4242);
        first.native_session_id = Some("native-chat-1".into());
        state.ingest(first).expect("primeiro chat");
        let mut second = started_event("claude:chat-2", 4242);
        second.native_session_id = Some("native-chat-2".into());
        state.ingest(second).expect("segundo chat");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn provisional_process_is_hidden_when_an_active_chat_has_the_same_context() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:chat-with-old-pid", 9999))
            .expect("chat");
        state
            .reconcile_processes(vec![discovered(4242)])
            .expect("descoberta");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "claude:chat-with-old-pid");
    }

    #[test]
    fn vscode_chat_hides_its_host_process_without_hiding_other_chats() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                process_id: 5252,
                working_directory: Some("/home/user/.vscode/extensions/openai.chatgpt".into()),
                source: SessionSource::Vscode,
            }])
            .expect("processo do VS Code");

        for chat in ["chat-1", "chat-2"] {
            state
                .ingest(HookEvent {
                    event: HookEventKind::Running,
                    session_id: format!("codex-app-server:{chat}"),
                    agent: AgentKind::Codex,
                    agent_label: Some("Codex".into()),
                    project: Some("lume".into()),
                    source: Some(SessionSource::Vscode),
                    source_app: None,
                    status_label: Some("Executando no VS Code".into()),
                    started_at: None,
                    process_id: None,
                    native_session_id: Some(chat.into()),
                    working_directory: Some("/work/lume".into()),
                    permission_profile: None,
                    permission: None,
                    wait_for_decision: false,
                })
                .expect("chat do VS Code");
        }
        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                process_id: 5252,
                working_directory: Some("/home/user/.vscode/extensions/openai.chatgpt".into()),
                source: SessionSource::Vscode,
            }])
            .expect("nova varredura");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 2);
        assert!(sessions
            .iter()
            .all(|session| !is_provisional_process(session)));
        assert!(sessions
            .iter()
            .all(|session| session.source == SessionSource::Vscode));
        assert_eq!(
            state
                .store
                .lock()
                .expect("store")
                .load_sessions()
                .expect("persistência")
                .len(),
            2
        );
    }

    #[test]
    fn process_scan_does_not_reopen_a_completed_chat() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:completed-chat", 4242))
            .expect("início");
        let mut completed = started_event("claude:completed-chat", 4242);
        completed.event = HookEventKind::Completed;
        completed.status_label = Some("Finalizado".into());
        state.ingest(completed).expect("conclusão");

        state
            .reconcile_processes(vec![discovered(4242)])
            .expect("redetecção");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert_eq!(sessions[0].status_label, "Finalizado");
    }

    #[test]
    fn disappearing_process_completes_a_hook_backed_session() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:session-2", 4343))
            .expect("hook");

        state
            .reconcile_processes(Vec::new())
            .expect("reconciliação");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert_eq!(sessions[0].status_label, "Processo encerrado");

        let history = state.history(10).expect("histórico");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].session_id, "claude:session-2");
        assert_eq!(history[0].event, "completed");
    }

    #[test]
    fn disappearing_process_releases_a_pending_permission_as_denied() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("claude:permission-session", 4545);
        event.event = HookEventKind::PermissionRequest;
        event.permission_profile = Some(PermissionProfile {
            mode: AccessMode::WorkspaceWrite,
            label: "Acesso ao projeto".into(),
            approval_policy: "Perguntar".into(),
            can_respond_from_lume: true,
            available_actions: vec![PermissionAction::AllowOnce, PermissionAction::Deny],
        });
        event.permission = Some(PermissionRequest {
            id: "permission-1".into(),
            kind: "command".into(),
            summary: "Executar comando".into(),
            resource: "cargo test".into(),
            risk: "medium".into(),
            requested_at: "agora".into(),
        });
        state.ingest(event).expect("permissão");

        state
            .reconcile_processes(Vec::new())
            .expect("reconciliação");

        assert_eq!(
            state
                .wait_for_decision("permission-1", Duration::ZERO)
                .expect("decisão"),
            Some(PermissionAction::Deny)
        );
    }

    #[test]
    fn restart_does_not_restore_a_stale_active_state() {
        let database_path = std::env::temp_dir().join(format!(
            "lume-restart-state-{}-{}.sqlite3",
            std::process::id(),
            now_millis()
        ));
        {
            let state = AppState::new(&database_path).expect("estado inicial");
            state
                .ingest(started_event("claude:stale-session", 4444))
                .expect("hook");
        }

        let restarted = AppState::new(&database_path).expect("reinício");
        let sessions = restarted.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert_eq!(sessions[0].status_label, "Aguardando redetecção");
        assert!(sessions[0].pending_permission.is_none());
        drop(restarted);

        let _ = std::fs::remove_file(&database_path);
        let _ = std::fs::remove_file(database_path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(database_path.with_extension("sqlite3-shm"));
    }
}
