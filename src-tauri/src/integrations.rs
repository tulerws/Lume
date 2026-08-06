use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Command,
    time::UNIX_EPOCH,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::domain::SessionActivity;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
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
pub struct ResumableSession {
    pub id: String,
    pub agent: IntegrationKind,
    pub name: String,
    pub project: String,
    pub working_directory: String,
    pub source: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub struct ResumePreview {
    pub response: String,
    pub updated_at: i64,
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

pub fn resumable_sessions(kind: &IntegrationKind) -> Result<Vec<ResumableSession>, String> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "Could not find the user home directory".to_string())?;
    let mut sessions = match kind {
        IntegrationKind::Codex => {
            let root = env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("sessions");
            codex_resumable_sessions(&root)
        }
        IntegrationKind::Claude => claude_resumable_sessions(&home.join(".claude/projects")),
        IntegrationKind::Gemini => Vec::new(),
    };
    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    sessions.truncate(250);
    Ok(sessions)
}

pub fn resume_preview(kind: &IntegrationKind, session_id: &str) -> Option<ResumePreview> {
    let path = resume_path(kind, session_id)?;
    let file = fs::File::open(&path).ok()?;
    let response = match kind {
        IntegrationKind::Codex => codex_last_response(BufReader::new(file)),
        IntegrationKind::Claude => claude_last_response(BufReader::new(file)),
        IntegrationKind::Gemini => None,
    }?;
    Some(ResumePreview {
        response,
        updated_at: file_updated_at(&path),
    })
}

pub fn resume_work_activities(kind: &IntegrationKind, session_id: &str) -> Vec<SessionActivity> {
    let Some(path) = resume_path(kind, session_id) else {
        return Vec::new();
    };
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    match kind {
        IntegrationKind::Codex => codex_work_activities(BufReader::new(file), session_id),
        IntegrationKind::Claude => claude_work_activities(BufReader::new(file), session_id),
        IntegrationKind::Gemini => Vec::new(),
    }
}

fn resume_path(kind: &IntegrationKind, session_id: &str) -> Option<PathBuf> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let filename_suffix = format!("-{session_id}");
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    let root = match kind {
        IntegrationKind::Codex => env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("sessions"),
        IntegrationKind::Claude => home.join(".claude/projects"),
        IntegrationKind::Gemini => return None,
    };
    resume_files(&root).into_iter().find(|path| {
        path.file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|stem| stem == session_id || stem.ends_with(&filename_suffix))
    })
}

fn codex_resumable_sessions(root: &Path) -> Vec<ResumableSession> {
    let names = codex_session_names(root);
    resume_files(root)
        .into_iter()
        .filter_map(|path| {
            let file = fs::File::open(&path).ok()?;
            let first_line = BufReader::new(file).lines().next()?.ok()?;
            let value = serde_json::from_str::<Value>(&first_line).ok()?;
            let mut session = codex_resume_metadata(&value, file_updated_at(&path))?;
            if let Some(name) = names.get(&session.id) {
                session.name = name.clone();
            }
            Some(session)
        })
        .collect()
}

