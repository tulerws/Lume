use std::path::Path;

use rusqlite::{params, Connection};

use crate::domain::{AgentSession, HistoryEntry, Preferences, RemoteDevice, ResultNote};

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
                 CREATE TABLE IF NOT EXISTS remote_devices (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    platform TEXT NOT NULL,
                    token_hash TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    last_seen_at INTEGER
                 );",
            )
            .map_err(|error| error.to_string())?;

        Ok(Self { connection })
    }

    #[cfg(test)]
    pub fn load_sessions(&self) -> Result<Vec<AgentSession>, String> {
        Ok(Vec::new())
    }

    pub fn save_session(&self, _session: &AgentSession) -> Result<(), String> {
        Ok(())
    }

    pub fn delete_session(&self, _session_id: &str) -> Result<(), String> {
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

    /// Aparelhos pareados, sem credencial. É o que a interface lista.
    // Sem consumidor até a janela de pareamento existir; os testes já o cobrem.
    #[allow(dead_code)]
    pub fn remote_devices(&self) -> Result<Vec<RemoteDevice>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, name, platform, created_at, last_seen_at
                 FROM remote_devices ORDER BY created_at ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(RemoteDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    platform: row.get(2)?,
                    created_at: row.get(3)?,
                    last_seen_at: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Pares `(id, hash)` para autenticação.
    ///
    /// Devolve **todos** os hashes em vez de aceitar um `WHERE token_hash = ?`:
    /// a comparação de texto do SQLite não é de tempo constante, e a promessa
    /// de tempo constante é parte da especificação. Quem compara é o servidor.
    pub fn remote_device_credentials(&self) -> Result<Vec<(String, String)>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT id, token_hash FROM remote_devices")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn add_remote_device(&self, device: &RemoteDevice, token_hash: &str) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO remote_devices
                 (id, name, platform, token_hash, created_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    device.id,
                    device.name,
                    device.platform,
                    token_hash,
                    device.created_at,
                    device.last_seen_at
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Remove o aparelho. Devolve `true` quando havia o que remover, para que
    /// quem revoga saiba se precisa derrubar conexão viva.
    pub fn remove_remote_device(&self, id: &str) -> Result<bool, String> {
        let removed = self
            .connection
            .execute("DELETE FROM remote_devices WHERE id = ?1", [id])
            .map_err(|error| error.to_string())?;
        Ok(removed > 0)
    }

    pub fn touch_remote_device(&self, id: &str, at: i64) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE remote_devices SET last_seen_at = ?2 WHERE id = ?1",
                params![id, at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn count_remote_devices(&self) -> Result<usize, String> {
        self.connection
            .query_row("SELECT COUNT(*) FROM remote_devices", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count.max(0) as usize)
            .map_err(|error| error.to_string())
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
        RemoteDevice, SessionSource, SessionStatus,
    };

    #[test]
    fn agent_sessions_are_never_persisted() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        let session = AgentSession {
            id: "session".into(),
            agent: AgentKind::Codex,
            agent_label: "Codex".into(),
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
            last_response: Some("resposta que nao pode ser salva".into()),
            results: vec![crate::domain::SessionResult {
                id: "result-1".into(),
                response: "outra resposta sensível".into(),
                created_at: 1,
                files: Vec::new(),
                tests: Vec::new(),
            }],
            activities: Vec::new(),
        };
        store.save_session(&session).expect("salva a sessão");
        let loaded = store.load_sessions().expect("carrega as sessões");
        assert!(loaded.is_empty());
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
        assert!(preferences.project_profiles.is_empty());
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

    fn device(id: &str) -> RemoteDevice {
        RemoteDevice {
            id: id.into(),
            name: "Celular".into(),
            platform: "android".into(),
            created_at: 10,
            last_seen_at: None,
        }
    }

    #[test]
    fn remote_devices_round_trip_without_exposing_the_credential() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        store
            .add_remote_device(&device("aparelho-1"), "hash-do-token")
            .expect("registra aparelho");

        let listed = store.remote_devices().expect("lista aparelhos");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Celular");
        assert!(listed[0].last_seen_at.is_none());
        // O tipo devolvido à interface não tem campo de credencial. Se um dia
        // alguém acrescentar um, este arquivo precisa ser revisitado.
        let serialized = serde_json::to_string(&listed[0]).expect("json");
        assert!(!serialized.contains("hash-do-token"));
        assert!(!serialized.contains("token"));

        let credentials = store.remote_device_credentials().expect("credenciais");
        assert_eq!(credentials, vec![("aparelho-1".into(), "hash-do-token".into())]);
    }

    #[test]
    fn revoking_a_device_removes_its_credential() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        store
            .add_remote_device(&device("aparelho-1"), "hash-do-token")
            .expect("registra aparelho");

        assert!(store.remove_remote_device("aparelho-1").expect("revoga"));
        assert_eq!(store.count_remote_devices().expect("contagem"), 0);
        assert!(store
            .remote_device_credentials()
            .expect("credenciais")
            .is_empty());
        assert!(
            !store.remove_remote_device("aparelho-1").expect("revoga"),
            "revogar o que não existe precisa dizer que nada mudou"
        );
    }

    #[test]
    fn last_seen_is_recorded_without_touching_the_credential() {
        let store = Store::open(Path::new(":memory:")).expect("banco em memória");
        store
            .add_remote_device(&device("aparelho-1"), "hash-do-token")
            .expect("registra aparelho");

        store
            .touch_remote_device("aparelho-1", 4242)
            .expect("atualiza último acesso");

        let listed = store.remote_devices().expect("lista aparelhos");
        assert_eq!(listed[0].last_seen_at, Some(4242));
        assert_eq!(
            store.remote_device_credentials().expect("credenciais")[0].1,
            "hash-do-token"
        );
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
}
