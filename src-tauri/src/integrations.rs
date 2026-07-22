use std::{env, fs, path::PathBuf, process::Command};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    Codex,
    Claude,
    Gemini,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    pub kind: IntegrationKind,
    pub label: String,
    pub installed: bool,
    pub configured: bool,
    pub direct_permissions: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionStatus {
    pub installed: bool,
    pub configured: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCheck {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationDiagnostic {
    pub kind: IntegrationKind,
    pub label: String,
    pub healthy: bool,
    pub checks: Vec<DiagnosticCheck>,
    pub last_event_at: Option<i64>,
}

pub fn lume_executable() -> Result<PathBuf, String> {
    if let Some(app_image) = env::var_os("APPIMAGE").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(app_image));
    }
    std::env::current_exe().map_err(|error| error.to_string())
}

pub fn statuses(executable: &str) -> Vec<IntegrationStatus> {
    crate::agent_plugins::catalog()
        .into_iter()
        .map(|plugin| {
            let kind = plugin.kind();
            let installed = crate::executables::available(plugin.executable());
            let configured = config_path(&kind)
                .and_then(|path| fs::read_to_string(path).ok())
                .is_some_and(|content| configured_content(&content, &kind, executable));
            let detail = if !installed {
                "CLI não encontrada".into()
            } else if configured {
                if kind == IntegrationKind::Codex {
                    "Hook conectado; /hooks está disponível no Codex CLI".into()
                } else if plugin.direct_permissions() {
                    "Monitoramento e decisões conectados".into()
                } else {
                    "Monitoramento conectado".into()
                }
            } else if kind == IntegrationKind::Codex {
                "Decisões diretas ao abrir uma sessão pelo Lume".into()
            } else {
                "Pronto para conectar".into()
            };
            IntegrationStatus {
                kind,
                label: plugin.label().into(),
                installed,
                configured,
                direct_permissions: plugin.direct_permissions(),
                detail,
            }
        })
        .collect()
}

pub fn diagnose(
    kind: &IntegrationKind,
    executable: &str,
    last_event_at: Option<i64>,
) -> Result<IntegrationDiagnostic, String> {
    let plugin =
        crate::agent_plugins::find(kind).ok_or_else(|| "Integração não reconhecida".to_string())?;
    let mut checks = Vec::new();
    let executable_path = crate::executables::path(plugin.executable());
    checks.push(DiagnosticCheck {
        id: "cli".into(),
        label: "CLI".into(),
        status: if executable_path.is_some() {
            "ok"
        } else {
            "error"
        }
        .into(),
        detail: executable_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{} não encontrado", plugin.executable())),
    });

    if executable_path.is_some() {
        let version = crate::executables::command(plugin.executable())
            .and_then(|mut command| {
                command
                    .arg("--version")
                    .output()
                    .map_err(|error| error.to_string())
            })
            .ok()
            .and_then(|output| {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                (!stdout.is_empty())
                    .then_some(stdout)
                    .or((!stderr.is_empty()).then_some(stderr))
            });
        checks.push(DiagnosticCheck {
            id: "version".into(),
            label: "Versão".into(),
            status: if version.is_some() { "ok" } else { "warning" }.into(),
            detail: version.unwrap_or_else(|| "Não foi possível consultar a versão".into()),
        });
    }

    let hook_path = config_path(kind);
    let configured = hook_path
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|content| configured_content(&content, kind, executable));
    checks.push(DiagnosticCheck {
        id: "hooks".into(),
        label: "Monitoramento".into(),
        status: if configured { "ok" } else { "warning" }.into(),
        detail: if configured {
            format!("{} eventos configurados", plugin.hook_events().len())
        } else {
            "Hook do Lume ainda não conectado".into()
        },
    });
    checks.push(DiagnosticCheck {
        id: "activity".into(),
        label: "Último evento".into(),
        status: if last_event_at.is_some() {
            "ok"
        } else {
            "warning"
        }
        .into(),
        detail: last_event_at
            .map(|timestamp| timestamp.to_string())
            .unwrap_or_else(|| "Nenhum evento recebido nesta execução".into()),
    });
    let healthy = checks.iter().all(|check| check.status != "error");
    Ok(IntegrationDiagnostic {
        kind: plugin.kind(),
        label: plugin.label().into(),
        healthy,
        checks,
        last_event_at,
    })
}

