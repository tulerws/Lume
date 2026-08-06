import type {
  AgentKind,
  AgentSession,
  PermissionAction,
  PromptAttachmentInput,
  PromptDelivery,
  QuestionAnswer,
} from "$lib/domain";
import type { SessionCapabilities } from "$lib/sessionCapabilities";

export const HUB_PROTOCOL_VERSION = 1;

export type WorkItemStatus = "pending" | "in_progress" | "completed";

export interface WorkItem {
  label: string;
  status: WorkItemStatus;
}

export interface AgentWorkSummary {
  plan?: {
    items: WorkItem[];
    explanation?: string;
    content?: string;
    updatedAt: number;
  };
  todo?: {
    items: WorkItem[];
    updatedAt: number;
  };
  goal?: {
    objective: string;
    status: "active" | "complete" | "blocked";
    startedAt: number;
    updatedAt: number;
  };
}

export type HubSession = AgentSession & {
  capabilities: SessionCapabilities;
  workSummary: AgentWorkSummary;
  activityTotal: number;
};

export interface HubSnapshot {
  protocolVersion: number;
  desktopVersion: string;
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
      delivery?: PromptDelivery;
    }
  | {
      type: "resolve_permission";
      sessionId: string;
      permissionId: string;
      action: PermissionAction;
    }
  | {
      type: "resolve_question";
      sessionId: string;
      questionId: string;
      answers: QuestionAnswer[];
    }
  | {
      type: "terminate_session";
      sessionId: string;
    }
  | {
      type: "interrupt_prompt";
      sessionId: string;
    }
  | {
      type: "download_response_file";
      sessionId: string;
      attachmentId: string;
    }
  | {
      type: "open_session_source";
      sessionId: string;
    }
  | {
      type: "refresh_rate_limits";
      agent: AgentKind;
    }
  | {
      type: "report_mobile_version";
      version: string;
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
  data?: unknown;
}

export interface HubEventEnvelope {
  protocolVersion: number;
  eventId: string;
  sequence: number;
  occurredAt: number;
  type: "sessions_changed";
}
