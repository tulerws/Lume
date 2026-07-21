use std::io::Read;

use serde_json::{json, Value};
use sysinfo::{get_current_pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::{
    domain::{
        AccessMode, AgentKind, HookEvent, HookEventKind, PermissionAction, PermissionProfile,
        PermissionRequest, SessionSource,
    },
    event_server,
    state::now_millis,
};

pub fn run_hook(provider: &str) -> i32 {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 0;
    }
    let raw: Value = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    let event = match map_event(provider, &raw) {
        Some(event) => event,
        None => return 0,
    };
    let payload = match serde_json::to_string(&event) {
        Ok(payload) => payload,
        Err(_) => return 0,
    };
    let response = match event_server::send_event(&payload) {
        Ok(response) => response,
        // A origem mantém seu fluxo nativo quando o Lume está fechado.
        Err(_) => return 0,
    };

    if provider == "claude" && event.wait_for_decision {
        let output = claude_permission_output(response.action, &raw);
        if let Some(output) = output {
            println!("{output}");
        }
    }
    0
}

fn map_event(provider: &str, raw: &Value) -> Option<HookEvent> {
    let agent = match provider {
        "codex" => AgentKind::Codex,
        "claude" => AgentKind::Claude,
        "gemini" => AgentKind::Gemini,
        _ => return None,
    };
    let hook_name = string(raw, "hook_event_name")?;
    let event = match (provider, hook_name.as_str()) {
        (_, "SessionStart") => HookEventKind::SessionStarted,
        ("codex", "UserPromptSubmit") | ("claude", "UserPromptSubmit") => HookEventKind::Running,
        ("gemini", "BeforeAgent") => HookEventKind::Running,
        (_, "PermissionRequest") => HookEventKind::PermissionRequest,
        ("gemini", "Notification")
            if string(raw, "notification_type").as_deref() == Some("ToolPermission") =>
        {
            HookEventKind::PermissionRequest
        }
        ("claude", "Notification")
            if matches!(
                string(raw, "notification_type").as_deref(),
                Some("idle_prompt" | "agent_needs_input")
            ) =>
        {
            HookEventKind::WaitingForInput
        }
        ("codex", "Stop") | ("claude", "Stop") | ("gemini", "AfterAgent") => {
            HookEventKind::Completed
        }
        (_, "StopFailure") => HookEventKind::Failed,
        (_, "SessionEnd") => HookEventKind::SessionEnded,
        _ => return None,
    };

    let session_id = string(raw, "session_id")?;
    let cwd = string(raw, "cwd");
    let (process_id, source) = agent_process_context(provider);
    let permission_mode = string(raw, "permission_mode");
    let is_permission = matches!(event, HookEventKind::PermissionRequest);
    let direct_response = provider == "claude" && hook_name == "PermissionRequest";
    let permission_profile = if is_permission {
        Some(permission_profile(
            provider,
            permission_mode.as_deref(),
            raw,
            direct_response,
        ))
    } else {
        None
    };
    let permission = if is_permission {
        Some(permission_request(provider, raw, &session_id))
    } else {
        None
    };

    Some(HookEvent {
        event,
        session_id: format!("{provider}:{session_id}"),
        agent,
        agent_label: None,
        project: cwd.as_deref().and_then(project_name),
        source: Some(source),
        source_app: None,
        status_label: status_label(hook_name.as_str()).map(str::to_string),
        started_at: string(raw, "timestamp"),
        process_id,
        native_session_id: Some(session_id),
        working_directory: cwd,
        permission_profile,
        permission,
        wait_for_decision: direct_response,
    })
}

