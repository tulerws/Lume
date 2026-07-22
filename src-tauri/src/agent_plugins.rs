use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::integrations::IntegrationKind;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExternalAgentPlugin {
    pub schema_version: u16,
    pub id: String,
    pub name: String,
    pub executable: String,
    pub process_names: Vec<String>,
    pub command_tokens: Vec<String>,
}

impl Default for ExternalAgentPlugin {
    fn default() -> Self {
        Self {
            schema_version: 1,
            id: String::new(),
            name: String::new(),
            executable: String::new(),
            process_names: Vec::new(),
            command_tokens: Vec::new(),
        }
    }
}

pub trait AgentPlugin: Send + Sync {
    fn kind(&self) -> IntegrationKind;
    fn label(&self) -> &'static str;
    fn executable(&self) -> &'static str;
    fn direct_permissions(&self) -> bool;
    fn hook_events(&self) -> &'static [&'static str];
}

struct BuiltInAgentPlugin {
    kind: IntegrationKind,
    label: &'static str,
    executable: &'static str,
    direct_permissions: bool,
    hook_events: &'static [&'static str],
}

impl AgentPlugin for BuiltInAgentPlugin {
    fn kind(&self) -> IntegrationKind {
        self.kind.clone()
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn executable(&self) -> &'static str {
        self.executable
    }

    fn direct_permissions(&self) -> bool {
        self.direct_permissions
    }

    fn hook_events(&self) -> &'static [&'static str] {
        self.hook_events
    }
}

pub fn catalog() -> Vec<Box<dyn AgentPlugin>> {
    vec![
        Box::new(BuiltInAgentPlugin {
            kind: IntegrationKind::Codex,
            label: "Codex",
            executable: "codex",
            direct_permissions: true,
            hook_events: &[
                "SessionStart",
                "UserPromptSubmit",
                "PermissionRequest",
                "PostToolUse",
                "Stop",
            ],
        }),
        Box::new(BuiltInAgentPlugin {
            kind: IntegrationKind::Claude,
            label: "Claude",
            executable: "claude",
            direct_permissions: true,
            hook_events: &[
                "SessionStart",
                "UserPromptSubmit",
                "PermissionRequest",
                "PostToolUse",
                "PostToolUseFailure",
                "Notification",
                "Stop",
                "StopFailure",
                "SessionEnd",
            ],
        }),
        Box::new(BuiltInAgentPlugin {
            kind: IntegrationKind::Gemini,
            label: "Gemini",
            executable: "gemini",
            direct_permissions: false,
            hook_events: &[
                "SessionStart",
                "BeforeAgent",
                "Notification",
                "AfterTool",
                "AfterAgent",
                "SessionEnd",
            ],
        }),
    ]
}

pub fn find(kind: &IntegrationKind) -> Option<Box<dyn AgentPlugin>> {
    catalog().into_iter().find(|plugin| plugin.kind() == *kind)
}

pub fn external_catalog(app: &AppHandle) -> Vec<ExternalAgentPlugin> {
    let Ok(directory) = plugin_directory(app) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut plugins = entries
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .filter_map(|content| serde_json::from_str::<ExternalAgentPlugin>(&content).ok())
        .filter(|plugin| validate_external_plugin(plugin).is_ok())
        .collect::<Vec<_>>();
    plugins.sort_by(|left, right| left.name.cmp(&right.name));
    plugins
}

pub fn install_external(app: &AppHandle, source: &Path) -> Result<ExternalAgentPlugin, String> {
    let content = fs::read_to_string(source)
        .map_err(|error| format!("Não foi possível ler o manifesto: {error}"))?;
    let plugin = serde_json::from_str::<ExternalAgentPlugin>(&content)
        .map_err(|error| format!("Manifesto de plugin inválido: {error}"))?;
    validate_external_plugin(&plugin)?;
    let directory = plugin_directory(app)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let payload = serde_json::to_vec_pretty(&plugin).map_err(|error| error.to_string())?;
    fs::write(directory.join(format!("{}.json", plugin.id)), payload)
        .map_err(|error| error.to_string())?;
    Ok(plugin)
}

pub fn remove_external(app: &AppHandle, id: &str) -> Result<(), String> {
    validate_plugin_id(id)?;
    let path = plugin_directory(app)?.join(format!("{id}.json"));
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn plugin_directory(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("plugins"))
        .map_err(|error| error.to_string())
}

fn validate_external_plugin(plugin: &ExternalAgentPlugin) -> Result<(), String> {
    if plugin.schema_version != 1 {
        return Err("O manifesto precisa usar schemaVersion 1".into());
    }
    validate_plugin_id(&plugin.id)?;
    if plugin.name.trim().is_empty() || plugin.name.chars().count() > 80 {
        return Err("O plugin precisa de um nome com até 80 caracteres".into());
    }
    if plugin.executable.trim().is_empty() {
        return Err("O plugin precisa informar o executável observado".into());
    }
    if plugin.process_names.is_empty() && plugin.command_tokens.is_empty() {
        return Err("Informe processNames ou commandTokens para detectar o agente".into());
    }
    if plugin
        .process_names
        .iter()
        .chain(plugin.command_tokens.iter())
        .any(|value| value.trim().is_empty() || value.len() > 160)
    {
        return Err("Os identificadores de processo do plugin são inválidos".into());
    }
    Ok(())
}

fn validate_plugin_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id.bytes().all(|value| {
            value.is_ascii_lowercase() || value.is_ascii_digit() || b"-_".contains(&value)
        })
    {
        return Err("O id do plugin deve usar apenas letras minúsculas, números, - ou _".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_plugins_expose_their_monitoring_contract() {
        let plugins = catalog();
        assert_eq!(plugins.len(), 3);
        let claude = plugins
            .iter()
            .find(|plugin| plugin.kind() == IntegrationKind::Claude)
            .expect("plugin Claude");
        assert!(claude.direct_permissions());
        assert!(claude.hook_events().contains(&"PermissionRequest"));
        assert!(claude.hook_events().contains(&"PostToolUse"));

        let codex = plugins
            .iter()
            .find(|plugin| plugin.kind() == IntegrationKind::Codex)
            .expect("plugin Codex");
        assert!(codex.hook_events().contains(&"PostToolUse"));

        let gemini = plugins
            .iter()
            .find(|plugin| plugin.kind() == IntegrationKind::Gemini)
            .expect("plugin Gemini");
        assert!(gemini.hook_events().contains(&"AfterTool"));
    }

    #[test]
    fn external_plugin_manifest_rejects_executable_code_without_detection_markers() {
        let plugin = ExternalAgentPlugin {
            id: "local-agent".into(),
            name: "Local Agent".into(),
            executable: "local-agent".into(),
            ..ExternalAgentPlugin::default()
        };
        assert!(validate_external_plugin(&plugin).is_err());
    }
}
