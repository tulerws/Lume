use std::{
    collections::{HashMap, HashSet},
    thread,
    time::Duration,
};

use sysinfo::{
    get_current_pid, Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind,
};
use tauri::AppHandle;

use crate::{
    agent_plugins::{self, ExternalAgentPlugin},
    domain::{AgentKind, SessionSource},
    state::AppState,
};

#[derive(Clone, Debug)]
pub struct DiscoveredProcess {
    pub agent: AgentKind,
    pub agent_label: String,
    pub process_id: u32,
    pub native_session_ids: Vec<String>,
    pub working_directory: Option<String>,
    pub source: SessionSource,
}

struct ProcessScan {
    discovered: Vec<DiscoveredProcess>,
    live_pids: HashSet<u32>,
}

pub fn start(state: AppState, app: AppHandle) -> Result<(), String> {
    thread::Builder::new()
        .name("lume-process-discovery".into())
        .spawn(move || {
            let mut system = System::new();
            loop {
                let plugins = agent_plugins::external_catalog(&app);
                let scan = scan(&mut system, &plugins);
                if let Ok(changed) =
                    state.reconcile_process_snapshot(scan.discovered, scan.live_pids)
                {
                    if changed {
                        crate::protocol::emit_sessions_changed(&app);
                    }
                }
                thread::sleep(Duration::from_secs(2));
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn scan(system: &mut System, external_plugins: &[ExternalAgentPlugin]) -> ProcessScan {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .without_tasks(),
    );
    let own_pid = get_current_pid().ok();
    let live_pids = system
        .processes()
        .keys()
        .map(|pid| pid.as_u32())
        .collect::<HashSet<_>>();
    let ignored_codex_pids = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            is_lume_codex_infrastructure_process(&command).then_some(*pid)
        })
        .collect::<Vec<_>>();
    let candidates = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            if Some(*pid) == own_pid {
                return None;
            }
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let name = process.name().to_string_lossy().to_lowercase();
            if is_lume_codex_infrastructure_process(&command) {
                return None;
            }
            if ignored_codex_pids
                .iter()
                .any(|root| process_descends_from(system, *pid, *root))
            {
                return None;
            }
            let (agent, agent_label) = detect_agent(&name, &command)
                .map(|agent| {
                    let label = match agent {
                        AgentKind::Codex => "Codex",
                        AgentKind::Claude => "Claude",
                        AgentKind::Gemini => "Gemini",
                        AgentKind::Unknown => "Agent",
                    };
                    (agent, label.to_string())
                })
                .or_else(|| detect_external_agent(&name, &command, external_plugins))?;
            let working_directory = command_working_directory(process.cmd());
            Some((
                *pid,
                process.parent(),
                agent,
                agent_label,
                working_directory,
            ))
        })
        .collect::<Vec<_>>();
    let candidate_pids = candidates
        .iter()
        .map(|(pid, _, _, _, _)| *pid)
        .collect::<Vec<_>>();
    if !candidate_pids.is_empty() {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&candidate_pids),
            true,
            ProcessRefreshKind::nothing()
                .with_cwd(UpdateKind::Always)
                .without_tasks(),
        );
    }
    let agents_by_pid = candidates
        .iter()
        .map(|(pid, _, agent, label, _)| (*pid, (agent.clone(), label.clone())))
        .collect::<HashMap<_, _>>();

    let discovered = candidates
        .into_iter()
        // Mantém o processo detectado mais próximo da raiz. Um comando executado
        // pelo agente pode conter "codex", "claude" ou "gemini" nos argumentos;
        // escolher esse descendente efêmero faria a sessão trocar de PID.
        .filter(|(pid, _, agent, label, _)| {
            !agents_by_pid
                .iter()
                .any(|(ancestor_pid, (ancestor_agent, ancestor_label))| {
                    ancestor_agent == agent
                        && ancestor_label == label
                        && process_descends_from(&system, *pid, *ancestor_pid)
                })
        })
        .filter_map(|(pid, _, agent, agent_label, explicit_working_directory)| {
            let process = system.process(pid)?;
            let working_directory = explicit_working_directory
                .or_else(|| process.cwd().map(|path| path.to_string_lossy().to_string()));
            if agent == AgentKind::Codex
                && working_directory
                    .as_deref()
                    .is_some_and(is_codex_internal_workspace)
            {
                return None;
            }
            Some(DiscoveredProcess {
                agent,
                agent_label,
                process_id: pid.as_u32(),
                native_session_ids: native_session_ids_for_process_tree(&system, pid),
                working_directory,
                source: source_for(&system, pid),
            })
        })
        .collect();

    ProcessScan {
        discovered,
        live_pids,
    }
}