fn permission_profile(
    provider: &str,
    mode: Option<&str>,
    raw: &Value,
    direct_response: bool,
) -> PermissionProfile {
    let (access_mode, label, policy) = match mode.unwrap_or("default") {
        "bypassPermissions" | "dontAsk" | "danger-full-access" => (
            AccessMode::FullAccess,
            "Acesso amplo",
            "A sessão normalmente não solicita confirmação",
        ),
        "plan" => (
            AccessMode::Plan,
            "Modo de planejamento",
            "Alterações não são permitidas",
        ),
        "acceptEdits" | "workspace-write" => (
            AccessMode::WorkspaceWrite,
            "Edições permitidas",
            "Outras ações ainda podem pedir confirmação",
        ),
        "read-only" => (
            AccessMode::ReadOnly,
            "Somente leitura",
            "Alterações exigem permissão",
        ),
        _ => (
            AccessMode::Custom,
            "Permissões da sessão",
            "Segue a configuração desta conversa",
        ),
    };

    let mut available_actions = if direct_response {
        vec![PermissionAction::AllowOnce, PermissionAction::Deny]
    } else {
        vec![PermissionAction::OpenSource]
    };
    if provider == "claude"
        && raw
            .get("permission_suggestions")
            .and_then(Value::as_array)
            .is_some_and(|suggestions| !suggestions.is_empty())
    {
        available_actions.insert(1, PermissionAction::AllowSession);
    }

    PermissionProfile {
        mode: access_mode,
        label: label.into(),
        approval_policy: policy.into(),
        can_respond_from_lume: direct_response,
        available_actions,
    }
}

fn permission_request(provider: &str, raw: &Value, session_id: &str) -> PermissionRequest {
    let tool_name = string(raw, "tool_name").unwrap_or_else(|| "Ferramenta".into());
    let tool_input = raw.get("tool_input").or_else(|| raw.get("details"));
    let resource = tool_input
        .and_then(resource_from_input)
        .or_else(|| string(raw, "message"))
        .unwrap_or_else(|| tool_name.clone());
    let description = tool_input
        .and_then(|input| input.get("description"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{tool_name} quer executar uma ação"));
    let timestamp = string(raw, "timestamp").unwrap_or_else(|| now_millis().to_string());
    let kind = if tool_name.to_lowercase().contains("bash")
        || tool_name.to_lowercase().contains("shell")
    {
        "command"
    } else if resource.contains("http://") || resource.contains("https://") {
        "network"
    } else if resource.contains('/') || resource.contains('\\') {
        "file"
    } else {
        "tool"
    };

    PermissionRequest {
        id: format!("{provider}:{session_id}:{}", now_millis()),
        kind: kind.into(),
        summary: truncate(&description, 180),
        resource: truncate(&resource, 320),
        risk: risk_for(&tool_name, &resource).into(),
        requested_at: timestamp,
    }
}

fn resource_from_input(input: &Value) -> Option<String> {
    for key in ["command", "file_path", "path", "url", "query"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }
    serde_json::to_string(input).ok()
}

fn risk_for(tool: &str, resource: &str) -> &'static str {
    let content = format!("{tool} {resource}").to_lowercase();
    if [
        "rm -rf",
        "format ",
        "del /",
        "sudo ",
        "reg delete",
        "drop table",
    ]
    .iter()
    .any(|pattern| content.contains(pattern))
    {
        "high"
    } else if ["write", "edit", "bash", "shell", "http", "mcp"]
        .iter()
        .any(|pattern| content.contains(pattern))
    {
        "medium"
    } else {
        "low"
    }
}

fn claude_permission_output(action: Option<PermissionAction>, raw: &Value) -> Option<Value> {
    let decision = match action? {
        PermissionAction::AllowOnce => json!({ "behavior": "allow" }),
        PermissionAction::AllowSession => {
            let suggestions = raw
                .get("permission_suggestions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|mut suggestion| {
                    if let Some(object) = suggestion.as_object_mut() {
                        object.insert("destination".into(), Value::String("session".into()));
                    }
                    suggestion
                })
                .collect::<Vec<_>>();
            json!({ "behavior": "allow", "updatedPermissions": suggestions })
        }
        PermissionAction::Deny => json!({
            "behavior": "deny",
            "message": "Permissão recusada no Lume",
            "interrupt": false
        }),
        PermissionAction::OpenSource => return None,
    };
    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": decision
        }
    }))
}

