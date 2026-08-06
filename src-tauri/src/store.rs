use std::path::Path;

use rusqlite::{params, Connection};

use crate::domain::{
    AgentSession, HistoryEntry, MobileScope, PairedDevice, Preferences, ResultNote, SessionActivity,
};

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA secure_delete = ON;
                 DROP TABLE IF EXISTS sessions;
                 CREATE TABLE IF NOT EXISTS history (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    agent_label TEXT NOT NULL,
                    project TEXT NOT NULL,
                    event TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_history_created_at
                    ON history(created_at DESC);
                 CREATE TABLE IF NOT EXISTS preferences (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    payload TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS result_notes (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    body TEXT NOT NULL,
                    agent_label TEXT NOT NULL,
                    project TEXT NOT NULL,
                    files TEXT NOT NULL,
                    tests TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS mobile_devices (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    token_hash TEXT NOT NULL UNIQUE,
                    created_at INTEGER NOT NULL,
                    last_seen_at INTEGER,
                    scopes TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS session_plans (
                    native_session_id TEXT PRIMARY KEY,
                    content TEXT NOT NULL,
                    source_activity_id TEXT,
                    updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS conversation_activities (
                    thread_key TEXT NOT NULL,
                    activity_id TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    PRIMARY KEY(thread_key, activity_id)
                 );
                 CREATE INDEX IF NOT EXISTS idx_conversation_activities_thread
                    ON conversation_activities(thread_key, created_at ASC);
                 CREATE TABLE IF NOT EXISTS workflow_runs (
                    workflow_id TEXT PRIMARY KEY,
                    payload TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                 );",
            )
            .map_err(|error| error.to_string())?;

        Ok(Self { connection })
    }

    #[cfg(test)]
    pub fn load_sessions(&self) -> Result<Vec<AgentSession>, String> {
        Ok(Vec::new())
    }

    pub fn save_session(&self, session: &AgentSession) -> Result<(), String> {
        let Some(thread_key) = Self::conversation_key(session) else {
            return Ok(());
        };
        let durable = session
            .activities
            .iter()
            .filter(|activity| Self::is_archivable_conversation_activity(activity))
            .collect::<Vec<_>>();
        if durable.is_empty() {
            return Ok(());
        }
        let exists = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM conversation_activities WHERE thread_key = ?1)",
                [&thread_key],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| error.to_string())?;
        let start = if exists {
            durable.len().saturating_sub(6)
        } else {
            0
        };
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        for activity in &durable[start..] {
            let mut archived = (*activity).clone();
            for attachment in &mut archived.attachments {
                attachment.preview_data_url.clear();
            }
            let payload = serde_json::to_string(&archived).map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "INSERT INTO conversation_activities(thread_key, activity_id, payload, created_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(thread_key, activity_id) DO UPDATE SET
                        payload = excluded.payload,
                        created_at = excluded.created_at",
                    params![thread_key, archived.id, payload, archived.created_at],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_session(&self, _session_id: &str) -> Result<(), String> {
        Ok(())
    }

    pub fn conversation_key(session: &AgentSession) -> Option<String> {
        let native_id = session.native_session_id.as_deref()?.trim();
        if native_id.is_empty() {
            return None;
        }
        let agent = serde_json::to_string(&session.agent).ok()?;
        Some(format!("{}:{native_id}", agent.trim_matches('"')))
    }

    pub fn is_archivable_conversation_activity(activity: &SessionActivity) -> bool {
        matches!(activity.kind.as_str(), "prompt" | "queued_prompt")
            || (activity.kind == "message" && activity.status != "running")
    }

    pub fn conversation_activities(
        &self,
        session: &AgentSession,
    ) -> Result<Vec<SessionActivity>, String> {
        let Some(thread_key) = Self::conversation_key(session) else {
            return Ok(Vec::new());
        };
        let mut statement = self
            .connection
            .prepare(
                "SELECT payload FROM conversation_activities
                 WHERE thread_key = ?1 ORDER BY created_at ASC, activity_id ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([thread_key], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut activities = Vec::new();
        for payload in rows {
            let payload = payload.map_err(|error| error.to_string())?;
            if let Ok(activity) = serde_json::from_str(&payload) {
                if Self::is_archivable_conversation_activity(&activity) {
                    activities.push(activity);
                }
            }
        }
        Ok(activities)
    }

    pub fn save_session_plan(
        &self,
        native_session_id: &str,
        content: &str,
        source_activity_id: Option<&str>,
        updated_at: i64,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO session_plans
                 (native_session_id, content, source_activity_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(native_session_id) DO UPDATE SET
                    content = excluded.content,
                    source_activity_id = excluded.source_activity_id,
                    updated_at = excluded.updated_at
                 WHERE excluded.updated_at >= session_plans.updated_at",
                params![native_session_id, content, source_activity_id, updated_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn session_plan(
        &self,
        native_session_id: &str,
    ) -> Result<Option<(String, Option<String>, i64)>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT content, source_activity_id, updated_at
                 FROM session_plans
                 WHERE native_session_id = ?1",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = statement
            .query(params![native_session_id])
            .map_err(|error| error.to_string())?;
        let Some(row) = rows.next().map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        Ok(Some((
            row.get(0).map_err(|error| error.to_string())?,
            row.get(1).map_err(|error| error.to_string())?,
            row.get(2).map_err(|error| error.to_string())?,
        )))
    }

    pub fn add_history(&self, entry: &HistoryEntry) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO history
                 (id, session_id, agent_label, project, event, summary, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.id,
                    entry.session_id,
                    entry.agent_label,
                    entry.project,
                    entry.event,
                    entry.summary,
                    entry.created_at
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn history(&self, limit: usize) -> Result<Vec<HistoryEntry>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, session_id, agent_label, project, event, summary, created_at
                 FROM history ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    agent_label: row.get(2)?,
                    project: row.get(3)?,
                    event: row.get(4)?,
                    summary: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn save_result_note(&self, note: &ResultNote) -> Result<(), String> {
        let files = serde_json::to_string(&note.files).map_err(|error| error.to_string())?;
        let tests = serde_json::to_string(&note.tests).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT OR REPLACE INTO result_notes
                 (id, title, body, agent_label, project, files, tests, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    note.id,
                    note.title,
                    note.body,
                    note.agent_label,
                    note.project,
                    files,
                    tests,
                    note.created_at
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn result_notes(&self, limit: usize) -> Result<Vec<ResultNote>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, title, body, agent_label, project, files, tests, created_at
                 FROM result_notes ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit as i64], |row| {
                let files: String = row.get(5)?;
                let tests: String = row.get(6)?;
                Ok(ResultNote {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    body: row.get(2)?,
                    agent_label: row.get(3)?,
                    project: row.get(4)?,
                    files: serde_json::from_str(&files).unwrap_or_default(),
                    tests: serde_json::from_str(&tests).unwrap_or_default(),
                    created_at: row.get(7)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn delete_result_note(&self, id: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM result_notes WHERE id = ?1", [id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn save_workflow_run(
        &self,
        workflow_id: &str,
        payload: &str,
        updated_at: i64,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO workflow_runs (workflow_id, payload, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(workflow_id) DO UPDATE SET
                    payload = excluded.payload,
                    updated_at = excluded.updated_at",
                params![workflow_id, payload, updated_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn workflow_runs(&self) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM workflow_runs ORDER BY updated_at DESC")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn save_mobile_device(
        &self,
        device: &PairedDevice,
        token_hash: &str,
    ) -> Result<(), String> {
        let scopes = serde_json::to_string(&device.scopes).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO mobile_devices
                 (id, name, token_hash, created_at, last_seen_at, scopes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    device.id,
                    device.name,
                    token_hash,
                    device.created_at,
                    device.last_seen_at,
                    scopes,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn mobile_devices(&self) -> Result<Vec<PairedDevice>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, name, created_at, last_seen_at, scopes
                 FROM mobile_devices ORDER BY created_at DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let scopes: String = row.get(4)?;
                Ok(PairedDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    last_seen_at: row.get(3)?,
                    scopes: serde_json::from_str::<Vec<MobileScope>>(&scopes)
                        .unwrap_or_else(|_| vec![MobileScope::Monitor]),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn mobile_device_for_token_hash(
        &self,
        token_hash: &str,
        seen_at: i64,
    ) -> Result<Option<PairedDevice>, String> {
        let result = self.connection.query_row(
            "SELECT id, name, created_at, last_seen_at, scopes
             FROM mobile_devices WHERE token_hash = ?1",
            [token_hash],
            |row| {
                let scopes: String = row.get(4)?;
                Ok(PairedDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    last_seen_at: row.get(3)?,
                    scopes: serde_json::from_str::<Vec<MobileScope>>(&scopes)
                        .unwrap_or_else(|_| vec![MobileScope::Monitor]),
                })
            },
        );
        match result {
            Ok(mut device) => {
                self.connection
                    .execute(
                        "UPDATE mobile_devices SET last_seen_at = ?1 WHERE id = ?2",
                        params![seen_at, device.id],
                    )
                    .map_err(|error| error.to_string())?;
                device.last_seen_at = Some(seen_at);
                Ok(Some(device))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn mobile_device_with_token_hash(
        &self,
        id: &str,
        seen_at: i64,
    ) -> Result<Option<(PairedDevice, String)>, String> {
        let result = self.connection.query_row(
            "SELECT id, name, token_hash, created_at, last_seen_at, scopes
             FROM mobile_devices WHERE id = ?1",
            [id],
            |row| {
                let scopes: String = row.get(5)?;
                Ok((
                    PairedDevice {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        created_at: row.get(3)?,
                        last_seen_at: row.get(4)?,
                        scopes: serde_json::from_str::<Vec<MobileScope>>(&scopes)
                            .unwrap_or_else(|_| vec![MobileScope::Monitor]),
                    },
                    row.get(2)?,
                ))
            },
        );
        match result {
            Ok((mut device, token_hash)) => {
                self.connection
                    .execute(
                        "UPDATE mobile_devices SET last_seen_at = ?1 WHERE id = ?2",
                        params![seen_at, device.id],
                    )
                    .map_err(|error| error.to_string())?;
                device.last_seen_at = Some(seen_at);
                Ok(Some((device, token_hash)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn revoke_mobile_device(&self, id: &str) -> Result<bool, String> {
        self.connection
            .execute("DELETE FROM mobile_devices WHERE id = ?1", [id])
            .map(|changed| changed > 0)
            .map_err(|error| error.to_string())
    }

    pub fn set_mobile_device_scopes(
        &self,
        id: &str,
        scopes: &[MobileScope],
    ) -> Result<bool, String> {
        let scopes = serde_json::to_string(scopes).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "UPDATE mobile_devices SET scopes = ?1 WHERE id = ?2",
                params![scopes, id],
            )
            .map(|changed| changed > 0)
            .map_err(|error| error.to_string())
    }

    pub fn load_preferences(&self) -> Result<Preferences, String> {
        let result =
            self.connection
                .query_row("SELECT payload FROM preferences WHERE id = 1", [], |row| {
                    row.get::<_, String>(0)
                });
        match result {
            Ok(payload) => {
                let mut preferences = serde_json::from_str::<Preferences>(&payload)
                    .map_err(|error| error.to_string())?;
                for profile in preferences.project_profiles.values_mut() {
                    for agent in &mut profile.preferred_agents {
                        if agent == "claude" {
                            *agent = "claude_code".into();
                        }
                    }
                }
                let legacy_claude_aliases = preferences
                    .session_aliases
                    .iter()
                    .filter_map(|(key, value)| {
                        key.strip_prefix("claude:")
                            .map(|native_id| (format!("claude_code:{native_id}"), value.clone()))
                    })
                    .collect::<Vec<_>>();
                for (key, value) in legacy_claude_aliases {
                    preferences.session_aliases.entry(key).or_insert(value);
                }
                for layout in &mut preferences.whiteboard_layouts {
                    for terminal in &mut layout.terminals {
                        if terminal.agent == crate::domain::AgentKind::Claude
                            && terminal.source != crate::domain::SessionSource::Web
                        {
                            terminal.agent = crate::domain::AgentKind::ClaudeCode;
                            terminal.agent_label = "Claude Code".into();
                        }
                    }
                }
                Ok(preferences)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Preferences::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<(), String> {
        let payload = serde_json::to_string(preferences).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO preferences(id, payload) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
                [payload],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn purge_history(&self, older_than: i64) -> Result<(), String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM history WHERE created_at < ?1", [older_than])
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM conversation_activities WHERE created_at < ?1",
                [older_than],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn scrub_deleted_content(&self) -> Result<(), String> {
        self.connection
            .execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AccessMode, AgentKind, PermissionAction, PermissionProfile, PermissionRequest,
        SessionSource, SessionStatus,
    };

    #[test]
    fn agent_sessions_are_never_persisted() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        let session = AgentSession {
            id: "session".into(),
            agent: AgentKind::Codex,
            agent_label: "Codex".into(),
            session_name: "Codex · Lume".into(),
            project: "Lume".into(),
            source: SessionSource::Cli,
            source_app: None,
            status: SessionStatus::PermissionRequired,
            status_label: "Aguardando permissão".into(),
            started_at: "0".into(),
            updated_at: 1,
            process_id: None,
            native_session_id: None,
            working_directory: Some("/work/segredo/projeto".into()),
            permission_profile: PermissionProfile {
                mode: AccessMode::Custom,
                label: "Sessão".into(),
                approval_policy: "on-request".into(),
                approvals_reviewer: None,
                can_respond_from_lume: true,
                available_actions: vec![PermissionAction::Deny],
            },
            pending_permission: Some(PermissionRequest {
                id: "permission".into(),
                kind: "command".into(),
                summary: "Executar".into(),
                resource: "segredo-que-nao-pode-ser-salvo".into(),
                risk: "high".into(),
                requested_at: "0".into(),
            }),
            pending_question: None,
            last_response: Some("resposta que nao pode ser salva".into()),
            results: vec![crate::domain::SessionResult {
                id: "result-1".into(),
                response: "outra resposta sensível".into(),
                created_at: 1,
                files: Vec::new(),
                tests: Vec::new(),
            }],
            activities: Vec::new(),
            rate_limits: Vec::new(),
        };
        store.save_session(&session).expect("salva a sessão");
        let loaded = store.load_sessions().expect("carrega as sessões");
        assert!(loaded.is_empty());

        let mut threaded = session.clone();
        threaded.native_session_id = Some("thread-archive".into());
        threaded.activities.push(SessionActivity {
            id: "message-1".into(),
            kind: "message".into(),
            title: "Codex".into(),
            detail: Some("resposta preservada".into()),
            status: "completed".into(),
            created_at: 2,
            files: Vec::new(),
            attachments: vec![crate::domain::PromptAttachment {
                id: "image-1".into(),
                name: "image.png".into(),
                mime_type: "image/png".into(),
                preview_data_url: "data:image/png;base64,secret".into(),
                path: Some("/tmp/image.png".into()),
            }],
            append_detail: false,
        });
        store.save_session(&threaded).expect("arquiva conversa");
        let archived = store
            .conversation_activities(&threaded)
            .expect("carrega conversa");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].detail.as_deref(), Some("resposta preservada"));
        assert!(archived[0].attachments[0].preview_data_url.is_empty());
    }

    #[test]
    fn session_plan_is_persisted_by_native_session_id() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        store
            .save_session_plan(
                "thread-1",
                "# Plan\n\n## Phase 1\nBuild",
                Some("message-1"),
                42,
            )
            .expect("salva plano");
        let plan = store
            .session_plan("thread-1")
            .expect("consulta plano")
            .expect("plano");
        assert_eq!(plan.0, "# Plan\n\n## Phase 1\nBuild");
        assert_eq!(plan.1.as_deref(), Some("message-1"));
        assert_eq!(plan.2, 42);
    }

    #[test]
    fn workflow_runs_round_trip_locally() {
        let store = Store::open(Path::new(":memory:")).expect("in-memory database");
        store
            .save_workflow_run("workflow-1", r#"{"status":"running"}"#, 42)
            .expect("save workflow run");
        assert_eq!(
            store.workflow_runs().expect("load workflow runs"),
            vec![r#"{"status":"running"}"#.to_string()]
        );

        store
            .save_workflow_run("workflow-1", r#"{"status":"completed"}"#, 43)
            .expect("update workflow run");
        assert_eq!(
            store.workflow_runs().expect("load updated workflow run"),
            vec![r#"{"status":"completed"}"#.to_string()]
        );
    }

    #[test]
    fn old_preferences_gain_optional_overlay_position() {
        let preferences: Preferences = serde_json::from_str(
            r#"{"soundEnabled":true,"autostart":true,"monitorId":null,"showOverFullscreen":false,"historyRetentionDays":30,"launchTarget":"auto"}"#,
        )
        .expect("preferências antigas");
        assert!(preferences.overlay_x.is_none());
        assert!(preferences.overlay_y.is_none());
        assert!(preferences.dark_mode.is_none());
        assert_eq!(preferences.language, "en");
        assert_eq!(preferences.sound_volume, 55);
        assert!(preferences.project_profiles.is_empty());
        assert!(preferences.session_aliases.is_empty());
        assert!(preferences.whiteboard_layouts.is_empty());
        assert_eq!(preferences.global_shortcut, "Ctrl+Shift+Space");
    }

    #[test]
    fn explicitly_saved_result_notes_round_trip_locally() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        let note = ResultNote {
            id: "note:result-1".into(),
            title: "Codex · Lume".into(),
            body: "Resposta final".into(),
            agent_label: "Codex".into(),
            project: "Lume".into(),
            files: vec!["src/main.rs".into()],
            tests: vec!["cargo test".into()],
            created_at: 42,
        };
        store.save_result_note(&note).expect("salva nota");

        let notes = store.result_notes(10).expect("carrega notas");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].body, "Resposta final");
        assert_eq!(notes[0].files, vec!["src/main.rs"]);

        store.delete_result_note(&note.id).expect("remove nota");
        assert!(store.result_notes(10).expect("notas vazias").is_empty());
    }

    #[test]
    fn old_project_profiles_gain_the_new_optional_fields() {
        let profile: crate::domain::ProjectProfile = serde_json::from_str(
            r#"{"label":"Lume","soundEnabled":true,"launchTarget":"terminal"}"#,
        )
        .expect("perfil antigo");
        assert!(profile.monitor_id.is_none());
        assert!(profile.permission_mode.is_none());
        assert!(profile.whiteboard_layout_id.is_none());
        assert!(profile.preferred_agents.is_empty());
    }

    #[test]
    fn legacy_claude_cli_preferences_migrate_without_changing_claude_web() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        let payload = r#"{
            "projectProfiles":{"lume":{"label":"Lume","preferredAgents":["claude"]}},
            "sessionAliases":{"claude:thread-1":"Review"},
            "whiteboardLayouts":[{
                "id":"layout","name":"Layout","terminals":[
                    {"agent":"claude","agentLabel":"Claude","project":"cli","source":"cli","x":0,"y":0,"width":400,"height":300,"groupId":null},
                    {"agent":"claude","agentLabel":"Claude","project":"web","source":"web","x":400,"y":0,"width":400,"height":300,"groupId":null}
                ]
            }]
        }"#;
        store
            .connection
            .execute(
                "INSERT INTO preferences(id, payload) VALUES (1, ?1)",
                [payload],
            )
            .expect("preferências antigas");

        let preferences = store.load_preferences().expect("preferências migradas");
        assert!(preferences.workflow_groups.is_empty());
        assert_eq!(
            preferences.project_profiles["lume"].preferred_agents,
            vec!["claude_code"]
        );
        assert_eq!(
            preferences
                .session_aliases
                .get("claude_code:thread-1")
                .map(String::as_str),
            Some("Review")
        );
        assert_eq!(
            preferences.whiteboard_layouts[0].terminals[0].agent,
            AgentKind::ClaudeCode
        );
        assert_eq!(
            preferences.whiteboard_layouts[0].terminals[1].agent,
            AgentKind::Claude
        );
    }
}
