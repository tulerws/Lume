use crate::domain::Preferences;

#[cfg(target_os = "linux")]
use std::{fs, path::PathBuf};

pub fn configure(preferences: &Preferences) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_lowercase();
        if desktop.contains("cosmic") {
            configure_cosmic(preferences)?;
            return Ok(true);
        }
        if desktop.contains("gnome") {
            configure_gnome(preferences)?;
            return Ok(true);
        }
    }
    let _ = preferences;
    Ok(false)
}

#[cfg(target_os = "linux")]
fn shortcut_entries(preferences: &Preferences) -> [(&str, &str, &str); 4] {
    [
        (&preferences.open_shortcut, "open", "Open Lume"),
        (
            &preferences.global_shortcut,
            "palette",
            "Lume command palette",
        ),
        (
            &preferences.new_session_shortcut,
            "new-session",
            "New Lume session",
        ),
        (
            &preferences.whiteboard_shortcut,
            "whiteboard",
            "Lume whiteboard",
        ),
    ]
}

#[cfg(target_os = "linux")]
fn split_shortcut(shortcut: &str) -> Result<(Vec<&'static str>, String), String> {
    let mut tokens = shortcut.split('+').map(str::trim).collect::<Vec<_>>();
    let key = tokens
        .pop()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| format!("Atalho inválido: {shortcut}"))?;
    let modifiers = tokens
        .into_iter()
        .map(|modifier| match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Ok("Ctrl"),
            "alt" | "option" => Ok("Alt"),
            "shift" => Ok("Shift"),
            "super" | "cmd" | "command" => Ok("Super"),
            _ => Err(format!("Modificador inválido em {shortcut}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let key = if key.eq_ignore_ascii_case("space") {
        "space".into()
    } else if key.len() == 1 {
        key.to_lowercase()
    } else {
        key.into()
    };
    Ok((modifiers, key))
}

#[cfg(target_os = "linux")]
fn executable_command(action: &str) -> Result<String, String> {
    let executable = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| std::env::current_exe().map_err(|error| error.to_string()))?;
    Ok(format!(
        "{} shortcut {action}",
        shell_quote(&executable.to_string_lossy())
    ))
}

#[cfg(target_os = "linux")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "linux")]
fn configure_cosmic(preferences: &Preferences) -> Result<(), String> {
    let path = cosmic_shortcuts_path()?;
    let current = fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
    let block = cosmic_shortcut_block(preferences)?;
    let merged = merge_cosmic_config(&current, &block)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Diretório de atalhos do COSMIC inválido".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(".lume-shortcuts.tmp");
    fs::write(&temporary, merged).map_err(|error| error.to_string())?;
    fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn cosmic_shortcuts_path() -> Result<PathBuf, String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| "Diretório de configuração do COSMIC não encontrado".to_string())?;
    Ok(config.join("cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom"))
}

#[cfg(target_os = "linux")]
fn cosmic_shortcut_block(preferences: &Preferences) -> Result<String, String> {
    let mut block = String::from("    // LUME_SHORTCUTS_BEGIN\n");
    for (shortcut, action, label) in shortcut_entries(preferences) {
        let (modifiers, key) = split_shortcut(shortcut)?;
        let command = executable_command(action)?;
        block.push_str(&format!(
            "    (\n        modifiers: [{}],\n        key: {},\n        description: Some({}),\n    ): Spawn({}),\n",
            modifiers.join(", "),
            json_string(&key),
            json_string(label),
            json_string(&command),
        ));
    }
    block.push_str("    // LUME_SHORTCUTS_END\n");
    Ok(block)
}