fn is_lume_codex_infrastructure_process(command: &str) -> bool {
    let normalized = command.replace('\\', "/");
    normalized.contains("127.0.0.1:43130")
        || (normalized.contains("app-server")
            && normalized.contains(".vscode/extensions")
            && normalized.contains("openai.chatgpt"))
}

#[cfg(target_os = "linux")]
fn native_session_ids_for_process_tree(system: &System, root: sysinfo::Pid) -> Vec<String> {
    let mut ids = system
        .processes()
        .keys()
        .filter(|pid| **pid == root || process_descends_from(system, **pid, root))
        .flat_map(|pid| native_session_ids_for_pid(*pid))
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(target_os = "linux")]
fn native_session_ids_for_pid(pid: sysinfo::Pid) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(format!("/proc/{}/fd", pid.as_u32())) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
            let id = stem.get(stem.len().checked_sub(36)?..)?;
            is_codex_session_id(id).then(|| id.to_string())
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn native_session_ids_for_process_tree(_system: &System, _pid: sysinfo::Pid) -> Vec<String> {
    Vec::new()
}

fn is_codex_session_id(value: &str) -> bool {
    value.len() == 36
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| match index {
                8 | 13 | 18 | 23 => character == '-',
                _ => character.is_ascii_hexdigit(),
            })
}

fn command_working_directory(command: &[std::ffi::OsString]) -> Option<String> {
    let mut parts = command.iter().map(|part| part.to_string_lossy());
    while let Some(part) = parts.next() {
        if part == "--command-cwd" {
            return parts.next().map(|value| value.into_owned());
        }
        if let Some(value) = part.strip_prefix("--command-cwd=") {
            return Some(value.to_string());
        }
    }
    None
}

fn is_codex_internal_workspace(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_lowercase();
    normalized.contains("/.codex/memories")
}

/// Subcomandos do Claude Code que são infraestrutura, não conversas.
fn is_claude_infrastructure(tokens: &[&str]) -> bool {
    const SUBCOMMANDS: [&str; 2] = ["daemon", "bg-pty-host"];
    tokens
        .windows(2)
        .any(|pair| pair[0] == "claude" && SUBCOMMANDS.contains(&pair[1]))
}

fn process_descends_from(system: &System, mut child: sysinfo::Pid, ancestor: sysinfo::Pid) -> bool {
    for _ in 0..12 {
        let Some(parent) = system.process(child).and_then(|process| process.parent()) else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        child = parent;
    }
    false
}

fn source_for(system: &System, mut pid: sysinfo::Pid) -> SessionSource {
    for _ in 0..8 {
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
        if name == "code"
            || name == "code.exe"
            || command.contains("visual studio code")
            || command.contains(".vscode/extensions")
        {
            return SessionSource::Vscode;
        }
        let Some(parent) = process.parent() else {
            break;
        };
        pid = parent;
    }
    SessionSource::Cli
}

/// Reconhece o executável versionado instalado abaixo de um diretório `claude`.
fn is_versioned_claude_executable(token: &str) -> bool {
    token
        .split(['/', '\\'])
        .any(|segment| segment.trim_matches(['"', '\'']) == "claude")
}

fn detect_agent(name: &str, command: &str) -> Option<AgentKind> {
    let raw_tokens = command.split_whitespace().collect::<Vec<_>>();
    let tokens = raw_tokens
        .iter()
        .map(|token| {
            token
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(token)
                .trim_matches(['"', '\''])
        })
        .collect::<Vec<_>>();
    if name == "codex" || tokens.iter().any(|token| token == &"codex") {
        Some(AgentKind::Codex)
    } else if is_claude_infrastructure(&tokens) {
        None
    } else if name == "claude"
        || tokens.iter().any(|token| token == &"claude")
        || raw_tokens
            .first()
            .is_some_and(|executable| is_versioned_claude_executable(executable))
    {
        Some(AgentKind::Claude)
    } else if name == "gemini" || tokens.iter().any(|token| token == &"gemini") {
        Some(AgentKind::Gemini)
    } else {
        None
    }
}

