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
  status: SessionStatus;
  statusLabel: string;
  startedAt: string;
  permissionProfile: PermissionProfile;
  pendingPermission?: PermissionRequest;
}

export interface HistoryEntry {
  id: string;
  sessionId: string;
  agentLabel: string;
  project: string;
  event: "completed" | "failed" | "permission_allowed" | "permission_denied";
  summary: string;
  createdAt: string;
}
