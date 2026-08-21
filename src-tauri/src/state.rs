use std::{
    collections::{HashMap, HashSet},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    discovery::DiscoveredProcess,
    domain::{
        AccessMode, AgentKind, AgentRateLimit, AgentSession, HistoryEntry, HookEvent,
        HookEventKind, PairedDevice, PermissionAction, PermissionProfile, PermissionRequest,
        Preferences, PromptAttachment, QuestionAnswer, ResultNote, SessionActivity,
        SessionControlOrigin, SessionModelOverride, SessionNote, SessionResult, SessionSource,
        SessionStatus, WorkflowHistoryRecord,
    },
    integrations::{self, IntegrationKind},
    store::Store,
};

const PROCESS_MISSING_SCAN_LIMIT: u8 = 2;
const WEB_SESSION_STALE_MS: i64 = 90_000;
const RECENT_NATIVE_SESSION_MS: i64 = 10 * 60 * 1_000;
const PIDLESS_CODEX_SESSION_TTL_MS: i64 = 60_000;
const LUME_ATTACHED_FILES_MARKER: &str = "Files attached through Lume. Inspect these local paths:";

#[derive(Clone, Debug)]
struct WorkspaceSnapshot {
    root: PathBuf,
    head: Option<String>,
    files: HashMap<String, u64>,
}

