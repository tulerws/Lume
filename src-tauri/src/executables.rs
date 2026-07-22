use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

pub fn command(name: &str) -> Result<Command, String> {
    let path = resolve(name).ok_or_else(|| {
        format!(
            "Could not find `{name}`. Open a terminal, confirm `{name} --version`, and restart Lume."
        )
    })?;
    let mut command = platform_command(&path);
    prepend_parent_to_path(&mut command, &path);
    Ok(command)
}

pub fn available(name: &str) -> bool {
    resolve(name).is_some()
}

pub fn path(name: &str) -> Option<PathBuf> {
    resolve(name)
}

fn resolve(name: &str) -> Option<PathBuf> {
    explicit_path(name)
        .or_else(|| path_lookup(name))
        .or_else(|| shell_lookup(name))
        .or_else(|| common_user_path(name))
}

fn explicit_path(name: &str) -> Option<PathBuf> {
    let variable = format!("LUME_{}_PATH", name.to_ascii_uppercase());
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn path_lookup(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|value| {
        env::split_paths(&value)
            .flat_map(|directory| executable_candidates(&directory, name))
            .find(|path| path.is_file())
    })
}

#[cfg(not(target_os = "windows"))]
fn shell_lookup(name: &str) -> Option<PathBuf> {
    let shell = env::var_os("SHELL").unwrap_or_else(|| "/bin/bash".into());
    let output = Command::new(shell)
        .args(["-ic", &format!("command -v -- {name}")])
        .output()
        .ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with('/'))
            .map(PathBuf::from)
    })?
}

#[cfg(target_os = "windows")]
fn shell_lookup(name: &str) -> Option<PathBuf> {
    let output = Command::new("where.exe").arg(name).output().ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(PathBuf::from)
    })?
}

fn common_user_path(name: &str) -> Option<PathBuf> {
    let home = env::var_os(if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    })
    .map(PathBuf::from)?;
    let mut directories = vec![home.join(".local/bin"), home.join(".cargo/bin")];
    #[cfg(target_os = "windows")]
    if let Some(app_data) = env::var_os("APPDATA") {
        directories.push(PathBuf::from(app_data).join("npm"));
    }
    #[cfg(not(target_os = "windows"))]
    if let Ok(versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        directories.extend(versions.flatten().map(|entry| entry.path().join("bin")));
    }
    directories
        .into_iter()
        .flat_map(|directory| executable_candidates(&directory, name))
        .find(|path| path.is_file())
}

fn executable_candidates(directory: &Path, name: &str) -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return ["exe", "cmd", "bat", ""]
            .into_iter()
            .map(|extension| {
                if extension.is_empty() {
                    directory.join(name)
                } else {
                    directory.join(format!("{name}.{extension}"))
                }
            })
            .collect();
    }
    #[cfg(not(target_os = "windows"))]
    vec![directory.join(name)]
}

#[cfg(not(target_os = "windows"))]
fn platform_command(path: &Path) -> Command {
    Command::new(path)
}

#[cfg(target_os = "windows")]
fn platform_command(path: &Path) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("cmd" | "bat")
    ) {
        let mut command = Command::new("cmd.exe");
        command
            .args(["/D", "/S", "/C"])
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW);
        command
    } else {
        Command::new(path)
    }
}

fn prepend_parent_to_path(command: &mut Command, executable: &Path) {
    let Some(parent) = executable.parent() else {
        return;
    };
    let mut paths = vec![parent.to_path_buf()];
    if let Some(current) = env::var_os("PATH") {
        paths.extend(env::split_paths(&current));
    }
    if let Ok(path) = env::join_paths(paths) {
        command.env("PATH", path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_candidates_include_the_plain_unix_name() {
        let candidates = executable_candidates(Path::new("/tmp/bin"), "codex");
        assert!(candidates.iter().any(|path| path.ends_with("codex")));
    }
}