fn detect_external_agent(
    name: &str,
    command: &str,
    plugins: &[ExternalAgentPlugin],
) -> Option<(AgentKind, String)> {
    plugins.iter().find_map(|plugin| {
        let matches_name = plugin
            .process_names
            .iter()
            .any(|candidate| name == candidate.to_lowercase());
        let matches_command = plugin
            .command_tokens
            .iter()
            .any(|candidate| command.contains(&candidate.to_lowercase()));
        (matches_name || matches_command).then(|| (AgentKind::Unknown, plugin.name.clone()))
    })
}

pub fn terminate_agent_process(process_id: u32, expected_agent: &AgentKind) -> Result<(), String> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .without_tasks(),
    );
    let target_pid = Pid::from_u32(process_id);
    let Some(target) = system.process(target_pid) else {
        return Ok(());
    };
    let command = target
        .cmd()
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let name = target.name().to_string_lossy().to_lowercase();
    if detect_agent(&name, &command).as_ref() != Some(expected_agent) {
        return Err("O PID da sessão não pertence mais ao agente esperado".into());
    }
    if get_current_pid().ok().is_some_and(|own_pid| {
        own_pid == target_pid || process_descends_from(&system, own_pid, target_pid)
    }) {
        return Err(
            "O Lume está sendo executado dentro desse processo e não pode encerrá-lo".into(),
        );
    }

    let mut targets = system
        .processes()
        .keys()
        .copied()
        .filter(|pid| *pid == target_pid || process_descends_from(&system, *pid, target_pid))
        .collect::<Vec<_>>();
    targets.sort_by_key(|pid| std::cmp::Reverse(process_depth(&system, *pid)));
    let mut terminated_root = false;
    for pid in targets {
        let Some(process) = system.process(pid) else {
            continue;
        };
        let terminated = process.kill_with(Signal::Term).unwrap_or(false) || process.kill();
        if pid == target_pid {
            terminated_root = terminated;
        }
    }
    if terminated_root {
        Ok(())
    } else {
        Err("O sistema recusou o encerramento do agente".into())
    }
}

#[cfg(not(target_os = "windows"))]
pub fn interrupt_agent_process(process_id: u32, expected_agent: &AgentKind) -> Result<(), String> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .without_tasks(),
    );
    let target_pid = Pid::from_u32(process_id);
    let Some(target) = system.process(target_pid) else {
        return Err("The agent process is no longer open".into());
    };
    let command = target
        .cmd()
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let name = target.name().to_string_lossy().to_lowercase();
    if detect_agent(&name, &command).as_ref() != Some(expected_agent) {
        return Err("The session PID no longer belongs to the expected agent".into());
    }
    if target.kill_with(Signal::Interrupt).unwrap_or(false) {
        Ok(())
    } else {
        Err("The operating system could not interrupt this prompt safely".into())
    }
}

#[cfg(target_os = "windows")]
pub fn interrupt_agent_process(
    _process_id: u32,
    _expected_agent: &AgentKind,
) -> Result<(), String> {
    Err("Prompt interruption for external CLI agents is not available on Windows yet".into())
}

