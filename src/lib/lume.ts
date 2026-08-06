import { invoke } from "@tauri-apps/api/core";
import type {
  HubCommandRequest,
  HubCommandResponse,
  HubSnapshot,
} from "$lib/hubProtocol";
import type {
  AgentSession,
  CompanionStatus,
  DockSide,
  HistoryEntry,
  IntegrationDiagnostic,
  IntegrationStatus,
  MobileGatewayStatus,
  MobilePairingOffer,
  MobileScope,
  PairedDevice,
  PermissionAction,
  Preferences,
  PromptAttachmentInput,
  PromptDelivery,
  QuestionAnswer,
  ResumableSession,
  ResultNote,
  RestoredTerminalPlacement,
  ExternalAgentPlugin,
  TerminalWindowState,
  WhiteboardLayout,
  WorkflowContextPackage,
  WorkflowGroupDefinition,
  WorkflowRole,
  WorkflowRoleContract,
  WorkflowRun,
} from "$lib/domain";
import { demoHistory, demoSessions } from "$lib/demo";

const inDesktop = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const defaultPreferences: Preferences = {
  language: "en",
  darkMode: undefined,
  soundEnabled: true,
  soundVolume: 55,
  popupNotificationsEnabled: true,
  autostart: true,
  overlayX: undefined,
  overlayY: undefined,
  showOverFullscreen: false,
  historyRetentionDays: 30,
  launchTarget: "auto",
  projectProfiles: {},
  sessionAliases: {},
  whiteboardLayouts: [],
  workflowEnabled: false,
  workflowGroups: [],
  globalShortcut: "Ctrl+Shift+Space",
  openShortcut: "Ctrl+Alt+Shift+L",
  newSessionShortcut: "Ctrl+Alt+Shift+N",
  whiteboardShortcut: "Ctrl+Alt+Shift+B",
};

export async function loadSessions(): Promise<AgentSession[]> {
  try {
    return await invoke<AgentSession[]>("list_sessions");
  } catch {
    return inDesktop() ? [] : structuredClone(demoSessions);
  }
}

export async function renameSession(sessionId: string, name: string): Promise<string> {
  return invoke<string>("rename_session", { sessionId, name });
}

export async function loadHubSnapshot(): Promise<HubSnapshot> {
  return invoke<HubSnapshot>("get_hub_snapshot");
}

export async function loadTerminalHubSnapshot(label: string, activityLimit = 60): Promise<HubSnapshot> {
  return invoke<HubSnapshot>("get_terminal_hub_snapshot", { label, activityLimit });
}

export async function loadMobileGatewayStatus(): Promise<MobileGatewayStatus> {
  return invoke<MobileGatewayStatus>("get_mobile_gateway_status");
}

export async function enableMobileGateway(): Promise<MobileGatewayStatus> {
  return invoke<MobileGatewayStatus>("enable_mobile_gateway");
}

export async function disableMobileGateway(): Promise<MobileGatewayStatus> {
  return invoke<MobileGatewayStatus>("disable_mobile_gateway");
}

export async function beginMobilePairing(): Promise<MobilePairingOffer> {
  return invoke<MobilePairingOffer>("begin_mobile_pairing");
}

export async function loadPairedDevices(): Promise<PairedDevice[]> {
  return invoke<PairedDevice[]>("list_paired_devices");
}

export async function revokePairedDevice(id: string): Promise<boolean> {
  return invoke<boolean>("revoke_paired_device", { id });
}

export async function setPairedDeviceScopes(
  id: string,
  scopes: MobileScope[],
): Promise<boolean> {
  return invoke<boolean>("set_paired_device_scopes", { id, scopes });
}

