const endpoint = "http://127.0.0.1:43120";
importScripts("shared.js");
const browserName = (async () => {
  if (/Edg\//.test(navigator.userAgent)) return "edge";
  if (navigator.brave?.isBrave && (await navigator.brave.isBrave())) return "brave";
  return "chrome";
})();
const tabSessions = new Map();

const forwardEvent = (event) =>
  browserName.then((browser) => fetch(`${endpoint}/events`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...event, browser }),
  }));

const acknowledgePrompt = (event, promptId, submitted) =>
  browserName.then((browser) => fetch(`${endpoint}/prompt-ack`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      provider: event.provider,
      sessionId: event.sessionId,
      promptId,
      submitted,
      browser,
    }),
  }));

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type === "lume:event") {
    const event = globalThis.LumeWebShared.eventForTab(message.event, sender.tab?.id);
    if (Number.isInteger(sender.tab?.id)) {
      const previous = tabSessions.get(sender.tab.id);
      if (previous && previous.sessionId !== event.sessionId) {
        void forwardEvent({
          ...previous,
          state: "closed",
          lastResponse: undefined,
        }).catch(() => {});
      }
      tabSessions.set(sender.tab.id, event);
    }
    forwardEvent(event)
      .then(async (response) => {
        const result = await response.json().catch(() => ({ ok: response.ok }));
        if (result.focus && sender.tab?.id) {
          await chrome.tabs.update(sender.tab.id, { active: true });
          if (Number.isInteger(sender.tab.windowId)) {
            await chrome.windows.update(sender.tab.windowId, { focused: true });
          }
        }
        sendResponse({
          ok: response.ok,
          focus: Boolean(result.focus),
          prompt: typeof result.prompt === "string" ? result.prompt : null,
          promptId: typeof result.promptId === "string" ? result.promptId : null,
        });
      })
      .catch(() => sendResponse({ ok: false }));
    return true;
  }
  if (message?.type === "lume:prompt-ack") {
    const event = Number.isInteger(sender.tab?.id) ? tabSessions.get(sender.tab.id) : null;
    if (!event || typeof message.promptId !== "string") {
      sendResponse({ ok: false });
      return false;
    }
    acknowledgePrompt(event, message.promptId, Boolean(message.submitted))
      .then((response) => sendResponse({ ok: response.ok }))
      .catch(() => sendResponse({ ok: false }));
    return true;
  }
  if (message?.type === "lume:health") {
    fetch(`${endpoint}/health`)
      .then((response) => sendResponse({ ok: response.ok }))
      .catch(() => sendResponse({ ok: false }));
    return true;
  }
  return false;
});

chrome.tabs.onRemoved.addListener((tabId) => {
  const event = tabSessions.get(tabId);
  if (!event) return;
  tabSessions.delete(tabId);
  void forwardEvent({
    ...event,
    state: "closed",
    lastResponse: undefined,
  }).catch(() => {});
});
