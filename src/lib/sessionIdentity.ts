import type { AgentSession, TerminalWindowState } from "$lib/domain";

function normalizedDirectory(value?: string) {
  return value?.trim().replaceAll("\\", "/").replace(/\/+$/, "").toLowerCase();
}

export function terminalMatchesSession(
  terminal: TerminalWindowState,
  session: AgentSession,
) {
  if (terminal.sessionId === session.id) return true;
  if (
    terminal.sessionNativeId &&
    session.nativeSessionId &&
    terminal.sessionNativeId === session.nativeSessionId
  ) {
    return true;
  }
  if (
    terminal.sessionProcessId &&
    session.processId &&
    terminal.sessionProcessId === session.processId &&
    terminal.sessionAgent === session.agent
  ) {
    return true;
  }
  return false;
}

export function resolveTerminalSession<T extends AgentSession>(
  terminal: TerminalWindowState,
  sessions: T[],
) {
  const direct = sessions.find((session) => terminalMatchesSession(terminal, session));
  if (direct) return direct;

  const expectedDirectory = normalizedDirectory(terminal.sessionWorkingDirectory);
  const contextual = sessions.filter((session) =>
    session.agent === terminal.sessionAgent &&
    session.source === terminal.sessionSource &&
    session.project === terminal.sessionProject &&
    (
      !expectedDirectory ||
      normalizedDirectory(session.workingDirectory) === expectedDirectory
    ),
  );
  return contextual.length === 1 ? contextual[0] : undefined;
}
