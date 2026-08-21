use std::{
    collections::{HashMap, HashSet},
    thread,
    time::Duration,
};

#[cfg(target_os = "linux")]
use std::{
    io::{BufRead, BufReader},
    path::Path,
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
            // A linha de comando de um PID não muda durante sua vida. Reabrir
            // /proc/<pid>/cmdline para todos os processos a cada dois segundos
            // monopolizava um núcleo em máquinas com muitos processos/threads.
            .with_cmd(UpdateKind::OnlyIfNotSet)
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
            if is_claude_headless_resume(&command) {
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
                        AgentKind::ChatGpt => "ChatGPT",
                        AgentKind::Claude => "Claude",
                        AgentKind::ClaudeCode => "Claude Code",
                        AgentKind::Antigravity => "Antigravity",
                        AgentKind::DeepSeek => "DeepSeek",
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
                && is_codex_internal_process(&system, pid, working_directory.as_deref())
            {
                return None;
            }
            let native_session_ids = if agent == AgentKind::Codex {
                native_session_ids_for_process_tree(&system, pid)
            } else {
                Vec::new()
            };
            Some(DiscoveredProcess {
                agent,
                agent_label,
                process_id: pid.as_u32(),
                native_session_ids,
                working_directory,
                source: source_for(&system, pid),
            })
        })
        .collect::<Vec<_>>();

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

fn native_session_ids_for_process_tree(system: &System, root: sysinfo::Pid) -> Vec<String> {
    let process_tree = system
        .processes()
        .keys()
        .filter(|pid| **pid == root || process_descends_from(system, **pid, root))
        .filter_map(|pid| system.process(*pid))
        .collect::<Vec<_>>();
    let command_ids = process_tree
        .iter()
        .flat_map(|process| native_session_ids_from_command(process.cmd()))
        .collect::<HashSet<_>>();
    if command_ids.len() == 1 {
        return command_ids.into_iter().collect();
    }

    #[cfg(target_os = "linux")]
    {
        // A CLI pode herdar rollouts do agente pai e também abrir rollouts internos
        // (por exemplo, o guardian). Depois de descartar os internos, o descritor
        // mais recente representa a conversa visível selecionada nessa CLI.
        let candidates = system
            .processes()
            .keys()
            .filter(|pid| **pid == root || process_descends_from(system, **pid, root))
            .flat_map(|pid| native_session_ids_for_pid(*pid))
            .collect::<Vec<_>>();
        return select_native_session_id(candidates).into_iter().collect();
    }

    #[cfg(not(target_os = "linux"))]
    Vec::new()
}

#[cfg(target_os = "linux")]
fn select_native_session_id(candidates: Vec<(u64, String)>) -> Option<String> {
    candidates
        .into_iter()
        .max_by_key(|(descriptor, _)| *descriptor)
        .map(|(_, id)| id)
}

#[cfg(target_os = "linux")]
fn native_session_ids_for_pid(pid: sysinfo::Pid) -> Vec<(u64, String)> {
    let Ok(entries) = std::fs::read_dir(format!("/proc/{}/fd", pid.as_u32())) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let descriptor = entry.file_name().to_str()?.parse::<u64>().ok()?;
            let path = std::fs::read_link(entry.path()).ok()?;
            let id = codex_rollout_id_from_path(&path)?;
            if !rollout_is_user_facing(&path) {
                return None;
            }
            Some((descriptor, id))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn codex_rollout_id_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".jsonl")?;
    let id = if let Some(rollout) = stem.strip_prefix("rollout-") {
        rollout.get(rollout.len().checked_sub(36)?..)?
    } else {
        stem
    };
    is_codex_session_id(id).then(|| id.to_string())
}

#[cfg(target_os = "linux")]
fn rollout_is_user_facing(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return true;
    };
    let mut first_line = String::new();
    if BufReader::new(file).read_line(&mut first_line).is_err() {
        return true;
    }
    let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&first_line) else {
        return true;
    };
    rollout_metadata_is_user_facing(&metadata)
}

#[cfg(target_os = "linux")]
fn rollout_metadata_is_user_facing(metadata: &serde_json::Value) -> bool {
    metadata
        .get("payload")
        .and_then(|payload| payload.get("source"))
        .and_then(serde_json::Value::as_object)
        .is_none_or(|source| !source.contains_key("subagent"))
}

fn native_session_ids_from_command(command: &[std::ffi::OsString]) -> Vec<String> {
    let parts = command
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>();
    let mut ids = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let candidate = if part == "resume" || part == "--resume" {
            parts.get(index + 1).map(|value| value.as_ref())
        } else {
            part.strip_prefix("--resume=")
        };
        if let Some(candidate) = candidate.filter(|value| is_codex_session_id(value)) {
            if !ids.iter().any(|existing| existing == candidate) {
                ids.push(candidate.to_string());
            }
        }
    }
    ids
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