pub fn configure(kind: &IntegrationKind, executable: &str, enabled: bool) -> Result<(), String> {
    if enabled && *kind == IntegrationKind::Codex {
        ensure_codex_hooks_enabled()?;
    }
    let path =
        config_path(kind).ok_or_else(|| "Diretório do usuário não encontrado".to_string())?;
    let mut root = read_config(&path)?;
    if !root.is_object() {
        return Err(format!(
            "A configuração {} não contém um objeto JSON",
            path.display()
        ));
    }
    let hooks = root
        .as_object_mut()
        .expect("validado acima")
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if !hooks.is_object() {
        return Err("A chave hooks existente não contém um objeto".into());
    }

    for event in events(kind) {
        remove_lume_handlers(hooks, event, kind, executable);
        if enabled {
            add_handler(hooks, event, kind, executable)?;
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if path.exists() {
        let backup = path.with_extension("lume-backup.json");
        if !backup.exists() {
            fs::copy(&path, &backup).map_err(|error| error.to_string())?;
        }
    }
    let payload = serde_json::to_string_pretty(&root).map_err(|error| error.to_string())?;
    fs::write(&path, format!("{payload}\n")).map_err(|error| error.to_string())
}

pub fn refresh_connected(executable: &str) {
    for plugin in crate::agent_plugins::catalog() {
        let kind = plugin.kind();
        let Some(path) = config_path(&kind) else {
            continue;
        };
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        if has_lume_handler(&content, &kind) {
            let _ = configure(&kind, executable, true);
        }
    }
}

pub fn vscode_status() -> CompanionStatus {
    let installed = command_available("code");
    let configured = installed
        && code_command()
            .arg("--list-extensions")
            .output()
            .ok()
            .is_some_and(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|extension| extension.eq_ignore_ascii_case("tulerws.lume"))
            });
    CompanionStatus {
        installed,
        configured,
        detail: if !installed {
            "VS Code não encontrado".into()
        } else if configured {
            "Terminal integrado conectado".into()
        } else {
            "Necessário para abrir sessões no editor".into()
        },
    }
}

pub fn configure_vscode(enabled: bool, vsix_path: &std::path::Path) -> Result<(), String> {
    let mut command = code_command();
    if enabled {
        if !vsix_path.exists() {
            return Err("O companion do VS Code não foi incluído no aplicativo".into());
        }
        command
            .arg("--install-extension")
            .arg(vsix_path)
            .arg("--force");
    } else {
        command.arg("--uninstall-extension").arg("tulerws.lume");
    }
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn add_handler(
    hooks: &mut Value,
    event: &str,
    kind: &IntegrationKind,
    executable: &str,
) -> Result<(), String> {
    let groups = hooks
        .as_object_mut()
        .expect("hooks validado")
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()));
    let groups = groups
        .as_array_mut()
        .ok_or_else(|| format!("A configuração do evento {event} não contém uma lista"))?;
    let provider = provider(kind);
    let timeout = if event == "PermissionRequest" {
        900
    } else {
        10
    };
    let status_message = if event == "PermissionRequest" {
        "Aguardando decisão no Lume"
    } else {
        "Sincronizando com o Lume"
    };
    let handler = match kind {
        IntegrationKind::Claude => json!({
            "type": "command",
            "name": "Lume",
            "command": executable,
            "args": ["hook", provider],
            "timeout": timeout,
            "statusMessage": status_message
        }),
        IntegrationKind::Gemini => json!({
            "type": "command",
            "name": "Lume",
            "command": shell_command(executable, provider),
            "timeout": timeout * 1_000,
            "description": "Envia o estado da sessão ao Lume"
        }),
        IntegrationKind::Codex => json!({
            "type": "command",
            "command": shell_command(executable, provider),
            "commandWindows": powershell_command(executable, provider),
            "timeout": timeout,
            "statusMessage": "Lume monitor"
        }),
    };
    let matcher = if matches!(event, "SessionStart" | "PermissionRequest" | "Notification") {
        json!("*")
    } else {
        Value::Null
    };
    let mut group = Map::new();
    if !matcher.is_null() {
        group.insert("matcher".into(), matcher);
    }
    group.insert("hooks".into(), Value::Array(vec![handler]));
    groups.push(Value::Object(group));
    Ok(())
}

fn remove_lume_handlers(hooks: &mut Value, event: &str, kind: &IntegrationKind, executable: &str) {
    let Some(groups) = hooks
        .as_object_mut()
        .and_then(|hooks| hooks.get_mut(event))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let provider_marker = marker(kind, executable);
    for group in groups.iter_mut() {
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        handlers.retain(|handler| {
            handler.get("name").and_then(Value::as_str) != Some("Lume")
                && handler.get("statusMessage").and_then(Value::as_str) != Some("Lume monitor")
                && !handler
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command.contains(&provider_marker))
        });
    }
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_none_or(|handlers| !handlers.is_empty())
    });
}

