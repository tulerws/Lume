import { invoke } from "@tauri-apps/api/core";
import type {
  AgentSession,
  CompanionStatus,
  HistoryEntry,
  IntegrationStatus,
  PermissionAction,
  Preferences,
} from "$lib/domain";
import { demoHistory, demoSessions } from "$lib/demo";

const inDesktop = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const defaultPreferences: Preferences = {
  soundEnabled: true,
  autostart: true,
  showOverFullscreen: false,
  historyRetentionDays: 30,
  launchTarget: "auto",
};

export async function loadSessions(): Promise<AgentSession[]> {
  try {
    return await invoke<AgentSession[]>("list_sessions");
  } catch {
    return inDesktop() ? [] : structuredClone(demoSessions);
  }
}

export async function decidePermission(
  sessionId: string,
  permissionId: string,
  action: PermissionAction,
): Promise<void> {
  await invoke("resolve_permission", {
    sessionId,
    permissionId,
    action,
  });
}

export async function openSessionSource(sessionId: string): Promise<void> {
  await invoke("open_session_source", { sessionId });
}

export async function loadHistory(): Promise<HistoryEntry[]> {
  try {
    return await invoke<HistoryEntry[]>("list_history", { limit: 100 });
  } catch {
    return inDesktop() ? [] : structuredClone(demoHistory);
  }
}

export async function loadPreferences(): Promise<Preferences> {
  try {
    return await invoke<Preferences>("get_preferences");
  } catch {
    return { ...defaultPreferences };
  }
}

export async function savePreferences(preferences: Preferences): Promise<void> {
  if (!("__TAURI_INTERNALS__" in window)) return;
  await invoke("set_preferences", { preferences });
}

export async function loadIntegrationStatuses(): Promise<IntegrationStatus[]> {
  if (!("__TAURI_INTERNALS__" in window)) {
    return [
      { kind: "codex", label: "Codex", installed: true, configured: false, directPermissions: false, detail: "Pronto para conectar" },
      { kind: "claude", label: "Claude", installed: true, configured: true, directPermissions: true, detail: "Monitoramento e decisões conectados" },
      { kind: "gemini", label: "Gemini", installed: true, configured: false, directPermissions: false, detail: "Pronto para conectar" },
    ];
  }
  return invoke<IntegrationStatus[]>("integration_statuses");
}

export async function configureIntegration(
  kind: IntegrationStatus["kind"],
  enabled: boolean,
): Promise<void> {
  await invoke("configure_integration", { kind, enabled });
}

export async function launchAgentSession(
  agent: IntegrationStatus["kind"],
  workingDirectory: string,
  resume: boolean,
  resumeId: string | undefined,
  target: Preferences["launchTarget"],
): Promise<void> {
  await invoke("launch_session", {
    request: { agent, workingDirectory, resume, resumeId, target },
  });
}

export async function loadVscodeStatus(): Promise<CompanionStatus> {
  if (!("__TAURI_INTERNALS__" in window)) {
    return { installed: true, configured: false, detail: "Necessário para abrir sessões no editor" };
  }
  return invoke<CompanionStatus>("vscode_status");
}

export async function configureVscode(enabled: boolean): Promise<void> {
  await invoke("configure_vscode", { enabled });
}

export async function revealBrowserCompanion(): Promise<string> {
  return invoke<string>("reveal_browser_companion");
}