fn is_codex_internal_process(
    system: &System,
    mut pid: sysinfo::Pid,
    working_directory: Option<&str>,
) -> bool {
    if working_directory.is_some_and(crate::session_filters::is_codex_internal_workspace) {
        return true;
    }
    for _ in 0..12 {
        let Some(process) = system.process(pid) else {
            break;
        };
        if process.cwd().is_some_and(|path| {
            crate::session_filters::is_codex_internal_workspace(&path.to_string_lossy())
        }) || command_has_codex_internal_workspace(process.cmd())
        {
            return true;
        }
        let Some(parent) = process.parent() else {
            break;
        };
        pid = parent;
    }
    false
}

fn command_has_codex_internal_workspace(command: &[std::ffi::OsString]) -> bool {
    let mut parts = command.iter().map(|part| part.to_string_lossy());
    while let Some(part) = parts.next() {
        if matches!(part.as_ref(), "--command-cwd" | "--sandbox-policy-cwd") {
            if parts
                .next()
                .is_some_and(|path| crate::session_filters::is_codex_internal_workspace(&path))
            {
                return true;
            }
            continue;
        }
        if ["--command-cwd=", "--sandbox-policy-cwd="]
            .iter()
            .find_map(|prefix| part.strip_prefix(prefix))
            .is_some_and(crate::session_filters::is_codex_internal_workspace)
        {
            return true;
        }
    }
    false
}

/// Subcomandos do Claude Code que são infraestrutura, não conversas.
fn is_claude_infrastructure(tokens: &[&str]) -> bool {
    const SUBCOMMANDS: [&str; 2] = ["daemon", "bg-pty-host"];
    tokens
        .windows(2)
        .any(|pair| pair[0] == "claude" && SUBCOMMANDS.contains(&pair[1]))
}

fn is_claude_headless_resume(command: &str) -> bool {
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    tokens.iter().any(|token| *token == "--print")
        && tokens.iter().any(|token| *token == "--resume")
        && tokens.iter().any(|token| {
            token
                .trim_matches(['"', '\''])
                .split(['/', '\\'])
                .next_back()
                == Some("claude")
        })
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
        Some(AgentKind::ClaudeCode)
    } else if matches!(name, "agy" | "agy.exe")
        || tokens
            .iter()
            .any(|token| matches!(*token, "agy" | "agy.exe"))
    {
        Some(AgentKind::Antigravity)
    } else if matches!(name, "dsh" | "dsh.exe")
        || tokens
            .iter()
            .any(|token| matches!(*token, "dsh" | "dsh.exe"))
    {
        Some(AgentKind::DeepSeek)
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
    let (system, target_pid, targets) = agent_process_tree(process_id, expected_agent)?;
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

pub fn release_agent_process_for_takeover(
    process_id: u32,
    expected_agent: &AgentKind,
) -> Result<(), String> {
    let (system, target_pid, targets) = agent_process_tree(process_id, expected_agent)?;
    let mut requested_root = false;
    for pid in targets {
        let Some(process) = system.process(pid) else {
            continue;
        };
        #[cfg(not(target_os = "windows"))]
        let requested = process.kill_with(Signal::Term).unwrap_or(false);
        #[cfg(target_os = "windows")]
        let requested = process.kill();
        if pid == target_pid {
            requested_root = requested;
        }
    }
    if !requested_root {
        return Err("The operating system refused to release this agent session".into());
    }

    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut refreshed = System::new();
        refreshed.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[target_pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        if refreshed.process(target_pid).is_none() {
            return Ok(());
        }
    }
    Err("The external CLI did not close in time; Lume did not take control".into())
}

fn agent_process_tree(
    process_id: u32,
    expected_agent: &AgentKind,
) -> Result<(System, Pid, Vec<Pid>), String> {
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
    Ok((system, target_pid, targets))
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

pub fn interrupt_resumed_prompt_process(
    native_session_id: &str,
    expected_agent: &AgentKind,
) -> Result<(), String> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .without_tasks(),
    );
    let process = system.processes().values().find(|process| {
        let command = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>();
        resumed_prompt_command_matches(&command, native_session_id, expected_agent)
    });
    let Some(process) = process else {
        return Err("The running Claude prompt process could not be found".into());
    };
    #[cfg(not(target_os = "windows"))]
    let interrupted = process.kill_with(Signal::Interrupt).unwrap_or(false);
    #[cfg(target_os = "windows")]
    let interrupted = process.kill();
    if interrupted {
        Ok(())
    } else {
        Err("The operating system could not interrupt this prompt safely".into())
    }
}

