export type AgentKind =
  | "codex"
  | "chatgpt"
  | "claude"
  | "claude_code"
  | "antigravity"
  | "deepseek"
  | "gemini"
  | "unknown";

export type SessionStatus =
  | "running"
  | "permission_required"
  | "waiting_for_input"
  | "completed"
  | "failed";

export type SessionControlOrigin = "external" | "lume";

export type PromptDelivery = "new_turn" | "steer" | "queue";

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

export interface QuestionOption {
  label: string;
  description: string;
}

export interface InteractiveQuestion {
  id: string;
  header: string;
  question: string;
  isOther: boolean;
  isSecret: boolean;
  options: QuestionOption[];
}

export interface PendingQuestion {
  id: string;
  questions: InteractiveQuestion[];
  requestedAt: string;
}

export interface QuestionAnswer {
  questionId: string;
  answers: string[];
}

export interface AgentSession {
  id: string;
  agent: AgentKind;
  agentLabel: string;
  sessionName?: string;
  project: string;
  source: "cli" | "vscode" | "web" | "desktop";
  sourceApp?: "chrome" | "edge" | "brave";
  controlOrigin: SessionControlOrigin;
  status: SessionStatus;
  statusLabel: string;
  startedAt: string;
  updatedAt: number;
  processId?: number;
  nativeSessionId?: string;
  workingDirectory?: string;
  permissionProfile: PermissionProfile;
  pendingPermission?: PermissionRequest;
  pendingQuestion?: PendingQuestion;
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
  path?: string;
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
  kind: "prompt" | "queued_prompt" | "message" | "activity" | "analysis" | "plan" | "plan_document" | "command" | "file" | "test" | "tool" | "permission" | "question";
  title: string;
  detail?: string;
  status: "running" | "completed" | "failed" | "waiting" | "interrupted";
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

export interface SessionNote {
  id: string;
  nativeSessionId: string;
  title: string;
  body: string;
  kind: "plan" | "note";
  pinned: boolean;
  createdAt: number;
  updatedAt: number;
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
  soundVolume: number;
  popupNotificationsEnabled: boolean;
  autostart: boolean;
  mobileGatewayEnabled: boolean;
  monitorId?: string;
  overlayX?: number;
  overlayY?: number;
  showOverFullscreen: boolean;
  historyRetentionDays: number;
  launchTarget: "auto" | "terminal" | "vscode";
  projectProfiles: Record<string, ProjectProfile>;
  sessionAliases: Record<string, string>;
  whiteboardLayouts: WhiteboardLayout[];
  workflowEnabled: boolean;
  workflowGroups: WorkflowGroupDefinition[];
  workflowSettings: WorkflowSettings;
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

export type WorkflowRole =
  | "planner"
  | "implementer"
  | "reviewer"
  | "tester"
  | "researcher"
  | "custom";

export interface WorkflowRoleContract {
  instruction: string;
  expectedInput: string;
  producedOutput: string;
  completionCondition: string;
}

export interface WorkflowStepDefinition {
  id: string;
  sessionNativeId: string;
  role: WorkflowRole;
  customRoleLabel: string;
  instruction: string;
  expectedInput: string;
  producedOutput: string;
  completionCondition: string;
  attempt: number;
}

export type WorkflowAdvanceMode = "manual" | "automatic";

export type WorkflowContextPolicy =
  | "minimal"
  | "standard"
  | "detailed"
  | "custom";

export interface WorkflowContextSelection {
  response: boolean;
  files: boolean;
  checks: boolean;
  plan: boolean;
  activity: boolean;
  diffs: boolean;
}

export interface WorkflowConnectionDefinition {
  id: string;
  fromStepId: string;
  toStepId: string;
  includeResponse: boolean;
  includeFiles: boolean;
  includeTests: boolean;
  contextPolicy: WorkflowContextPolicy;
  contextSelection: WorkflowContextSelection;
  additionalInstruction: string;
  requiresApproval: boolean;
  advanceMode: WorkflowAdvanceMode;
}

export interface WorkflowGroupDefinition {
  id: string;
  terminalGroupId: string;
  steps: WorkflowStepDefinition[];
  connections: WorkflowConnectionDefinition[];
}

export interface WorkflowSettings {
  maxTransitions: number;
  maxAttemptsPerStep: number;
  stepTimeoutMinutes: number;
  maxContextTokens: number;
  requireApprovalForSensitiveContext: boolean;
  pauseOnRateLimit: boolean;
  minimumRateLimitRemainingPercent: number;
}

export type WorkflowRunStatus =
  | "draft"
  | "ready"
  | "running"
  | "waiting_for_approval"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export type WorkflowStepRunStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "skipped";

export interface WorkflowStepRun {
  stepId: string;
  status: WorkflowStepRunStatus;
  attempt: number;
  startedAt?: number;
  completedAt?: number;
  resultId?: string;
  error?: string;
}

export interface WorkflowRun {
  id: string;
  workflowId: string;
  objective: string;
  status: WorkflowRunStatus;
  currentStepId?: string;
  pendingConnectionId?: string;
  handoffApproved: boolean;
  recovering: boolean;
  transitionCount: number;
  steps: WorkflowStepRun[];
  error?: string;
  createdAt: number;
  updatedAt: number;
}

export type WorkflowHistoryEventKind =
  | "started"
  | "step_started"
  | "step_completed"
  | "handoff_ready"
  | "handoff_approved"
  | "paused"
  | "resumed"
  | "step_failed"
  | "step_retried"
  | "step_skipped"
  | "session_replaced"
  | "guardrail_paused"
  | "completed"
  | "cancelled";

export interface WorkflowHistoryEvent {
  id: string;
  kind: WorkflowHistoryEventKind;
  stepId?: string;
  connectionId?: string;
  summary: string;
  createdAt: number;
}

export interface WorkflowStepHistory {
  stepId: string;
  sessionNativeId: string;
  roleLabel: string;
  agentLabel: string;
  sessionName: string;
  project: string;
  response?: string;
  files: string[];
  tests: string[];
  capturedAt?: number;
}

export interface WorkflowHistoryRecord {
  run: WorkflowRun;
  group: WorkflowGroupDefinition;
  events: WorkflowHistoryEvent[];
  steps: WorkflowStepHistory[];
}

export interface WorkflowContextFile {
  path: string;
  external: boolean;
  added: number;
  removed: number;
  diff?: string;
}

export interface WorkflowContextCheck {
  summary: string;
}

export interface WorkflowContextActivity {
  kind: string;
  title: string;
  detail?: string;
  status: string;
  createdAt: number;
}

export interface WorkflowContextRedaction {
  kind: string;
  summary: string;
  count: number;
}

export interface WorkflowContextPackage {
  version: number;
  workflowId: string;
  sourceStepId: string;
  targetStepId: string;
  sourceResultId: string;
  policy: WorkflowContextPolicy;
  objective: string;
  sourceRole: string;
  targetRole: string;
  result?: string;
  files: WorkflowContextFile[];
  checks: WorkflowContextCheck[];
  plan?: string;
  relevantActivity: WorkflowContextActivity[];
  nextInstruction: string;
  redactions: WorkflowContextRedaction[];
  estimatedTokens: number;
  markdown: string;
}

export interface IntegrationStatus {
  kind: "codex" | "claude" | "antigravity" | "deepseek" | "gemini";
  label: string;
  installed: boolean;
  configured: boolean;
  canConfigure: boolean;
  canLaunch: boolean;
  directPermissions: boolean;
  detail: string;
}

export interface ResumableSession {
  id: string;
  agent: IntegrationStatus["kind"];
  name: string;
  project: string;
  workingDirectory: string;
  source: string;
  updatedAt: number;
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
  bridgeSides: Array<"left" | "right" | "top" | "bottom">;
  workflowBridgeOpen: boolean;
  workflowEnabled: boolean;
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
    proximity: number;
  } | null;
}