fn status_label(hook: &str) -> Option<&'static str> {
    match hook {
        "SessionStart" => Some("Sessão detectada"),
        "UserPromptSubmit" | "BeforeAgent" => Some("Executando"),
        "PermissionRequest" | "Notification" => Some("Aguardando permissão"),
        "Stop" | "AfterAgent" | "SessionEnd" => Some("Finalizado"),
        "StopFailure" => Some("Encerrado com erro"),
        _ => None,
    }
}

fn agent_process_context(provider: &str) -> (Option<u32>, SessionSource) {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .without_tasks(),
    );
    let Some(current_pid) = get_current_pid().ok() else {
        return (None, SessionSource::Cli);
    };
    // O processo atual é `lume hook <provider>` e contém o nome do agente nos
    // próprios argumentos. A busca precisa começar no processo pai para não
    // associar o chat ao PID efêmero do hook.
    let Some(mut pid) = system
        .process(current_pid)
        .and_then(|process| process.parent())
    else {
        return (None, SessionSource::Cli);
    };
    let mut agent_pid = None;
    let mut source = SessionSource::Cli;
    for _ in 0..10 {
        let Some(process) = system.process(pid) else {
            break;
        };
        let name = process.name().to_string_lossy().to_lowercase();
        let command = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if agent_pid.is_none() && command.contains(provider) {
            agent_pid = Some(pid.as_u32());
        }
        if name == "code"
            || name == "code.exe"
            || command.contains("visual studio code")
            || command.contains(".vscode/extensions")
        {
            source = SessionSource::Vscode;
        }
        let Some(parent) = process.parent() else {
            break;
        };
        pid = parent;
    }
    (agent_pid, source)
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn project_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let shortened = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_permission_uses_session_only_suggestion() {
        let raw = json!({
            "permission_suggestions": [{
                "type": "addRules",
                "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }],
                "behavior": "allow",
                "destination": "localSettings"
            }]
        });
        let output = claude_permission_output(Some(PermissionAction::AllowSession), &raw)
            .expect("resposta Claude");
        assert_eq!(
            output["hookSpecificOutput"]["decision"]["updatedPermissions"][0]["destination"],
            "session"
        );
        assert_eq!(
            output["hookSpecificOutput"]["decision"]["behavior"],
            "allow"
        );
    }

    #[test]
    fn gemini_tool_permission_is_observation_only() {
        let raw = json!({
            "session_id": "gemini-session",
            "cwd": "/work/project",
            "hook_event_name": "Notification",
            "notification_type": "ToolPermission",
            "message": "Permitir ferramenta?",
            "details": { "file_path": "/work/project/file.txt" }
        });
        let event = map_event("gemini", &raw).expect("evento Gemini");
        let profile = event.permission_profile.expect("perfil");
        assert!(!profile.can_respond_from_lume);
        assert_eq!(
            profile.available_actions,
            vec![PermissionAction::OpenSource]
        );
        assert!(!event.wait_for_decision);
    }

    #[test]
    fn claude_permission_profile_follows_each_session_mode() {
        let raw = json!({
            "session_id": "claude-session",
            "cwd": "/work/project",
            "hook_event_name": "PermissionRequest",
            "permission_mode": "plan",
            "tool_name": "Bash",
            "tool_input": { "command": "npm test" }
        });
        let event = map_event("claude", &raw).expect("evento Claude");
        let profile = event.permission_profile.expect("perfil");
        assert_eq!(profile.mode, AccessMode::Plan);
        assert!(profile.can_respond_from_lume);
        assert_eq!(
            profile.available_actions,
            vec![PermissionAction::AllowOnce, PermissionAction::Deny]
        );
    }
}
