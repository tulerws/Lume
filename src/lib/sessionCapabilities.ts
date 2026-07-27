import type { AgentSession } from "$lib/domain";

export type PromptUnavailableReason =
  | "unsupported_agent"
  | "session_not_connected"
  | "working_directory_missing";

export interface SessionCapabilities {
  canPrompt: boolean;
  promptUnavailableReason?: PromptUnavailableReason;
  canApprove: boolean;
  canTerminate: boolean;
  canOpenSource: boolean;
  canReadResults: boolean;
  canAttachImages: boolean;
}

export function sessionCapabilities(session: AgentSession): SessionCapabilities {
  let promptUnavailableReason: PromptUnavailableReason | undefined;
  if (session.source !== "web") {
    if (session.agent === "unknown") {
      promptUnavailableReason = "unsupported_agent";
    } else if (!session.nativeSessionId) {
      promptUnavailableReason = "session_not_connected";
    } else if (session.agent !== "codex" && !session.workingDirectory) {
      promptUnavailableReason = "working_directory_missing";
    }
  }

  return {
    canPrompt: !promptUnavailableReason,
    promptUnavailableReason,
    canApprove: Boolean(
      session.pendingPermission && session.permissionProfile.canRespondFromLume,
    ),
    canTerminate: session.source === "cli" && Boolean(session.processId),
    canOpenSource: session.source === "web" || session.source === "vscode",
    canReadResults: session.results.length > 0 || Boolean(session.lastResponse),
    canAttachImages: session.source !== "web" && session.agent !== "unknown",
  };
}
