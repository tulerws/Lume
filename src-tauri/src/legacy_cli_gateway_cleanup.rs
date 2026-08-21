use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::{json, Value};

const LEGACY_GATEWAY_ADDRESS: &str = "127.0.0.1:43132";
const LEGACY_WRAPPER_MARKER: &str = "LUME_CODEX_GATEWAY_WRAPPER";
const PATH_BLOCK_START: &str = "# >>> Lume Codex Gateway >>>";
const PATH_BLOCK_END: &str = "# <<< Lume Codex Gateway <<<";

pub fn cleanup(app_data_directory: &Path) -> Result<(), String> {
    let state_directory = app_data_directory.join("gateway");
    let wrapper_paths = legacy_wrapper_paths(&state_directory);
    let wrapper_directories = wrapper_paths
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    let mut errors = Vec::new();

    stop_legacy_gateway(&state_directory);
    for directory in &wrapper_directories {
        remove_from_current_path(directory);
    }

    #[cfg(not(target_os = "windows"))]
    if let Err(error) = cleanup_unix_profiles() {
        errors.push(error);
    }
    #[cfg(target_os = "windows")]
    for directory in &wrapper_directories {
        if let Err(error) = cleanup_windows_path(directory) {
            errors.push(error);
        }
    }

    for path in &wrapper_paths {
        if let Err(error) = remove_owned_file(path, LEGACY_WRAPPER_MARKER) {
            errors.push(error);
        }
    }
    for name in [
        "codex-launcher.json",
        "gateway.sqlite3",
        "gateway.sqlite3-shm",
        "gateway.sqlite3-wal",
        "gateway.token",
    ] {
        if let Err(error) = remove_known_file(&state_directory.join(name)) {
            errors.push(error);
        }
    }
    for directory in &wrapper_directories {
        remove_empty_directory(directory);
    }
    remove_empty_directory(&state_directory);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn stop_legacy_gateway(state_directory: &Path) {
    let Ok(token) = fs::read_to_string(state_directory.join("gateway.token")) else {
        return;
    };
    let token = token.trim();
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return;
    }
    let Ok(address) = LEGACY_GATEWAY_ADDRESS.parse::<SocketAddr>() else {
        return;
    };
    for protocol_version in (1..=5).rev() {
        let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(120))
        else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(180)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(180)));
        let request = json!({
            "protocolVersion": protocol_version,
            "token": token,
            "command": "shutdown"
        });
        if serde_json::to_writer(&mut stream, &request).is_err()
            || stream.write_all(b"\n").is_err()
            || stream.flush().is_err()
        {
            continue;
        }
        let mut response = String::new();
        if BufReader::new(stream).read_line(&mut response).is_ok()
            && serde_json::from_str::<Value>(&response)
                .ok()
                .and_then(|value| value.get("ok").and_then(Value::as_bool))
                == Some(true)
        {
            break;
        }
    }
}

fn remove_from_current_path(wrapper_directory: &Path) {
    let Some(current) = env::var_os("PATH") else {
        return;
    };
    let filtered = env::split_paths(&current)
        .filter(|entry| !same_path(entry, wrapper_directory))
        .collect::<Vec<_>>();
    if let Ok(path) = env::join_paths(filtered) {
        env::set_var("PATH", path);
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || left
            .canonicalize()
            .ok()
            .zip(right.canonicalize().ok())
            .is_some_and(|(left, right)| left == right)
}

#[cfg(not(target_os = "windows"))]
fn cleanup_unix_profiles() -> Result<(), String> {
    let home = env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        "Could not locate the home directory for legacy Gateway cleanup".to_string()
    })?;
    for profile in [
        home.join(".profile"),
        home.join(".bashrc"),
        home.join(".zshrc"),
    ] {
        if !profile.exists() {
            continue;
        }
        let current = fs::read_to_string(&profile)
            .map_err(|error| format!("Could not read {}: {error}", profile.display()))?;
        let updated = remove_path_blocks(&current);
        if updated != current {
            fs::write(&profile, updated)
                .map_err(|error| format!("Could not update {}: {error}", profile.display()))?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn remove_path_blocks(contents: &str) -> String {
    let mut updated = contents.to_string();
    while let Some(start) = updated.find(PATH_BLOCK_START) {
        let Some(relative_end) = updated[start..].find(PATH_BLOCK_END) else {
            break;
        };
        let mut end = start + relative_end + PATH_BLOCK_END.len();
        if updated[end..].starts_with('\n') {
            end += 1;
        }
        updated.replace_range(start..end, "");
    }
    while updated.ends_with("\n\n") {
        updated.pop();
    }
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated
}

#[cfg(target_os = "windows")]
fn cleanup_windows_path(wrapper_directory: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$bin=$env:LUME_LEGACY_GATEWAY_BIN; $current=[Environment]::GetEnvironmentVariable('Path','User'); $items=@($current -split ';' | Where-Object { $_ -and $_.TrimEnd('\\') -ine $bin.TrimEnd('\\') }); [Environment]::SetEnvironmentVariable('Path',($items -join ';'),'User')",
        ])
        .env("LUME_LEGACY_GATEWAY_BIN", wrapper_directory)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("Could not clean the Windows user PATH: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Windows refused the legacy Gateway PATH cleanup".to_string())
}

#[cfg(not(target_os = "windows"))]
fn legacy_wrapper_paths(state_directory: &Path) -> Vec<PathBuf> {
    vec![state_directory.join("bin/codex")]
}

#[cfg(target_os = "windows")]
fn legacy_wrapper_paths(state_directory: &Path) -> Vec<PathBuf> {
    let mut paths = vec![state_directory.join("bin/codex.cmd")];
    if let Some(directory) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        let legacy = directory.join("Lume/bin/codex.cmd");
        if !paths.contains(&legacy) {
            paths.push(legacy);
        }
    }
    paths
}

fn remove_owned_file(path: &Path, marker: &str) -> Result<(), String> {
    if !path.is_file() {
        return Ok(());
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if contents.contains(marker) {
        fs::remove_file(path)
            .map_err(|error| format!("Could not remove {}: {error}", path.display()))?;
    }
    Ok(())
}

fn remove_known_file(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("Could not remove {}: {error}", path.display()))?;
    }
    Ok(())
}

fn remove_empty_directory(path: &Path) {
    let _ = fs::remove_dir(path);
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "windows"))]
    use super::{remove_path_blocks, PATH_BLOCK_END, PATH_BLOCK_START};

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn removes_only_lume_gateway_path_blocks() {
        let original = format!(
            "export PATH=\"$HOME/.local/bin:$PATH\"\n{PATH_BLOCK_START}\nLUME_CODEX_GATEWAY_BIN='/tmp/lume'\nexport PATH=\"$LUME_CODEX_GATEWAY_BIN:$PATH\"\n{PATH_BLOCK_END}\nexport KEEP_ME=1\n"
        );
        assert_eq!(
            remove_path_blocks(&original),
            "export PATH=\"$HOME/.local/bin:$PATH\"\nexport KEEP_ME=1\n"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn path_cleanup_is_idempotent() {
        let contents = "export KEEP_ME=1\n";
        assert_eq!(remove_path_blocks(contents), contents);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn incomplete_marker_never_truncates_the_user_profile() {
        let contents = format!("export KEEP_BEFORE=1\n{PATH_BLOCK_START}\nexport KEEP_AFTER=1\n");
        assert_eq!(remove_path_blocks(&contents), contents);
    }
}
