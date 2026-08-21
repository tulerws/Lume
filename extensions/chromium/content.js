(() => {
  const host = location.hostname;
  const provider = globalThis.LumeWebShared?.providerForHost(host);
  if (!provider) return;
  let lastState = "";
  let lastPath = "";
  let lastResponseSignature = "";
  let submittingPromptId = "";
  let timer;

  const visible = (element) => {
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden";
  };

  const buttonText = () =>
    [...document.querySelectorAll("button")]
      .filter(visible)
      .slice(-80)
      .map((button) => `${button.textContent ?? ""} ${button.getAttribute("aria-label") ?? ""}`.trim().toLowerCase());

  const detectState = (previousState = lastState) => {
    const buttons = buttonText();
    const permissionDialog = [...document.querySelectorAll('[role="dialog"], [data-state="open"]')]
      .filter(visible)
      .some((dialog) => {
        const text = dialog.textContent?.toLowerCase() ?? "";
        const actions = [...dialog.querySelectorAll("button")]
          .filter(visible)
          .map((button) => `${button.textContent ?? ""} ${button.getAttribute("aria-label") ?? ""}`.trim().toLowerCase());
        const explicitPermission =
          /permission required|approval required|allow once|run command|permissão necessária|aprovação necessária|permitir uma vez|executar comando/.test(text);
        const hasAllow = actions.some((action) =>
          /^(allow|approve|permitir|aprovar|aceitar)( once| this time)?$/.test(action),
        );
        const hasDeny = actions.some((action) =>
          /^(deny|decline|reject|recusar|negar)$/.test(action),
        );
        return explicitPermission || (hasAllow && hasDeny);
      });
    if (permissionDialog) return "permission_required";

    const runningSelectors = [
      'button[data-testid*="stop"]',
      'button[aria-label*="Stop"]',
      'button[aria-label*="Parar"]',
      '[data-testid="stop-button"]',
    ];
    if (
      runningSelectors.some((selector) => [...document.querySelectorAll(selector)].some(visible)) ||
      buttons.some((text) => text === "stop" || text === "parar")
    ) {
      return "running";
    }

    const alerts = [...document.querySelectorAll('[role="alert"]')]
      .filter(visible)
      .map((alert) => alert.textContent?.toLowerCase() ?? "")
      .join(" ");
    if (/failed|something went wrong|erro|falhou/.test(alerts)) return "failed";
    if (previousState === "running" || previousState === "completed") return "completed";
    if (previousState === "failed") return "failed";
    return "waiting_for_input";
  };

  const hash = (value) => {
    let result = 2166136261;
    for (const char of value) {
      result ^= char.charCodeAt(0);
      result = Math.imul(result, 16777619);
    }
    return (result >>> 0).toString(36);
  };

  const cleanTitle = () =>
    document.title
      .replace(/\s*[|·-]\s*(ChatGPT|Claude|DeepSeek|Gemini).*$/i, "")
      .trim()
      .slice(0, 100) || "Sessão web";

  const finalResponse = () => {
    const selectors = provider === "chatgpt"
      ? ['[data-message-author-role="assistant"]']
      : provider === "claude"
        ? ['[data-testid="assistant-message"]', '.font-claude-response']
        : provider === "deepseek"
          ? ['[data-role="assistant"] .ds-markdown', '.ds-markdown', '[class*="ds-markdown"]']
          : ['model-response .markdown', 'model-response', '.model-response-text'];
    const candidates = selectors.flatMap((selector) => [...document.querySelectorAll(selector)]);
    const response = candidates
      .filter((element) => element.textContent?.trim())
      .at(-1)
      ?.textContent?.trim();
    return response?.slice(0, 32768);
  };

  const submitPrompt = async (text) => {
    const candidates = [
      ...document.querySelectorAll(
        'textarea, [contenteditable="true"][role="textbox"], [contenteditable="true"].ProseMirror, [contenteditable="true"][data-lexical-editor="true"]',
      ),
    ].filter((element) => visible(element) && !element.disabled);
    const composer = candidates.at(-1);
    if (!composer) return false;

    composer.focus();
    if (composer instanceof HTMLTextAreaElement || composer instanceof HTMLInputElement) {
      const prototype = composer instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
      setter?.call(composer, text);
    } else {
      composer.textContent = text;
    }
    composer.dispatchEvent(
      new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }),
    );
    composer.dispatchEvent(new Event("change", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 90));

    const scope = composer.closest("form") ?? document;
    const sendButton = [...scope.querySelectorAll("button")]
      .filter((button) => visible(button) && !button.disabled)
      .find((button) => {
        const label = `${button.textContent ?? ""} ${button.getAttribute("aria-label") ?? ""} ${button.dataset.testid ?? ""}`.toLowerCase();
        return /(^|\s)(send|enviar|submit|enviar mensagem|send message)(\s|$)/.test(label);
      });
    if (sendButton) {
      sendButton.click();
      return true;
    }
    if (scope instanceof HTMLFormElement) {
      scope.requestSubmit();
      return true;
    }
    composer.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true }),
    );
    return true;
  };

  const report = (force = false) => {
    const path = location.pathname;
    const state = detectState(path === lastPath ? lastState : "");
    const lastResponse = state === "completed" ? finalResponse() : undefined;
    const responseSignature = lastResponse ? hash(lastResponse) : "";
    if (!force && state === lastState && path === lastPath && responseSignature === lastResponseSignature) return;
    lastState = state;
    lastPath = path;
    lastResponseSignature = responseSignature;
    void chrome.runtime.sendMessage({
      type: "lume:event",
      event: {
        provider,
        protocolVersion: 2,
        sessionId: hash(`${provider}:${path}`),
        title: cleanTitle(),
        origin: location.origin,
        state,
        lastResponse,
      },
    }).then(async (response) => {
      if (!response?.prompt || response.promptId === submittingPromptId) return;
      submittingPromptId = response.promptId || "";
      const submitted = await submitPrompt(response.prompt);
      if (response.promptId) {
        await chrome.runtime.sendMessage({
          type: "lume:prompt-ack",
          promptId: response.promptId,
          submitted,
        }).catch(() => {});
      }
      submittingPromptId = "";
    }).catch(() => {});
  };

  const schedule = () => {
    clearTimeout(timer);
    timer = setTimeout(report, 450);
  };
  new MutationObserver(schedule).observe(document.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["aria-label", "data-state", "disabled"],
  });
  window.addEventListener("popstate", schedule);
  window.addEventListener("focus", () => report(true));
  window.addEventListener("pageshow", () => report(true));
  document.addEventListener("visibilitychange", () => report(true));
  setInterval(() => report(true), 15_000);
  schedule();
})();
