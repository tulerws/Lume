use std::{collections::HashMap, thread, time::Duration};

use sysinfo::{get_current_pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Emitter};

use crate::{
    domain::{AgentKind, SessionSource},
    state::AppState,
};

#[derive(Clone, Debug)]
pub struct DiscoveredProcess {
    pub agent: AgentKind,
    pub process_id: u32,
    pub working_directory: Option<String>,
    pub source: SessionSource,
}

pub fn start(state: AppState, app: AppHandle) -> Result<(), String> {
    thread::Builder::new()
        .name("lume-process-discovery".into())
        .spawn(move || loop {
            if let Ok(changed) = state.reconcile_processes(scan()) {
                if changed {
                    let _ = app.emit("lume://sessions-changed", ());
                }
            }
            thread::sleep(Duration::from_secs(5));
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn scan() -> Vec<DiscoveredProcess> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .without_tasks(),
    );
    let own_pid = get_current_pid().ok();
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
            if command.contains("app-server") || command.contains("--remote ws://127.0.0.1:43131") {
                return None;
            }
            let agent = detect_agent(&name, &command)?;
            Some((*pid, process.parent(), agent))
        })
        .collect::<Vec<_>>();
    let agents_by_pid = candidates
        .iter()
        .map(|(pid, _, agent)| (*pid, agent.clone()))
        .collect::<HashMap<_, _>>();

    candidates
        .into_iter()
        // Launchers commonly spawn the real Node/native process. Keep the leaf only.
        .filter(|(pid, _, agent)| {
            !agents_by_pid.iter().any(|(child_pid, child_agent)| {
                child_agent == agent
                    && system
                        .process(*child_pid)
                        .and_then(|process| process.parent())
                        == Some(*pid)
            })
        })
        .filter_map(|(pid, _, agent)| {
            let process = system.process(pid)?;
            Some(DiscoveredProcess {
                agent,
                process_id: pid.as_u32(),
                working_directory: process.cwd().map(|path| path.to_string_lossy().to_string()),
                source: source_for(&system, pid),
            })
        })
        .collect()
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

fn detect_agent(name: &str, command: &str) -> Option<AgentKind> {
    let tokens = command
        .split_whitespace()
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
    } else if name == "claude" || tokens.iter().any(|token| token == &"claude") {
        Some(AgentKind::Claude)
    } else if name == "gemini" || tokens.iter().any(|token| token == &"gemini") {
        Some(AgentKind::Gemini)
    } else {
        None
    }
}