fn codex_session_names(root: &Path) -> HashMap<String, String> {
    let Some(index_path) = root.parent().map(|home| home.join("session_index.jsonl")) else {
        return HashMap::new();
    };
    let Ok(file) = fs::File::open(index_path) else {
        return HashMap::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter_map(|value| codex_session_name_entry(&value))
        .collect()
}

fn codex_session_name_entry(value: &Value) -> Option<(String, String)> {
    let id = value.get("id")?.as_str()?.trim();
    let name = value.get("thread_name")?.as_str()?.trim();
    (!id.is_empty() && !name.is_empty()).then(|| (id.to_string(), name.to_string()))
}

fn codex_resume_metadata(value: &Value, updated_at: i64) -> Option<ResumableSession> {
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = value.get("payload")?;
    if payload
        .get("parent_thread_id")
        .is_some_and(|value| !value.is_null())
        || payload.get("thread_source").and_then(Value::as_str) == Some("subagent")
        || payload.pointer("/source/subagent").is_some()
    {
        return None;
    }
    let id = payload.get("id")?.as_str()?.to_string();
    let working_directory = payload.get("cwd")?.as_str()?.to_string();
    if crate::session_filters::is_codex_internal_workspace(&working_directory) {
        return None;
    }
    let source = match (
        payload.get("originator").and_then(Value::as_str),
        payload.get("source").and_then(Value::as_str),
    ) {
        (Some("codex_vscode"), _) | (_, Some("vscode")) => "VS Code",
        _ => "CLI",
    }
    .to_string();
    let project = resume_project_name(&working_directory);
    Some(ResumableSession {
        id,
        agent: IntegrationKind::Codex,
        name: project.clone(),
        project,
        working_directory,
        source,
        updated_at,
    })
}

fn claude_resumable_sessions(root: &Path) -> Vec<ResumableSession> {
    let mut seen = HashSet::new();
    resume_files(root)
        .into_iter()
        .filter_map(|path| {
            let file = fs::File::open(&path).ok()?;
            let mut bytes_read = 0usize;
            let mut identity: Option<(String, String)> = None;
            let mut name = None;
            for line in BufReader::new(file).lines().take(128) {
                let line = line.ok()?;
                bytes_read = bytes_read.saturating_add(line.len());
                if bytes_read > 256 * 1024 {
                    break;
                }
                let value = serde_json::from_str::<Value>(&line).ok()?;
                if value.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                if identity.is_none() {
                    if let (Some(id), Some(working_directory)) = (
                        value.get("sessionId").and_then(Value::as_str),
                        value.get("cwd").and_then(Value::as_str),
                    ) {
                        identity = Some((id.to_string(), working_directory.to_string()));
                    }
                }
                name = name.or_else(|| claude_session_name(&value));
                if identity.is_some() && name.is_some() {
                    break;
                }
            }
            let (id, working_directory) = identity?;
            if !seen.insert(id.clone()) {
                return None;
            }
            let project = resume_project_name(&working_directory);
            Some(ResumableSession {
                id,
                agent: IntegrationKind::Claude,
                name: name.unwrap_or_else(|| project.clone()),
                project,
                working_directory,
                source: "CLI".into(),
                updated_at: file_updated_at(&path),
            })
        })
        .collect()
}

fn claude_session_name(value: &Value) -> Option<String> {
    let content = value.pointer("/message/content")?.as_str()?.trim();
    if content.is_empty() || content.starts_with('/') || content.starts_with("<command-") {
        return None;
    }
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let name = chars.by_ref().take(72).collect::<String>();
    Some(if chars.next().is_some() {
        format!("{name}…")
    } else {
        name
    })
}

fn codex_work_activities(reader: impl BufRead, session_id: &str) -> Vec<SessionActivity> {
    let mut activities = Vec::new();
    let mut pending_goals = HashMap::<String, (String, Option<String>, i64)>::new();
    for (index, line) in reader.lines().map_while(Result::ok).enumerate() {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = record.get("payload") else {
            continue;
        };
        let created_at = record_timestamp(&record).unwrap_or(index as i64);
        match payload.get("type").and_then(Value::as_str) {
            Some("function_call" | "custom_tool_call") => {
                let Some(name) = payload.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let call_id = payload
                    .get("call_id")
                    .or_else(|| payload.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or(name);
                let input = payload
                    .get("arguments")
                    .and_then(json_text)
                    .or_else(|| payload.get("input").and_then(json_text));
                if name == "update_plan" {
                    activities.push(work_activity(
                        format!("codex:{session_id}:plan:{call_id}"),
                        "plan",
                        "Plan updated",
                        input,
                        created_at,
                    ));
                } else if matches!(name, "create_goal" | "get_goal" | "update_goal") {
                    pending_goals
                        .insert(call_id.to_string(), (name.to_string(), input, created_at));
                }
            }
            Some("function_call_output" | "custom_tool_call_output") => {
                let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some((name, input, started_at)) = pending_goals.remove(call_id) else {
                    continue;
                };
                let output = payload.get("output").and_then(json_text);
                let detail = if name == "get_goal" {
                    output.or(input)
                } else {
                    input.or(output)
                };
                activities.push(work_activity(
                    format!("codex:{session_id}:goal:{call_id}"),
                    "tool",
                    format!("functions · {name}"),
                    detail,
                    created_at.max(started_at),
                ));
            }
            _ => {}
        }
    }
    activities
}

fn claude_work_activities(reader: impl BufRead, session_id: &str) -> Vec<SessionActivity> {
    let mut activities = Vec::new();
    for (line_index, line) in reader.lines().map_while(Result::ok).enumerate() {
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let created_at = record_timestamp(&entry).unwrap_or(line_index as i64);
        let Some(blocks) = entry.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for (block_index, block) in blocks.iter().enumerate() {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let Some(name) = block.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !name.to_ascii_lowercase().contains("todo") {
                continue;
            }
            let detail = block.get("input").and_then(json_text);
            let id = block
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{line_index}:{block_index}"));
            activities.push(work_activity(
                format!("claude:{session_id}:todo:{id}"),
                "tool",
                name,
                detail,
                created_at,
            ));
        }
    }
    activities
}

fn work_activity(
    id: String,
    kind: &str,
    title: impl Into<String>,
    detail: Option<String>,
    created_at: i64,
) -> SessionActivity {
    SessionActivity {
        id,
        kind: kind.into(),
        title: title.into(),
        detail,
        status: "completed".into(),
        created_at,
        files: Vec::new(),
        attachments: Vec::new(),
        append_detail: false,
    }
}

fn json_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        Value::Null => None,
        value => Some(value.to_string()),
    }
}

