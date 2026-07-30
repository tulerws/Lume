import assert from "node:assert/strict";
import {
  resolveTerminalSession,
  terminalMatchesSession,
} from "../src/lib/sessionIdentity.ts";

function terminal(overrides = {}) {
  return {
    label: "terminal-test",
    sessionId: "stale-session-id",
    sessionNativeId: undefined,
    sessionProcessId: undefined,
    sessionAgent: "codex",
    sessionSource: "cli",
    sessionProject: "Lume",
    sessionWorkingDirectory: "/home/user/Documents/Projetos/Ideias/Lume",
    ...overrides,
  };
}

function session(id, overrides = {}) {
  return {
    id,
    agent: "codex",
    source: "cli",
    project: "Lume",
    processId: undefined,
    nativeSessionId: undefined,
    workingDirectory: "/home/user/Documents/Projetos/Ideias/Lume",
    ...overrides,
  };
}

const sharedProcessTerminal = terminal({
  sessionNativeId: "thread-lume",
  sessionProcessId: 43130,
});
const vibePay = session("codex:thread-vibepay", {
  nativeSessionId: "thread-vibepay",
  processId: 43130,
  project: "vibepay",
  workingDirectory: "/home/user/Documents/Projetos/ideias/vibepay",
});
const lume = session("codex:thread-lume", {
  nativeSessionId: "thread-lume",
  processId: 43130,
});

assert.equal(
  resolveTerminalSession(sharedProcessTerminal, [vibePay, lume])?.id,
  lume.id,
  "a shared app-server PID must not replace the terminal's native thread",
);
assert.equal(
  terminalMatchesSession(sharedProcessTerminal, vibePay),
  false,
  "a native terminal must not match another thread through the shared PID",
);
assert.equal(
  resolveTerminalSession(sharedProcessTerminal, [vibePay]),
  undefined,
  "a missing native thread must not fall back to another thread on the same PID",
);

const provisionalTerminal = terminal({ sessionProcessId: 4242 });
const promoted = session("codex:promoted", {
  nativeSessionId: "thread-promoted",
  processId: 4242,
});
assert.equal(
  resolveTerminalSession(provisionalTerminal, [promoted])?.id,
  promoted.id,
  "a provisional process terminal may follow one unambiguous promoted session",
);
assert.equal(
  resolveTerminalSession(
    provisionalTerminal,
    [session("codex:promoted-source", {
      nativeSessionId: "thread-promoted-source",
      processId: 4242,
      source: "vscode",
    })],
  )?.id,
  "codex:promoted-source",
  "a terminal must survive a temporary source correction while its process is promoted",
);

const contextualTerminal = terminal({ sessionProcessId: 43130 });
assert.equal(
  resolveTerminalSession(contextualTerminal, [vibePay, lume])?.id,
  lume.id,
  "an ambiguous shared PID must be disambiguated by working directory",
);

const caseSensitiveTerminal = terminal({
  sessionProject: "shared",
  sessionWorkingDirectory: "/home/user/Documents/Projetos/Ideias/shared",
});
const lowerCaseParent = session("lower-parent", {
  project: "shared",
  workingDirectory: "/home/user/Documents/Projetos/ideias/shared",
});
const upperCaseParent = session("upper-parent", {
  project: "shared",
  workingDirectory: "/home/user/Documents/Projetos/Ideias/shared",
});
assert.equal(
  resolveTerminalSession(caseSensitiveTerminal, [lowerCaseParent, upperCaseParent])?.id,
  upperCaseParent.id,
  "Linux directory matching must remain case-sensitive",
);

console.log("session identity regression tests passed");
