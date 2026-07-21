const endpoint = "http://127.0.0.1:43120";

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type === "lume:event") {
    fetch(`${endpoint}/events`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(message.event),
    })
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
        });
      })
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