fn read_config(path: &PathBuf) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(content) if !content.trim().is_empty() => {
            serde_json::from_str(&content).map_err(|error| {
                format!(
                    "A configuração {} contém JSON inválido: {error}",
                    path.display()
                )
            })
        }
        Ok(_) => Ok(Value::Object(Map::new())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(error) => Err(error.to_string()),
    }
}

fn events(kind: &IntegrationKind) -> &'static [&'static str] {
    crate::agent_plugins::find(kind)
        .map(|plugin| plugin.hook_events())
        .unwrap_or_default()
}

fn config_path(kind: &IntegrationKind) -> Option<PathBuf> {
    let user_home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    let directory = match kind {
        IntegrationKind::Codex => ".codex/hooks.json",
        IntegrationKind::Claude => ".claude/settings.json",
        IntegrationKind::Gemini => ".gemini/settings.json",
    };
    Some(PathBuf::from(user_home).join(directory))
}

fn codex_user_config_path() -> Option<PathBuf> {
    let user_home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(user_home).join(".codex/config.toml"))
}

fn ensure_codex_hooks_enabled() -> Result<(), String> {
    let path = codex_user_config_path()
        .ok_or_else(|| "Diretório de configuração do Codex não encontrado".to_string())?;
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    let Some(updated) = config_with_hooks_enabled(&content)? else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if path.exists() {
        let backup = path.with_extension("lume-backup.toml");
        if !backup.exists() {
            fs::copy(&path, &backup).map_err(|error| error.to_string())?;
        }
    }
    fs::write(path, updated).map_err(|error| error.to_string())
}

fn config_with_hooks_enabled(content: &str) -> Result<Option<String>, String> {
    let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    if let Some(features_index) = lines.iter().position(|line| line.trim() == "[features]") {
        let section_end = lines
            .iter()
            .enumerate()
            .skip(features_index + 1)
            .find(|(_, line)| line.trim().starts_with('['))
            .map(|(index, _)| index)
            .unwrap_or(lines.len());
        if let Some(line) = lines[features_index + 1..section_end].iter().find(|line| {
            line.split_once('=')
                .is_some_and(|(key, _)| key.trim() == "hooks")
        }) {
            let value = line
                .split_once('=')
                .map(|(_, value)| value.split('#').next().unwrap_or_default().trim())
                .unwrap_or_default();
            return match value {
                "true" => Ok(None),
                "false" => Err(
                    "Os hooks estão desativados em ~/.codex/config.toml; ative features.hooks para conectar o Lume"
                        .into(),
                ),
                _ => Err("A opção features.hooks do Codex não é válida".into()),
            };
        }
        lines.insert(features_index + 1, "hooks = true".into());
    } else {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.extend(["[features]".into(), "hooks = true".into()]);
    }
    Ok(Some(format!("{}\n", lines.join("\n"))))
}

fn provider(kind: &IntegrationKind) -> &'static str {
    match kind {
        IntegrationKind::Codex => "codex",
        IntegrationKind::Claude => "claude",
        IntegrationKind::Gemini => "gemini",
    }
}

fn marker(kind: &IntegrationKind, executable: &str) -> String {
    format!("{} hook {}", executable, provider(kind))
}

fn configured_content(content: &str, kind: &IntegrationKind, executable: &str) -> bool {
    let Ok(root) = serde_json::from_str::<Value>(content) else {
        return false;
    };
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    hooks.values().any(|groups| {
        groups.as_array().is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.iter().any(|handler| {
                            let command =
                                handler.get("command").and_then(Value::as_str).unwrap_or("");
                            let exec_form = command == executable
                                && handler.get("args").and_then(Value::as_array).is_some_and(
                                    |args| {
                                        args.first().and_then(Value::as_str) == Some("hook")
                                            && args.get(1).and_then(Value::as_str)
                                                == Some(provider(kind))
                                    },
                                );
                            let shell_form = command.contains(executable)
                                && command.contains(&format!(" hook {}", provider(kind)));
                            let windows_form = handler
                                .get("commandWindows")
                                .and_then(Value::as_str)
                                .is_some_and(|command| {
                                    command.contains(executable)
                                        && command.contains(&format!(" hook {}", provider(kind)))
                                });
                            exec_form || shell_form || windows_form
                        })
                    })
            })
        })
    })
}

