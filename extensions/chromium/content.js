(() => {
  const host = location.hostname;
  const provider = host.includes("claude")
    ? "claude"
    : host.includes("gemini")
      ? "gemini"
      : "codex";
  let lastState = "";
  let lastPath = "";
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

  const detectState = () => {
    const buttons = buttonText();
    const permissionWords = [
      "allow once",
      "allow",
      "approve",
      "run command",
      "permitir",
      "permitir uma vez",
      "aprovar",
      "aceitar",
      "executar comando",
    ];
    const permissionDialog = [...document.querySelectorAll('[role="dialog"], [data-state="open"]')]
      .filter(visible)
      .some((dialog) => permissionWords.some((word) => dialog.textContent?.toLowerCase().includes(word)));
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
    if (lastState === "running" || lastState === "completed") return "completed";
    if (lastState === "failed") return "failed";
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
      .replace(/\s*[|·-]\s*(ChatGPT|Claude|Gemini).*$/i, "")
      .trim()
      .slice(0, 100) || "Sessão web";

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
    const state = detectState();
    const path = location.pathname;
    if (!force && state === lastState && path === lastPath) return;
    lastState = state;
    lastPath = path;
    void chrome.runtime.sendMessage({
      type: "lume:event",
      event: {
        provider,
        sessionId: hash(`${provider}:${path}`),
        title: cleanTitle(),
        origin: location.origin,
        state,
      },
    }).then((response) => {
      if (response?.prompt) void submitPrompt(response.prompt);
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
  setInterval(() => report(true), 4000);
  schedule();
})();