#[derive(Clone)]
pub struct AppState {
    sessions: Arc<Mutex<Vec<AgentSession>>>,
    store: Arc<Mutex<Store>>,
    decisions: Arc<(Mutex<HashMap<String, PermissionAction>>, Condvar)>,
    question_answers: Arc<(Mutex<HashMap<String, Vec<QuestionAnswer>>>, Condvar)>,
    missing_process_scans: Arc<Mutex<HashMap<String, u8>>>,
    workspace_snapshots: Arc<Mutex<HashMap<String, WorkspaceSnapshot>>>,
    agent_rate_limits: Arc<Mutex<HashMap<AgentKind, Vec<AgentRateLimit>>>>,
    session_aliases: Arc<Mutex<HashMap<String, String>>>,
    archived_conversations: Arc<Mutex<HashMap<String, Vec<SessionActivity>>>>,
    session_model_overrides: Arc<Mutex<HashMap<(AgentKind, String), SessionModelOverride>>>,
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
            question_answers: Arc::new((Mutex::new(HashMap::new()), Condvar::new())),
            missing_process_scans: Arc::new(Mutex::new(HashMap::new())),
            workspace_snapshots: Arc::new(Mutex::new(HashMap::new())),
            agent_rate_limits: Arc::new(Mutex::new(HashMap::new())),
            session_aliases: Arc::new(Mutex::new(preferences.session_aliases)),
            archived_conversations: Arc::new(Mutex::new(HashMap::new())),
            session_model_overrides: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sessions(&self) -> Result<Vec<AgentSession>, String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?
            .clone();
        self.attach_archived_conversations(&mut sessions)?;
        self.attach_session_plans(&mut sessions)?;
        self.finalize_sessions(sessions)
    }

    pub fn terminal_sessions<F>(
        &self,
        activity_limit: usize,
        matches_terminal: F,
    ) -> Result<Vec<AgentSession>, String>
    where
        F: Fn(&AgentSession) -> bool,
    {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?
            .iter()
            .filter(|session| matches_terminal(session))
            .cloned()
            .collect::<Vec<_>>();
        self.attach_recent_archived_conversations(&mut sessions, activity_limit)?;
        self.attach_session_plans(&mut sessions)?;
        for session in &mut sessions {
            let start = session
                .activities
                .len()
                .saturating_sub(activity_limit.max(1));
            if start > 0 {
                let mut recent = session.activities.split_off(start);
                recent.extend(session.activities.drain(..).filter(|activity| {
                    activity.kind == "plan_document"
                        || (activity.kind == "queued_prompt" && activity.status == "waiting")
                }));
                recent.sort_by_key(|activity| activity.created_at);
                session.activities = recent;
            }
            let result_start = session.results.len().saturating_sub(activity_limit.max(1));
            if result_start > 0 {
                session.results.drain(..result_start);
            }
        }
        self.finalize_sessions(sessions)
    }

    pub fn bounded_sessions(&self, activity_limit: usize) -> Result<Vec<AgentSession>, String> {
        self.terminal_sessions(activity_limit, |_| true)
    }

    pub fn connected_sessions(&self) -> Result<Vec<AgentSession>, String> {
        self.sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())
            .map(|sessions| sessions.clone())
    }

    pub fn connected_session(&self, session_id: &str) -> Result<AgentSession, String> {
        self.sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
            .ok_or_else(|| "Session not found".to_string())
    }

    pub fn session_with_history(&self, session_id: &str) -> Result<AgentSession, String> {
        let mut sessions = vec![self.connected_session(session_id)?];
        self.attach_archived_conversations(&mut sessions)?;
        self.attach_session_plans(&mut sessions)?;
        self.finalize_sessions(sessions)?
            .into_iter()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Session not found".to_string())
    }

    pub fn session_status(
        &self,
        session_id: &str,
        native_session_id: Option<&str>,
    ) -> Result<Option<SessionStatus>, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?;
        Ok(sessions
            .iter()
            .find(|session| {
                session.id == session_id
                    || native_session_id.is_some_and(|native_id| {
                        session.native_session_id.as_deref() == Some(native_id)
                    })
            })
            .map(|session| session.status.clone()))
    }

    pub fn session_automatically_approves(
        &self,
        session_id: &str,
        native_session_id: Option<&str>,
    ) -> Result<bool, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?;
        Ok(sessions.iter().any(|session| {
            (session.id == session_id
                || native_session_id.is_some_and(|native_id| {
                    session.native_session_id.as_deref() == Some(native_id)
                }))
                && session.permission_profile.automatically_approves()
        }))
    }

    fn finalize_sessions(
        &self,
        mut sessions: Vec<AgentSession>,
    ) -> Result<Vec<AgentSession>, String> {
        sessions.retain(|session| {
            !(session.agent == AgentKind::Codex
                && session
                    .working_directory
                    .as_deref()
                    .is_some_and(crate::session_filters::is_codex_internal_workspace))
        });
        let aliases = self
            .session_aliases
            .lock()
            .map_err(|_| "Não foi possível acessar os nomes das sessões".to_string())?
            .clone();
        apply_session_aliases(&mut sessions, &aliases);
        let agent_rate_limits = self
            .agent_rate_limits
            .lock()
            .map_err(|_| "Não foi possível acessar os limites dos agentes".to_string())?
            .clone();
        for session in &mut sessions {
            session.rate_limits = agent_rate_limits
                .get(&session.agent)
                .cloned()
                .unwrap_or_default();
        }
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
                }) && !(session.source == SessionSource::Vscode
                    && native_vscode_agents.contains(&session.agent)))
        });
        let mut sessions = deduplicated;
        ensure_unique_session_names(&mut sessions);
        sessions.sort_by_key(|session| (status_priority(&session.status), -session.updated_at));
        Ok(sessions)
    }

    fn attach_recent_archived_conversations(
        &self,
        sessions: &mut [AgentSession],
        activity_limit: usize,
    ) -> Result<(), String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "Não foi possível acessar o histórico das conversas".to_string())?;
        for session in sessions {
            let archived = store.recent_conversation_activities(session, activity_limit)?;
            for activity in archived {
                if !session
                    .activities
                    .iter()
                    .any(|current| current.id == activity.id)
                {
                    session.activities.push(activity);
                }
            }
            session
                .activities
                .sort_by_key(|activity| activity.created_at);
        }
        Ok(())
    }

    fn attach_session_plans(&self, sessions: &mut [AgentSession]) -> Result<(), String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "Não foi possível acessar os planejamentos".to_string())?;
        for session in sessions {
            let Some(native_session_id) = session.native_session_id.as_deref() else {
                continue;
            };
            if let Some(activity) = session.activities.iter().rev().find(|activity| {
                activity.kind == "message"
                    && activity
                        .detail
                        .as_deref()
                        .is_some_and(crate::protocol::looks_like_full_plan_document)
            }) {
                if let Some(content) = activity.detail.as_deref() {
                    store.save_session_plan(
                        native_session_id,
                        content,
                        Some(&activity.id),
                        activity.created_at,
                    )?;
                    store.save_session_note_if_absent(&SessionNote {
                        id: format!("plan-document:{native_session_id}:{}", activity.id),
                        native_session_id: native_session_id.to_string(),
                        title: plan_note_title(content, "Saved plan"),
                        body: content.chars().take(128 * 1024).collect(),
                        kind: "plan".into(),
                        pinned: false,
                        created_at: activity.created_at,
                        updated_at: activity.created_at,
                    })?;
                }
            }
            let mut plan_groups: Vec<(&SessionActivity, String, String)> = Vec::new();
            for activity in session
                .activities
                .iter()
                .filter(|activity| activity.kind == "plan" || activity.kind == "tool")
            {
                let Some((signature, body)) =
                    crate::protocol::archived_plan_from_activity(activity)
                else {
                    continue;
                };
                if let Some(last) = plan_groups.last_mut().filter(|last| last.1 == signature) {
                    *last = (activity, signature, body);
                } else {
                    plan_groups.push((activity, signature, body));
                }
            }
            let archived_count = plan_groups.len().saturating_sub(1);
            for (activity, _, body) in plan_groups.into_iter().take(archived_count) {
                store.save_session_note_if_absent(&SessionNote {
                    id: format!("plan-history:{native_session_id}:{}", activity.id),
                    native_session_id: native_session_id.to_string(),
                    title: plan_note_title(&body, "Previous plan"),
                    body: body.chars().take(128 * 1024).collect(),
                    kind: "plan".into(),
                    pinned: false,
                    created_at: activity.created_at,
                    updated_at: activity.created_at,
                })?;
            }
            let Some((content, _, updated_at)) = store.session_plan(native_session_id)? else {
                continue;
            };
            session
                .activities
                .retain(|activity| activity.kind != "plan_document");
            session.activities.push(SessionActivity {
                id: format!("plan-document:{native_session_id}"),
                kind: "plan_document".into(),
                title: "Plan".into(),
                detail: Some(content),
                status: "completed".into(),
                created_at: updated_at,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            });
            session
                .activities
                .sort_by_key(|activity| activity.created_at);
        }
        Ok(())
    }

    fn attach_archived_conversations(&self, sessions: &mut [AgentSession]) -> Result<(), String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "Não foi possível acessar o histórico das conversas".to_string())?;
        let mut cache = self
            .archived_conversations
            .lock()
            .map_err(|_| "Não foi possível acessar o cache das conversas".to_string())?;
        for session in sessions {
            let Some(key) = Store::conversation_key(session) else {
                continue;
            };
            if !cache.contains_key(&key) {
                cache.insert(key.clone(), store.conversation_activities(session)?);
            }
            let archived = cache.entry(key).or_default();
            for activity in session
                .activities
                .iter()
                .filter(|activity| Store::is_archivable_conversation_activity(activity))
            {
                if let Some(existing) = archived.iter_mut().find(|item| item.id == activity.id) {
                    let existing_length = existing.detail.as_deref().map(str::len).unwrap_or(0);
                    let incoming_length = activity.detail.as_deref().map(str::len).unwrap_or(0);
                    if incoming_length >= existing_length {
                        *existing = activity.clone();
                    }
                } else {
                    archived.push(activity.clone());
                }
            }
            for activity in archived.iter() {
                if !session
                    .activities
                    .iter()
                    .any(|current| current.id == activity.id)
                {
                    session.activities.push(activity.clone());
                }
            }
            session
                .activities
                .sort_by_key(|activity| activity.created_at);
        }
        Ok(())
    }

    pub fn rename_session(&self, session_id: &str, name: &str) -> Result<String, String> {
        let (_, unique_name) = self.session_rename_plan(session_id, name)?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível renomear a sessão".to_string())?;
        let mut aliases = self
            .session_aliases
            .lock()
            .map_err(|_| "Não foi possível atualizar o nome da sessão".to_string())?;
        let alias_key = {
            let session = sessions
                .iter_mut()
                .find(|session| session.id == session_id)
                .ok_or_else(|| "Sessão não encontrada".to_string())?;
            session.session_name.clone_from(&unique_name);
            persistent_session_alias_key(session)
        };
        if let Some(key) = alias_key {
            aliases.insert(key, unique_name.clone());
            let store = self
                .store
                .lock()
                .map_err(|_| "Não foi possível salvar o nome da sessão".to_string())?;
            let mut preferences = store.load_preferences()?;
            preferences.session_aliases.clone_from(&aliases);
            store.save_preferences(&preferences)?;
        }
        Ok(unique_name)
    }

    pub fn session_rename_plan(
        &self,
        session_id: &str,
        name: &str,
    ) -> Result<(AgentSession, String), String> {
        let requested = normalized_session_name(name)
            .ok_or_else(|| "O nome da sessão não pode ficar vazio".to_string())?;
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível renomear a sessão".to_string())?;
        let session = sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
            .ok_or_else(|| "Sessão não encontrada".to_string())?;
        let aliases = self
            .session_aliases
            .lock()
            .map_err(|_| "Não foi possível atualizar o nome da sessão".to_string())?;
        let mut named_sessions = sessions.clone();
        apply_session_aliases(&mut named_sessions, &aliases);
        ensure_unique_session_names(&mut named_sessions);
        let used = named_sessions
            .iter()
            .filter(|session| session.id != session_id)
            .map(|session| session.session_name.to_lowercase())
            .collect::<HashSet<_>>();
        let unique_name = unique_session_name(&requested, &used);
        Ok((session, unique_name))
    }

    pub fn record_prompt_activity(
        &self,
        session_id: &str,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<(), String> {
        let now = now_millis();
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível atualizar o prompt da sessão".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Sessão não encontrada".to_string())?;
        remember_activity(
            session,
            SessionActivity {
                id: format!("local:{session_id}:{now}"),
                kind: "prompt".into(),
                title: "Prompt enviado pelo Lume".into(),
                detail: (!prompt.is_empty()).then(|| prompt.to_string()),
                status: "completed".into(),
                created_at: now,
                files: Vec::new(),
                attachments,
                append_detail: false,
            },
        );
        session.status = SessionStatus::Running;
        session.status_label = "Executando".into();
        session.updated_at = now;
        Ok(())
    }

    pub fn rebind_codex_thread(
        &self,
        session_id: &str,
        native_session_id: String,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Could not reconnect the Codex session".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        if session.agent != AgentKind::Codex {
            return Err("Only Codex sessions can be rebound to a thread".into());
        }
        session.native_session_id = Some(native_session_id);
        session.updated_at = now_millis();
        Ok(())
    }

    pub fn mark_session_lume_controlled(&self, session_id: &str) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Could not transfer control of this session".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        session.control_origin = SessionControlOrigin::Lume;
        session.source = SessionSource::Cli;
        session.source_app = None;
        session.process_id = None;
        session.status = SessionStatus::WaitingForInput;
        session.status_label = "Ready in Lume".into();
        session.pending_permission = None;
        session.pending_question = None;
        session.permission_profile.can_respond_from_lume = true;
        session.permission_profile.available_actions = vec![
            PermissionAction::AllowOnce,
            PermissionAction::AllowSession,
            PermissionAction::Deny,
        ];
        session.updated_at = now_millis();
        Ok(())
    }

    pub fn session_model_override(&self, session_id: &str) -> Result<SessionModelOverride, String> {
        let session = self.connected_session(session_id)?;
        let key = (
            session.agent,
            session
                .native_session_id
                .unwrap_or_else(|| session.id.clone()),
        );
        self.session_model_overrides
            .lock()
            .map_err(|_| "Could not read the session model settings".to_string())
            .map(|settings| settings.get(&key).cloned().unwrap_or_default())
    }

    pub fn set_session_model_override(
        &self,
        session_id: &str,
        settings: SessionModelOverride,
    ) -> Result<SessionModelOverride, String> {
        let session = self.connected_session(session_id)?;
        let key = (
            session.agent,
            session
                .native_session_id
                .unwrap_or_else(|| session.id.clone()),
        );
        self.session_model_overrides
            .lock()
            .map_err(|_| "Could not save the session model settings".to_string())?
            .insert(key, settings.clone());
        Ok(settings)
    }

    pub fn record_queued_prompt_activity(
        &self,
        session_id: &str,
        activity_id: &str,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<(), String> {
        let now = now_millis();
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Could not update the Codex prompt queue".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        remember_activity(
            session,
            SessionActivity {
                id: activity_id.to_string(),
                kind: "queued_prompt".into(),
                title: "Queued prompt".into(),
                detail: (!prompt.is_empty()).then(|| prompt.to_string()),
                status: "waiting".into(),
                created_at: now,
                files: Vec::new(),
                attachments,
                append_detail: false,
            },
        );
        session.updated_at = now;
        Ok(())
    }

    pub fn promote_queued_prompt_activity(
        &self,
        session_id: &str,
        activity_id: &str,
    ) -> Result<(), String> {
        let now = now_millis();
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Could not start the queued Codex prompt".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        let activity = session
            .activities
            .iter_mut()
            .find(|activity| activity.id == activity_id)
            .ok_or_else(|| "Queued prompt not found".to_string())?;
        activity.kind = "prompt".into();
        activity.title = "Prompt sent by Lume".into();
        activity.status = "completed".into();
        activity.created_at = now;
        session.updated_at = now;
        Ok(())
    }

    pub fn remove_queued_prompt_activity(
        &self,
        session_id: &str,
        activity_id: &str,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Could not update the Codex prompt queue".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        session
            .activities
            .retain(|activity| activity.id != activity_id);
        session.updated_at = now_millis();
        Ok(())
    }

    pub fn mark_queued_prompt_needs_attention(
        &self,
        session_id: &str,
        activity_id: &str,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Could not update the Codex prompt queue".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        if let Some(activity) = session
            .activities
            .iter_mut()
            .find(|activity| activity.id == activity_id)
        {
            activity.title = "Queued prompt was not replayed".into();
            activity.status = "failed".into();
        } else {
            remember_activity(
                session,
                SessionActivity {
                    id: activity_id.to_string(),
                    kind: "queued_prompt".into(),
                    title: "Queued prompt was not replayed".into(),
                    detail: Some(
                        "Lume could not confirm delivery and did not resend it automatically."
                            .into(),
                    ),
                    status: "failed".into(),
                    created_at: now_millis(),
                    files: Vec::new(),
                    attachments: Vec::new(),
                    append_detail: false,
                },
            );
        }
        session.updated_at = now_millis();
        Ok(())
    }

    pub fn mark_prompt_interrupted(&self, session_id: &str) -> Result<(), String> {
        let now = now_millis();
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Could not update the interrupted prompt".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        session.status = SessionStatus::WaitingForInput;
        session.status_label = "Prompt interrupted".into();
        session.pending_permission = None;
        session.pending_question = None;
        for activity in session
            .activities
            .iter_mut()
            .filter(|activity| activity.status == "running")
        {
            activity.status = "interrupted".into();
        }
        remember_activity(
            session,
            SessionActivity {
                id: format!("local:{session_id}:interrupted:{now}"),
                kind: "activity".into(),
                title: "Prompt interrupted".into(),
                detail: None,
                status: "completed".into(),
                created_at: now,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            },
        );
        session.updated_at = now;
        Ok(())
    }

    pub fn set_agent_rate_limits(
        &self,
        agent: AgentKind,
        limits: Vec<AgentRateLimit>,
    ) -> Result<bool, String> {
        let mut current = self
            .agent_rate_limits
            .lock()
            .map_err(|_| "Não foi possível atualizar os limites dos agentes".to_string())?;
        let changed = current.get(&agent) != Some(&limits);
        current.insert(agent, limits);
        Ok(changed)
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
        let mut preferences = preferences.clone();
        preferences.session_aliases = self
            .session_aliases
            .lock()
            .map_err(|_| "Não foi possível preservar os nomes das sessões".to_string())?
            .clone();
        let store = self
            .store
            .lock()
            .map_err(|_| "Não foi possível salvar as preferências".to_string())?;
        store.save_preferences(&preferences)?;
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

    pub fn session_notes(&self, session_id: &str) -> Result<Vec<SessionNote>, String> {
        let native_session_id = self.session_native_id(session_id)?;
        self.store
            .lock()
            .map_err(|_| "Could not access session notes".to_string())?
            .session_notes(&native_session_id)
    }

    pub fn save_session_note(
        &self,
        session_id: &str,
        note_id: Option<&str>,
        title: &str,
        body: &str,
        kind: &str,
        pinned: bool,
    ) -> Result<SessionNote, String> {
        let native_session_id = self.session_native_id(session_id)?;
        let title = title.trim();
        let body = body.trim();
        if body.is_empty() {
            return Err("A note cannot be empty".into());
        }
        let kind = if kind == "plan" { "plan" } else { "note" };
        let now = now_millis();
        let id = note_id
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("session-note:{native_session_id}:{now}"));
        let existing = self
            .store
            .lock()
            .map_err(|_| "Could not access session notes".to_string())?
            .session_notes(&native_session_id)?
            .into_iter()
            .find(|note| note.id == id);
        if note_id.is_some() && existing.is_none() {
            return Err("This note does not belong to the selected session".into());
        }
        let note = SessionNote {
            id,
            native_session_id,
            title: if title.is_empty() {
                if kind == "plan" { "Saved plan" } else { "Note" }.into()
            } else {
                title.chars().take(120).collect()
            },
            body: body.chars().take(128 * 1024).collect(),
            kind: kind.into(),
            pinned,
            created_at: existing.as_ref().map(|note| note.created_at).unwrap_or(now),
            updated_at: now,
        };
        self.store
            .lock()
            .map_err(|_| "Could not save the session note".to_string())?
            .save_session_note(&note)?;
        Ok(note)
    }

    pub fn delete_session_note(&self, id: &str) -> Result<(), String> {
        self.store
            .lock()
            .map_err(|_| "Could not delete the session note".to_string())?
            .delete_session_note(id)
    }

    fn session_native_id(&self, session_id: &str) -> Result<String, String> {
        self.connected_session(session_id)?
            .native_session_id
            .ok_or_else(|| "This session does not have a persistent conversation id".to_string())
    }

    pub fn save_workflow_run(
        &self,
        workflow_id: &str,
        payload: &str,
        updated_at: i64,
    ) -> Result<(), String> {
        self.store
            .lock()
            .map_err(|_| "Could not persist the workflow run".to_string())?
            .save_workflow_run(workflow_id, payload, updated_at)
    }

    pub fn workflow_runs(&self) -> Result<Vec<String>, String> {
        self.store
            .lock()
            .map_err(|_| "Could not restore workflow runs".to_string())?
            .workflow_runs()
    }

    pub fn save_workflow_history(&self, record: &WorkflowHistoryRecord) -> Result<(), String> {
        self.store
            .lock()
            .map_err(|_| "Could not persist the workflow history".to_string())?
            .save_workflow_history(record)
    }

    pub fn workflow_history(&self, limit: usize) -> Result<Vec<WorkflowHistoryRecord>, String> {
        self.store
            .lock()
            .map_err(|_| "Could not load the workflow history".to_string())?
            .workflow_history(limit)
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

    pub fn save_mobile_device(
        &self,
        device: &PairedDevice,
        token_hash: &str,
    ) -> Result<(), String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível salvar o dispositivo".to_string())?
            .save_mobile_device(device, token_hash)
    }

    pub fn mobile_devices(&self) -> Result<Vec<PairedDevice>, String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível acessar os dispositivos".to_string())?
            .mobile_devices()
    }

    pub fn authenticate_mobile_device(
        &self,
        token_hash: &str,
    ) -> Result<Option<PairedDevice>, String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível autenticar o dispositivo".to_string())?
            .mobile_device_for_token_hash(token_hash, now_millis())
    }

    pub fn mobile_device_with_token_hash(
        &self,
        id: &str,
    ) -> Result<Option<(PairedDevice, String)>, String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível autenticar o dispositivo".to_string())?
            .mobile_device_with_token_hash(id, now_millis())
    }

    pub fn revoke_mobile_device(&self, id: &str) -> Result<bool, String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível revogar o dispositivo".to_string())?
            .revoke_mobile_device(id)
    }

    pub fn set_mobile_device_scopes(
        &self,
        id: &str,
        scopes: &[crate::domain::MobileScope],
    ) -> Result<bool, String> {
        self.store
            .lock()
            .map_err(|_| "Não foi possível atualizar o dispositivo".to_string())?
            .set_mobile_device_scopes(id, scopes)
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
        if event.agent == AgentKind::Codex
            && event
                .working_directory
                .as_deref()
                .is_some_and(crate::session_filters::is_codex_internal_workspace)
        {
            return Ok(None);
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
        let exact_provisional_ids = sessions
            .iter()
            .filter(|session| {
                is_provisional_process(session)
                    && session.agent == event.agent
                    && event
                        .process_id
                        .is_some_and(|process_id| session.process_id == Some(process_id))
            })
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();
        let contextual_provisional_ids = exact_provisional_ids
            .is_empty()
            .then(|| {
                sessions
                    .iter()
                    .filter(|session| {
                        is_provisional_process(session)
                            && session.agent == event.agent
                            && match event.source.as_ref() {
                                Some(SessionSource::Cli) => {
                                    session.source == SessionSource::Cli
                                        && same_directory(
                                            session.working_directory.as_deref(),
                                            event.working_directory.as_deref(),
                                        )
                                }
                                Some(SessionSource::Vscode) => {
                                    (session.source == SessionSource::Vscode
                                        && event.working_directory.is_none())
                                        || same_directory(
                                            session.working_directory.as_deref(),
                                            event.working_directory.as_deref(),
                                        )
                                }
                                _ => false,
                            }
                    })
                    .map(|session| session.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let provisional_ids = if !exact_provisional_ids.is_empty() {
            exact_provisional_ids
        } else if contextual_provisional_ids.len() == 1 {
            contextual_provisional_ids
        } else {
            Vec::new()
        };

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
        for activity in &event.activities {
            remember_activity(session, activity.clone());
        }
        if let Some(activity) = event.activity.as_ref() {
            remember_activity(session, activity.clone());
        }
        session.updated_at = now;
        self.missing_process_scans
            .lock()
            .map_err(|_| "Não foi possível atualizar a presença da sessão".to_string())?
            .remove(&target_session_id);
        let repeated_permission = matches!(&event.event, HookEventKind::PermissionRequest)
            && session
                .pending_permission
                .as_ref()
                .zip(event.permission.as_ref())
                .is_some_and(|(current, incoming)| same_permission_request(current, incoming));
        let repeated_question = matches!(&event.event, HookEventKind::QuestionRequest)
            && session.pending_question.as_ref() == event.question.as_ref();
        let superseded_permission_id = session
            .pending_permission
            .as_ref()
            .filter(|_| {
                !matches!(&event.event, HookEventKind::PermissionRequest) || !repeated_permission
            })
            .map(|permission| permission.id.clone());
        if let Some(permission_id) = superseded_permission_id.as_ref() {
            if let Some(activity) = session
                .activities
                .iter_mut()
                .find(|activity| activity.id == format!("permission:{permission_id}"))
            {
                activity.status = "completed".into();
            }
        }
        let superseded_question_id = session
            .pending_question
            .as_ref()
            .filter(|_| {
                !matches!(&event.event, HookEventKind::QuestionRequest) || !repeated_question
            })
            .map(|question| question.id.clone());
        let starts_task = matches!(&event.event, HookEventKind::Running)
            && !matches!(
                session.status,
                SessionStatus::Running | SessionStatus::PermissionRequired
            );
        if starts_task {
            if let Some(snapshot) = workspace_snapshot(session.working_directory.as_deref()) {
                self.workspace_snapshots
                    .lock()
                    .map_err(|_| "Não foi possível iniciar o rastreio de arquivos".to_string())?
                    .insert(target_session_id.clone(), snapshot);
            }
        }
        let finishes_task = matches!(
            &event.event,
            HookEventKind::Completed | HookEventKind::Failed | HookEventKind::SessionEnded
        );
        let observed_files = if finishes_task {
            self.workspace_changes(&target_session_id, session.working_directory.as_deref())?
        } else {
            Vec::new()
        };

        let permission_id = match event.event {
            HookEventKind::SessionStarted => {
                session.status = SessionStatus::WaitingForInput;
                session.status_label = "Esperando ação".into();
                session.pending_permission = None;
                session.pending_question = None;
                None
            }
            HookEventKind::Running => {
                session.status = SessionStatus::Running;
                session.status_label = event.status_label.unwrap_or_else(|| "Executando".into());
                session.pending_permission = None;
                session.pending_question = None;
                session.last_response = None;
                None
            }
            HookEventKind::Activity => None,
            HookEventKind::PermissionRequest => {
                if session.permission_profile.automatically_approves() {
                    session.status = SessionStatus::Running;
                    session.status_label = "Executando".into();
                    session.pending_permission = None;
                    session.pending_question = None;
                    None
                } else {
                    let mut permission = event
                        .permission
                        .ok_or_else(|| "A solicitação não contém a permissão".to_string())?;
                    if repeated_permission {
                        if let Some(current) = session.pending_permission.as_ref() {
                            permission.id.clone_from(&current.id);
                            permission.requested_at.clone_from(&current.requested_at);
                        }
                    }
                    let id = permission.id.clone();
                    session.status = SessionStatus::PermissionRequired;
                    session.status_label = event
                        .status_label
                        .unwrap_or_else(|| "Aguardando permissão".into());
                    session.pending_permission = Some(permission);
                    session.pending_question = None;
                    remember_activity(
                        session,
                        SessionActivity {
                            id: format!("permission:{id}"),
                            kind: "permission".into(),
                            title: session
                                .pending_permission
                                .as_ref()
                                .map(|permission| permission.summary.clone())
                                .unwrap_or_else(|| "Permissão solicitada".into()),
                            detail: session
                                .pending_permission
                                .as_ref()
                                .map(|permission| permission.resource.clone()),
                            status: "waiting".into(),
                            created_at: now,
                            files: Vec::new(),
                            attachments: Vec::new(),
                            append_detail: false,
                        },
                    );
                    Some(id)
                }
            }
            HookEventKind::QuestionRequest => {
                let question = event
                    .question
                    .ok_or_else(|| "A solicitação não contém uma pergunta".to_string())?;
                let id = question.id.clone();
                session.status = SessionStatus::WaitingForInput;
                session.status_label = event
                    .status_label
                    .unwrap_or_else(|| "Aguardando sua resposta".into());
                session.pending_permission = None;
                session.pending_question = Some(question);
                remember_activity(
                    session,
                    SessionActivity {
                        id: format!("question:{id}"),
                        kind: "question".into(),
                        title: "Pergunta do agente".into(),
                        detail: session
                            .pending_question
                            .as_ref()
                            .and_then(|request| request.questions.first())
                            .map(|question| question.question.clone()),
                        status: "waiting".into(),
                        created_at: now,
                        files: Vec::new(),
                        attachments: Vec::new(),
                        append_detail: false,
                    },
                );
                None
            }
            HookEventKind::WaitingForInput => {
                session.status = SessionStatus::WaitingForInput;
                session.status_label = if session.pending_question.is_some() {
                    "Aguardando sua resposta".into()
                } else {
                    event
                        .status_label
                        .unwrap_or_else(|| "Aguardando sua resposta".into())
                };
                session.pending_permission = None;
                None
            }
            HookEventKind::Completed | HookEventKind::SessionEnded => {
                session.status = SessionStatus::Completed;
                session.status_label = event.status_label.unwrap_or_else(|| "Finalizado".into());
                session.pending_permission = None;
                session.pending_question = None;
                if session.last_response.is_some() {
                    remove_superseded_streamed_messages(session);
                }
                for activity in session
                    .activities
                    .iter_mut()
                    .filter(|activity| activity.status == "running")
                {
                    activity.status = "completed".into();
                }
                None
            }
            HookEventKind::Failed => {
                session.status = SessionStatus::Failed;
                session.status_label = event.status_label.unwrap_or_else(|| "Falhou".into());
                session.pending_permission = None;
                session.pending_question = None;
                for activity in session
                    .activities
                    .iter_mut()
                    .filter(|activity| activity.status == "running")
                {
                    activity.status = "failed".into();
                }
                None
            }
        };

        if !observed_files.is_empty() {
            remember_activity(
                session,
                SessionActivity {
                    id: format!("workspace:{}:{now}", target_session_id),
                    kind: "file".into(),
                    title: if observed_files.len() == 1 {
                        observed_files[0].clone()
                    } else {
                        format!("{} arquivos alterados", observed_files.len())
                    },
                    detail: None,
                    status: if matches!(&event.event, HookEventKind::Failed) {
                        "failed".into()
                    } else {
                        "completed".into()
                    },
                    created_at: now,
                    files: observed_files.clone(),
                    attachments: Vec::new(),
                    append_detail: false,
                },
            );
        }
        if matches!(
            &event.event,
            HookEventKind::Completed | HookEventKind::SessionEnded
        ) {
            remember_result(session, now, &observed_files);
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
        if let Some(question_id) = superseded_question_id {
            let (answers, changed) = &*self.question_answers;
            answers
                .lock()
                .map_err(|_| "Não foi possível liberar a pergunta antiga".to_string())?
                .entry(question_id)
                .or_default();
            changed.notify_all();
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

    fn workspace_changes(
        &self,
        session_id: &str,
        working_directory: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let baseline = self
            .workspace_snapshots
            .lock()
            .map_err(|_| "Não foi possível concluir o rastreio de arquivos".to_string())?
            .remove(session_id);
        let Some(baseline) = baseline else {
            return Ok(Vec::new());
        };
        let Some(current) = workspace_snapshot(working_directory) else {
            return Ok(Vec::new());
        };
        if current.root != baseline.root {
            return Ok(Vec::new());
        }
        let mut changed = current
            .files
            .iter()
            .filter(|(path, fingerprint)| baseline.files.get(*path) != Some(*fingerprint))
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        changed.extend(
            baseline
                .files
                .keys()
                .filter(|path| !current.files.contains_key(*path))
                .cloned(),
        );
        if baseline.head != current.head {
            if let (Some(before), Some(after)) = (baseline.head.as_deref(), current.head.as_deref())
            {
                changed.extend(git_paths(
                    &current.root,
                    &["diff", "--name-only", "-z", before, after],
                ));
            }
        }
        changed.sort();
        changed.dedup();
        changed.truncate(64);
        Ok(changed)
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
        if let Some(activity) = session
            .activities
            .iter_mut()
            .find(|activity| activity.id == format!("permission:{permission_id}"))
        {
            activity.status = if action == PermissionAction::Deny {
                "failed".into()
            } else {
                "completed".into()
            };
        }

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

    pub fn resolve_question(
        &self,
        session_id: &str,
        request_id: &str,
        answers: Vec<QuestionAnswer>,
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
            .pending_question
            .as_ref()
            .ok_or_else(|| "A sessão não possui uma pergunta pendente".to_string())?;
        if pending.id != request_id {
            return Err("A pergunta não corresponde à sessão".into());
        }
        for question in &pending.questions {
            let answer = answers
                .iter()
                .find(|answer| answer.question_id == question.id)
                .ok_or_else(|| format!("Responda à pergunta \"{}\"", question.header))?;
            if answer.answers.iter().all(|answer| answer.trim().is_empty()) {
                return Err(format!(
                    "A resposta para \"{}\" está vazia",
                    question.header
                ));
            }
        }
        let answer_summary = pending
            .questions
            .iter()
            .filter_map(|question| {
                let answer = answers
                    .iter()
                    .find(|answer| answer.question_id == question.id)?;
                let value = if question.is_secret {
                    "••••••".into()
                } else {
                    answer.answers.join(", ")
                };
                Some(if pending.questions.len() == 1 {
                    value
                } else {
                    format!("{}: {value}", question.header)
                })
            })
            .collect::<Vec<_>>()
            .join("\n");

        if let Some(activity) = session
            .activities
            .iter_mut()
            .find(|activity| activity.id == format!("question:{request_id}"))
        {
            activity.status = "completed".into();
        }
        remember_activity(
            session,
            SessionActivity {
                id: format!("question-answer:{request_id}:{}", now_millis()),
                kind: "prompt".into(),
                title: "Resposta enviada".into(),
                detail: Some(answer_summary),
                status: "completed".into(),
                created_at: now_millis(),
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            },
        );
        session.status = SessionStatus::Running;
        session.status_label = "Continuando a tarefa".into();
        session.pending_question = None;
        session.updated_at = now_millis();
        let snapshot = session.clone();
        drop(sessions);

        self.store
            .lock()
            .map_err(|_| "Não foi possível salvar a resposta".to_string())?
            .save_session(&snapshot)?;
        let (values, changed) = &*self.question_answers;
        values
            .lock()
            .map_err(|_| "Não foi possível entregar a resposta".to_string())?
            .insert(request_id.into(), answers);
        changed.notify_all();
        Ok(())
    }

    pub fn wait_for_question_answer(
        &self,
        request_id: &str,
        timeout: Duration,
    ) -> Result<Option<Vec<QuestionAnswer>>, String> {
        let (answers, changed) = &*self.question_answers;
        let values = answers
            .lock()
            .map_err(|_| "Não foi possível aguardar a resposta".to_string())?;
        let (mut values, _) = changed
            .wait_timeout_while(values, timeout, |values| !values.contains_key(request_id))
            .map_err(|_| "Não foi possível aguardar a resposta".to_string())?;
        Ok(values.remove(request_id))
    }

    pub fn expire_question(&self, request_id: &str) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Não foi possível acessar as sessões".to_string())?;
        let Some(session) = sessions.iter_mut().find(|session| {
            session
                .pending_question
                .as_ref()
                .is_some_and(|question| question.id == request_id)
        }) else {
            return Ok(());
        };
        session.pending_question = None;
        session.status = SessionStatus::WaitingForInput;
        session.status_label = "Esperando ação".into();
        session.updated_at = now_millis();
        if let Some(activity) = session
            .activities
            .iter_mut()
            .find(|activity| activity.id == format!("question:{request_id}"))
        {
            activity.status = "failed".into();
        }
        let snapshot = session.clone();
        drop(sessions);
        self.store
            .lock()
            .map_err(|_| "Não foi possível salvar a sessão".to_string())?
            .save_session(&snapshot)
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
        let recovered_identities = recover_process_identities(&discovered, &sessions);
        let recovered_work = recovered_identities
            .iter()
            .filter(|(process_id, (native_id, _))| {
                !sessions.iter().any(|session| {
                    session.process_id == Some(**process_id)
                        || (session.native_session_id.as_deref() == Some(native_id.as_str())
                            && !session.activities.is_empty())
                })
            })
            .map(|(process_id, (native_id, _))| {
                let kind = discovered
                    .iter()
                    .find(|process| {
                        process.process_id == *process_id
                            && process.native_session_ids.contains(native_id)
                    })
                    .and_then(|process| integration_kind_for_agent(&process.agent));
                let activities = kind
                    .as_ref()
                    .map(|kind| integrations::resume_work_activities(kind, native_id))
                    .unwrap_or_default();
                (*process_id, activities)
            })
            .collect::<HashMap<_, _>>();
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
            let exact_native_chat_ids = sessions
                .iter()
                .filter(|session| {
                    session
                        .native_session_id
                        .as_ref()
                        .is_some_and(|native_id| process.native_session_ids.contains(native_id))
                })
                .map(|session| session.id.clone())
                .collect::<Vec<_>>();
            if !exact_native_chat_ids.is_empty() {
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
                    .filter(|session| exact_native_chat_ids.contains(&session.id))
                {
                    let mut refreshed = false;
                    if let Some(identity) = recovered_identities.get(&process.process_id) {
                        refreshed |= apply_recovered_identity(session, identity);
                    }
                    if session.process_id != Some(process.process_id) {
                        session.process_id = Some(process.process_id);
                        refreshed = true;
                    }
                    if session.source != process.source {
                        session.source = process.source.clone();
                        refreshed = true;
                    }
                    if session.activities.is_empty() {
                        if let Some(activities) = recovered_work.get(&process.process_id) {
                            if !activities.is_empty() {
                                session.activities.clone_from(activities);
                                refreshed = true;
                            }
                        }
                    }
                    if refreshed {
                        session.updated_at = now;
                        snapshots.push(session.clone());
                        changed = true;
                    }
                }
                continue;
            }
            let has_recent_native_vscode_chat = process.source == SessionSource::Vscode
                && sessions.iter().any(|session| {
                    !is_provisional_process(session)
                        && session_matches_process(session, &process)
                        && session.source == SessionSource::Vscode
                        && session_can_own_process(session, now, &active_pids)
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
            let exact_contextual_chat_ids = process
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
                                && session_can_own_process(session, now, &active_pids)
                                && session.process_id.is_none_or(|pid| {
                                    pid == process.process_id || !active_pids.contains(&pid)
                                })
                        })
                        .map(|session| session.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let contextual_chat_ids = if exact_contextual_chat_ids.is_empty() {
                let fallback = sessions
                    .iter()
                    .filter(|session| !is_provisional_process(session))
                    .filter(|session| {
                        session_matches_process(session, &process)
                            && session.source == process.source
                            && session_can_own_process(session, now, &active_pids)
                            && session.process_id.is_none_or(|pid| {
                                pid == process.process_id || !active_pids.contains(&pid)
                            })
                            && (is_home_directory(process.working_directory.as_deref())
                                || is_home_directory(session.working_directory.as_deref()))
                    })
                    .map(|session| session.id.clone())
                    .collect::<Vec<_>>();
                if fallback.len() == 1 {
                    fallback
                } else {
                    Vec::new()
                }
            } else {
                exact_contextual_chat_ids
            };
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
                    if is_provisional_process(session) {
                        session.project = process
                            .working_directory
                            .as_deref()
                            .and_then(|path| Path::new(path).file_name())
                            .and_then(|name| name.to_str())
                            .unwrap_or("Sessão local")
                            .to_string();
                    }
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
                if let Some(identity) = recovered_identities.get(&process.process_id) {
                    refreshed |= apply_recovered_identity(session, identity);
                }
                if session.activities.is_empty() {
                    if let Some(activities) = recovered_work.get(&process.process_id) {
                        if !activities.is_empty() {
                            session.activities.clone_from(activities);
                            refreshed = true;
                        }
                    }
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
                    && session
                        .process_id
                        .is_none_or(|pid| pid == process.process_id || !active_pids.contains(&pid))
                    && same_directory(
                        session.working_directory.as_deref(),
                        process.working_directory.as_deref(),
                    )
            }) {
                session.process_id = Some(process.process_id);
                if let Some((native_id, name)) = recovered_identities.get(&process.process_id) {
                    session.native_session_id = Some(native_id.clone());
                    session.session_name = name.clone();
                }
                if let Some(activities) = recovered_work.get(&process.process_id) {
                    session.activities.clone_from(activities);
                }
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
            let recovered_identity = recovered_identities.get(&process.process_id);
            let session = AgentSession {
                id: format!(
                    "process:{}:{}",
                    agent_name.to_lowercase(),
                    process.process_id
                ),
                agent: process.agent.clone(),
                agent_label: agent_name,
                session_name: recovered_identity
                    .map(|(_, name)| name.clone())
                    .unwrap_or_default(),
                project,
                source: process.source,
                source_app: None,
                control_origin: SessionControlOrigin::External,
                status: SessionStatus::WaitingForInput,
                status_label: "Esperando ação".into(),
                started_at: now.to_string(),
                updated_at: now,
                process_id: Some(process.process_id),
                native_session_id: recovered_identity.map(|(id, _)| id.clone()),
                working_directory: process.working_directory,
                permission_profile: default_profile(&process.agent),
                pending_permission: None,
                pending_question: None,
                last_response: None,
                results: Vec::new(),
                activities: recovered_work
                    .get(&process.process_id)
                    .cloned()
                    .unwrap_or_default(),
                rate_limits: Vec::new(),
            };
            snapshots.push(session.clone());
            sessions.push(session);
            changed = true;
        }

        let process_is_present = |session: &AgentSession| {
            session.process_id.is_some_and(|pid| {
                if is_provisional_process(session) && session.source == SessionSource::Vscode {
                    active_pids.contains(&pid)
                } else {
                    live_pids.contains(&pid)
                }
            })
        };
        for session in sessions
            .iter()
            .filter(|session| process_is_present(session))
        {
            missing_process_scans.remove(&session.id);
        }
        missing_process_scans
            .retain(|session_id, _| sessions.iter().any(|session| &session.id == session_id));

        let mut closed_session_ids = Vec::new();
        for session in sessions
            .iter()
            .filter(|session| session.process_id.is_some() && !process_is_present(session))
        {
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
        session_name: event.session_name.clone().unwrap_or_default(),
        project,
        source: event.source.clone().unwrap_or(SessionSource::Cli),
        source_app: event.source_app.clone(),
        control_origin: event.control_origin.clone(),
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
        pending_question: None,
        last_response: event.last_response.clone(),
        results: Vec::new(),
        activities: Vec::new(),
        rate_limits: Vec::new(),
    }
}

fn remember_activity(session: &mut AgentSession, mut activity: SessionActivity) {
    if activity.kind == "prompt" {
        normalize_lume_prompt_activity(&mut activity);
        let incoming_key = activity
            .detail
            .as_deref()
            .map(normalized_chat_text)
            .unwrap_or_default();
        let duplicate = session.activities.iter().rposition(|existing| {
            existing.kind == "prompt"
                && !(existing.id.starts_with("local:") && activity.id.starts_with("local:"))
                && existing
                    .detail
                    .as_deref()
                    .map(normalized_chat_text)
                    .unwrap_or_default()
                    == incoming_key
                && (existing.created_at - activity.created_at).abs() < 60_000
        });
        if let Some(index) = duplicate.filter(|index| {
            !session.activities[index.saturating_add(1)..]
                .iter()
                .any(|existing| matches!(existing.kind.as_str(), "prompt" | "message"))
        }) {
            let existing = &mut session.activities[index];
            existing.status = activity.status;
            merge_attachments(&mut existing.attachments, activity.attachments);
            return;
        }
    }
    if activity.kind == "message" {
        let discovered = response_attachments(
            activity.detail.as_deref().unwrap_or_default(),
            session.working_directory.as_deref(),
        );
        merge_attachments(&mut activity.attachments, discovered);
    }
    if let Some(existing) = session
        .activities
        .iter_mut()
        .find(|existing| existing.id == activity.id)
    {
        if activity.append_detail {
            if let Some(delta) = activity.detail.take() {
                let detail = existing.detail.get_or_insert_with(String::new);
                detail.push_str(&delta);
                *detail = detail.chars().take(32 * 1024).collect();
            }
            existing.status = activity.status;
            for file in activity.files {
                if !existing.files.contains(&file) {
                    existing.files.push(file);
                }
            }
            merge_attachments(&mut existing.attachments, activity.attachments);
            return;
        }
        activity.created_at = existing.created_at;
        *existing = activity;
        return;
    }
    if activity.kind == "message" {
        let activity_detail = activity.detail.clone();
        let duplicate = session.activities.iter().rposition(|existing| {
            existing.kind == "message"
                && match (existing.detail.as_deref(), activity_detail.as_deref()) {
                    (Some(existing), Some(incoming)) => same_chat_response(existing, incoming),
                    (None, None) => true,
                    _ => false,
                }
        });
        if let Some(index) = duplicate.filter(|index| {
            !session.activities[index.saturating_add(1)..]
                .iter()
                .any(|existing| existing.kind == "prompt")
        }) {
            if let Some(incoming) = activity.detail.take() {
                let should_replace =
                    session.activities[index]
                        .detail
                        .as_deref()
                        .map_or(true, |existing| {
                            normalized_chat_text(&incoming).len()
                                > normalized_chat_text(existing).len()
                        });
                if should_replace {
                    session.activities[index].detail = Some(incoming);
                }
            }
            session.activities[index].status = activity.status;
            for file in activity.files {
                if !session.activities[index].files.contains(&file) {
                    session.activities[index].files.push(file);
                }
            }
            merge_attachments(
                &mut session.activities[index].attachments,
                activity.attachments,
            );
            return;
        }
    }
    session.activities.push(activity);
    session
        .activities
        .sort_by_key(|activity| activity.created_at);
    prune_transient_activities(&mut session.activities, 160);
}

fn prune_transient_activities(activities: &mut Vec<SessionActivity>, limit: usize) {
    let transient_count = activities
        .iter()
        .filter(|activity| {
            !matches!(
                activity.kind.as_str(),
                "prompt" | "message" | "queued_prompt"
            )
        })
        .count();
    let mut remove = transient_count.saturating_sub(limit);
    if remove == 0 {
        return;
    }
    activities.retain(|activity| {
        let transient = !matches!(
            activity.kind.as_str(),
            "prompt" | "message" | "queued_prompt"
        );
        if transient && remove > 0 {
            remove -= 1;
            false
        } else {
            true
        }
    });
}

fn normalize_lume_prompt_activity(activity: &mut SessionActivity) {
    let Some(detail) = activity.detail.as_deref() else {
        return;
    };
    let normalized = normalized_chat_text(detail);
    let Some((visible, transport)) = normalized.split_once(LUME_ATTACHED_FILES_MARKER) else {
        return;
    };
    let attachments = transport.lines().filter_map(|line| {
        let raw_path = line
            .trim()
            .strip_prefix("- ")?
            .trim()
            .trim_matches(['`', '"', '\'']);
        response_attachment(raw_path, None)
    });
    merge_attachments(&mut activity.attachments, attachments);
    activity.detail = (!visible.trim().is_empty()).then(|| visible.trim().to_string());
}

fn merge_attachments(
    current: &mut Vec<PromptAttachment>,
    incoming: impl IntoIterator<Item = PromptAttachment>,
) {
    for attachment in incoming {
        let duplicate = current.iter().any(|existing| {
            existing
                .path
                .as_deref()
                .zip(attachment.path.as_deref())
                .is_some_and(|(left, right)| same_directory(Some(left), Some(right)))
                || (existing.path.is_none()
                    && attachment.path.is_none()
                    && existing.name == attachment.name)
        });
        if !duplicate {
            current.push(attachment);
        }
    }
}

fn response_attachments(detail: &str, working_directory: Option<&str>) -> Vec<PromptAttachment> {
    let candidates = explicit_response_targets(detail);
    let mut attachments = Vec::new();
    for candidate in candidates {
        let Some(attachment) = response_attachment(&candidate, working_directory) else {
            continue;
        };
        merge_attachments(&mut attachments, [attachment]);
        if attachments.len() == 8 {
            break;
        }
    }
    attachments
}

fn explicit_response_targets(value: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in value.lines() {
        let mut remaining = line;
        while let Some(close) = remaining.find("](") {
            let before = &remaining[..close];
            let Some(open) = before.rfind('[') else {
                break;
            };
            let label = &before[open + 1..];
            let image = open > 0 && before[..open].ends_with('!');
            let target = &remaining[close + 2..];
            let Some(end) = target.find(')') else {
                break;
            };
            if image || has_response_delivery_cue(&format!("{} {label}", &before[..open])) {
                let raw = target[..end].trim();
                let raw = if let Some(raw) = raw.strip_prefix('<') {
                    raw.split_once('>').map(|(path, _)| path).unwrap_or(raw)
                } else {
                    raw.split_once(" \"")
                        .or_else(|| raw.split_once(" '"))
                        .map(|(path, _)| path)
                        .unwrap_or(raw)
                };
                push_response_target(&mut targets, raw);
            }
            remaining = &target[end + 1..];
        }
        if !has_response_delivery_cue(line) {
            continue;
        }
        let mut code = line;
        while let Some(start) = code.find('`') {
            let tail = &code[start + 1..];
            let Some(end) = tail.find('`') else {
                break;
            };
            push_response_target(&mut targets, &tail[..end]);
            code = &tail[end + 1..];
        }
        if !line.contains("](") {
            if let Some((_, tail)) = line.split_once(':') {
                push_response_target(&mut targets, tail);
            }
        }
    }
    targets
}

fn has_response_delivery_cue(value: &str) -> bool {
    let value = value.to_lowercase();
    [
        "download",
        "baixar",
        "baixe",
        "arquivo final",
        "arquivo gerado",
        "ficheiro final",
        "final file",
        "final pdf",
        "final image",
        "final document",
        "generated file",
        "generated pdf",
        "generated image",
        "pdf final",
        "output file",
        "deliverable",
        "attachment",
        "anexo",
        "exported",
        "exportado",
        "saved at",
        "saved to",
        "salvo em",
        "salva em",
        "available for download",
        "disponível para baixar",
        "resultado final",
        "entrega final",
    ]
    .iter()
    .any(|cue| value.contains(cue))
}

fn push_response_target(targets: &mut Vec<String>, raw: &str) {
    let raw = raw
        .trim()
        .trim_matches(['`', '"', '\'', '<', '>', '.', ',', ';'])
        .replace("%20", " ");
    if !raw.is_empty() && !targets.iter().any(|target| target == &raw) {
        targets.push(raw);
    }
}

fn response_attachment(
    raw_path: &str,
    working_directory: Option<&str>,
) -> Option<PromptAttachment> {
    let raw_path = raw_path
        .trim()
        .trim_matches(['`', '"', '\'', '<', '>'])
        .strip_prefix("file://")
        .unwrap_or(raw_path.trim().trim_matches(['`', '"', '\'', '<', '>']));
    if raw_path.is_empty()
        || raw_path.contains(['\n', '\r', '\0'])
        || raw_path.starts_with("http://")
        || raw_path.starts_with("https://")
    {
        return None;
    }
    let raw_path = strip_path_line_number(raw_path);
    let candidate = Path::new(raw_path);
    let candidate = if candidate.is_absolute() || looks_like_windows_path(raw_path) {
        candidate.to_path_buf()
    } else {
        Path::new(working_directory?).join(candidate)
    };
    let canonical = fs::canonicalize(candidate).ok()?;
    if !canonical.is_file() || sensitive_response_path(&canonical) {
        return None;
    }
    let name = canonical.file_name()?.to_string_lossy().to_string();
    let path = canonical.to_string_lossy().to_string();
    let mime_type = response_file_mime(&canonical).to_string();
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    Some(PromptAttachment {
        id: format!("response-file:{:x}", hasher.finish()),
        name,
        mime_type,
        preview_data_url: String::new(),
        path: Some(path),
    })
}

fn strip_path_line_number(path: &str) -> &str {
    let Some((candidate, suffix)) = path.rsplit_once(':') else {
        return path;
    };
    if suffix.chars().all(|character| character.is_ascii_digit())
        && !(candidate.len() == 1 && candidate.as_bytes()[0].is_ascii_alphabetic())
    {
        candidate
    } else {
        path
    }
}

fn looks_like_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn sensitive_response_path(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        value == ".ssh" || value == ".gnupg" || value == ".aws"
    }) || path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy().to_ascii_lowercase();
        name == ".env"
            || name.starts_with(".env.")
            || matches!(
                name.as_str(),
                "id_rsa" | "id_ed25519" | ".netrc" | ".npmrc" | ".pypirc"
            )
            || [".pem", ".key", ".p12", ".pfx"]
                .iter()
                .any(|extension| name.ends_with(extension))
    })
}

fn response_file_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" => "text/html",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn remove_superseded_streamed_messages(session: &mut AgentSession) {
    let latest_prompt_at = session
        .activities
        .iter()
        .filter(|activity| activity.kind == "prompt")
        .map(|activity| activity.created_at)
        .max()
        .unwrap_or(i64::MIN);
    session.activities.retain(|activity| {
        if activity.kind != "message"
            || activity.created_at < latest_prompt_at
            || !activity.id.starts_with("codex:")
        {
            return true;
        }
        activity.status != "running"
    });
}

fn normalized_chat_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn comparable_chat_response(value: &str) -> String {
    let normalized = normalized_chat_text(value);
    normalized
        .strip_suffix('…')
        .or_else(|| normalized.strip_suffix("..."))
        .unwrap_or(&normalized)
        .trim_end()
        .to_string()
}

fn same_chat_response(left: &str, right: &str) -> bool {
    let left = comparable_chat_response(left);
    let right = comparable_chat_response(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    let (shorter, longer) = if left.len() <= right.len() {
        (&left, &right)
    } else {
        (&right, &left)
    };
    shorter.len() >= 256 && longer.starts_with(shorter)
}

fn same_permission_request(current: &PermissionRequest, incoming: &PermissionRequest) -> bool {
    current.kind == incoming.kind
        && current.summary == incoming.summary
        && current.resource == incoming.resource
        && current.risk == incoming.risk
}

fn has_prompt_between(session: &AgentSession, left: i64, right: i64) -> bool {
    let start = left.min(right);
    let end = left.max(right);
    session.activities.iter().any(|activity| {
        activity.kind == "prompt" && activity.created_at > start && activity.created_at <= end
    })
}

fn equivalent_result_in_same_turn(
    session: &AgentSession,
    existing: &SessionResult,
    response: &str,
    created_at: i64,
) -> bool {
    same_chat_response(&existing.response, response)
        && !has_prompt_between(session, existing.created_at, created_at)
}

fn remember_result(session: &mut AgentSession, now: i64, observed_files: &[String]) {
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
        .iter()
        .any(|result| equivalent_result_in_same_turn(session, result, response, now))
    {
        return;
    }
    let (mut files, tests) = extract_result_artifacts(response);
    for file in observed_files {
        if !files.contains(file) {
            files.push(file.clone());
        }
    }
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

fn workspace_snapshot(working_directory: Option<&str>) -> Option<WorkspaceSnapshot> {
    let working_directory = Path::new(working_directory?);
    let root = git_text(working_directory, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)?;
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    let mut paths = git_paths(&root, &["diff", "--name-only", "-z"]);
    paths.extend(git_paths(&root, &["diff", "--cached", "--name-only", "-z"]));
    paths.extend(git_paths(
        &root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    ));
    paths.sort();
    paths.dedup();
    let files = paths
        .into_iter()
        .map(|path| {
            let fingerprint = file_fingerprint(&root.join(&path));
            (path, fingerprint)
        })
        .collect();
    Some(WorkspaceSnapshot { root, head, files })
}

fn git_text(root: &Path, args: &[&str]) -> Option<String> {
    let output = git_output(root, args)?;
    let value = String::from_utf8_lossy(&output).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn git_paths(root: &Path, args: &[&str]) -> Vec<String> {
    git_output(root, args)
        .map(|output| {
            output
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
                .map(|path| String::from_utf8_lossy(path).to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn git_output(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut command = crate::executables::command("git").ok()?;
    let output = command.arg("-C").arg(root).args(args).output().ok()?;
    output.status.success().then_some(output.stdout)
}

fn file_fingerprint(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    if let Ok(metadata) = fs::metadata(path) {
        metadata.len().hash(&mut hasher);
        metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .hash(&mut hasher);
        if metadata.len() <= 8 * 1024 * 1024 {
            fs::read(path).ok().hash(&mut hasher);
        }
    } else {
        "missing".hash(&mut hasher);
    }
    hasher.finish()
}

pub(crate) fn extract_result_artifacts(response: &str) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut tests = Vec::new();
    const FILE_EXTENSIONS: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "svelte", "json", "toml", "yaml", "yml", "md", "css",
        "scss", "html", "py", "go", "java", "cs", "sh", "sql", "txt",
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
            if let Some(file) = reported_file_candidate(token, FILE_EXTENSIONS) {
                if !files.contains(&file) {
                    files.push(file);
                }
            }
        }
    }
    files.truncate(24);
    tests.truncate(12);
    (files, tests)
}

fn reported_file_candidate(token: &str, extensions: &[&str]) -> Option<String> {
    let token =
        token.trim_matches(|character: char| matches!(character, '`' | '"' | '\'' | ',' | ';'));
    let candidate = if let Some(link_start) = token.find("](") {
        let target = &token[link_start + 2..];
        let target_end = target.find(')')?;
        &target[..target_end]
    } else {
        token
    };
    let mut candidate = candidate
        .trim_matches(|character: char| matches!(character, '(' | ')' | '[' | ']' | '`'))
        .trim_end_matches(['.', '`']);
    if candidate.starts_with("http://")
        || candidate.starts_with("https://")
        || candidate.contains(['\n', '\r', '\0'])
        || candidate.contains("***")
        || candidate.contains("](")
    {
        return None;
    }
    if let Some((path, suffix)) = candidate.rsplit_once(':') {
        if !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit()) {
            candidate = path;
        }
    }
    let extension = Path::new(candidate)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    if !extensions.contains(&extension.as_str()) {
        return None;
    }
    let sanitized = if Path::new(candidate).is_absolute() {
        Path::new(candidate).file_name()?.to_str()?.to_string()
    } else {
        candidate.trim_start_matches("./").to_string()
    };
    (!sanitized.is_empty()).then_some(sanitized)
}

fn merge_results(target: &mut AgentSession, source: &AgentSession) {
    merge_permission_scope(&mut target.permission_profile, &source.permission_profile);
    for activity in &source.activities {
        remember_activity(target, activity.clone());
    }
    for result in &source.results {
        if !target.results.iter().any(|existing| {
            existing.id == result.id
                || equivalent_result_in_same_turn(
                    target,
                    existing,
                    &result.response,
                    result.created_at,
                )
        }) {
            target.results.push(result.clone());
        }
    }
    target.results.sort_by_key(|result| result.created_at);
    if target.results.len() > 12 {
        target.results.drain(..target.results.len() - 12);
    }
}

fn merge_permission_scope(target: &mut PermissionProfile, source: &PermissionProfile) {
    if permission_scope_is_explicit(source) && !permission_scope_is_explicit(target) {
        copy_permission_scope(target, source);
    }
    if source.can_respond_from_lume && !target.can_respond_from_lume {
        target.can_respond_from_lume = true;
        target
            .available_actions
            .clone_from(&source.available_actions);
    }
}

fn permission_scope_is_explicit(profile: &PermissionProfile) -> bool {
    profile.mode != AccessMode::Custom
        || profile.approvals_reviewer.is_some()
        || matches!(
            profile.approval_policy.trim().to_ascii_lowercase().as_str(),
            "never" | "on-request" | "on-failure" | "untrusted" | "unless-trusted"
        )
}

fn copy_permission_scope(target: &mut PermissionProfile, source: &PermissionProfile) {
    target.mode = source.mode.clone();
    target.label = source.label.clone();
    target.approval_policy = source.approval_policy.clone();
    target.approvals_reviewer = source.approvals_reviewer.clone();
}

fn apply_permission_profile(target: &mut PermissionProfile, source: &PermissionProfile) {
    if permission_scope_is_explicit(source) || !permission_scope_is_explicit(target) {
        copy_permission_scope(target, source);
    }
    if source.can_respond_from_lume || !target.can_respond_from_lume {
        target.can_respond_from_lume = source.can_respond_from_lume;
        target
            .available_actions
            .clone_from(&source.available_actions);
    }
}

fn apply_metadata(session: &mut AgentSession, event: &HookEvent) {
    if let Some(label) = &event.agent_label {
        session.agent_label = label.clone();
    }
    if let Some(name) = event
        .session_name
        .as_deref()
        .and_then(normalized_session_name)
    {
        session.session_name = name;
    }
    if let Some(project) = &event.project {
        session.project = project.clone();
    }
    let keeps_bound_process_source = session.agent == AgentKind::Codex
        && session.process_id.is_some()
        && event.process_id.is_none()
        && event.native_session_id.is_some();
    if !keeps_bound_process_source {
        if let Some(source) = &event.source {
            session.source = source.clone();
        }
    }
    if let Some(source_app) = &event.source_app {
        session.source_app = Some(source_app.clone());
    }
    if event.control_origin == SessionControlOrigin::Lume {
        session.control_origin = SessionControlOrigin::Lume;
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
        apply_permission_profile(&mut session.permission_profile, profile);
    }
    if let Some(response) = &event.last_response {
        session.last_response = Some(response.clone());
    }
}

fn default_profile(agent: &AgentKind) -> PermissionProfile {
    match agent {
        AgentKind::ClaudeCode => PermissionProfile {
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

fn recover_process_identities(
    discovered: &[DiscoveredProcess],
    sessions: &[AgentSession],
) -> HashMap<u32, (String, String)> {
    let unresolved = discovered
        .iter()
        .filter(|process| {
            sessions
                .iter()
                .find(|session| session.process_id == Some(process.process_id))
                .is_none_or(session_needs_recovered_identity)
        })
        .collect::<Vec<_>>();
    if unresolved.is_empty() {
        return HashMap::new();
    }

    let mut indexed_names = unresolved
        .iter()
        .filter_map(|process| integration_kind_for_agent(&process.agent))
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|kind| {
            let names = integrations::indexed_session_names(&kind).unwrap_or_default();
            (kind, names)
        })
        .collect::<HashMap<_, _>>();
    for process in &unresolved {
        let Some(kind) = integration_kind_for_agent(&process.agent) else {
            continue;
        };
        let Some(names) = indexed_names.get_mut(&kind) else {
            continue;
        };
        for native_id in &process.native_session_ids {
            if names.contains_key(native_id) {
                continue;
            }
            if let Some(title) = integrations::native_session_title(&kind, native_id) {
                names.insert(native_id.clone(), title);
            }
        }
    }

    recover_process_identities_from_index(unresolved, sessions, &indexed_names)
}

fn recover_process_identities_from_index(
    unresolved: Vec<&DiscoveredProcess>,
    sessions: &[AgentSession],
    indexed_names: &HashMap<IntegrationKind, HashMap<String, String>>,
) -> HashMap<u32, (String, String)> {
    unresolved
        .into_iter()
        .filter_map(|process| {
            let kind = integration_kind_for_agent(&process.agent)?;
            let native_id = process.native_session_ids.first()?.clone();
            let name = indexed_names
                .get(&kind)
                .and_then(|names| names.get(&native_id))
                .cloned()
                .or_else(|| {
                    sessions
                        .iter()
                        .find(|session| {
                            session.process_id == Some(process.process_id)
                                && session.native_session_id.as_deref() == Some(&native_id)
                        })
                        .map(|session| session.session_name.trim().to_string())
                        .filter(|name| !name.is_empty())
                })
                .or_else(|| {
                    process
                        .working_directory
                        .as_deref()
                        .and_then(|directory| Path::new(directory).file_name())
                        .and_then(|name| name.to_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| process.agent_label.clone());
            Some((process.process_id, (native_id, name)))
        })
        .collect()
}

fn session_needs_recovered_identity(session: &AgentSession) -> bool {
    session.native_session_id.is_none()
        || session.session_name.trim().is_empty()
        || (session.agent == AgentKind::Codex
            && normalized_session_name(&session.session_name)
                == normalized_session_name(&session.project))
}

fn apply_recovered_identity(
    session: &mut AgentSession,
    (native_id, name): &(String, String),
) -> bool {
    let mut changed = false;
    if session.native_session_id.as_deref() != Some(native_id.as_str()) {
        session.native_session_id = Some(native_id.clone());
        changed = true;
    }
    if session.session_name != *name {
        session.session_name = name.clone();
        changed = true;
    }
    changed
}

fn integration_kind_for_agent(agent: &AgentKind) -> Option<IntegrationKind> {
    match agent {
        AgentKind::Codex => Some(IntegrationKind::Codex),
        AgentKind::ClaudeCode => Some(IntegrationKind::Claude),
        AgentKind::Antigravity => Some(IntegrationKind::Antigravity),
        AgentKind::DeepSeek => Some(IntegrationKind::DeepSeek),
        AgentKind::Gemini => Some(IntegrationKind::Gemini),
        AgentKind::ChatGpt | AgentKind::Claude | AgentKind::Unknown => None,
    }
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
        && left.process_id.is_some()
        && left.process_id == right.process_id
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

fn session_can_own_process(
    session: &AgentSession,
    now: i64,
    active_pids: &std::collections::HashSet<u32>,
) -> bool {
    match session.status {
        SessionStatus::Running
        | SessionStatus::PermissionRequired
        | SessionStatus::WaitingForInput => {
            if let Some(process_id) = session.process_id {
                active_pids.contains(&process_id) || session.source == SessionSource::Cli
            } else if session.agent == AgentKind::Codex && session.source == SessionSource::Cli {
                now.saturating_sub(session.updated_at) < PIDLESS_CODEX_SESSION_TTL_MS
            } else {
                true
            }
        }
        _ => now.saturating_sub(session.updated_at) < RECENT_NATIVE_SESSION_MS,
    }
}

fn is_home_directory(directory: Option<&str>) -> bool {
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
        return false;
    };
    same_directory(directory, home.to_str())
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
                && existing.process_id == process.process_id
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
        AgentKind::ChatGpt => "ChatGPT",
        AgentKind::Claude => "Claude",
        AgentKind::ClaudeCode => "Claude Code",
        AgentKind::Antigravity => "Antigravity",
        AgentKind::DeepSeek => "DeepSeek",
        AgentKind::Gemini => "Gemini",
        AgentKind::Unknown => "Agente",
    }
}

fn ensure_unique_session_names(sessions: &mut [AgentSession]) {
    let mut used = HashSet::new();
    for session in sessions {
        let base = normalized_session_name(&session.session_name)
            .unwrap_or_else(|| default_session_name(session));
        let unique = unique_session_name(&base, &used);
        used.insert(unique.to_lowercase());
        session.session_name = unique;
    }
}

fn apply_session_aliases(sessions: &mut [AgentSession], aliases: &HashMap<String, String>) {
    for session in sessions {
        let Some(alias) = persistent_session_alias_key(session)
            .and_then(|key| aliases.get(&key))
            .and_then(|alias| normalized_session_name(alias))
        else {
            continue;
        };
        session.session_name = alias;
    }
}

fn persistent_session_alias_key(session: &AgentSession) -> Option<String> {
    let native_id = session.native_session_id.as_deref()?.trim();
    if native_id.is_empty() {
        return None;
    }
    let agent = match session.agent {
        AgentKind::Codex => return None,
        AgentKind::ChatGpt => "chatgpt".to_string(),
        AgentKind::Claude => "claude".to_string(),
        AgentKind::ClaudeCode => "claude_code".to_string(),
        AgentKind::Antigravity => "antigravity".to_string(),
        AgentKind::DeepSeek => "deepseek".to_string(),
        AgentKind::Gemini => "gemini".to_string(),
        AgentKind::Unknown => format!("unknown:{}", session.agent_label.to_lowercase()),
    };
    Some(format!("{agent}:{native_id}"))
}

fn default_session_name(session: &AgentSession) -> String {
    let agent = normalized_session_name(&session.agent_label)
        .unwrap_or_else(|| agent_label(&session.agent).to_string());
    let Some(project) = normalized_session_name(&session.project) else {
        return agent;
    };
    if project.eq_ignore_ascii_case("Sessão local")
        || project.eq_ignore_ascii_case("Sessão sem projeto")
    {
        agent
    } else {
        project
    }
}

fn normalized_session_name(name: &str) -> Option<String> {
    let normalized = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = normalized.chars().take(80).collect::<String>();
    (!normalized.is_empty()).then_some(normalized)
}

fn unique_session_name(base: &str, used: &HashSet<String>) -> String {
    if !used.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{base} ({suffix})");
        if !used.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!("há sempre um próximo sufixo numérico")
}

fn plan_note_title(content: &str, fallback: &str) -> String {
    content
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches('#')
                .trim_start_matches(['-', '*'])
                .trim()
        })
        .find(|line| !line.is_empty() && !line.starts_with('['))
        .map(|line| line.chars().take(80).collect())
        .unwrap_or_else(|| fallback.to_string())
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
            agent: AgentKind::ClaudeCode,
            agent_label: "Claude Code".into(),
            process_id,
            native_session_ids: Vec::new(),
            working_directory: Some("/work/lume".into()),
            source: SessionSource::Cli,
        }
    }

    fn started_event(session_id: &str, process_id: u32) -> HookEvent {
        HookEvent {
            event: HookEventKind::SessionStarted,
            session_id: session_id.into(),
            agent: AgentKind::ClaudeCode,
            agent_label: None,
            session_name: None,
            project: Some("lume".into()),
            source: Some(SessionSource::Cli),
            source_app: None,
            control_origin: SessionControlOrigin::External,
            status_label: Some("Sessão detectada".into()),
            started_at: None,
            process_id: Some(process_id),
            native_session_id: Some("native-session".into()),
            working_directory: Some("/work/lume".into()),
            permission_profile: None,
            permission: None,
            question: None,
            last_response: None,
            activity: None,
            activities: Vec::new(),
            wait_for_decision: false,
        }
    }

    #[test]
    fn codex_internal_memory_events_never_become_sessions() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("codex:memory-maintenance", 4242);
        event.agent = AgentKind::Codex;
        event.project = Some("memories".into());
        event.working_directory = Some("/home/user/.codex/memories".into());

        state.ingest(event).expect("evento interno ignorado");

        assert!(state.sessions().expect("sessões").is_empty());
    }

    #[test]
    fn activity_limit_never_discards_conversation_messages() {
        let mut session = session_from_event(&started_event("codex:long-chat", 4242), 1);
        for (index, kind) in ["prompt", "message", "prompt", "message"]
            .into_iter()
            .enumerate()
        {
            remember_activity(
                &mut session,
                SessionActivity {
                    id: format!("chat-{index}"),
                    kind: kind.into(),
                    title: kind.into(),
                    detail: Some(format!("conversation-{index}")),
                    status: "completed".into(),
                    created_at: index as i64,
                    files: Vec::new(),
                    attachments: Vec::new(),
                    append_detail: false,
                },
            );
        }
        for index in 0..220 {
            remember_activity(
                &mut session,
                SessionActivity {
                    id: format!("tool-{index}"),
                    kind: "tool".into(),
                    title: "Command".into(),
                    detail: Some(format!("command-{index}")),
                    status: "completed".into(),
                    created_at: 10 + index,
                    files: Vec::new(),
                    attachments: Vec::new(),
                    append_detail: false,
                },
            );
        }

        assert_eq!(
            session
                .activities
                .iter()
                .filter(|activity| matches!(activity.kind.as_str(), "prompt" | "message"))
                .count(),
            4
        );
        assert_eq!(
            session
                .activities
                .iter()
                .filter(|activity| activity.kind == "tool")
                .count(),
            160
        );
        assert!(session
            .activities
            .iter()
            .any(|activity| activity.id == "chat-0"));
    }

    #[test]
    fn native_thread_restores_archived_conversation_without_restoring_closed_agent() {
        let path = std::env::temp_dir().join(format!(
            "lume-conversation-archive-{}-{}.db",
            std::process::id(),
            now_millis()
        ));
        {
            let state = AppState::new(&path).expect("primeiro estado");
            state
                .ingest(started_event("codex:archive", 4242))
                .expect("inicia sessão");
            let mut response = started_event("codex:archive", 4242);
            response.event = HookEventKind::Activity;
            response.activity = Some(SessionActivity {
                id: "archived-message".into(),
                kind: "message".into(),
                title: "Codex".into(),
                detail: Some("mensagem histórica".into()),
                status: "completed".into(),
                created_at: now_millis(),
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            });
            state.ingest(response).expect("arquiva resposta");
        }
        {
            let state = AppState::new(&path).expect("segundo estado");
            assert!(state
                .sessions()
                .expect("sem agentes restaurados")
                .is_empty());
            state
                .ingest(started_event("codex:redetected", 5252))
                .expect("redetecta thread");
            let session = state.sessions().expect("sessão reaberta").remove(0);
            assert!(session
                .activities
                .iter()
                .any(|activity| activity.id == "archived-message"));
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn codex_session_can_rebind_an_ephemeral_thread() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("codex:ephemeral", 4242);
        event.agent = AgentKind::Codex;
        event.native_session_id = Some("old-thread".into());
        state.ingest(event).expect("sessão");

        state
            .rebind_codex_thread("codex:ephemeral", "new-thread".into())
            .expect("reatar thread");

        assert_eq!(
            state.sessions().expect("sessões")[0]
                .native_session_id
                .as_deref(),
            Some("new-thread")
        );
    }

    #[test]
    fn transferred_session_becomes_promptable_without_losing_its_thread() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("codex:external", 4242);
        event.agent = AgentKind::Codex;
        event.source = Some(SessionSource::Vscode);
        event.native_session_id = Some("thread-external".into());
        state.ingest(event).expect("sessão externa");

        state
            .mark_session_lume_controlled("codex:external")
            .expect("transferência");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.control_origin, SessionControlOrigin::Lume);
        assert_eq!(session.source, SessionSource::Cli);
        assert_eq!(
            session.native_session_id.as_deref(),
            Some("thread-external")
        );
        assert_eq!(session.status, SessionStatus::WaitingForInput);
        assert!(session.process_id.is_none());
        assert!(session.permission_profile.can_respond_from_lume);
    }

    #[test]
    fn session_names_are_unique_per_native_thread() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let first = started_event("claude:first", 4101);
        let mut second = started_event("claude:second", 4102);
        second.native_session_id = Some("native-session-2".into());
        state.ingest(first).expect("primeira sessão");
        state.ingest(second).expect("segunda sessão");

        let names = state
            .sessions()
            .expect("sessões")
            .into_iter()
            .map(|session| session.session_name)
            .collect::<HashSet<_>>();
        assert_eq!(names, HashSet::from(["lume".into(), "lume (2)".into()]));
    }

    #[test]
    fn codex_project_fallback_is_replaced_by_the_native_thread_name() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("codex:thread-1", 4103);
        event.agent = AgentKind::Codex;
        event.project = Some("user".into());
        event.native_session_id = Some("thread-1".into());
        state.ingest(event).expect("sessão");

        let mut session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.session_name, "user");
        assert!(session_needs_recovered_identity(&session));
        assert!(apply_recovered_identity(
            &mut session,
            &("thread-1".into(), "Lume principal".into()),
        ));
        assert_eq!(session.session_name, "Lume principal");
        assert_eq!(session.native_session_id.as_deref(), Some("thread-1"));
    }

    #[test]
    fn live_codex_identity_uses_the_exact_indexed_thread_name() {
        let process = DiscoveredProcess {
            agent: AgentKind::Codex,
            agent_label: "Codex".into(),
            process_id: 4104,
            native_session_ids: vec!["thread-main".into()],
            working_directory: Some("/home/user".into()),
            source: SessionSource::Cli,
        };
        let names = HashMap::from([(
            IntegrationKind::Codex,
            HashMap::from([("thread-main".into(), "Lume principal".into())]),
        )]);

        let recovered = recover_process_identities_from_index(vec![&process], &[], &names);

        assert_eq!(
            recovered.get(&4104),
            Some(&("thread-main".into(), "Lume principal".into()))
        );
    }

    #[test]
    fn provider_thread_name_is_kept_separate_from_project() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("claude:named", 4150);
        event.session_name = Some("Review authentication".into());
        state.ingest(event).expect("sessão");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.session_name, "Review authentication");
        assert_eq!(session.project, "lume");
        assert_eq!(session.agent_label, "Claude Code");
    }

    #[test]
    fn provider_thread_name_update_replaces_the_previous_name() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("codex:named", 4151);
        event.agent = AgentKind::Codex;
        event.agent_label = Some("Codex".into());
        event.native_session_id = Some("thread-1".into());
        event.session_name = Some("Nome anterior".into());
        state.ingest(event.clone()).expect("sessão");

        event.event = HookEventKind::Activity;
        event.session_name = Some("Lume principal".into());
        state.ingest(event).expect("nome atualizado");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.session_name, "Lume principal");
    }

    #[test]
    fn renaming_a_session_adds_a_suffix_instead_of_colliding() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let first = started_event("claude:first", 4201);
        let mut second = started_event("claude:second", 4202);
        second.native_session_id = Some("native-session-2".into());
        state.ingest(first).expect("primeira sessão");
        state.ingest(second).expect("segunda sessão");

        assert_eq!(
            state
                .rename_session("claude:first", " Release review ")
                .expect("renomeação"),
            "Release review"
        );
        assert_eq!(
            state
                .rename_session("claude:second", "release review")
                .expect("renomeação"),
            "release review (2)"
        );
        assert_eq!(
            state
                .preferences()
                .expect("preferências")
                .session_aliases
                .get("claude_code:native-session-2")
                .map(String::as_str),
            Some("release review (2)")
        );
    }

    #[test]
    fn activity_updates_the_feed_without_changing_session_status() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:activity", 4242))
            .expect("sessão");
        let mut activity = started_event("claude:activity", 4242);
        activity.event = HookEventKind::Activity;
        activity.activity = Some(SessionActivity {
            id: "tool-1".into(),
            kind: "command".into(),
            title: "npm test".into(),
            detail: None,
            status: "running".into(),
            created_at: 10,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        state.ingest(activity.clone()).expect("atividade iniciada");
        activity.activity.as_mut().expect("atividade").status = "completed".into();
        state.ingest(activity).expect("atividade concluída");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.status, SessionStatus::WaitingForInput);
        assert_eq!(session.activities.len(), 1);
        assert_eq!(session.activities[0].status, "completed");
    }

    #[test]
    fn queued_prompt_is_promoted_only_when_it_starts() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:queue", 4242))
            .expect("sessão");
        state
            .record_queued_prompt_activity(
                "claude:queue",
                "queued-1",
                "Run after this task",
                Vec::new(),
            )
            .expect("prompt na fila");

        let queued = state.sessions().expect("sessões").remove(0);
        assert_eq!(queued.activities[0].kind, "queued_prompt");
        assert_eq!(queued.activities[0].status, "waiting");

        state
            .promote_queued_prompt_activity("claude:queue", "queued-1")
            .expect("prompt iniciado");
        let started = state.sessions().expect("sessões").remove(0);
        assert_eq!(started.activities[0].kind, "prompt");
        assert_eq!(started.activities[0].status, "completed");
    }

    #[test]
    fn uncertain_queued_prompt_is_marked_failed_instead_of_replayed() {
        let state = AppState::new(Path::new(":memory:")).expect("state");
        state
            .ingest(started_event("codex:queue-recovery", 4242))
            .expect("session");
        state
            .record_queued_prompt_activity(
                "codex:queue-recovery",
                "queued-1",
                "Do not replay blindly",
                Vec::new(),
            )
            .expect("queued prompt");
        state
            .mark_queued_prompt_needs_attention("codex:queue-recovery", "queued-1")
            .expect("attention state");
        let session = state.sessions().expect("sessions").remove(0);
        assert_eq!(session.activities[0].kind, "queued_prompt");
        assert_eq!(session.activities[0].status, "failed");
        assert_eq!(
            session.activities[0].title,
            "Queued prompt was not replayed"
        );

        let recovered = AppState::new(Path::new(":memory:")).expect("recovered state");
        recovered
            .ingest(started_event("codex:queue-recovery", 4242))
            .expect("recovered session");
        recovered
            .mark_queued_prompt_needs_attention("codex:queue-recovery", "queued-after-restart")
            .expect("recovered attention state");
        let session = recovered.sessions().expect("sessions").remove(0);
        assert_eq!(session.activities[0].id, "queued-after-restart");
        assert_eq!(session.activities[0].status, "failed");
    }

    #[test]
    fn lume_attachment_transport_is_merged_with_the_local_prompt() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let directory = std::env::temp_dir().join(format!(
            "lume-prompt-dedup-{}-{}",
            std::process::id(),
            now_millis()
        ));
        fs::create_dir_all(&directory).expect("diretório");
        let file = directory.join("presentation.pdf");
        fs::write(&file, b"fake pdf").expect("arquivo");
        let path = fs::canonicalize(&file)
            .expect("caminho")
            .to_string_lossy()
            .to_string();
        let mut started = started_event("claude:prompt-attachment", 4242);
        started.working_directory = Some(directory.to_string_lossy().to_string());
        state.ingest(started.clone()).expect("sessão");
        state
            .record_prompt_activity(
                "claude:prompt-attachment",
                "Create a presentation",
                vec![PromptAttachment {
                    id: "local-file".into(),
                    name: "presentation.pdf".into(),
                    mime_type: "application/pdf".into(),
                    preview_data_url: String::new(),
                    path: Some(path.clone()),
                }],
            )
            .expect("prompt local");

        let mut transcript = started;
        transcript.event = HookEventKind::Activity;
        transcript.activity = Some(SessionActivity {
            id: "codex:prompt-attachment:user-message".into(),
            kind: "prompt".into(),
            title: "Prompt sent".into(),
            detail: Some(format!(
                "Create a presentation\n\n{LUME_ATTACHED_FILES_MARKER}\n- \"{path}\""
            )),
            status: "completed".into(),
            created_at: now_millis(),
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        state.ingest(transcript.clone()).expect("evento ao vivo");
        transcript.activity.as_mut().expect("atividade").id =
            "codex-rollout:prompt-attachment".into();
        state.ingest(transcript).expect("evento do rollout");

        let session = state.sessions().expect("sessões").remove(0);
        let prompts = session
            .activities
            .iter()
            .filter(|activity| activity.kind == "prompt")
            .collect::<Vec<_>>();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].detail.as_deref(), Some("Create a presentation"));
        assert_eq!(prompts[0].attachments.len(), 1);
        assert_eq!(
            prompts[0].attachments[0].path.as_deref(),
            Some(path.as_str())
        );

        let _ = fs::remove_file(file);
        let _ = fs::remove_dir(directory);
    }

    #[test]
    fn agent_message_exposes_existing_local_files_but_not_sensitive_files() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let directory = std::env::temp_dir().join(format!(
            "lume-response-files-{}-{}",
            std::process::id(),
            now_millis()
        ));
        fs::create_dir_all(&directory).expect("diretório");
        let image = directory.join("preview.png");
        let secret = directory.join(".env");
        fs::write(&image, b"not decoded during discovery").expect("imagem");
        fs::write(&secret, b"TOKEN=fake").expect("segredo");
        let mut event = started_event("claude:response-file", 4242);
        event.working_directory = Some(directory.to_string_lossy().to_string());
        state.ingest(event.clone()).expect("sessão");
        event.event = HookEventKind::Activity;
        event.activity = Some(SessionActivity {
            id: "response-with-file".into(),
            kind: "message".into(),
            title: "Agent response".into(),
            detail: Some(format!(
                "Final image: [preview](preview.png) and [secret]({})",
                secret.to_string_lossy()
            )),
            status: "completed".into(),
            created_at: now_millis(),
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        state.ingest(event).expect("resposta");

        let session = state.sessions().expect("sessões").remove(0);
        let message = session
            .activities
            .iter()
            .find(|activity| activity.kind == "message")
            .expect("mensagem");
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(message.attachments[0].name, "preview.png");
        assert_eq!(message.attachments[0].mime_type, "image/png");

        let _ = fs::remove_file(image);
        let _ = fs::remove_file(secret);
        let _ = fs::remove_dir(directory);
    }

    #[test]
    fn agent_message_does_not_expose_code_spans_as_downloads() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let directory = std::env::temp_dir().join(format!(
            "lume-response-code-span-{}-{}",
            std::process::id(),
            now_millis()
        ));
        fs::create_dir_all(&directory).expect("diretório");
        let generator = directory.join("generate.mjs");
        fs::write(&generator, b"export default true;").expect("gerador");
        let mut event = started_event("claude:response-code-span", 4242);
        event.working_directory = Some(directory.to_string_lossy().to_string());
        state.ingest(event.clone()).expect("sessão");
        event.event = HookEventKind::Activity;
        event.activity = Some(SessionActivity {
            id: "response-with-code-span".into(),
            kind: "message".into(),
            title: "Agent response".into(),
            detail: Some(format!(
                "The editable generator is in `{}`.",
                generator.to_string_lossy()
            )),
            status: "completed".into(),
            created_at: now_millis(),
            files: vec![generator.to_string_lossy().to_string()],
            attachments: Vec::new(),
            append_detail: false,
        });
        state.ingest(event).expect("resposta");

        let session = state.sessions().expect("sessões").remove(0);
        let message = session
            .activities
            .iter()
            .find(|activity| activity.kind == "message")
            .expect("mensagem");
        assert!(message.attachments.is_empty());

        let _ = fs::remove_file(generator);
        let _ = fs::remove_dir(directory);
    }

    #[test]
    fn response_downloads_require_an_explicit_delivery_cue() {
        assert!(explicit_response_targets(
            "Main files: [state.rs](/work/lume/src/state.rs) and [TerminalWindow.svelte](/work/lume/src/TerminalWindow.svelte)"
        )
        .is_empty());
        assert_eq!(
            explicit_response_targets(
                "Final PDF: [presentation](/work/lume/output/presentation.pdf)"
            ),
            vec!["/work/lume/output/presentation.pdf"]
        );
        assert_eq!(
            explicit_response_targets("Download: `/work/lume/output/package.zip`"),
            vec!["/work/lume/output/package.zip"]
        );
    }

    #[test]
    fn transcript_activity_batch_is_ingested_and_deduplicated() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("claude:transcript", 4242);
        event.activities = vec![
            SessionActivity {
                id: "transcript-thinking".into(),
                kind: "thinking".into(),
                title: "Thinking".into(),
                detail: Some("Inspecting".into()),
                status: "completed".into(),
                created_at: 10,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            },
            SessionActivity {
                id: "transcript-message".into(),
                kind: "message".into(),
                title: "Claude".into(),
                detail: Some("Done".into()),
                status: "completed".into(),
                created_at: 20,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            },
        ];
        state.ingest(event.clone()).expect("primeiro lote");
        state.ingest(event).expect("lote repetido");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.activities.len(), 2);
        assert_eq!(session.activities[0].kind, "thinking");
        assert_eq!(session.activities[1].kind, "message");
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
    fn identical_agent_messages_from_two_sources_are_merged() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("codex:messages", 4242))
            .expect("sessão");
        for (id, created_at) in [("app-server", 10_000), ("rollout", 41_000)] {
            let mut event = started_event("codex:messages", 4242);
            event.event = HookEventKind::Activity;
            event.activity = Some(SessionActivity {
                id: id.into(),
                kind: "message".into(),
                title: "Resposta do agente".into(),
                detail: Some("Resposta final".into()),
                status: "completed".into(),
                created_at,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            });
            state.ingest(event).expect("mensagem");
        }

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(
            session
                .activities
                .iter()
                .filter(|activity| activity.kind == "message")
                .count(),
            1
        );
    }

    #[test]
    fn truncated_agent_message_is_replaced_by_the_complete_response() {
        let event = started_event("codex:complete-answer", 4242);
        let mut session = session_from_event(&event, 1_000);
        let complete = format!("{} resposta completa", "a".repeat(300));
        let truncated = format!("{}…", "a".repeat(300));

        for (id, detail, created_at) in [
            ("app-server", truncated, 10_000),
            ("rollout", complete.clone(), 11_000),
        ] {
            remember_activity(
                &mut session,
                SessionActivity {
                    id: id.into(),
                    kind: "message".into(),
                    title: "Resposta do agente".into(),
                    detail: Some(detail),
                    status: "completed".into(),
                    created_at,
                    files: Vec::new(),
                    attachments: Vec::new(),
                    append_detail: false,
                },
            );
        }

        assert_eq!(session.activities.len(), 1);
        assert_eq!(
            session.activities[0].detail.as_deref(),
            Some(complete.as_str())
        );
    }

    #[test]
    fn streamed_message_deltas_update_their_item_before_content_deduplication() {
        let event = started_event("codex:stream-order", 4242);
        let mut session = session_from_event(&event, 1_000);
        remember_activity(
            &mut session,
            SessionActivity {
                id: "other-message".into(),
                kind: "message".into(),
                title: "Resposta do agente".into(),
                detail: Some(" world".into()),
                status: "completed".into(),
                created_at: 9_000,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            },
        );
        remember_activity(
            &mut session,
            SessionActivity {
                id: "codex:thread:item-1".into(),
                kind: "message".into(),
                title: "Resposta do agente".into(),
                detail: Some("Hello".into()),
                status: "running".into(),
                created_at: 10_000,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            },
        );
        remember_activity(
            &mut session,
            SessionActivity {
                id: "codex:thread:item-1".into(),
                kind: "message".into(),
                title: "Resposta do agente".into(),
                detail: Some(" world".into()),
                status: "running".into(),
                created_at: 11_000,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: true,
            },
        );

        assert_eq!(
            session
                .activities
                .iter()
                .find(|activity| activity.id == "codex:thread:item-1")
                .and_then(|activity| activity.detail.as_deref()),
            Some("Hello world")
        );
    }

    #[test]
    fn final_response_removes_the_corrupted_stream_from_the_same_turn() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut started = started_event("codex:final-stream", 4242);
        started.agent = AgentKind::Codex;
        started.agent_label = Some("Codex".into());
        state.ingest(started).expect("sessão");

        for (id, kind, detail, status, created_at) in [
            ("prompt-1", "prompt", "Apply the fix", "completed", 10_000),
            (
                "codex:thread:item-1",
                "message",
                "Browser tabs identity corrections ChatGPT separated from Codex. Prompts acknowledged after delivery. Claude Code browser tabs keep their identity. Implemented corrections Claude web separated from Claude. Tests passed successfully. Prompts delivery.",
                "running",
                11_000,
            ),
        ] {
            let mut activity = started_event("codex:final-stream", 4242);
            activity.agent = AgentKind::Codex;
            activity.agent_label = Some("Codex".into());
            activity.event = HookEventKind::Activity;
            activity.activity = Some(SessionActivity {
                id: id.into(),
                kind: kind.into(),
                title: "Chat".into(),
                detail: Some(detail.into()),
                status: status.into(),
                created_at,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            });
            state.ingest(activity).expect("atividade");
        }

        let final_response = "Implemented the corrections. ChatGPT is now separated from Codex. Claude web is separated from Claude Code. Browser tabs keep their own identity. Prompts are acknowledged after delivery. Tests passed successfully.";
        let mut completed = started_event("codex:final-stream", 4242);
        completed.agent = AgentKind::Codex;
        completed.agent_label = Some("Codex".into());
        completed.event = HookEventKind::Completed;
        completed.last_response = Some(final_response.into());
        state.ingest(completed).expect("resposta final");

        let session = state.sessions().expect("sessões").remove(0);
        assert!(!session.activities.iter().any(|activity| {
            activity.kind == "message"
                && activity
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.starts_with("Browser tabs identity"))
        }));
        assert_eq!(session.results.len(), 1);
        assert_eq!(session.results[0].response, final_response);
    }

    #[test]
    fn identical_agent_messages_after_a_new_prompt_are_kept() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("codex:repeated-answer", 4242))
            .expect("sessão");
        for (id, kind, detail, created_at) in [
            ("answer-1", "message", "Resposta final", 10_000),
            ("prompt-2", "prompt", "Faça novamente", 20_000),
            ("answer-2", "message", "Resposta final", 30_000),
        ] {
            let mut event = started_event("codex:repeated-answer", 4242);
            event.event = HookEventKind::Activity;
            event.activity = Some(SessionActivity {
                id: id.into(),
                kind: kind.into(),
                title: "Chat".into(),
                detail: Some(detail.into()),
                status: "completed".into(),
                created_at,
                files: Vec::new(),
                attachments: Vec::new(),
                append_detail: false,
            });
            state.ingest(event).expect("atividade");
        }

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(
            session
                .activities
                .iter()
                .filter(|activity| activity.kind == "message")
                .count(),
            2
        );
    }

    #[test]
    fn merged_sessions_deduplicate_equivalent_recent_results() {
        let event = started_event("codex:merged-results", 4242);
        let mut target = session_from_event(&event, 1_000);
        let mut source = target.clone();
        target.results.push(SessionResult {
            id: "app-server-result".into(),
            response: "Resposta final".into(),
            created_at: 10_000,
            files: Vec::new(),
            tests: Vec::new(),
        });
        source.results.push(SessionResult {
            id: "rollout-result".into(),
            response: "  Resposta final\n".into(),
            created_at: 42_000,
            files: Vec::new(),
            tests: Vec::new(),
        });

        merge_results(&mut target, &source);

        assert_eq!(target.results.len(), 1);
    }

    #[test]
    fn merged_sessions_keep_identical_results_after_a_new_prompt() {
        let event = started_event("codex:separate-results", 4242);
        let mut target = session_from_event(&event, 1_000);
        let mut source = target.clone();
        target.results.push(SessionResult {
            id: "first-result".into(),
            response: "Resposta final".into(),
            created_at: 10_000,
            files: Vec::new(),
            tests: Vec::new(),
        });
        source.activities.push(SessionActivity {
            id: "second-prompt".into(),
            kind: "prompt".into(),
            title: "Prompt".into(),
            detail: Some("Faça novamente".into()),
            status: "completed".into(),
            created_at: 20_000,
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
        source.results.push(SessionResult {
            id: "second-result".into(),
            response: "Resposta final".into(),
            created_at: 30_000,
            files: Vec::new(),
            tests: Vec::new(),
        });

        merge_results(&mut target, &source);

        assert_eq!(target.results.len(), 2);
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
    fn final_response_extracts_markdown_link_targets_without_markdown_fragments() {
        let (files, _) = extract_result_artifacts(
            "- [source.txt](/workspace/sample-project/source.txt): atualizado\n\
             - [result.txt](/workspace/sample-project/result.txt): criado\n\
             - [.env](/workspace/sample-project/.env): omitido\n\
             - [test_content.py](/workspace/sample-project/test_content.py): criado",
        );

        assert_eq!(files, vec!["source.txt", "result.txt", "test_content.py"]);
        assert!(files.iter().all(|file| !file.contains("](")));
    }

    #[test]
    fn completed_task_reports_files_changed_since_it_started() {
        let root = std::env::temp_dir().join(format!(
            "lume-workspace-test-{}-{}",
            std::process::id(),
            now_millis()
        ));
        fs::create_dir_all(&root).expect("diretório temporário");
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .expect("git");
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.name", "Lume Test"]);
        git(&["config", "user.email", "lume@example.invalid"]);
        fs::write(root.join("tracked.txt"), "before\n").expect("arquivo inicial");
        git(&["add", "tracked.txt"]);
        git(&["commit", "--quiet", "-m", "initial"]);

        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("claude:workspace", 4242);
        event.working_directory = Some(root.to_string_lossy().into_owned());
        state.ingest(event.clone()).expect("sessão");
        event.event = HookEventKind::Running;
        state.ingest(event.clone()).expect("início da tarefa");

        fs::write(root.join("tracked.txt"), "after\n").expect("alteração");
        fs::write(root.join("new.txt"), "new\n").expect("novo arquivo");
        event.event = HookEventKind::Completed;
        event.last_response = Some("Pronto".into());
        state.ingest(event).expect("fim da tarefa");

        let session = state.sessions().expect("sessões").remove(0);
        let files = session
            .results
            .first()
            .map(|result| result.files.clone())
            .expect("resultado");
        assert_eq!(files, vec!["new.txt", "tracked.txt"]);
        assert!(session.activities.iter().any(|activity| {
            activity.kind == "file"
                && activity.files == vec!["new.txt".to_string(), "tracked.txt".to_string()]
        }));

        fs::remove_dir_all(&root).expect("limpeza");
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
    fn cli_hook_without_pid_reuses_the_only_process_in_the_project() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![discovered(4242)])
            .expect("processo provisório");
        let mut event = started_event("claude:session-without-pid", 4242);
        event.process_id = None;

        state.ingest(event).expect("hook");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "claude:session-without-pid");
        assert_eq!(sessions[0].process_id, Some(4242));
        assert_eq!(
            sessions[0].native_session_id.as_deref(),
            Some("native-session")
        );
    }

    #[test]
    fn resumed_vscode_thread_keeps_its_active_cli_process() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
                process_id: 4242,
                native_session_ids: Vec::new(),
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
        assert_eq!(sessions[0].source, SessionSource::Cli);
        assert_eq!(sessions[0].process_id, Some(4242));
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert_eq!(sessions[0].last_response.as_deref(), Some("Tudo pronto"));
    }

    #[test]
    fn active_rollout_file_binds_the_exact_cli_process() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("codex-app-server:thread-1", 0);
        event.agent = AgentKind::Codex;
        event.source = Some(SessionSource::Vscode);
        event.process_id = None;
        event.native_session_id = Some("thread-1".into());
        event.working_directory = Some("/home/user".into());
        state.ingest(event).expect("chat nativo");

        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
                process_id: 4242,
                native_session_ids: vec!["thread-1".into()],
                working_directory: Some("/home/user".into()),
                source: SessionSource::Cli,
            }])
            .expect("processo exato");

        let mut running = started_event("codex-app-server:thread-1", 0);
        running.agent = AgentKind::Codex;
        running.source = Some(SessionSource::Vscode);
        running.process_id = None;
        running.native_session_id = Some("thread-1".into());
        running.working_directory = Some("/home/user".into());
        running.event = HookEventKind::Running;
        state
            .ingest(running)
            .expect("evento passivo do rollout retomado");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].process_id, Some(4242));
        assert_eq!(sessions[0].source, SessionSource::Cli);
        assert_eq!(sessions[0].status, SessionStatus::Running);
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
    fn corrected_process_directory_updates_the_provisional_project_name() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut process = discovered(4242);
        process.working_directory = Some("/home/user".into());
        state
            .reconcile_processes(vec![process.clone()])
            .expect("descoberta inicial");
        process.working_directory = Some("/home/user/Documents/Projetos/Ideias/Lume".into());
        state
            .reconcile_processes(vec![process])
            .expect("diretório corrigido");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].project, "Lume");
    }

    #[test]
    fn independent_processes_in_the_same_context_create_distinct_sessions() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![discovered(4242), discovered(4343)])
            .expect("descoberta");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 2);
        assert!(sessions
            .iter()
            .any(|session| session.process_id == Some(4242)));
        assert!(sessions
            .iter()
            .any(|session| session.process_id == Some(4343)));
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
        duplicate.process_id = Some(4242);
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
            .reconcile_processes(vec![discovered(4242)])
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
    fn generic_followup_keeps_the_explicit_thread_permission_scope() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut explicit = started_event("codex:chat-1", 4242);
        explicit.permission_profile = Some(PermissionProfile {
            mode: AccessMode::WorkspaceWrite,
            label: "Acesso ao projeto".into(),
            approval_policy: "on-request".into(),
            approvals_reviewer: Some("auto_review".into()),
            can_respond_from_lume: false,
            available_actions: vec![PermissionAction::OpenSource],
        });
        state.ingest(explicit).expect("perfil da thread");

        let mut generic = started_event("codex:chat-1", 4242);
        generic.event = HookEventKind::Running;
        generic.permission_profile = Some(PermissionProfile {
            mode: AccessMode::Custom,
            label: "Permissões desta sessão".into(),
            approval_policy: "Decisões encaminhadas pelo Codex App Server".into(),
            approvals_reviewer: None,
            can_respond_from_lume: true,
            available_actions: vec![PermissionAction::AllowOnce, PermissionAction::Deny],
        });
        state.ingest(generic).expect("evento genérico");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.permission_profile.mode, AccessMode::WorkspaceWrite);
        assert_eq!(
            session.permission_profile.approvals_reviewer.as_deref(),
            Some("auto_review")
        );
        assert!(session.permission_profile.can_respond_from_lume);
    }

    #[test]
    fn automatic_profile_never_enters_the_permission_state() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut started = started_event("codex:chat-1", 4242);
        started.permission_profile = Some(PermissionProfile {
            mode: AccessMode::WorkspaceWrite,
            label: "Aprovar por mim".into(),
            approval_policy: "on-request".into(),
            approvals_reviewer: Some("auto_review".into()),
            can_respond_from_lume: false,
            available_actions: vec![PermissionAction::OpenSource],
        });
        state.ingest(started).expect("perfil automático");

        let mut permission = started_event("codex:chat-1", 4242);
        permission.event = HookEventKind::PermissionRequest;
        permission.permission = Some(PermissionRequest {
            id: "permission-1".into(),
            kind: "command".into(),
            summary: "Executar comando".into(),
            resource: "cargo test".into(),
            risk: "medium".into(),
            requested_at: "1".into(),
        });
        state.ingest(permission).expect("permissão automática");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.status, SessionStatus::Running);
        assert!(session.pending_permission.is_none());
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
    fn distinct_live_processes_remain_visible_after_one_receives_a_native_event() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .ingest(started_event("claude:active-chat", 4242))
            .expect("chat ativo");
        state
            .reconcile_processes(vec![discovered(4242), discovered(4343)])
            .expect("descoberta");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 2);
        assert!(sessions
            .iter()
            .any(|session| session.process_id == Some(4242)));
        assert!(sessions
            .iter()
            .any(|session| session.process_id == Some(4343)));
    }
    #[test]
    fn vscode_chat_hides_its_host_process_without_hiding_other_chats() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
                process_id: 5252,
                native_session_ids: Vec::new(),
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
                    session_name: None,
                    project: Some("lume".into()),
                    source: Some(SessionSource::Vscode),
                    source_app: None,
                    control_origin: SessionControlOrigin::External,
                    status_label: Some("Executando no VS Code".into()),
                    started_at: None,
                    process_id: None,
                    native_session_id: Some(chat.into()),
                    working_directory: Some("/work/lume".into()),
                    permission_profile: None,
                    permission: None,
                    question: None,
                    last_response: None,
                    activity: None,
                    activities: Vec::new(),
                    wait_for_decision: false,
                })
                .expect("chat do VS Code");
        }
        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
                process_id: 5252,
                native_session_ids: Vec::new(),
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
    fn vscode_host_without_a_chat_is_removed_even_while_the_host_is_alive() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
                process_id: 5252,
                native_session_ids: Vec::new(),
                working_directory: Some("/home/user/.vscode/extensions/openai.chatgpt".into()),
                source: SessionSource::Vscode,
            }])
            .expect("host inicialmente detectado");

        for _ in 0..PROCESS_MISSING_SCAN_LIMIT {
            state
                .reconcile_process_snapshot(Vec::new(), HashSet::from([5252]))
                .expect("host filtrado");
        }

        assert!(state.sessions().expect("sessões").is_empty());
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
    fn completed_native_chat_without_pid_absorbs_the_matching_process() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("claude:completed-without-pid", 4242);
        event.process_id = None;
        state.ingest(event.clone()).expect("início");
        event.event = HookEventKind::Completed;
        event.status_label = Some("Finalizado".into());
        state.ingest(event).expect("conclusão");

        state
            .reconcile_processes(vec![discovered(4242)])
            .expect("redetecção");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "claude:completed-without-pid");
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert_eq!(sessions[0].process_id, Some(4242));
    }

    #[test]
    fn home_process_does_not_duplicate_a_unique_recent_project_chat() {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .expect("diretório do usuário");
        let project = home.join("Documents").join("Lume");
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut event = started_event("codex-app-server:chat-home", 4242);
        event.agent = AgentKind::Codex;
        event.agent_label = Some("Codex".into());
        event.project = Some("Lume".into());
        event.process_id = None;
        event.native_session_id = Some("chat-home".into());
        event.working_directory = Some(project.to_string_lossy().into_owned());
        state.ingest(event.clone()).expect("início");
        event.event = HookEventKind::Completed;
        event.status_label = Some("Tarefa finalizada".into());
        state.ingest(event).expect("conclusão");

        state
            .reconcile_processes(vec![DiscoveredProcess {
                agent: AgentKind::Codex,
                agent_label: "Codex".into(),
                process_id: 4242,
                native_session_ids: Vec::new(),
                working_directory: Some(home.to_string_lossy().into_owned()),
                source: SessionSource::Cli,
            }])
            .expect("redetecção");

        let sessions = state.sessions().expect("sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "codex-app-server:chat-home");
        assert_eq!(sessions[0].project, "Lume");
        assert_eq!(sessions[0].status, SessionStatus::Completed);
        assert_eq!(sessions[0].process_id, Some(4242));
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
        let mut event = started_event("web:chatgpt:chat", 4343);
        event.agent = AgentKind::ChatGpt;
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
    fn question_answer_is_delivered_without_becoming_a_permission() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut question = started_event("codex:chat-1", 4242);
        question.event = HookEventKind::QuestionRequest;
        question.question = Some(crate::domain::PendingQuestion {
            id: "question-request".into(),
            questions: vec![crate::domain::InteractiveQuestion {
                id: "approach".into(),
                header: "Approach".into(),
                question: "Which approach?".into(),
                is_other: true,
                is_secret: false,
                options: vec![crate::domain::QuestionOption {
                    label: "Safe".into(),
                    description: String::new(),
                }],
            }],
            requested_at: "1".into(),
        });
        state.ingest(question).expect("pergunta");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.status, SessionStatus::WaitingForInput);
        assert!(session.pending_permission.is_none());
        assert!(session.pending_question.is_some());

        state
            .resolve_question(
                "codex:chat-1",
                "question-request",
                vec![QuestionAnswer {
                    question_id: "approach".into(),
                    answers: vec!["Safe".into()],
                }],
            )
            .expect("resposta");
        let answers = state
            .wait_for_question_answer("question-request", Duration::ZERO)
            .expect("entrega")
            .expect("resposta presente");
        assert_eq!(answers[0].answers, vec!["Safe"]);
    }

    #[test]
    fn claude_input_notification_does_not_hide_an_interactive_question() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut question = started_event("claude:chat-1", 4242);
        question.agent = AgentKind::ClaudeCode;
        question.event = HookEventKind::QuestionRequest;
        question.question = Some(crate::domain::PendingQuestion {
            id: "question-request".into(),
            questions: vec![crate::domain::InteractiveQuestion {
                id: "approach".into(),
                header: "Approach".into(),
                question: "Which approach?".into(),
                is_other: true,
                is_secret: false,
                options: vec![crate::domain::QuestionOption {
                    label: "Safe".into(),
                    description: String::new(),
                }],
            }],
            requested_at: "1".into(),
        });
        state.ingest(question).expect("pergunta");

        let mut waiting = started_event("claude:chat-1", 4242);
        waiting.agent = AgentKind::ClaudeCode;
        waiting.event = HookEventKind::WaitingForInput;
        waiting.status_label = Some("Esperando ação".into());
        state.ingest(waiting).expect("notificação");

        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(session.status, SessionStatus::WaitingForInput);
        assert_eq!(session.status_label, "Aguardando sua resposta");
        assert_eq!(
            session
                .pending_question
                .as_ref()
                .map(|question| question.id.as_str()),
            Some("question-request")
        );
    }

    #[test]
    fn repeated_permission_keeps_the_original_pending_decision() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let mut first = started_event("codex:chat-1", 4242);
        first.event = HookEventKind::PermissionRequest;
        first.permission = Some(PermissionRequest {
            id: "permission-1".into(),
            kind: "command".into(),
            summary: "Executar comando".into(),
            resource: "npm test".into(),
            risk: "medium".into(),
            requested_at: "1".into(),
        });
        state.ingest(first).expect("primeira permissão");

        let mut repeated = started_event("codex:chat-1", 4242);
        repeated.event = HookEventKind::PermissionRequest;
        repeated.permission = Some(PermissionRequest {
            id: "permission-2".into(),
            kind: "command".into(),
            summary: "Executar comando".into(),
            resource: "npm test".into(),
            risk: "medium".into(),
            requested_at: "2".into(),
        });
        let id = state.ingest(repeated).expect("permissão repetida");

        assert_eq!(id.as_deref(), Some("permission-1"));
        let session = state.sessions().expect("sessões").remove(0);
        assert_eq!(
            session
                .pending_permission
                .as_ref()
                .map(|value| value.id.as_str()),
            Some("permission-1")
        );
        assert_eq!(
            session
                .activities
                .iter()
                .filter(|activity| activity.kind == "permission")
                .count(),
            1
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