fn has_lume_handler(content: &str, kind: &IntegrationKind) -> bool {
    let Ok(root) = serde_json::from_str::<Value>(content) else {
        return false;
    };
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    let provider_suffix = format!(" hook {}", provider(kind));
    hooks.values().any(|groups| {
        groups.as_array().is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.iter().any(|handler| {
                            handler.get("name").and_then(Value::as_str) == Some("Lume")
                                || handler.get("statusMessage").and_then(Value::as_str)
                                    == Some("Lume monitor")
                                || handler
                                    .get("command")
                                    .and_then(Value::as_str)
                                    .is_some_and(|command| command.contains(&provider_suffix))
                        })
                    })
            })
        })
    })
}

fn shell_command(executable: &str, provider: &str) -> String {
    format!("\"{}\" hook {provider}", executable.replace('"', "\\\""))
}

fn powershell_command(executable: &str, provider: &str) -> String {
    format!("& '{}' hook {provider}", executable.replace('\'', "''"))
}

#[cfg(not(target_os = "windows"))]
fn command_available(command: &str) -> bool {
    Command::new(command).arg("--version").output().is_ok()
}

#[cfg(target_os = "windows")]
fn command_available(command: &str) -> bool {
    Command::new("where.exe")
        .arg(command)
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn code_command() -> Command {
    Command::new("code")
}

#[cfg(target_os = "windows")]
pub(crate) fn code_command() -> Command {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("cmd.exe");
    command
        .args(["/D", "/S", "/C", "code"])
        .creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_and_removing_lume_keeps_existing_hooks() {
        let mut hooks = json!({
            "Stop": [{
                "hooks": [{ "type": "command", "command": "notify-existing" }]
            }]
        });
        add_handler(
            &mut hooks,
            "Stop",
            &IntegrationKind::Claude,
            "/opt/Lume/lume",
        )
        .expect("adiciona o hook");
        assert_eq!(
            hooks["Stop"].as_array().expect("grupos").len(),
            2,
            "o hook existente deve ser preservado"
        );

        remove_lume_handlers(
            &mut hooks,
            "Stop",
            &IntegrationKind::Claude,
            "/opt/Lume/lume",
        );
        assert_eq!(hooks["Stop"].as_array().expect("grupos").len(), 1);
        assert_eq!(hooks["Stop"][0]["hooks"][0]["command"], "notify-existing");
    }

    #[test]
    fn hook_commands_keep_executable_paths_as_single_arguments() {
        let mut hooks = json!({});
        add_handler(
            &mut hooks,
            "PermissionRequest",
            &IntegrationKind::Claude,
            "/opt/Lume App/lume",
        )
        .expect("adiciona o hook");
        let handler = &hooks["PermissionRequest"][0]["hooks"][0];
        assert_eq!(handler["command"], "/opt/Lume App/lume");
        assert_eq!(handler["args"], json!(["hook", "claude"]));
        let root = json!({ "hooks": hooks });
        assert!(configured_content(
            &root.to_string(),
            &IntegrationKind::Claude,
            "/opt/Lume App/lume"
        ));
    }

    #[test]
    fn recognizes_connected_lume_hooks_from_an_older_executable() {
        let root = json!({
            "hooks": {
                "PermissionRequest": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "name": "Lume",
                        "command": "/old/Lume/lume",
                        "args": ["hook", "claude"]
                    }]
                }]
            }
        });

        assert!(has_lume_handler(
            &root.to_string(),
            &IntegrationKind::Claude
        ));
    }

    #[test]
    fn quoted_codex_command_is_recognized_as_connected() {
        let root = json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "type": "command",
                        "command": "\"/usr/bin/lume\" hook codex",
                        "statusMessage": "Lume monitor"
                    }]
                }]
            }
        });

        assert!(configured_content(
            &root.to_string(),
            &IntegrationKind::Codex,
            "/usr/bin/lume"
        ));
    }

    #[test]
    fn enables_codex_hooks_without_replacing_other_features() {
        let content = "model = \"gpt\"\n[features]\nmemories = true\n\n[projects.test]\ntrust_level = \"trusted\"\n";
        let updated = config_with_hooks_enabled(content)
            .expect("configuração válida")
            .expect("mudança necessária");
        assert!(updated.contains("[features]\nhooks = true\nmemories = true"));
        assert!(updated.contains("[projects.test]"));
        assert!(config_with_hooks_enabled(&updated)
            .expect("configuração válida")
            .is_none());
    }

    #[test]
    fn respects_an_explicit_codex_hooks_disable() {
        let result = config_with_hooks_enabled("[features]\nhooks = false\n");
        assert!(result.is_err());
    }
}
