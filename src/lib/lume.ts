import { invoke } from "@tauri-apps/api/core";
import type {
  AgentSession,
  CompanionStatus,
  HistoryEntry,
  IntegrationDiagnostic,
  IntegrationStatus,
  PermissionAction,
  Preferences,
  ResultNote,
  RestoredTerminalPlacement,
  ExternalAgentPlugin,
  TerminalWindowState,
  WhiteboardLayout,
} from "$lib/domain";
import { demoHistory, demoSessions } from "$lib/demo";

const inDesktop = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const defaultPreferences: Preferences = {
  language: "en",
  darkMode: undefined,
  soundEnabled: true,
  autostart: true,
  overlayX: undefined,
  overlayY: undefined,
  showOverFullscreen: false,
  historyRetentionDays: 30,
  launchTarget: "auto",
  projectProfiles: {},
  whiteboardLayouts: [],
  globalShortcut: "Ctrl+Shift+Space",
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

export async function moveOverlay(
  x: number,
  y: number,
  persist: boolean,
  monitorId?: string,
): Promise<void> {
  await invoke("move_overlay", { x: Math.round(x), y: Math.round(y), persist, monitorId });
}

export async function submitPrompt(sessionId: string, prompt: string): Promise<void> {
  await invoke("submit_prompt", { sessionId, prompt });
}

export async function terminateSession(sessionId: string): Promise<void> {
  await invoke("terminate_session", { sessionId });
}

export async function openTerminalWindow(sessionId: string): Promise<string> {
  return invoke<string>("open_terminal_window", { sessionId });
}

export async function loadTerminalWindows(): Promise<TerminalWindowState[]> {
  if (!inDesktop()) return [];
  return invoke<TerminalWindowState[]>("list_terminal_windows");
}

export async function loadTerminalWindowState(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("get_terminal_window_state", { label });
}

export async function closeTerminalWindow(label: string): Promise<void> {
  await invoke("close_terminal_window", { label });
}

export async function moveTerminalWindow(
  label: string,
  x: number,
  y: number,
  finalize: boolean,
): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("move_terminal_window", {
    label,
    x: Math.round(x),
    y: Math.round(y),
    finalize,
  });
}

export async function cancelTerminalWindowMove(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("cancel_terminal_window_move", { label });
}

export async function syncTerminalWindowPosition(
  label: string,
  x: number,
  y: number,
  finalize: boolean,
): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("sync_terminal_window_position", {
    label,
    x: Math.round(x),
    y: Math.round(y),
    finalize,
  });
}

export async function resizeTerminalWindow(
  label: string,
  x: number,
  y: number,
  width: number,
  height: number,
): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("resize_terminal_window", {
    label,
    x: Math.round(x),
    y: Math.round(y),
    width: Math.round(width),
    height: Math.round(height),
  });
}

export async function beginLayeredTerminalResize(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("begin_layered_terminal_resize", { label });
}

export async function finishLayeredTerminalResize(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("finish_layered_terminal_resize", { label });
}

export async function undockTerminalWindow(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("undock_terminal_window", { label });
}

export async function restoreTerminalLayout(
  entries: RestoredTerminalPlacement[],
): Promise<TerminalWindowState[]> {
  return invoke<TerminalWindowState[]>("restore_terminal_layout", { entries });
}

export async function loadHistory(): Promise<HistoryEntry[]> {
  try {
    return await invoke<HistoryEntry[]>("list_history", { limit: 100 });
  } catch {
    return inDesktop() ? [] : structuredClone(demoHistory);
  }
}

export async function loadResultNotes(): Promise<ResultNote[]> {
  if (!inDesktop()) return [];
  return invoke<ResultNote[]>("list_result_notes", { limit: 100 });
}

export async function saveResultNote(
  sessionId: string,
  resultId: string,
  title: string,
): Promise<ResultNote> {
  return invoke<ResultNote>("save_result_note", { sessionId, resultId, title });
}

export async function deleteResultNote(id: string): Promise<void> {
  await invoke("delete_result_note", { id });
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

export async function diagnoseIntegration(
  kind: IntegrationStatus["kind"],
): Promise<IntegrationDiagnostic> {
  return invoke<IntegrationDiagnostic>("diagnose_integration", { kind });
}

export async function launchAgentSession(
  agent: IntegrationStatus["kind"],
  workingDirectory: string,
  resume: boolean,
  resumeId: string | undefined,
  target: Preferences["launchTarget"],
  permissionMode?: Preferences["projectProfiles"][string]["permissionMode"],
  approvalPolicy?: Preferences["projectProfiles"][string]["approvalPolicy"],
): Promise<void> {
  await invoke("launch_session", {
    request: {
      agent,
      workingDirectory,
      resume,
      resumeId,
      target,
      initialPrompt: undefined,
      permissionMode,
      approvalPolicy,
    },
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

export async function loadExternalPlugins(): Promise<ExternalAgentPlugin[]> {
  if (!inDesktop()) return [];
  return invoke<ExternalAgentPlugin[]>("list_external_plugins");
}

export async function installExternalPlugin(path: string): Promise<ExternalAgentPlugin> {
  return invoke<ExternalAgentPlugin>("install_external_plugin", { path });
}

export async function removeExternalPlugin(id: string): Promise<void> {
  await invoke("remove_external_plugin", { id });
}

export async function revealPluginDirectory(): Promise<string> {
  return invoke<string>("reveal_plugin_directory");
}
