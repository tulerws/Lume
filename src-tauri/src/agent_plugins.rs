use crate::integrations::IntegrationKind;

pub trait AgentPlugin: Send + Sync {
    fn kind(&self) -> IntegrationKind;
    fn label(&self) -> &'static str;
    fn executable(&self) -> &'static str;
    fn direct_permissions(&self) -> bool;
    fn hook_events(&self) -> &'static [&'static str];
}

struct BuiltInAgentPlugin {
    kind: IntegrationKind,
    label: &'static str,
    executable: &'static str,
    direct_permissions: bool,
    hook_events: &'static [&'static str],
}

impl AgentPlugin for BuiltInAgentPlugin {
    fn kind(&self) -> IntegrationKind {
        self.kind.clone()
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn executable(&self) -> &'static str {
        self.executable
    }

    fn direct_permissions(&self) -> bool {
        self.direct_permissions
    }

    fn hook_events(&self) -> &'static [&'static str] {
        self.hook_events
    }
}

pub fn catalog() -> Vec<Box<dyn AgentPlugin>> {
    vec![
        Box::new(BuiltInAgentPlugin {
            kind: IntegrationKind::Codex,
            label: "Codex",
            executable: "codex",
            direct_permissions: true,
            hook_events: &[
                "SessionStart",
                "UserPromptSubmit",
                "PermissionRequest",
                "Stop",
            ],
        }),
        Box::new(BuiltInAgentPlugin {
            kind: IntegrationKind::Claude,
            label: "Claude",
            executable: "claude",
            direct_permissions: true,
            hook_events: &[
                "SessionStart",
                "UserPromptSubmit",
                "PermissionRequest",
                "Notification",
                "Stop",
                "StopFailure",
                "SessionEnd",
            ],
        }),
        Box::new(BuiltInAgentPlugin {
            kind: IntegrationKind::Gemini,
            label: "Gemini",
            executable: "gemini",
            direct_permissions: false,
            hook_events: &[
                "SessionStart",
                "BeforeAgent",
                "Notification",
                "AfterAgent",
                "SessionEnd",
            ],
        }),
    ]
}

pub fn find(kind: &IntegrationKind) -> Option<Box<dyn AgentPlugin>> {
    catalog().into_iter().find(|plugin| plugin.kind() == *kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_plugins_expose_their_monitoring_contract() {
        let plugins = catalog();
        assert_eq!(plugins.len(), 3);
        let claude = plugins
            .iter()
            .find(|plugin| plugin.kind() == IntegrationKind::Claude)
            .expect("plugin Claude");
        assert!(claude.direct_permissions());
        assert!(claude.hook_events().contains(&"PermissionRequest"));
    }
}