export async function executeHubCommand(
  request: HubCommandRequest,
): Promise<HubCommandResponse> {
  return invoke<HubCommandResponse>("execute_hub_command", { request });
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

export async function answerQuestion(
  sessionId: string,
  questionId: string,
  answers: QuestionAnswer[],
): Promise<void> {
  await invoke("resolve_question", {
    sessionId,
    questionId,
    answers,
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

export async function resizeOverlaySurface(width: number, height: number): Promise<void> {
  await invoke("resize_overlay_surface", {
    width: Math.max(1, Math.round(width)),
    height: Math.max(1, Math.round(height)),
  });
}

export async function submitPrompt(
  sessionId: string,
  prompt: string,
  attachments: PromptAttachmentInput[] = [],
  delivery: PromptDelivery = "new_turn",
): Promise<void> {
  await invoke("submit_prompt", { sessionId, prompt, attachments, delivery });
}

export async function readLocalImageDataUrl(path: string): Promise<string> {
  return invoke<string>("read_local_image_data_url", { path });
}

export async function exportLocalFile(sourcePath: string, destinationPath: string): Promise<void> {
  await invoke("export_local_file", { sourcePath, destinationPath });
}

export async function setTerminalFileDialogActive(
  label: string,
  active: boolean,
): Promise<void> {
  await invoke("set_terminal_file_dialog_active", { label, active });
}

export async function refreshAgentRateLimits(agent: AgentSession["agent"]): Promise<void> {
  await invoke("refresh_agent_rate_limits", { agent });
}

export async function terminateSession(sessionId: string): Promise<void> {
  await invoke("terminate_session", { sessionId });
}

export async function interruptPrompt(sessionId: string): Promise<void> {
  await invoke("interrupt_prompt", { sessionId });
}

export type CollaborationMode = "default" | "plan";

export async function getSessionCollaborationMode(
  sessionId: string,
): Promise<CollaborationMode> {
  return invoke<CollaborationMode>("get_session_collaboration_mode", { sessionId });
}

export async function setSessionCollaborationMode(
  sessionId: string,
  mode: CollaborationMode,
): Promise<CollaborationMode> {
  return invoke<CollaborationMode>("set_session_collaboration_mode", { sessionId, mode });
}

export async function steerQueuedPrompt(
  sessionId: string,
  activityId: string,
): Promise<void> {
  await invoke("steer_queued_prompt", { sessionId, activityId });
}

export async function openTerminalWindow(sessionId: string): Promise<string> {
  return invoke<string>("open_terminal_window", { sessionId });
}

export async function markTerminalFrontendReady(label: string): Promise<void> {
  await invoke("terminal_frontend_ready", { label });
}

export async function toggleTerminalGroupFullscreen(label: string): Promise<boolean | null> {
  return invoke<boolean | null>("toggle_terminal_group_fullscreen", { label });
}

export async function terminalGroupFullscreenActive(label: string): Promise<boolean> {
  return invoke<boolean>("terminal_group_fullscreen_active", { label });
}

export async function loadTerminalWindows(): Promise<TerminalWindowState[]> {
  if (!inDesktop()) return [];
  return invoke<TerminalWindowState[]>("list_terminal_windows");
}

export async function setTerminalWindowsVisible(visible: boolean): Promise<void> {
  if (!inDesktop()) return;
  await invoke("set_terminal_windows_visible", { visible });
}

export async function loadTerminalWindowState(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("get_terminal_window_state", { label });
}

export async function minimizeTerminalWindow(label: string): Promise<void> {
  await invoke("minimize_terminal_window", { label });
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

export async function loadTerminalDragSnapshot(
  label: string,
): Promise<{ pressed: boolean; x: number; y: number }> {
  return invoke("terminal_drag_snapshot", { label });
}

export async function beginTerminalNativeDrag(label: string): Promise<void> {
  await invoke("begin_terminal_native_drag", { label });
}

export async function resizeTerminalWindow(
  label: string,
  x: number,
  y: number,
  width: number,
  height: number,
  fromLeft: boolean,
  fromTop: boolean,
): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("resize_terminal_window", {
    label,
    x: Math.round(x),
    y: Math.round(y),
    width: Math.round(width),
    height: Math.round(height),
    fromLeft,
    fromTop,
  });
}

export async function beginLayeredTerminalResize(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("begin_layered_terminal_resize", { label });
}

export async function finishLayeredTerminalResize(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("finish_layered_terminal_resize", { label });
}

export type WorkflowBridgeContext = {
  groupId: string;
  sourceSessionNativeId: string;
  targetSessionNativeId: string;
  side: DockSide;
  nativeConnectors: boolean;
  height: number;
};

export async function openWorkflowBridgeWindow(label: string, side: DockSide): Promise<string> {
  return invoke<string>("open_workflow_bridge_window", { label, side });
}

export async function setWorkflowConnectionHover(
  label: string,
  side: DockSide,
  hovered: boolean,
): Promise<void> {
  await invoke("set_workflow_connection_hover", { label, side, hovered });
}

export async function prepareWorkflowBridgeWindow(label: string, side: DockSide): Promise<string> {
  return invoke<string>("prepare_workflow_bridge_window", { label, side });
}

export async function discardPreparedWorkflowBridgeWindow(label: string): Promise<void> {
  await invoke("discard_prepared_workflow_bridge_window", { label });
}

export async function loadWorkflowBridgeContext(label: string): Promise<WorkflowBridgeContext> {
  return invoke<WorkflowBridgeContext>("get_workflow_bridge_context", { label });
}

export async function setWorkflowBridgeExpanded(
  label: string,
  expanded: boolean,
  contentHeight?: number,
): Promise<void> {
  await invoke("set_workflow_bridge_expanded", { label, expanded, contentHeight });
}

export async function undockTerminalWindow(label: string): Promise<TerminalWindowState> {
  return invoke<TerminalWindowState>("undock_terminal_window", { label });
}

export async function setTerminalWorkflowEnabled(
  enabled: boolean,
): Promise<TerminalWindowState[]> {
  return invoke<TerminalWindowState[]>("set_terminal_workflow_enabled", { enabled });
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

export async function loadWorkflowRoleContract(
  role: WorkflowRole,
): Promise<WorkflowRoleContract> {
  return invoke<WorkflowRoleContract>("get_workflow_role_contract", { role });
}

export async function previewWorkflowContext(
  group: WorkflowGroupDefinition,
  connectionId: string,
  objective: string,
  sourceResultId?: string,
): Promise<WorkflowContextPackage> {
  return invoke<WorkflowContextPackage>("preview_workflow_context", {
    group,
    connectionId,
    objective,
    sourceResultId,
  });
}

export async function loadWorkflowRun(workflowId: string): Promise<WorkflowRun | null> {
  return invoke<WorkflowRun | null>("get_workflow_run", { workflowId });
}

export async function startWorkflowRun(
  group: WorkflowGroupDefinition,
  objective: string,
): Promise<WorkflowRun> {
  return invoke<WorkflowRun>("start_workflow_run", { group, objective });
}

export async function approveWorkflowHandoff(workflowId: string): Promise<WorkflowRun> {
  return invoke<WorkflowRun>("approve_workflow_handoff", { workflowId });
}

export async function advanceWorkflowRun(workflowId: string): Promise<WorkflowRun> {
  return invoke<WorkflowRun>("advance_workflow_run", { workflowId });
}

export async function pauseWorkflowRun(workflowId: string): Promise<WorkflowRun> {
  return invoke<WorkflowRun>("pause_workflow_run", { workflowId });
}

export async function resumeWorkflowRun(workflowId: string): Promise<WorkflowRun> {
  return invoke<WorkflowRun>("resume_workflow_run", { workflowId });
}

export async function retryWorkflowStep(workflowId: string): Promise<WorkflowRun> {
  return invoke<WorkflowRun>("retry_workflow_step", { workflowId });
}

export async function skipWorkflowStep(workflowId: string): Promise<WorkflowRun> {
  return invoke<WorkflowRun>("skip_workflow_step", { workflowId });
}

export async function cancelWorkflowRun(workflowId: string): Promise<WorkflowRun> {
  return invoke<WorkflowRun>("cancel_workflow_run", { workflowId });
}

export type DisplayBackend =
  | "native"
  | "native-gnome"
  | "xwayland-fallback"
  | "gnome-wayland-limited";

export async function loadDisplayBackend(): Promise<DisplayBackend> {
  if (!inDesktop()) return "native";
  try {
    return await invoke("display_backend");
  } catch {
    return "native";
  }
}

export async function loadOverlayPosition(): Promise<{ x: number; y: number }> {
  return invoke("get_overlay_position");
}

export async function savePreferences(preferences: Preferences): Promise<void> {
  if (!("__TAURI_INTERNALS__" in window)) return;
  await invoke("set_preferences", { preferences });
}

export async function takePendingShortcutAction(): Promise<
  "open" | "palette" | "new-session" | "whiteboard" | null
> {
  if (!inDesktop()) return null;
  return invoke("take_pending_shortcut_action");
}

export async function loadIntegrationStatuses(): Promise<IntegrationStatus[]> {
  if (!("__TAURI_INTERNALS__" in window)) {
    return [
      { kind: "codex", label: "Codex", installed: true, configured: false, directPermissions: false, detail: "Ready to connect" },
      { kind: "claude", label: "Claude Code", installed: true, configured: true, directPermissions: true, detail: "Monitoring and decisions connected" },
      { kind: "gemini", label: "Gemini", installed: true, configured: false, directPermissions: false, detail: "Ready to connect" },
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

export async function loadResumableSessions(
  kind: IntegrationStatus["kind"],
): Promise<ResumableSession[]> {
  if (!("__TAURI_INTERNALS__" in window)) return [];
  return invoke<ResumableSession[]>("list_resumable_sessions", { kind });
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