fn record_timestamp(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.timestamp_millis())
}

fn codex_last_response(reader: impl BufRead) -> Option<String> {
    let mut last_message = None;
    let mut last_final = None;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some((message, is_final)) = codex_agent_message(&value) else {
            continue;
        };
        last_message = Some(message.clone());
        if is_final {
            last_final = Some(message);
        }
    }
    last_final.or(last_message)
}

fn codex_agent_message(value: &Value) -> Option<(String, bool)> {
    let payload = value.get("payload")?;
    let phase = payload.get("phase").and_then(Value::as_str);
    let is_final = matches!(phase, Some("final" | "final_answer"));
    let message = match (
        value.get("type").and_then(Value::as_str),
        payload.get("type").and_then(Value::as_str),
    ) {
        (Some("event_msg"), Some("agent_message")) => {
            payload.get("message").and_then(Value::as_str)?.to_string()
        }
        (Some("response_item"), Some("message"))
            if payload.get("role").and_then(Value::as_str) == Some("assistant") =>
        {
            content_text(payload.get("content")?, "output_text")?
        }
        _ => return None,
    };
    non_empty_response(message).map(|message| (message, is_final))
}

fn claude_last_response(reader: impl BufRead) -> Option<String> {
    let mut last_message = None;
    let mut last_final = None;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if value.get("type").and_then(Value::as_str) == Some("result") {
            if let Some(message) = value
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_string)
                .and_then(non_empty_response)
            {
                last_final = Some(message);
            }
            continue;
        }
        if value.get("type").and_then(Value::as_str) != Some("assistant")
            || value.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
        {
            continue;
        }
        if let Some(message) = value
            .pointer("/message/content")
            .and_then(|content| content_text(content, "text"))
            .and_then(non_empty_response)
        {
            last_message = Some(message);
        }
    }
    last_final.or(last_message)
}

fn content_text(content: &Value, block_type: &str) -> Option<String> {
    let text = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some(block_type))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    non_empty_response(text)
}

fn non_empty_response(response: String) -> Option<String> {
    let response = response.trim().to_string();
    (!response.is_empty()).then_some(response)
}

fn resume_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_resume_files(root, &mut files);
    files
}

fn collect_resume_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_resume_files(&path, files);
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("jsonl")
        {
            files.push(path);
        }
    }
}

