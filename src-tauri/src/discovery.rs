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
                        crate::remote_server::announce_sessions_changed(&app);
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
            if is_lume_codex_process(&command) {
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
            Some((*pid, process.parent(), agent, agent_label))
        })
        .collect::<Vec<_>>();
    let candidate_pids = candidates
        .iter()
        .map(|(pid, _, _, _)| *pid)
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
        .map(|(pid, _, agent, label)| (*pid, (agent.clone(), label.clone())))
        .collect::<HashMap<_, _>>();

    let discovered = candidates
        .into_iter()
        // Mantém o processo detectado mais próximo da raiz. Um comando executado
        // pelo agente pode conter "codex", "claude" ou "gemini" nos argumentos;
        // escolher esse descendente efêmero faria a sessão trocar de PID.
        .filter(|(pid, _, agent, label)| {
            !agents_by_pid
                .iter()
                .any(|(ancestor_pid, (ancestor_agent, ancestor_label))| {
                    ancestor_agent == agent
                        && ancestor_label == label
                        && process_descends_from(&system, *pid, *ancestor_pid)
                })
        })
        .filter_map(|(pid, _, agent, agent_label)| {
            let process = system.process(pid)?;
            Some(DiscoveredProcess {
                agent,
                agent_label,
                process_id: pid.as_u32(),
                working_directory: process.cwd().map(|path| path.to_string_lossy().to_string()),
                source: source_for(&system, pid),
            })
        })
        .collect();

    ProcessScan {
        discovered,
        live_pids,
    }
}

fn is_lume_codex_process(command: &str) -> bool {
    command.contains("127.0.0.1:43130") || command.contains("--remote ws://127.0.0.1:43131")
}

