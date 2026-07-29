import type { AgentSession, PromptDelivery } from "$lib/domain";

export type PromptUnavailableReason =
  | "unsupported_agent"
  | "session_not_connected"
  | "working_directory_missing";

export interface SessionCapabilities {
  canPrompt: boolean;
  promptUnavailableReason?: PromptUnavailableReason;
  canApprove: boolean;
  canAnswerQuestion: boolean;
  canTerminate: boolean;
  canOpenSource: boolean;
  canReadResults: boolean;
  canAttachImages: boolean;
  canInterrupt: boolean;
  promptDeliveries: PromptDelivery[];
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
    canAnswerQuestion: Boolean(session.pendingQuestion),
    canTerminate: session.source === "cli" && Boolean(session.processId),
    canOpenSource: session.source === "web" || session.source === "vscode",
    canReadResults: session.results.length > 0 || Boolean(session.lastResponse),
    canAttachImages: session.source !== "web" && session.agent !== "unknown",
    canInterrupt:
      ["running", "permission_required"].includes(session.status)
      && session.agent === "codex"
      && Boolean(session.nativeSessionId),
    promptDeliveries:
      session.agent === "codex"
        ? ["new_turn", "steer", "queue"]
        : ["new_turn"],
  };
}
