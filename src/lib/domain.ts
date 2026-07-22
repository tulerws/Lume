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
}

export interface SessionResult {
  id: string;
  response: string;
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
}

export interface ProjectProfile {
  label: string;
  soundEnabled: boolean;
  launchTarget?: Preferences["launchTarget"];
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

export interface TerminalWindowState {
  label: string;
  sessionId: string;
  x: number;
  y: number;
  width: number;
  height: number;
  docked: boolean;
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
