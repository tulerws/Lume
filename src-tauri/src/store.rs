use std::path::Path;

use rusqlite::{params, Connection};

use crate::domain::{AgentSession, HistoryEntry, Preferences};

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
                 CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY,
                    payload TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                 );
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
                 );",
            )
            .map_err(|error| error.to_string())?;

        Ok(Self { connection })
    }

    pub fn load_sessions(&self) -> Result<Vec<AgentSession>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM sessions ORDER BY updated_at DESC")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;

        rows.map(|row| {
            let payload = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&payload).map_err(|error| error.to_string())
        })
        .collect()
    }

    pub fn save_session(&self, session: &AgentSession) -> Result<(), String> {
        // Solicitações são mantidas apenas em memória e apagadas após a decisão.
        let mut sanitized = session.clone();
        sanitized.pending_permission = None;
        sanitized.working_directory = None;
        let payload = serde_json::to_string(&sanitized).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO sessions(id, payload, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET payload = excluded.payload,
                    updated_at = excluded.updated_at",
                params![session.id, payload, session.updated_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM sessions WHERE id = ?1", [session_id])
            .map_err(|error| error.to_string())?;
        Ok(())
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

    pub fn load_preferences(&self) -> Result<Preferences, String> {
        let result =
            self.connection
                .query_row("SELECT payload FROM preferences WHERE id = 1", [], |row| {
                    row.get::<_, String>(0)
                });
        match result {
            Ok(payload) => serde_json::from_str(&payload).map_err(|error| error.to_string()),
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
        self.connection
            .execute("DELETE FROM history WHERE created_at < ?1", [older_than])
            .map_err(|error| error.to_string())?;
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
    fn pending_permission_payload_is_never_persisted() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        let session = AgentSession {
            id: "session".into(),
            agent: AgentKind::Codex,
            agent_label: "Codex".into(),
            project: "Lume".into(),
            source: SessionSource::Cli,
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
        };
        store.save_session(&session).expect("salva a sessão");
        let loaded = store.load_sessions().expect("carrega as sessões");
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].pending_permission.is_none());
        assert!(loaded[0].working_directory.is_none());
    }

    #[test]
    fn old_preferences_gain_optional_overlay_position() {
        let preferences: Preferences = serde_json::from_str(
            r#"{"soundEnabled":true,"autostart":true,"monitorId":null,"showOverFullscreen":false,"historyRetentionDays":30,"launchTarget":"auto"}"#,
        )
        .expect("preferências antigas");
        assert!(preferences.overlay_x.is_none());
        assert!(preferences.overlay_y.is_none());
    }
}