#[cfg(target_os = "linux")]
fn merge_cosmic_config(current: &str, block: &str) -> Result<String, String> {
    const START: &str = "// LUME_SHORTCUTS_BEGIN";
    const END: &str = "// LUME_SHORTCUTS_END";
    let mut config = current.to_string();
    if let Some(start) = config.find(START) {
        let line_start = config[..start].rfind('\n').map_or(start, |index| index + 1);
        let end = config[start..]
            .find(END)
            .map(|index| start + index + END.len())
            .ok_or_else(|| "Bloco de atalhos do Lume incompleto no COSMIC".to_string())?;
        let line_end = config[end..]
            .find('\n')
            .map_or(end, |index| end + index + 1);
        config.replace_range(line_start..line_end, "");
    }
    let closing = config
        .rfind('}')
        .ok_or_else(|| "Configuração de atalhos do COSMIC inválida".to_string())?;
    let body_end = config[..closing].trim_end().len();
    config.replace_range(body_end..closing, "\n");
    let closing = config
        .rfind('}')
        .ok_or_else(|| "Configuração de atalhos do COSMIC inválida".to_string())?;
    config.insert_str(closing, block);
    Ok(config)
}

#[cfg(target_os = "linux")]
fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

#[cfg(target_os = "linux")]
fn configure_gnome(preferences: &Preferences) -> Result<(), String> {
    const BASE: &str = "org.gnome.settings-daemon.plugins.media-keys";
    let paths = shortcut_entries(preferences)
        .iter()
        .map(|(_, action, _)| {
            format!(
                "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/lume-{action}/"
            )
        })
        .collect::<Vec<_>>();
    let current = gsettings_output(&["get", BASE, "custom-keybindings"])?;
    let mut configured_paths = gvariant_strings(&current);
    for path in &paths {
        if !configured_paths.contains(path) {
            configured_paths.push(path.clone());
        }
    }
    let list = format!(
        "[{}]",
        configured_paths
            .iter()
            .map(|path| format!("'{}'", path.replace('\'', "\\'")))
            .collect::<Vec<_>>()
            .join(", ")
    );
    gsettings(&["set", BASE, "custom-keybindings", &list])?;

    for ((shortcut, action, label), path) in shortcut_entries(preferences).into_iter().zip(paths) {
        let schema =
            format!("org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:{path}");
        let binding = gnome_binding(shortcut)?;
        let command = executable_command(action)?;
        gsettings(&["set", &schema, "name", label])?;
        gsettings(&["set", &schema, "command", &command])?;
        gsettings(&["set", &schema, "binding", &binding])?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn gnome_binding(shortcut: &str) -> Result<String, String> {
    let (modifiers, key) = split_shortcut(shortcut)?;
    let modifiers = modifiers
        .iter()
        .map(|modifier| match *modifier {
            "Ctrl" => "<Control>",
            "Alt" => "<Alt>",
            "Shift" => "<Shift>",
            "Super" => "<Super>",
            _ => "",
        })
        .collect::<String>();
    Ok(format!("{modifiers}{key}"))
}

#[cfg(target_os = "linux")]
fn gsettings(args: &[&str]) -> Result<(), String> {
    let mut command = crate::executables::command("gsettings")?;
    let output = command
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(target_os = "linux")]
fn gsettings_output(args: &[&str]) -> Result<String, String> {
    let mut command = crate::executables::command("gsettings")?;
    let output = command
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "linux")]
fn gvariant_strings(value: &str) -> Vec<String> {
    value
        .split('\'')
        .enumerate()
        .filter_map(|(index, part)| (index % 2 == 1).then(|| part.to_string()))
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_the_lume_block_in_cosmic_config() {
        let before = "{\n    (modifiers: [], key: \"F9\"): System(Screenshot),\n}";
        let first = merge_cosmic_config(
            before,
            "\n    // LUME_SHORTCUTS_BEGIN\n    first,\n    // LUME_SHORTCUTS_END\n",
        )
        .expect("primeira gravação");
        let second = merge_cosmic_config(
            &first,
            "\n    // LUME_SHORTCUTS_BEGIN\n    second,\n    // LUME_SHORTCUTS_END\n",
        )
        .expect("atualização");

        assert!(second.contains("System(Screenshot)"));
        assert!(!second.contains("first"));
        assert_eq!(second.matches("LUME_SHORTCUTS_BEGIN").count(), 1);
    }

    #[test]
    fn converts_shortcuts_for_gnome() {
        assert_eq!(
            gnome_binding("Ctrl+Shift+Space").expect("atalho"),
            "<Control><Shift>space"
        );
    }
}
