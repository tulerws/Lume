import type { AgentSession, TerminalWindowState } from "$lib/domain";

function normalizedDirectory(value?: string) {
  const normalized = value?.trim().replaceAll("\\", "/").replace(/\/+$/, "");
  if (!normalized) return undefined;
  return /^[a-z]:\//i.test(normalized) ? normalized.toLowerCase() : normalized;
}

export function terminalMatchesSession(
  terminal: TerminalWindowState,
  session: AgentSession,
) {
  if (terminal.sessionId === session.id) return true;
  if (terminal.sessionNativeId) {
    return (
      terminal.sessionNativeId === session.nativeSessionId &&
      terminal.sessionAgent === session.agent &&
      terminal.sessionSource === session.source
    );
  }
  return Boolean(
    terminal.sessionProcessId &&
    session.processId &&
    terminal.sessionProcessId === session.processId &&
    terminal.sessionAgent === session.agent &&
    terminal.sessionSource === session.source &&
    !session.nativeSessionId
  );
}

export function resolveTerminalSession<T extends AgentSession>(
  terminal: TerminalWindowState,
  sessions: T[],
) {
  const exact = sessions.find((session) => terminal.sessionId === session.id);
  if (exact) return exact;

  if (terminal.sessionNativeId) {
    return sessions.find((session) =>
      terminal.sessionNativeId === session.nativeSessionId &&
      terminal.sessionAgent === session.agent &&
      terminal.sessionSource === session.source
    );
  }

  const expectedDirectory = normalizedDirectory(terminal.sessionWorkingDirectory);
  if (terminal.sessionProcessId) {
    const processMatches = sessions.filter((session) =>
      terminal.sessionProcessId === session.processId &&
      terminal.sessionAgent === session.agent &&
      terminal.sessionSource === session.source
    );
    const contextualProcessMatches = expectedDirectory
      ? processMatches.filter((session) =>
          normalizedDirectory(session.workingDirectory) === expectedDirectory
        )
      : [];
    if (contextualProcessMatches.length === 1) return contextualProcessMatches[0];
    if (processMatches.length === 1) return processMatches[0];
    return undefined;
  }

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
