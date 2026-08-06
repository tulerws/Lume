import assert from "node:assert/strict";
import {
  buildHandoffBody,
  buildHandoffPrompt,
  parseHandoffPrompt,
} from "../src/lib/handoff.ts";

const body = buildHandoffBody({
  text: "Implementation finished.",
  files: [{ path: "src/lib/handoff.ts", added: 12, removed: 2 }],
  includeText: true,
  includeFiles: true,
  note: "Review this implementation.",
});
assert.equal(
  body,
  "Review this implementation.\n\nImplementation finished.\n\nFiles changed:\n- src/lib/handoff.ts (+12 -2)",
);

const prompt = buildHandoffPrompt("Codex", "Lume", body);
assert.deepEqual(parseHandoffPrompt(prompt), {
  source: "Codex",
  body,
});

const filesOnly = buildHandoffBody({
  text: "",
  files: [{ path: "src/lib/handoff.ts", added: 4, removed: 1 }],
  includeText: false,
  includeFiles: true,
  note: "",
});
assert.equal(
  filesOnly,
  "Files changed:\n- src/lib/handoff.ts (+4 -1)",
);

assert.equal(parseHandoffPrompt("A regular user prompt"), null);

console.log("handoff test suite passed");
