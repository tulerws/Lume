use std::path::PathBuf;

pub fn is_codex_internal_workspace(path: &str) -> bool {
    let normalized = normalize(path);
    if normalized.contains("/.codex/memories") {
        return true;
    }
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .map(|root| normalize(&root.join("memories").to_string_lossy()))
        .is_some_and(|memories| {
            normalized == memories || normalized.starts_with(&format!("{memories}/"))
        })
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_the_internal_codex_memories_workspace() {
        assert!(is_codex_internal_workspace("/home/user/.codex/memories"));
        assert!(is_codex_internal_workspace(
            "C:\\Users\\user\\.codex\\memories\\rollout_summaries"
        ));
        assert!(!is_codex_internal_workspace(
            "/home/user/Documents/memories"
        ));
    }
}
