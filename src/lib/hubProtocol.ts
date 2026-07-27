import type {
  AgentKind,
  AgentSession,
  PermissionAction,
  PromptAttachmentInput,
} from "$lib/domain";
import type { SessionCapabilities } from "$lib/sessionCapabilities";

export const HUB_PROTOCOL_VERSION = 1;

export type HubSession = AgentSession & {
  capabilities: SessionCapabilities;
};

export interface HubSnapshot {
  protocolVersion: number;
  generatedAt: number;
  features: string[];
  sessions: HubSession[];
}

export type HubCommand =
  | {
      type: "submit_prompt";
      sessionId: string;
      prompt: string;
      attachments?: PromptAttachmentInput[];
    }
  | {
      type: "resolve_permission";
      sessionId: string;
      permissionId: string;
      action: PermissionAction;
    }
  | {
      type: "terminate_session";
      sessionId: string;
    }
  | {
      type: "open_session_source";
      sessionId: string;
    }
  | {
      type: "refresh_rate_limits";
      agent: AgentKind;
    };

export type HubCommandRequest = HubCommand & {
  requestId: string;
};

export interface ProtocolError {
  code: string;
  message: string;
}

export interface HubCommandResponse {
  protocolVersion: number;
  requestId: string;
  ok: boolean;
  error?: ProtocolError;
}

export interface HubEventEnvelope {
  protocolVersion: number;
  eventId: string;
  sequence: number;
  occurredAt: number;
  type: "sessions_changed";
}
