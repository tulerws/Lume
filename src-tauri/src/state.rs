use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    discovery::DiscoveredProcess,
    domain::{
        AccessMode, AgentKind, AgentSession, HistoryEntry, HookEvent, HookEventKind,
        PermissionAction, PermissionProfile, Preferences, ResultNote, SessionResult, SessionSource,
        SessionStatus,
    },
    store::Store,
};

const PROCESS_MISSING_SCAN_LIMIT: u8 = 2;
const WEB_SESSION_STALE_MS: i64 = 12_000;

#[derive(Clone)]
pub struct AppState {
    sessions: Arc<Mutex<Vec<AgentSession>>>,
    store: Arc<Mutex<Store>>,
    decisions: Arc<(Mutex<HashMap<String, PermissionAction>>, Condvar)>,
    missing_process_scans: Arc<Mutex<HashMap<String, u8>>>,
}

impl AppState {
    pub fn new(database_path: &Path) -> Result<Self, String> {
        let store = Store::open(database_path)?;
        let sessions = Vec::new();
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
            missing_process_scans: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn sessions(&self) -> Result<Vec<AgentSession>, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?
            .clone();
        let mut deduplicated = Vec::<AgentSession>::new();
        for mut session in sessions {
            let duplicate = deduplicated
                .iter()
                .position(|existing| same_session_identity(existing, &session));
            if let Some(index) = duplicate {
                if prefer_session(&session, &deduplicated[index]) {
                    merge_results(&mut session, &deduplicated[index]);
                    deduplicated[index] = session;
                } else {
                    merge_results(&mut deduplicated[index], &session);
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
                                && same_directory(Some(native_directory), Some(directory))
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

    pub fn result_notes(&self, limit: usize) -> Result<Vec<ResultNote>, String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível acessar as notas".to_string())?
            .result_notes(limit.min(200))
    }

    pub fn save_result_note(
        &self,
        session_id: &str,
        result_id: &str,
        title: &str,
    ) -> Result<ResultNote, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar o resultado".to_string())?;
        let session = sessions
            .iter()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Sessão não encontrada".to_string())?;
        let result = session
            .results
            .iter()
            .find(|result| result.id == result_id)
            .ok_or_else(|| "Resultado não encontrado".to_string())?;
        let title = title.trim();
        let note = ResultNote {
            id: format!("note:{result_id}"),
            title: if title.is_empty() {
                format!("{} · {}", session.agent_label, session.project)
            } else {
                title.chars().take(120).collect()
            },
            body: result.response.chars().take(64 * 1024).collect(),
            agent_label: session.agent_label.clone(),
            project: session.project.clone(),
            files: result.files.clone(),
            tests: result.tests.clone(),
            created_at: now_millis(),
        };
        drop(sessions);
        self.store
            .lock()
            .map_err(|_| "Não foi possível salvar a nota".to_string())?
            .save_result_note(&note)?;
        Ok(note)
    }

    pub fn delete_result_note(&self, id: &str) -> Result<(), String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível remover a nota".to_string())?
            .delete_result_note(id)
    }

    pub fn mark_process_terminated(&self, process_id: u32) -> Result<bool, String> {
        let now = now_millis();
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível encerrar as sessões".to_string())?;
        let removed = sessions
            .iter()
            .filter(|session| session.process_id == Some(process_id))
            .cloned()
            .collect::<Vec<_>>();
        if removed.is_empty() {
            return Ok(false);
        }
        let mut history = Vec::new();
        let mut cancelled_permissions = Vec::new();
        for session in &removed {
            if let Some(permission) = session.pending_permission.as_ref() {
                cancelled_permissions.push(permission.id.clone());
            }
            history.push(HistoryEntry {
                id: format!("{}-{}-terminated", now, session.id),
                session_id: session.id.clone(),
                agent_label: session.agent_label.clone(),
                project: session.project.clone(),
                event: "completed".into(),
                summary: "Agente encerrado pelo Lume".into(),
                created_at: now,
            });
        }
        let removed_ids = removed
            .iter()
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();
        sessions.retain(|session| !removed_ids.contains(&session.id));
        drop(sessions);
        self.missing_process_scans
            .lock()
            .map_err(|_| "Não foi possível limpar a presença do processo".to_string())?
            .retain(|session_id, _| !removed_ids.contains(session_id));

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
        let store = self
            .store
            .lock()
            .map_err(|_| "Não foi possível persistir o encerramento".to_string())?;
        for entry in history {
            store.add_history(&entry)?;
        }
        Ok(true)
    }

    pub fn ingest(&self, event: HookEvent) -> Result<Option<String>, String> {
        if event.session_id.trim().is_empty() {
            return Err("O evento não informou uma sessão".into());
        }

        let now = now_millis();
        let session_ended = matches!(&event.event, HookEventKind::SessionEnded);
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
                            && ((session.source == SessionSource::Vscode
                                && event.working_directory.is_none())
                                || same_directory(
                                    session.working_directory.as_deref(),
                                    event.working_directory.as_deref(),
                                ))))
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
        self.missing_process_scans
            .lock()
            .map_err(|_| "Não foi possível atualizar a presença da sessão".to_string())?
            .remove(&target_session_id);
        let superseded_permission_id = (!matches!(&event.event, HookEventKind::PermissionRequest))
            .then(|| {
                session
                    .pending_permission
                    .as_ref()
                    .map(|permission| permission.id.clone())
            })
            .flatten();

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
                session.last_response = None;
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

