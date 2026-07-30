import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";

const root = new URL("../", import.meta.url);
const sharedSource = await readFile(new URL("extensions/chromium/shared.js", root), "utf8");
const manifest = JSON.parse(
  await readFile(new URL("extensions/chromium/manifest.json", root), "utf8"),
);
const context = {};
vm.runInNewContext(sharedSource, context);

const { providerForHost, eventForTab } = context.LumeWebShared;
assert.equal(providerForHost("chatgpt.com"), "chatgpt");
assert.equal(providerForHost("chat.openai.com"), "chatgpt");
assert.equal(providerForHost("claude.ai"), "claude");
assert.equal(providerForHost("gemini.google.com"), "gemini");
assert.equal(providerForHost("example.com"), null);

const sourceEvent = { provider: "chatgpt", sessionId: "thread" };
assert.equal(eventForTab(sourceEvent, 41).sessionId, "thread.41");
assert.equal(eventForTab(sourceEvent, 42).sessionId, "thread.42");
assert.equal(sourceEvent.sessionId, "thread");

const contentScripts = manifest.content_scripts?.[0]?.js ?? [];
assert.deepEqual(contentScripts, ["shared.js", "content.js"]);
assert.ok(manifest.content_scripts[0].matches.includes("https://chatgpt.com/*"));
assert.ok(manifest.content_scripts[0].matches.includes("https://claude.ai/*"));
assert.ok(manifest.content_scripts[0].matches.includes("https://gemini.google.com/*"));

console.log("web companion regression tests passed");