fn resumed_prompt_command_matches(
    command: &[std::borrow::Cow<'_, str>],
    native_session_id: &str,
    expected_agent: &AgentKind,
) -> bool {
    let joined = command.join(" ").to_lowercase();
    let executable = command
        .first()
        .and_then(|part| std::path::Path::new(part.as_ref()).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();
    detect_agent(&executable, &joined).as_ref() == Some(expected_agent)
        && command.iter().any(|part| part.as_ref() == "--print")
        && command
            .windows(2)
            .any(|parts| parts[0].as_ref() == "--resume" && parts[1].as_ref() == native_session_id)
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
    fn resumed_thread_id_is_recovered_from_the_process_command() {
        let command = [
            "codex",
            "--remote",
            "ws://127.0.0.1:43131",
            "resume",
            "019f8061-7032-7521-b333-84f84c744fa8",
        ]
        .map(std::ffi::OsString::from);
        assert_eq!(
            native_session_ids_from_command(&command),
            vec!["019f8061-7032-7521-b333-84f84c744fa8"],
        );
    }

    #[test]
    fn unrelated_uuid_in_a_prompt_is_not_treated_as_a_thread() {
        let command = ["codex", "explain", "019f8061-7032-7521-b333-84f84c744fa8"]
            .map(std::ffi::OsString::from);
        assert!(native_session_ids_from_command(&command).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_rollout_uses_the_session_opened_last() {
        assert_eq!(
            select_native_session_id(vec![
                (32, "019f8061-7032-7521-b333-84f84c744fa8".into()),
                (54, "019fcdac-85c9-77e2-871a-3583aa965a75".into()),
            ]),
            Some("019fcdac-85c9-77e2-871a-3583aa965a75".into()),
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn internal_subagent_rollouts_are_not_used_as_visible_session_identity() {
        let guardian = serde_json::json!({
            "type": "session_meta",
            "payload": { "source": { "subagent": { "other": "guardian" } } }
        });
        let cli = serde_json::json!({
            "type": "session_meta",
            "payload": { "source": "cli" }
        });
        assert!(!rollout_metadata_is_user_facing(&guardian));
        assert!(rollout_metadata_is_user_facing(&cli));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn terminal_descriptors_are_rejected_before_any_rollout_read() {
        assert!(codex_rollout_id_from_path(Path::new("/dev/pts/7")).is_none());
        assert!(codex_rollout_id_from_path(Path::new(
            "/home/user/.codex/thread-writer-locks/019f8061-7032-7521-b333-84f84c744fa8.lock"
        ))
        .is_none());
        assert_eq!(
            codex_rollout_id_from_path(Path::new(
                "/home/user/.codex/sessions/2026/07/20/rollout-2026-07-20T13-34-57-019f8061-7032-7521-b333-84f84c744fa8.jsonl"
            )),
            Some("019f8061-7032-7521-b333-84f84c744fa8".into())
        );
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
        let command = [
            "codex-linux-sandbox",
            "--sandbox-policy-cwd",
            "/home/user/.codex/memories",
            "codex",
        ]
        .map(std::ffi::OsString::from);
        assert!(command_has_codex_internal_workspace(&command));
        let regular = [
            "codex-linux-sandbox",
            "--command-cwd=/home/user/Documents/memories",
            "codex",
        ]
        .map(std::ffi::OsString::from);
        assert!(!command_has_codex_internal_workspace(&regular));
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
    fn claude_headless_resume_is_not_a_second_process_session() {
        assert!(is_claude_headless_resume(
            "/home/user/.local/bin/claude --print --resume session-id prompt"
        ));
        assert!(!is_claude_headless_resume(
            "/home/user/.local/bin/claude --resume session-id"
        ));
    }

    #[test]
    fn interruption_targets_only_the_headless_process_for_the_exact_claude_session() {
        let command = [
            std::borrow::Cow::Borrowed("/home/user/.local/bin/claude"),
            std::borrow::Cow::Borrowed("--print"),
            std::borrow::Cow::Borrowed("--resume"),
            std::borrow::Cow::Borrowed("session-id"),
            std::borrow::Cow::Borrowed("Continue"),
        ];
        assert!(resumed_prompt_command_matches(
            &command,
            "session-id",
            &AgentKind::ClaudeCode
        ));
        assert!(!resumed_prompt_command_matches(
            &command,
            "another-session",
            &AgentKind::ClaudeCode
        ));
        assert!(!resumed_prompt_command_matches(
            &command,
            "session-id",
            &AgentKind::Codex
        ));
    }

    #[test]
    fn versioned_claude_binary_is_a_session() {
        assert_eq!(
            detect_agent(
                "2.1.220",
                "/home/user/.local/share/claude/versions/2.1.220 --session-id 9b7acb3c --fork-session"
            ),
            Some(AgentKind::ClaudeCode)
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
    fn claude_detection_does_not_change_other_built_in_agents() {
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
            detect_agent("agy", "/usr/local/bin/agy"),
            Some(AgentKind::Antigravity)
        );
        assert_eq!(
            detect_agent("bash", "agy --conversation abc"),
            Some(AgentKind::Antigravity)
        );
        assert_eq!(
            detect_agent("agy.exe", r#"C:\Users\dev\bin\agy.exe"#),
            Some(AgentKind::Antigravity)
        );
        assert_eq!(
            detect_agent("dsh", "/usr/local/bin/dsh --profile tui"),
            Some(AgentKind::DeepSeek)
        );
        assert_eq!(
            detect_agent("dsh.exe", r#"C:\Users\dev\bin\dsh.exe --profile tui"#),
            Some(AgentKind::DeepSeek)
        );
        assert_eq!(
            detect_agent("claude", "/usr/bin/claude"),
            Some(AgentKind::ClaudeCode)
        );
        assert_eq!(
            detect_agent("bash", "claude --resume abc"),
            Some(AgentKind::ClaudeCode)
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