        if matches!(
            &event.event,
            HookEventKind::Completed | HookEventKind::SessionEnded
        ) {
            remember_result(session, now);
        }

        let snapshot = session.clone();
        let history = history_for_event(&snapshot, &event.event, now);
        if session_ended {
            sessions.retain(|session| session.id != target_session_id);
            self.missing_process_scans
                .lock()
                .map_err(|_| "Não foi possível remover a sessão encerrada".to_string())?
                .remove(&target_session_id);
        }
        drop(sessions);

        if let Some(permission_id) = superseded_permission_id {
            let (decisions, decision_changed) = &*self.decisions;
            decisions
                .lock()
                .map_err(|_| "Não foi possível liberar a permissão antiga".to_string())?
                .insert(permission_id, PermissionAction::Deny);
            decision_changed.notify_all();
        }

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

    #[cfg(test)]
    pub fn reconcile_processes(&self, discovered: Vec<DiscoveredProcess>) -> Result<bool, String> {
        let live_pids = discovered
            .iter()
            .map(|process| process.process_id)
            .collect();
        self.reconcile_process_snapshot(discovered, live_pids)
    }

    pub fn reconcile_process_snapshot(
        &self,
        discovered: Vec<DiscoveredProcess>,
        live_pids: HashSet<u32>,
    ) -> Result<bool, String> {
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
        let mut missing_process_scans = self
            .missing_process_scans
            .lock()
            .map_err(|_| "Não foi possível atualizar a presença dos processos".to_string())?;
        let mut changed = false;
        let mut snapshots = Vec::new();
        let mut history = Vec::new();
        let mut cancelled_permissions = Vec::new();
        let mut removed_sessions = Vec::new();

        let stale_web_ids = sessions
            .iter()
            .filter(|session| {
                session.source == SessionSource::Web
                    && now - session.updated_at > WEB_SESSION_STALE_MS
            })
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();
        if !stale_web_ids.is_empty() {
            for session in sessions
                .iter()
                .filter(|session| stale_web_ids.contains(&session.id))
            {
                if let Some(permission) = session.pending_permission.as_ref() {
                    cancelled_permissions.push(permission.id.clone());
                }
            }
            sessions.retain(|session| !stale_web_ids.contains(&session.id));
            removed_sessions.extend(stale_web_ids);
            changed = true;
        }

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
                        && session_matches_process(session, &process)
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
                            && session_matches_process(session, &process)
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
                            session_matches_process(session, &process)
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
                if session.agent_label != process.agent_label {
                    session.agent_label = process.agent_label.clone();
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
                    && session_matches_process(session, &process)
                    && session.source == process.source
                    && same_directory(
                        session.working_directory.as_deref(),
                        process.working_directory.as_deref(),
                    )
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
            let agent_name = process.agent_label.clone();
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
                last_response: None,
                results: Vec::new(),
            };
            snapshots.push(session.clone());
            sessions.push(session);
            changed = true;
        }

