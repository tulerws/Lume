import { invoke } from "@tauri-apps/api/core";
import type { AgentSession, PermissionAction } from "$lib/domain";
import { demoSessions } from "$lib/demo";

export async function loadSessions(): Promise<AgentSession[]> {
  try {
    return await invoke<AgentSession[]>("list_sessions");
  } catch {
    return structuredClone(demoSessions);
  }
}

export async function decidePermission(
  sessionId: string,
  permissionId: string,
  action: PermissionAction,
): Promise<void> {
  try {
    await invoke("resolve_permission", {
      sessionId,
      permissionId,
      action,
    });
  } catch {
    // Browser preview and adapters not yet connected use local demo state.
  }
}
