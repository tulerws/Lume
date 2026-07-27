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
  rateLimits?: AgentRateLimit[];
}

export interface PromptAttachment {
  id: string;
  name: string;
  mimeType: string;
  previewDataUrl: string;
}

export interface PromptAttachmentInput {
  name: string;
  mimeType: string;
  path?: string;
  dataBase64?: string;
  previewDataUrl?: string;
}

export interface AgentRateLimit {
  id: string;
  label: string;
  usedPercent: number;
  resetsAt?: number;
  windowMinutes?: number;
}

export interface SessionActivity {
  id: string;
  kind: "prompt" | "message" | "analysis" | "plan" | "command" | "file" | "test" | "tool" | "permission";
  title: string;
  detail?: string;
  status: "running" | "completed" | "failed" | "waiting";
  createdAt: number;
  files: string[];
  attachments?: PromptAttachment[];
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

export type MobileScope = "monitor" | "prompt" | "approve" | "terminate";

export interface MobileGatewayStatus {
  running: boolean;
  address: string;
  networkReachable: boolean;
  transport: string;
}

export interface MobilePairingOffer {
  protocolVersion: number;
  code: string;
  expiresAt: number;
  payload: string;
}

export interface PairedDevice {
  id: string;
  name: string;
  createdAt: number;
  lastSeenAt?: number;
  scopes: MobileScope[];
}

export interface TerminalWindowState {
  label: string;
  sessionId: string;
  sessionNativeId?: string;
  sessionProcessId?: number;
  sessionAgent: AgentKind;
  sessionSource: AgentSession["source"];
  sessionProject: string;
  sessionWorkingDirectory?: string;
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