fn file_updated_at(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|updated| updated.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn resume_project_name(working_directory: &str) -> String {
    working_directory
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(working_directory)
        .to_string()
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
    let interactive_claude_hook =
        *kind == IntegrationKind::Claude && matches!(event, "PermissionRequest" | "PreToolUse");
    let timeout = if interactive_claude_hook { 900 } else { 10 };
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
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("where.exe")
        .arg(command)
        .creation_flags(CREATE_NO_WINDOW)
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
    fn codex_resume_metadata_excludes_subagents_and_keeps_project_context() {
        let session = codex_resume_metadata(
            &json!({
                "type": "session_meta",
                "payload": {
                    "id": "thread-1",
                    "cwd": "/work/lume",
                    "originator": "codex_vscode",
                    "source": "vscode"
                }
            }),
            42,
        )
        .expect("sessão retomável");
        assert_eq!(session.name, "lume");
        assert_eq!(session.project, "lume");
        assert_eq!(session.source, "VS Code");
        assert_eq!(session.updated_at, 42);

        assert!(codex_resume_metadata(
            &json!({
                "type": "session_meta",
                "payload": {
                    "id": "thread-child",
                    "cwd": "/work/lume",
                    "source": { "subagent": { "other": "worker" } }
                }
            }),
            43,
        )
        .is_none());
        assert!(codex_resume_metadata(
            &json!({
                "type": "session_meta",
                "payload": {
                    "id": "memory-thread",
                    "cwd": "/home/user/.codex/memories",
                    "originator": "codex-tui",
                    "source": "cli"
                }
            }),
            44,
        )
        .is_none());
    }

    #[test]
    fn codex_session_index_exposes_the_saved_thread_name() {
        let entry = codex_session_name_entry(&json!({
            "id": "thread-1",
            "thread_name": "Lume principal"
        }))
        .expect("nome salvo");

        assert_eq!(entry, ("thread-1".into(), "Lume principal".into()));
    }

    #[test]
    fn claude_resume_name_uses_the_first_meaningful_prompt() {
        assert!(claude_session_name(&json!({
            "message": { "content": "/plan" }
        }))
        .is_none());
        assert_eq!(
            claude_session_name(&json!({
                "message": { "content": "  Revise   o fluxo de autenticação  " }
            }))
            .as_deref(),
            Some("Revise o fluxo de autenticação")
        );
    }

    #[test]
    fn codex_resume_preview_prefers_the_latest_final_answer() {
        let transcript = concat!(
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"phase\":\"commentary\",\"message\":\"Analisando\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Tudo pronto.\"}]}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"phase\":\"commentary\",\"message\":\"Novo trabalho iniciado\"}}\n",
        );

        assert_eq!(
            codex_last_response(std::io::Cursor::new(transcript)).as_deref(),
            Some("Tudo pronto.")
        );
    }

    #[test]
    fn claude_resume_preview_reads_the_last_agent_text() {
        let transcript = concat!(
            "{\"type\":\"assistant\",\"isSidechain\":false,\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"internal\"}]}}\n",
            "{\"type\":\"assistant\",\"isSidechain\":false,\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Resposta anterior do Claude.\"}]}}\n",
        );

        assert_eq!(
            claude_last_response(std::io::Cursor::new(transcript)).as_deref(),
            Some("Resposta anterior do Claude.")
        );
    }

    #[test]
    fn codex_work_state_is_rebuilt_from_plan_and_goal_records() {
        let transcript = concat!(
            "{\"timestamp\":\"2026-07-30T12:00:00Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"update_plan\",\"call_id\":\"plan-1\",\"arguments\":\"{\\\"explanation\\\":\\\"Phase 1\\\",\\\"plan\\\":[{\\\"step\\\":\\\"Test handoff\\\",\\\"status\\\":\\\"completed\\\"}]}\"}}\n",
            "{\"timestamp\":\"2026-07-30T12:01:00Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"get_goal\",\"call_id\":\"goal-1\",\"arguments\":\"{}\"}}\n",
            "{\"timestamp\":\"2026-07-30T12:01:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"goal-1\",\"output\":\"{\\\"goal\\\":{\\\"objective\\\":\\\"Workflow\\\",\\\"status\\\":\\\"active\\\",\\\"createdAt\\\":1785400000}}\"}}\n",
        );
        let activities = codex_work_activities(std::io::Cursor::new(transcript), "thread-1");
        assert_eq!(activities.len(), 2);
        assert_eq!(activities[0].kind, "plan");
        assert!(activities[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Test handoff")));
        assert_eq!(activities[1].title, "functions · get_goal");
        assert!(activities[1]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Workflow")));
    }

    #[test]
    fn claude_work_state_is_rebuilt_from_todo_write() {
        let transcript = "{\"timestamp\":\"2026-07-30T12:00:00Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"todo-1\",\"name\":\"TodoWrite\",\"input\":{\"todos\":[{\"content\":\"Inspect workflow\",\"status\":\"in_progress\"}]}}]}}\n";
        let activities = claude_work_activities(std::io::Cursor::new(transcript), "session-1");
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].title, "TodoWrite");
        assert!(activities[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Inspect workflow")));
    }

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
    fn claude_question_hook_waits_for_the_lume_response() {
        let mut hooks = json!({});
        add_handler(
            &mut hooks,
            "PreToolUse",
            &IntegrationKind::Claude,
            "/opt/Lume/lume",
        )
        .expect("adiciona o hook");
        assert_eq!(hooks["PreToolUse"][0]["hooks"][0]["timeout"], 900);
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