/// Subcomandos do Claude Code que são **infraestrutura**, não conversa.
///
/// O `claude daemon` supervisiona sessões de segundo plano e o `claude
/// bg-pty-host` hospeda o terminal de uma delas. Nenhum dos dois é uma sessão:
/// não têm conversa, não têm identificador de retomada, e o `cwd` do daemon é o
/// diretório de onde o serviço subiu — quase sempre `$HOME`.
///
/// Sem esta exclusão eles não apenas apareciam: eles **venciam**. A regra do
/// ancestral mais próximo da raiz existe para colapsar uma sessão e seus
/// subcomandos efêmeros num processo só, e supõe que o mais alto da cadeia é a
/// sessão. O daemon inverteu a suposição — ele é ancestral de várias sessões
/// independentes, então era ele quem sobrava, levando o `$HOME` junto. Um prompt
/// enviado do celular abria terminal no diretório errado.
///
/// **Escopado ao Claude de propósito.** A comparação exige a forma `claude
/// <subcomando>`, e não a palavra solta: um processo do Codex que trouxesse
/// `daemon` nos argumentos não pode ser atingido por isto. Codex e Gemini
/// atravessam esta função sem serem tocados, e há teste fixando isso.
fn is_claude_infrastructure(tokens: &[&str]) -> bool {
    const SUBCOMANDOS: [&str; 2] = ["daemon", "bg-pty-host"];
    tokens.windows(2).any(|par| {
        par[0] == "claude" && SUBCOMANDOS.contains(&par[1])
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

/// O executável mora dentro de um diretório chamado exatamente `claude`?
///
/// É como o Claude Code aparece quando roda a partir da versão instalada:
/// `~/.local/share/claude/versions/2.1.220`. O último segmento é o número da
/// versão, então a comparação por nome de arquivo — que é a que todos os outros
/// agentes usam — não encontra nada. O processo da sessão de verdade era
/// invisível para a descoberta, e sobrava apenas a infraestrutura ao redor dele.
///
/// A comparação é por **segmento exato**, e isso é o que limita o estrago:
/// `~/.claude/...` tem o segmento `.claude`, e `~/claude-notes/...` tem
/// `claude-notes` — nenhum dos dois casa. Só um diretório com o nome cru.
fn caminho_do_claude(token: &str) -> bool {
    token
        .split(['/', '\\'])
        .any(|segmento| segmento.trim_matches(['"', '\'']) == "claude")
}

fn detect_agent(name: &str, command: &str) -> Option<AgentKind> {
    let brutos = command.split_whitespace().collect::<Vec<_>>();
    let tokens = brutos
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
        // Depois do Codex, e não antes: preservar a ordem original é o que
        // garante que nenhum processo hoje classificado como Codex mude de
        // resposta por causa desta adição.
        None
    } else if name == "claude"
        || tokens.iter().any(|token| token == &"claude")
        // Só o executável, nunca os argumentos. `vim ~/claude/notas.md` tem um
        // segmento `claude` num argumento e não é sessão de agente nenhuma.
        || brutos.first().is_some_and(|executavel| caminho_do_claude(executavel))
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
    fn vscode_codex_app_server_is_not_ignored() {
        assert!(!is_lume_codex_process(
            "/home/user/.vscode/extensions/openai.chatgpt/bin/codex app-server"
        ));
    }

    #[test]
    fn lume_codex_bridge_processes_are_ignored() {
        assert!(is_lume_codex_process(
            "codex app-server --listen ws://127.0.0.1:43130"
        ));
        assert!(is_lume_codex_process(
            "codex --remote ws://127.0.0.1:43131 resume chat"
        ));
    }

    /// O caso que produziu o defeito.
    ///
    /// O `claude daemon` roda a partir de `$HOME` e é ancestral das sessões de
    /// segundo plano. Enquanto ele era candidato, a regra do ancestral mais
    /// próximo da raiz descartava as sessões e mantinha ele — e um prompt vindo
    /// do celular abria terminal em `$HOME`.
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

    /// O processo da sessão de verdade: o último segmento é a versão, não o nome.
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

    /// O segmento tem de ser o nome cru, e só no executável.
    #[test]
    fn claude_lookalikes_are_not_sessions() {
        // Diretório de configuração, não instalação.
        assert_eq!(detect_agent("nvim", "/usr/bin/nvim /home/user/.claude/settings.json"), None);
        // Nome que apenas começa igual.
        assert_eq!(detect_agent("nvim", "/usr/bin/nvim /home/user/claude-notes/a.md"), None);
        // Segmento certo, mas num argumento: quem abre o arquivo não é o agente.
        assert_eq!(detect_agent("nvim", "/usr/bin/nvim /home/user/claude/a.md"), None);
    }

    /// A promessa desta mudança: **Codex e Gemini não mudam de classificação.**
    ///
    /// O ramo do Codex é avaliado antes de tudo que foi acrescentado, e o do
    /// Gemini não foi tocado. Este teste é o que impede uma refatoração futura de
    /// reordenar os ramos sem perceber o que quebrou.
    #[test]
    fn codex_and_gemini_are_untouched() {
        assert_eq!(detect_agent("codex", "/usr/bin/codex"), Some(AgentKind::Codex));
        assert_eq!(detect_agent("bash", "codex resume abc"), Some(AgentKind::Codex));
        // `daemon` nos argumentos de um processo Codex não pode excluí-lo: a
        // exclusão exige a forma `claude <subcomando>`.
        assert_eq!(detect_agent("codex", "/usr/bin/codex daemon"), Some(AgentKind::Codex));
        assert_eq!(detect_agent("gemini", "/usr/bin/gemini"), Some(AgentKind::Gemini));
        assert_eq!(detect_agent("bash", "gemini chat"), Some(AgentKind::Gemini));
        // Claude comum, do jeito que sempre foi detectado.
        assert_eq!(detect_agent("claude", "/usr/bin/claude"), Some(AgentKind::Claude));
        assert_eq!(detect_agent("bash", "claude --resume abc"), Some(AgentKind::Claude));
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
