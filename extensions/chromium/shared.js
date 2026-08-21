(function exposeLumeWebShared(scope) {
  const providerForHost = (host) => {
    const normalized = String(host || "").toLowerCase();
    if (normalized === "claude.ai" || normalized.endsWith(".claude.ai")) return "claude";
    if (normalized === "chat.deepseek.com" || normalized.endsWith(".chat.deepseek.com")) return "deepseek";
    if (normalized === "gemini.google.com" || normalized.endsWith(".gemini.google.com")) return "gemini";
    if (
      normalized === "chatgpt.com"
      || normalized.endsWith(".chatgpt.com")
      || normalized === "chat.openai.com"
      || normalized.endsWith(".chat.openai.com")
    ) return "chatgpt";
    return null;
  };

  const eventForTab = (event, tabId) => {
    if (!event || !Number.isInteger(tabId)) return event;
    return {
      ...event,
      sessionId: `${event.sessionId}.${tabId}`,
    };
  };

  scope.LumeWebShared = Object.freeze({ providerForHost, eventForTab });
})(globalThis);
