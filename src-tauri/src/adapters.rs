use std::io::Read;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sysinfo::{get_current_pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::{
    domain::{
        AccessMode, AgentKind, HookEvent, HookEventKind, PermissionAction, PermissionProfile,
        PermissionRequest, SessionActivity, SessionSource,
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
        ("codex" | "claude", "PreToolUse") | ("gemini", "BeforeTool") => HookEventKind::Running,
        ("codex", "PostToolUse")
        | ("claude", "PostToolUse" | "PostToolUseFailure")
        | ("gemini", "AfterTool") => HookEventKind::Running,
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
    let permission_profile = if is_permission || permission_mode.is_some() {
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
    let last_response = matches!(
        &event,
        HookEventKind::Completed | HookEventKind::SessionEnded
    )
    .then(|| hook_response(raw))
    .flatten();
    let activity = hook_activity(
        provider,
        hook_name.as_str(),
        raw,
        &session_id,
        last_response.as_deref(),
    );

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
        last_response,
        activity,
        wait_for_decision: direct_response,
    })
}

fn hook_activity(
    provider: &str,
    hook_name: &str,
    raw: &Value,
    session_id: &str,
    last_response: Option<&str>,
) -> Option<SessionActivity> {
    if matches!(hook_name, "UserPromptSubmit" | "BeforeAgent") {
        let prompt = ["prompt", "user_prompt", "message"]
            .into_iter()
            .find_map(|key| string(raw, key));
        return prompt.map(|prompt| SessionActivity {
            id: format!("{provider}:{session_id}:prompt:{}", now_millis()),
            kind: "prompt".into(),
            title: "Prompt enviado".into(),
            detail: Some(truncate(prompt.trim(), 16 * 1024)),
            status: "completed".into(),
            created_at: now_millis(),
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
    }
    if matches!(hook_name, "Stop" | "AfterAgent") {
        return last_response.map(|response| SessionActivity {
            id: format!("{provider}:{session_id}:response:{}", now_millis()),
            kind: "message".into(),
            title: "Resposta do agente".into(),
            detail: Some(truncate(response, 32 * 1024)),
            status: "completed".into(),
            created_at: now_millis(),
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        });
    }
    if !matches!(
        hook_name,
        "PreToolUse" | "BeforeTool" | "PostToolUse" | "PostToolUseFailure" | "AfterTool"
    ) {
        return None;
    }

    let tool_name = string(raw, "tool_name")
        .or_else(|| string(raw, "tool"))
        .unwrap_or_else(|| "Ferramenta".into());
    let input = raw.get("tool_input").or_else(|| raw.get("details"));
    let resource = input
        .and_then(resource_from_input)
        .unwrap_or_else(|| tool_name.clone());
    let lower_tool = tool_name.to_lowercase();
    let lower_resource = resource.to_lowercase();
    let is_command = lower_tool.contains("bash")
        || lower_tool.contains("shell")
        || lower_tool.contains("command");
    let kind = if is_command && is_test_command(&lower_resource) {
        "test"
    } else if is_command {
        "command"
    } else if ["write", "edit", "patch", "file"]
        .iter()
        .any(|needle| lower_tool.contains(needle))
    {
        "file"
    } else {
        "tool"
    };
    let status = match hook_name {
        "PreToolUse" | "BeforeTool" => "running",
        "PostToolUseFailure" => "failed",
        _ => "completed",
    };
    let result = raw
        .get("tool_response")
        .or_else(|| raw.get("tool_result"))
        .or_else(|| raw.get("result"))
        .and_then(|value| serde_json::to_string_pretty(value).ok());
    let detail =
        result.or_else(|| input.and_then(|value| serde_json::to_string_pretty(value).ok()));
    let tool_id = string(raw, "tool_use_id")
        .or_else(|| string(raw, "tool_call_id"))
        .unwrap_or_else(|| now_millis().to_string());
    let files = if kind == "file" {
        input
            .and_then(resource_from_input)
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Some(SessionActivity {
        id: format!("{provider}:{session_id}:tool:{tool_id}"),
        kind: kind.into(),
        title: truncate(&resource, 240),
        detail: detail.map(|detail| truncate(&detail, 16 * 1024)),
        status: status.into(),
        created_at: now_millis(),
        files,
        attachments: Vec::new(),
        append_detail: false,
    })
}

fn is_test_command(command: &str) -> bool {
    [
        "npm test",
        "pnpm test",
        "yarn test",
        "cargo test",
        "dotnet test",
        "go test",
        "flutter test",
        "pytest",
        "vitest",
        "jest",
        "mvn test",
        "gradle test",
        "gradlew test",
    ]
    .iter()
    .any(|pattern| command.contains(pattern))
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
        _ => (AccessMode::Custom, "Permissões da sessão", ""),
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
        approvals_reviewer: string(raw, "approvals_reviewer")
            .or_else(|| string(raw, "approvalsReviewer")),
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
    let request_key = [
        "permission_request_id",
        "permissionRequestId",
        "tool_use_id",
        "toolUseId",
        "tool_call_id",
        "toolCallId",
        "request_id",
        "requestId",
        "id",
    ]
    .into_iter()
    .find_map(|key| string(raw, key))
    .unwrap_or_else(|| {
        let bucket = timestamp
            .parse::<i64>()
            .unwrap_or_else(|_| now_millis())
            .div_euclid(30_000);
        let fingerprint = format!("{provider}\n{session_id}\n{tool_name}\n{resource}\n{bucket}");
        format!("{:x}", Sha256::digest(fingerprint.as_bytes()))
    });
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
        id: format!("{provider}:{session_id}:{request_key}"),
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
        "UserPromptSubmit" | "BeforeAgent" | "PostToolUse" | "PostToolUseFailure" | "AfterTool" => {
            Some("Executando")
        }
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
        // Hooks de Codex/Gemini podem passar por um shell efêmero cujo comando
        // também contém o provider. Continua subindo para guardar o processo
        // estável mais externo da sessão, em vez do wrapper que termina logo
        // após enviar o evento ao Lume.
        if command.contains(provider) {
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

fn hook_response(value: &Value) -> Option<String> {
    ["last_assistant_message", "prompt_response", "response"]
        .into_iter()
        .find_map(|key| string(value, key))
        .map(|response| truncate(response.trim(), 32 * 1024))
        .filter(|response| !response.is_empty())
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

    #[test]
    fn tool_completion_returns_the_session_to_running() {
        for (provider, hook) in [
            ("codex", "PostToolUse"),
            ("claude", "PostToolUse"),
            ("claude", "PostToolUseFailure"),
            ("gemini", "AfterTool"),
        ] {
            let raw = json!({
                "session_id": format!("{provider}-session"),
                "cwd": "/work/project",
                "hook_event_name": hook,
                "tool_name": "Bash"
            });
            let event = map_event(provider, &raw).expect("evento pós-ferramenta");
            assert!(matches!(event.event, HookEventKind::Running));
            assert_eq!(event.status_label.as_deref(), Some("Executando"));
        }
    }

    #[test]
    fn non_permission_hooks_keep_the_active_full_access_mode() {
        let raw = json!({
            "session_id": "claude-session",
            "cwd": "/work/project",
            "hook_event_name": "UserPromptSubmit",
            "permission_mode": "bypassPermissions"
        });
        let event = map_event("claude", &raw).expect("evento Claude");
        let profile = event.permission_profile.expect("perfil");
        assert_eq!(profile.mode, AccessMode::FullAccess);
        assert!(!profile.can_respond_from_lume);
    }

    #[test]
    fn completed_hook_carries_the_final_agent_response() {
        let raw = json!({
            "session_id": "claude-session",
            "cwd": "/work/project",
            "hook_event_name": "Stop",
            "last_assistant_message": "Resposta final do agente"
        });
        let event = map_event("claude", &raw).expect("evento final do Claude");
        assert!(matches!(event.event, HookEventKind::Completed));
        assert_eq!(
            event.last_response.as_deref(),
            Some("Resposta final do agente")
        );
        assert_eq!(
            event
                .activity
                .as_ref()
                .map(|activity| activity.kind.as_str()),
            Some("message")
        );
    }

    #[test]
    fn tool_hooks_expose_commands_and_files_as_activity() {
        let command = map_event(
            "claude",
            &json!({
                "session_id": "claude-session",
                "cwd": "/work/project",
                "hook_event_name": "PostToolUse",
                "tool_use_id": "tool-1",
                "tool_name": "Bash",
                "tool_input": { "command": "npm test" },
                "tool_response": { "output": "12 tests passed" }
            }),
        )
        .expect("evento de comando")
        .activity
        .expect("atividade de comando");
        assert_eq!(command.kind, "test");
        assert_eq!(command.title, "npm test");
        assert!(command
            .detail
            .as_deref()
            .is_some_and(|value| value.contains("12 tests passed")));

        let file = map_event(
            "claude",
            &json!({
                "session_id": "claude-session",
                "cwd": "/work/project",
                "hook_event_name": "PreToolUse",
                "tool_use_id": "tool-2",
                "tool_name": "Edit",
                "tool_input": { "file_path": "/work/project/src/app.ts" }
            }),
        )
        .expect("evento de arquivo")
        .activity
        .expect("atividade de arquivo");
        assert_eq!(file.kind, "file");
        assert_eq!(file.files, vec!["/work/project/src/app.ts"]);
    }
}
