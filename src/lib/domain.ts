export type AgentKind = "codex" | "claude" | "gemini" | "unknown";

export type SessionStatus =
  | "running"
  | "permission_required"
  | "waiting_for_input"
  | "completed"
  | "failed";

export type AccessMode =
  | "full_access"
  | "workspace_write"
  | "read_only"
  | "plan"
  | "custom";

export type PermissionAction =
  | "allow_once"
  | "allow_session"
  | "deny"
  | "open_source";

export interface PermissionProfile {
  mode: AccessMode;
  label: string;
  approvalPolicy: string;
  approvalsReviewer?: "user" | "auto_review" | string;
  canRespondFromLume: boolean;
  availableActions: PermissionAction[];
}

export interface PermissionRequest {
  id: string;
  kind: "command" | "file" | "network" | "tool";
  summary: string;
  resource: string;
  risk: "low" | "medium" | "high";
  requestedAt: string;
}

export interface AgentSession {
  id: string;
  agent: AgentKind;
  agentLabel: string;
  project: string;
  source: "cli" | "vscode" | "web" | "desktop";
  sourceApp?: "chrome" | "edge" | "brave";
  status: SessionStatus;
  statusLabel: string;
  startedAt: string;
  updatedAt: number;
  processId?: number;
  nativeSessionId?: string;
  workingDirectory?: string;
  permissionProfile: PermissionProfile;
  pendingPermission?: PermissionRequest;
  lastResponse?: string;
  results: SessionResult[];
  activities: SessionActivity[];
}

export interface SessionActivity {
  id: string;
  kind: "prompt" | "message" | "analysis" | "plan" | "command" | "file" | "test" | "tool" | "permission";
  title: string;
  detail?: string;
  status: "running" | "completed" | "failed" | "waiting";
  createdAt: number;
  files: string[];
}

export interface SessionResult {
  id: string;
  response: string;
  createdAt: number;
  files: string[];
  tests: string[];
}

export interface ResultNote {
  id: string;
  title: string;
  body: string;
  agentLabel: string;
  project: string;
  files: string[];
  tests: string[];
  createdAt: number;
}

export interface HistoryEntry {
  id: string;
  sessionId: string;
  agentLabel: string;
  project: string;
  event: "completed" | "failed" | "permission_allowed" | "permission_denied";
  summary: string;
  createdAt: number;
}

export interface Preferences {
  language: "en" | "pt-BR";
  darkMode?: boolean;
  soundEnabled: boolean;
  autostart: boolean;
  monitorId?: string;
  overlayX?: number;
  overlayY?: number;
  showOverFullscreen: boolean;
  historyRetentionDays: number;
  launchTarget: "auto" | "terminal" | "vscode";
  projectProfiles: Record<string, ProjectProfile>;
  whiteboardLayouts: WhiteboardLayout[];
  globalShortcut: string;
  openShortcut: string;
  newSessionShortcut: string;
  whiteboardShortcut: string;
}

export interface ProjectProfile {
  label: string;
  soundEnabled: boolean;
  launchTarget?: Preferences["launchTarget"];
  monitorId?: string;
  overlayX?: number;
  overlayY?: number;
  permissionMode?: AccessMode;
  approvalPolicy?: "untrusted" | "on-request" | "never";
  whiteboardLayoutId?: string;
  preferredAgents: AgentKind[];
}

export interface WhiteboardLayoutTerminal {
  agent: AgentKind;
  agentLabel: string;
  project: string;
  source: AgentSession["source"];
  x: number;
  y: number;
  width: number;
  height: number;
  groupId?: string;
  monitorId?: string;
}

export interface WhiteboardLayout {
  id: string;
  name: string;
  terminals: WhiteboardLayoutTerminal[];
}

export interface IntegrationStatus {
  kind: "codex" | "claude" | "gemini";
  label: string;
  installed: boolean;
  configured: boolean;
  directPermissions: boolean;
  detail: string;
}

export interface DiagnosticCheck {
  id: string;
  label: string;
  status: "ok" | "warning" | "error";
  detail: string;
}

export interface IntegrationDiagnostic {
  kind: IntegrationStatus["kind"];
  label: string;
  healthy: boolean;
  checks: DiagnosticCheck[];
  lastEventAt?: number;
}

export interface CompanionStatus {
  installed: boolean;
  configured: boolean;
  detail: string;
}

export interface RemoteStatus {
  available: boolean;
  enabled: boolean;
  port: number;
  pairedDevices: number;
}

/// Espelho de `RemoteDevice` em `src-tauri/src/domain.rs`. Sem credencial, e
/// esse silêncio é deliberado: o hash do token nunca chega ao webview.
export interface RemoteDevice {
  id: string;
  name: string;
  platform: string;
  createdAt: number;
  lastSeenAt?: number;
}

/// O QR já desenhado. Nem o código nem a URI vêm aqui — só os módulos.
export interface PairingInvitation {
  qrSvg: string;
  hostname: string;
  hosts: string[];
  port: number;
  expiresInSeconds: number;
}

export interface PairingProgress {
  active: boolean;
  expiresInSeconds: number;
  pairedDevices: number;
}

export interface TerminalWindowState {
  label: string;
  sessionId: string;
  x: number;
  y: number;
  width: number;
  height: number;
  docked: boolean;
  groupId?: string;
  connectedSides: Array<"left" | "right" | "top" | "bottom">;
  monitorId: string;
  layered: boolean;
  scale: number;
}

export interface RestoredTerminalPlacement {
  sessionId: string;
  x: number;
  y: number;
  width: number;
  height: number;
  groupId?: string;
  monitorId?: string;
}

export interface ExternalAgentPlugin {
  schemaVersion: number;
  id: string;
  name: string;
  executable: string;
  processNames: string[];
  commandTokens: string[];
}

export type DockSide = "left" | "right" | "top" | "bottom";

export interface DockPreviewEvent {
  movingLabel: string;
  preview: {
    targetLabel: string;
    side: DockSide;
    x: number;
    y: number;
    width: number;
    height: number;
  } | null;
}