fn process_depth(system: &System, mut pid: Pid) -> usize {
    let mut depth = 0;
    for _ in 0..32 {
        let Some(parent) = system.process(pid).and_then(|process| process.parent()) else {
            break;
        };
        depth += 1;
        pid = parent;
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vscode_codex_app_server_is_ignored_until_a_real_chat_emits_events() {
        assert!(is_lume_codex_infrastructure_process(
            "/home/user/.vscode/extensions/openai.chatgpt/bin/codex app-server"
        ));
        assert!(is_lume_codex_infrastructure_process(
            r"C:\Users\user\.vscode\extensions\openai.chatgpt\bin\codex.exe app-server"
        ));
    }

    #[test]
    fn recognizes_rollout_session_ids_without_accepting_arbitrary_names() {
        assert!(is_codex_session_id("019f8061-7032-7521-b333-84f84c744fa8"));
        assert!(!is_codex_session_id("rollout-memory-maintenance"));
    }

    #[test]
    fn lume_codex_app_server_is_ignored_but_its_user_cli_is_detectable() {
        assert!(is_lume_codex_infrastructure_process(
            "codex app-server --listen ws://127.0.0.1:43130"
        ));
        assert!(!is_lume_codex_infrastructure_process(
            "codex --remote ws://127.0.0.1:43131 resume chat"
        ));
        assert_eq!(
            detect_agent("codex", "codex --remote ws://127.0.0.1:43131 resume chat"),
            Some(AgentKind::Codex)
        );
    }

    #[test]
    fn codex_sandbox_command_cwd_wins_over_wrapper_directory() {
        let command = [
            "codex-linux-sandbox",
            "--sandbox-policy-cwd",
            "/home/user",
            "--command-cwd",
            "/home/user/Documents/Projetos/Ideias/Lume",
            "codex",
        ]
        .map(std::ffi::OsString::from);
        assert_eq!(
            command_working_directory(&command).as_deref(),
            Some("/home/user/Documents/Projetos/Ideias/Lume")
        );
    }

    #[test]
    fn command_cwd_equals_syntax_is_supported() {
        let command = [
            "codex.exe",
            "--command-cwd=C:\\Users\\user\\Documents\\Lume",
        ]
        .map(std::ffi::OsString::from);
        assert_eq!(
            command_working_directory(&command).as_deref(),
            Some("C:\\Users\\user\\Documents\\Lume")
        );
    }

    #[test]
    fn codex_memory_maintenance_is_not_a_user_session() {
        assert!(is_codex_internal_workspace("/home/user/.codex/memories"));
        assert!(is_codex_internal_workspace(
            "C:\\Users\\user\\.codex\\memories\\2026"
        ));
        assert!(!is_codex_internal_workspace(
            "/home/user/Documents/memories"
        ));
    }

    #[test]
    fn claude_infrastructure_is_not_a_session() {
        assert_eq!(
            detect_agent("claude", "/home/user/.local/bin/claude daemon"),
            None
        );
        assert_eq!(
            detect_agent(
                "2.1.220",
                "claude bg-pty-host --bg-pty-host /tmp/cc-daemon/x.sock 200 50 -- /home/user/.local/share/claude/versions/2.1.220"
            ),
            None
        );
    }

    #[test]
    fn versioned_claude_binary_is_a_session() {
        assert_eq!(
            detect_agent(
                "2.1.220",
                "/home/user/.local/share/claude/versions/2.1.220 --session-id 9b7acb3c --fork-session"
            ),
            Some(AgentKind::Claude)
        );
    }

    #[test]
    fn claude_lookalikes_are_not_sessions() {
        assert_eq!(
            detect_agent("nvim", "/usr/bin/nvim /home/user/.claude/settings.json"),
            None
        );
        assert_eq!(
            detect_agent("nvim", "/usr/bin/nvim /home/user/claude-notes/a.md"),
            None
        );
        assert_eq!(
            detect_agent("nvim", "/usr/bin/nvim /home/user/claude/a.md"),
            None
        );
    }

    #[test]
    fn claude_detection_does_not_change_codex_or_gemini() {
        assert_eq!(
            detect_agent("codex", "/usr/bin/codex"),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            detect_agent("bash", "codex resume abc"),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            detect_agent("codex", "/usr/bin/codex daemon"),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            detect_agent("gemini", "/usr/bin/gemini"),
            Some(AgentKind::Gemini)
        );
        assert_eq!(detect_agent("bash", "gemini chat"), Some(AgentKind::Gemini));
        assert_eq!(
            detect_agent("claude", "/usr/bin/claude"),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            detect_agent("bash", "claude --resume abc"),
            Some(AgentKind::Claude)
        );
    }

    #[test]
    fn external_manifest_detects_a_custom_cli_process() {
        let plugin = ExternalAgentPlugin {
            id: "local-agent".into(),
            name: "Local Agent".into(),
            executable: "local-agent".into(),
            process_names: vec!["local-agent".into(), "local-agent.exe".into()],
            command_tokens: vec!["local-agent".into()],
            ..ExternalAgentPlugin::default()
        };
        assert_eq!(
            detect_external_agent("local-agent", "/usr/bin/local-agent", &[plugin]),
            Some((AgentKind::Unknown, "Local Agent".into()))
        );
    }
}