        for session in sessions.iter().filter(|session| {
            session
                .process_id
                .is_some_and(|pid| live_pids.contains(&pid))
        }) {
            missing_process_scans.remove(&session.id);
        }
        missing_process_scans
            .retain(|session_id, _| sessions.iter().any(|session| &session.id == session_id));

        let mut closed_session_ids = Vec::new();
        for session in sessions.iter().filter(|session| {
            session
                .process_id
                .is_some_and(|pid| !live_pids.contains(&pid))
        }) {
            let missing_scans = missing_process_scans
                .entry(session.id.clone())
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
            if *missing_scans < PROCESS_MISSING_SCAN_LIMIT {
                continue;
            }
            missing_process_scans.remove(&session.id);
            closed_session_ids.push(session.id.clone());
            if let Some(permission) = session.pending_permission.as_ref() {
                cancelled_permissions.push(permission.id.clone());
            }
            history.push(HistoryEntry {
                id: format!("{}-{}-completed", now, session.id),
                session_id: session.id.clone(),
                agent_label: session.agent_label.clone(),
                project: session.project.clone(),
                event: "completed".into(),
                summary: "Sessão encerrada".into(),
                created_at: now,
            });
        }
        if !closed_session_ids.is_empty() {
            sessions.retain(|session| !closed_session_ids.contains(&session.id));
            removed_sessions.extend(closed_session_ids);
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
        last_response: event.last_response.clone(),
        results: Vec::new(),
    }
}

fn remember_result(session: &mut AgentSession, now: i64) {
    let Some(response) = session
        .last_response
        .as_deref()
        .map(str::trim)
        .filter(|response| !response.is_empty())
    else {
        return;
    };
    if session
        .results
        .last()
        .is_some_and(|result| result.response == response)
    {
        return;
    }
    let (files, tests) = extract_result_artifacts(response);
    session.results.push(SessionResult {
        id: format!("{}-result-{}", session.id, now),
        response: response.to_string(),
        created_at: now,
        files,
        tests,
    });
    if session.results.len() > 12 {
        session.results.drain(..session.results.len() - 12);
    }
}

fn extract_result_artifacts(response: &str) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut tests = Vec::new();
    const FILE_EXTENSIONS: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "svelte", "json", "toml", "yaml", "yml", "md", "css",
        "scss", "html", "py", "go", "java", "cs", "sh", "sql",
    ];
    const TEST_MARKERS: &[&str] = &[
        "cargo test",
        "npm test",
        "npm run test",
        "npm run check",
        "npm run build",
        "pnpm test",
        "yarn test",
        "pytest",
        "dotnet test",
        "mvn test",
        "gradle test",
        "flutter test",
        "flutter analyze",
    ];

    for line in response
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let lower = line.to_lowercase();
        if TEST_MARKERS.iter().any(|marker| lower.contains(marker)) {
            let check = line
                .trim_start_matches(['-', '*', ' ', '`'])
                .trim_end_matches('`')
                .replace('`', "")
                .chars()
                .take(180)
                .collect::<String>();
            if !tests.contains(&check) {
                tests.push(check);
            }
        }

        for token in line.split_whitespace() {
            let cleaned = token
                .trim_matches(|character: char| {
                    matches!(
                        character,
                        '`' | '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';'
                    )
                })
                .trim_end_matches(['.', '`']);
            let candidate = cleaned.split(':').next().unwrap_or_default();
            let extension = candidate
                .rsplit_once('.')
                .map(|(_, extension)| extension.trim_end_matches(['.', ':']))
                .unwrap_or_default()
                .to_lowercase();
            if !FILE_EXTENSIONS.contains(&extension.as_str()) || candidate.starts_with("http") {
                continue;
            }
            let sanitized = if Path::new(candidate).is_absolute() {
                Path::new(candidate)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(candidate)
                    .to_string()
            } else {
                candidate.trim_start_matches("./").to_string()
            };
            if !sanitized.is_empty() && !files.contains(&sanitized) {
                files.push(sanitized);
            }
        }
    }
    files.truncate(24);
    tests.truncate(12);
    (files, tests)
}

