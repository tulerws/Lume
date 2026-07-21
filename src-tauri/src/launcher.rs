use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

use crate::{integrations::IntegrationKind, state::now_millis};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRequest {
    pub agent: IntegrationKind,
    pub working_directory: String,
    pub resume: bool,
    pub resume_id: Option<String>,
    pub target: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct TerminalPayload {
    command: String,
    arguments: Vec<String>,
    working_directory: String,
}

pub fn launch(
    request: LaunchRequest,
    executable: &Path,
    app_data_dir: &Path,
    codex_remote: Option<&str>,
) -> Result<(), String> {
    let payload = payload_for(&request, codex_remote);
    if request.target == "vscode" {
        if crate::integrations::vscode_status().configured {
            return launch_vscode(&payload, &request.agent);
        }
        return Err("Conecte o Lume Companion ao VS Code nos Ajustes".into());
    }
    launch_terminal(payload, executable, app_data_dir)
}

pub fn run_terminal_payload(path: &str) -> i32 {
    let path = PathBuf::from(path);
    let payload = match fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|value| {
            serde_json::from_str::<TerminalPayload>(&value).map_err(|error| error.to_string())
        }) {
        Ok(payload) => payload,
        Err(error) => {
            eprintln!("Não foi possível abrir a sessão do Lume: {error}");
            return 1;
        }
    };
    let _ = fs::remove_file(path);
    match Command::new(&payload.command)
        .args(&payload.arguments)
        .current_dir(&payload.working_directory)
        .status()
    {
        Ok(status) => status.code().unwrap_or_default(),
        Err(error) => {
            eprintln!("Não foi possível iniciar {}: {error}", payload.command);
            1
        }
    }
}

fn payload_for(request: &LaunchRequest, codex_remote: Option<&str>) -> TerminalPayload {
    let (command, mut arguments) = match request.agent {
        IntegrationKind::Codex => {
            let mut arguments = Vec::new();
            if let Some(remote) = codex_remote {
                arguments.extend(["--remote".into(), remote.into()]);
            }
            ("codex".to_string(), arguments)
        }
        IntegrationKind::Claude => ("claude".to_string(), Vec::new()),
        IntegrationKind::Gemini => ("gemini".to_string(), Vec::new()),
    };
    if request.resume {
        match request.agent {
            IntegrationKind::Codex => {
                arguments.push("resume".into());
                if let Some(id) = &request.resume_id {
                    arguments.push(id.clone());
                }
            }
            IntegrationKind::Claude | IntegrationKind::Gemini => {
                arguments.push("--resume".into());
                if let Some(id) = &request.resume_id {
                    arguments.push(id.clone());
                }
            }
        }
    }
    TerminalPayload {
        command,
        arguments,
        working_directory: request.working_directory.clone(),
    }
}

#[cfg(target_os = "linux")]
fn launch_terminal(
    payload: TerminalPayload,
    executable: &Path,
    app_data_dir: &Path,
) -> Result<(), String> {
    let launches = app_data_dir.join("launches");
    fs::create_dir_all(&launches).map_err(|error| error.to_string())?;
    let id = now_millis();
    let payload_path = launches.join(format!("{id}.json"));
    let desktop_path = launches.join(format!("{id}.desktop"));
    fs::write(
        &payload_path,
        serde_json::to_vec(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=Lume session\nExec=\"{}\" terminal-run \"{}\"\nTerminal=true\nNoDisplay=true\n",
        desktop_escape(executable),
        desktop_escape(&payload_path)
    );
    fs::write(&desktop_path, desktop).map_err(|error| error.to_string())?;
    Command::new("gio")
        .arg("launch")
        .arg(&desktop_path)
        .spawn()
        .map_err(|error| format!("Não foi possível abrir o terminal: {error}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_terminal(
    payload: TerminalPayload,
    _executable: &Path,
    _app_data_dir: &Path,
) -> Result<(), String> {
    if command_available("wt.exe") {
        Command::new("wt.exe")
            .arg("-d")
            .arg(&payload.working_directory)
            .arg(&payload.command)
            .args(&payload.arguments)
            .spawn()
            .map_err(|error| error.to_string())?;
    } else {
        Command::new("cmd.exe")
            .args(["/C", "start", "", "cmd.exe", "/K", &payload.command])
            .args(&payload.arguments)
            .current_dir(&payload.working_directory)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn launch_terminal(
    _payload: TerminalPayload,
    _executable: &Path,
    _app_data_dir: &Path,
) -> Result<(), String> {
    Err("Esta plataforma ainda não possui um lançador de terminal".into())
}

fn launch_vscode(payload: &TerminalPayload, agent: &IntegrationKind) -> Result<(), String> {
    let request = serde_json::json!({
        "agent": match agent {
            IntegrationKind::Codex => "codex",
            IntegrationKind::Claude => "claude",
            IntegrationKind::Gemini => "gemini",
        },
        "cwd": payload.working_directory,
        "args": payload.arguments,
    });
    let encoded = percent_encode(&request.to_string());
    Command::new("code")
        .arg("--reuse-window")
        .arg(format!("vscode://tulerws.lume/session?payload={encoded}"))
        .spawn()
        .map_err(|error| format!("Não foi possível abrir o VS Code: {error}"))?;
    Ok(())
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn desktop_escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
}

#[cfg(target_os = "windows")]
fn command_available(command: &str) -> bool {
    Command::new(command).arg("--version").output().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(agent: IntegrationKind, resume: bool, resume_id: Option<&str>) -> LaunchRequest {
        LaunchRequest {
            agent,
            working_directory: "/work/project".into(),
            resume,
            resume_id: resume_id.map(str::to_string),
            target: "auto".into(),
        }
    }

    #[test]
    fn codex_remote_is_applied_before_resume_subcommand() {
        let payload = payload_for(
            &request(IntegrationKind::Codex, true, Some("thread-id")),
            Some("ws://127.0.0.1:43131"),
        );
        assert_eq!(payload.command, "codex");
        assert_eq!(
            payload.arguments,
            vec!["--remote", "ws://127.0.0.1:43131", "resume", "thread-id"]
        );
    }

    #[test]
    fn claude_resume_keeps_native_cli_shape() {
        let payload = payload_for(
            &request(IntegrationKind::Claude, true, Some("session-id")),
            None,
        );
        assert_eq!(payload.command, "claude");
        assert_eq!(payload.arguments, vec!["--resume", "session-id"]);
    }
}