fn merge_results(target: &mut AgentSession, source: &AgentSession) {
    for result in &source.results {
        if !target
            .results
            .iter()
            .any(|existing| existing.id == result.id)
        {
            target.results.push(result.clone());
        }
    }
    target.results.sort_by_key(|result| result.created_at);
    if target.results.len() > 12 {
        target.results.drain(..target.results.len() - 12);
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
        } else {
            session.permission_profile.mode = profile.mode.clone();
            session.permission_profile.label = profile.label.clone();
            session.permission_profile.approval_policy = profile.approval_policy.clone();
            session.permission_profile.approvals_reviewer = profile.approvals_reviewer.clone();
        }
    }
    if let Some(response) = &event.last_response {
        session.last_response = Some(response.clone());
    }
}

fn default_profile(agent: &AgentKind) -> PermissionProfile {
    match agent {
        AgentKind::Claude => PermissionProfile {
            mode: AccessMode::Custom,
            label: "Permissões da sessão".into(),
            approval_policy: "Ações disponíveis conforme o hook".into(),
            approvals_reviewer: None,
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
            approvals_reviewer: None,
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
        && same_agent_identity(
            &left.agent,
            &left.agent_label,
            &right.agent,
            &right.agent_label,
        )
        && left.source == right.source
        && (left.process_id.is_some() && left.process_id == right.process_id
            || same_directory(
                left.working_directory.as_deref(),
                right.working_directory.as_deref(),
            ))
}

fn same_session_identity(left: &AgentSession, right: &AgentSession) -> bool {
    if !same_agent_identity(
        &left.agent,
        &left.agent_label,
        &right.agent,
        &right.agent_label,
    ) {
        return false;
    }
    match (&left.native_session_id, &right.native_session_id) {
        (Some(left_id), Some(right_id)) => left_id == right_id,
        (None, None) => same_provisional_context(left, right),
        _ => false,
    }
}

fn same_agent_identity(
    left_agent: &AgentKind,
    left_label: &str,
    right_agent: &AgentKind,
    right_label: &str,
) -> bool {
    left_agent == right_agent
        && (*left_agent != AgentKind::Unknown || left_label.eq_ignore_ascii_case(right_label))
}

fn session_matches_process(session: &AgentSession, process: &DiscoveredProcess) -> bool {
    same_agent_identity(
        &session.agent,
        &session.agent_label,
        &process.agent,
        &process.agent_label,
    )
}

fn same_directory(left: Option<&str>, right: Option<&str>) -> bool {
    match (
        left.and_then(normalize_directory),
        right.and_then(normalize_directory),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn normalize_directory(directory: &str) -> Option<String> {
    let normalized = directory.trim().replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    if normalized.is_empty() {
        return None;
    }
    #[cfg(target_os = "windows")]
    return Some(normalized.to_ascii_lowercase());
    #[cfg(not(target_os = "windows"))]
    Some(normalized.to_string())
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
            same_agent_identity(
                &existing.agent,
                &existing.agent_label,
                &process.agent,
                &process.agent_label,
            ) && existing.source == process.source
                && (existing.process_id == process.process_id
                    || same_directory(
                        existing.working_directory.as_deref(),
                        process.working_directory.as_deref(),
                    ))
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
            agent_label: "Claude".into(),
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
            last_response: None,
            wait_for_decision: false,
        }
    }

    #[test]
    fn identity_normalizes_directory_separators_and_trailing_slashes() {
        assert!(same_directory(Some("/work/lume/"), Some("/work/lume")));
        assert!(same_directory(
            Some("C:\\work\\lume\\"),
            Some("C:/work/lume")
        ));
    }

    #[test]
    fn processes_without_context_are_not_merged_by_agent_name_alone() {
        let mut first = discovered(4242);
        first.working_directory = None;
        let mut second = discovered(4343);
        second.working_directory = None;
        assert_eq!(coalesce_discovered_processes(vec![first, second]).len(), 2);
    }

    #[test]
    fn external_plugins_with_different_labels_are_not_merged() {
        let mut first = discovered(4242);
        first.agent = AgentKind::Unknown;
        first.agent_label = "Local Agent A".into();
        let mut second = discovered(4343);
        second.agent = AgentKind::Unknown;
        second.agent_label = "Local Agent B".into();

        let discovered = coalesce_discovered_processes(vec![first, second]);
        assert_eq!(discovered.len(), 2);
    }

    #[test]
    fn completed_responses_are_kept_in_memory_per_chat_without_duplicates() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut completed = started_event("claude:results", 4242);
        completed.event = HookEventKind::Completed;
        completed.last_response = Some("Resposta final".into());
        state.ingest(completed.clone()).expect("primeiro resultado");
        state.ingest(completed).expect("evento repetido");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions[0].results.len(), 1);
        assert_eq!(sessions[0].results[0].response, "Resposta final");
    }

    #[test]
    fn final_response_extracts_reported_files_and_checks() {
        let (files, tests) = extract_result_artifacts(
            "Alterei `src/state.rs` e `src/lib/lume.ts`.\n- `cargo test --lib`: passou",
        );
        assert_eq!(files, vec!["src/state.rs", "src/lib/lume.ts"]);
        assert_eq!(tests, vec!["cargo test --lib: passou"]);
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
        assert!(persisted.is_empty());
    }

    #[test]
    fn vscode_completion_reuses_a_process_misclassified_as_cli() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
                process_id: 4242,
                working_directory: Some("/work/lume".into()),
                source: SessionSource::Cli,
            }])
            .expect("processo provisório");
        let mut event = started_event("codex-app-server:chat-1", 4242);
        event.agent = AgentKind::Codex;
        event.source = Some(SessionSource::Vscode);
        event.process_id = None;
        event.native_session_id = Some("chat-1".into());
        event.event = HookEventKind::Running;
        state.ingest(event.clone()).expect("execução");

        event.event = HookEventKind::Completed;
        event.status_label = Some("Tarefa finalizada".into());
        event.last_response = Some("Tudo pronto".into());
        state.ingest(event).expect("conclusão");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "codex-app-server:chat-1");
        assert_eq!(sessions[0].source, SessionSource::Vscode);
        assert_eq!(sessions[0].process_id, Some(4242));
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert_eq!(sessions[0].last_response.as_deref(), Some("Tudo pronto"));
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
    fn transient_process_gap_keeps_the_same_session_active() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:session-gap", 4242))
            .expect("hook");

        for _ in 0..PROCESS_MISSING_SCAN_LIMIT - 1 {
            state
                .reconcile_processes(Vec::new())
                .expect("ausência transitória");
        }
        state
            .reconcile_processes(vec![discovered(4343)])
            .expect("redetecção");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "claude:session-gap");
        assert_eq!(sessions[0].process_id, Some(4343));
        assert_eq!(sessions[0].status, SessionStatus::WaitingForInput);
        assert!(state.history(10).expect("histórico").is_empty());
    }

    #[test]
    fn live_pid_survives_a_temporary_agent_classification_gap() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut running = started_event("claude:live-session", 4242);
        running.event = HookEventKind::Running;
        state.ingest(running).expect("hook");

        for _ in 0..=PROCESS_MISSING_SCAN_LIMIT {
            state
                .reconcile_process_snapshot(Vec::new(), HashSet::from([4242]))
                .expect("processo ainda vivo");
        }

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "claude:live-session");
        assert_eq!(sessions[0].status, SessionStatus::Running);
        assert!(state.history(10).expect("histórico").is_empty());

        for _ in 0..PROCESS_MISSING_SCAN_LIMIT {
            state
                .reconcile_process_snapshot(Vec::new(), HashSet::new())
                .expect("processo encerrado");
        }

        assert!(state.sessions().expect("sessões").is_empty());
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
        assert!(state
            .store
            .lock()
            .expect("store")
            .load_sessions()
            .expect("persistência")
            .is_empty());
    }

    #[test]
    fn integrations_with_the_same_native_chat_become_one_session() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut direct = started_event("claude-direct:chat-1", 4242);
        direct.permission_profile = Some(PermissionProfile {
            mode: AccessMode::Custom,
            label: "Integração direta".into(),
            approval_policy: "Perguntar".into(),
            approvals_reviewer: None,
            can_respond_from_lume: true,
            available_actions: vec![PermissionAction::AllowOnce],
        });
        state.ingest(direct).expect("integração direta");

        let mut hook = started_event("claude-hook:chat-1", 4242);
        hook.event = HookEventKind::Completed;
        hook.status_label = Some("Finalizado pelo hook".into());
        hook.permission_profile = Some(PermissionProfile {
            mode: AccessMode::ReadOnly,
            label: "Somente observação".into(),
            approval_policy: "Abrir origem".into(),
            approvals_reviewer: Some("auto_review".into()),
            can_respond_from_lume: false,
            available_actions: vec![PermissionAction::OpenSource],
        });
        state.ingest(hook).expect("hook");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "claude-direct:chat-1");
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert!(sessions[0].permission_profile.can_respond_from_lume);
        assert_eq!(sessions[0].permission_profile.mode, AccessMode::ReadOnly);
        assert_eq!(
            sessions[0].permission_profile.approvals_reviewer.as_deref(),
            Some("auto_review")
        );
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
    fn terminating_a_process_removes_all_of_its_chats() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut first = started_event("claude:chat-1", 4242);
        first.native_session_id = Some("native-chat-1".into());
        state.ingest(first).expect("primeiro chat");
        let mut second = started_event("claude:chat-2", 4242);
        second.native_session_id = Some("native-chat-2".into());
        state.ingest(second).expect("segundo chat");

        assert!(state.mark_process_terminated(4242).expect("encerramento"));

        assert!(state.sessions().expect("sessões").is_empty());
        assert_eq!(state.history(10).expect("histórico").len(), 2);
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
                agent_label: "Codex".into(),
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
                    last_response: None,
                    wait_for_decision: false,
                })
                .expect("chat do VS Code");
        }
        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
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
        assert!(state
            .store
            .lock()
            .expect("store")
            .load_sessions()
            .expect("persistência")
            .is_empty());
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
    fn disappearing_process_removes_a_hook_backed_session() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:session-2", 4343))
            .expect("hook");

        for _ in 0..PROCESS_MISSING_SCAN_LIMIT {
            state
                .reconcile_processes(Vec::new())
                .expect("reconciliação");
        }

        assert!(state.sessions().expect("sessões").is_empty());

        let history = state.history(10).expect("histórico");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].session_id, "claude:session-2");
        assert_eq!(history[0].event, "completed");
    }

    #[test]
    fn session_end_removes_the_agent_immediately() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:closed-session", 4343))
            .expect("início");
        let mut ended = started_event("claude:closed-session", 4343);
        ended.event = HookEventKind::SessionEnded;

        state.ingest(ended).expect("fim da sessão");

        assert!(state.sessions().expect("sessões").is_empty());
    }

    #[test]
    fn stale_browser_heartbeat_removes_the_web_agent() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("web:codex:chat", 4343);
        event.source = Some(SessionSource::Web);
        event.process_id = None;
        state.ingest(event).expect("evento web");
        state.sessions.lock().expect("sessões")[0].updated_at =
            now_millis() - WEB_SESSION_STALE_MS - 1;

        state.reconcile_processes(Vec::new()).expect("limpeza");

        assert!(state.sessions().expect("sessões").is_empty());
    }

    #[test]
    fn later_running_event_clears_a_stale_permission() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut permission = started_event("codex:chat-1", 4242);
        permission.event = HookEventKind::PermissionRequest;
        permission.permission = Some(PermissionRequest {
            id: "permission-1".into(),
            kind: "command".into(),
            summary: "Executar comando".into(),
            resource: "npm test".into(),
            risk: "medium".into(),
            requested_at: "1".into(),
        });
        state.ingest(permission).expect("permissão");

        let mut running = started_event("codex:chat-1", 4242);
        running.event = HookEventKind::Running;
        state.ingest(running).expect("execução");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.status, SessionStatus::Running);
        assert!(session.pending_permission.is_none());
        assert_eq!(
            state
                .wait_for_decision("permission-1", Duration::ZERO)
                .expect("decisão"),
            Some(PermissionAction::Deny)
        );
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
            approvals_reviewer: None,
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

        for _ in 0..PROCESS_MISSING_SCAN_LIMIT {
            state
                .reconcile_processes(Vec::new())
                .expect("reconciliação");
        }

        assert_eq!(
            state
                .wait_for_decision("permission-1", Duration::ZERO)
                .expect("decisão"),
            Some(PermissionAction::Deny)
        );
    }

    #[test]
    fn restart_does_not_restore_agent_sessions() {
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
        assert!(restarted.sessions().expect("sessões").is_empty());
        drop(restarted);

        let _ = std::fs::remove_file(&database_path);
        let _ = std::fs::remove_file(database_path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(database_path.with_extension("sqlite3-shm"));
    }
}
