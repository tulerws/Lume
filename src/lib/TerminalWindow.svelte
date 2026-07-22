<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { AgentSession, DockPreviewEvent, PermissionAction, Preferences, TerminalWindowState } from "$lib/domain";
  import BrandIcon from "$lib/BrandIcon.svelte";
  import LumeLogo from "$lib/LumeLogo.svelte";
  import { displayText, localize, type Language } from "$lib/i18n";
  import {
    closeTerminalWindow,
    decidePermission,
    loadPreferences,
    loadSessions,
    loadTerminalWindowState,
    openSessionSource,
    submitPrompt,
    syncTerminalWindowPosition,
    terminateSession,
    undockTerminalWindow,
  } from "$lib/lume";

  const currentWindow = getCurrentWindow();
  const label = currentWindow.label;
  type ResizeDirection = "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";
  let windowState = $state<TerminalWindowState | null>(null);
  let session = $state<AgentSession | null>(null);
  let prompt = $state("");
  let message = $state<string | null>(null);
  let sending = $state(false);
  let dragging = $state(false);
  let dragMoved = false;
  let dragFinalizeTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingMove: { x: number; y: number } | null = null;
  let lastMove: { x: number; y: number } | null = null;
  let moveSyncRunning = false;
  let finalizeRequested = false;
  let settling = $state(false);
  let dockMovingLabel = $state<string | null>(null);
  let dockPreview = $state<NonNullable<DockPreviewEvent["preview"]> | null>(null);
  let terminateConfirm = $state(false);
  let terminating = $state(false);
  let language = $state<Language>("en");
  let darkMode = $state<boolean | undefined>(undefined);
  let systemDark = $state(false);
  const effectiveDark = $derived(darkMode ?? systemDark);
  $effect(() => {
    document.documentElement.dataset.theme = effectiveDark ? "dark" : "light";
  });

  function tr(english: string, portuguese: string) {
    return localize(language, english, portuguese);
  }

  const canSubmit = $derived(
    Boolean(
      session &&
        (session.source === "web" ||
          (session.agent !== "unknown" && session.nativeSessionId)),
    ),
  );
  const readyForPrompt = $derived(
    Boolean(session && ["completed", "failed", "waiting_for_input"].includes(session.status)),
  );

  onMount(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let stopMoved: (() => void) | undefined;
    let stopPreferences: (() => void) | undefined;
    let stopDockPreview: (() => void) | undefined;
    const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    const syncSystemTheme = (event: MediaQueryListEvent | MediaQueryList) => {
      systemDark = event.matches;
    };
    syncSystemTheme(colorScheme);
    colorScheme.addEventListener("change", syncSystemTheme);
    void (async () => {
      const [nextWindowState, nextPreferences] = await Promise.all([
        loadTerminalWindowState(label),
        loadPreferences(),
      ]);
      windowState = nextWindowState;
      language = nextPreferences.language;
      darkMode = nextPreferences.darkMode;
      await refresh();
      if (disposed) return;
      stopListening = await listen("lume://sessions-changed", () => void refresh());
      stopPreferences = await listen<Preferences>("lume://preferences-changed", ({ payload }) => {
        language = payload.language;
        darkMode = payload.darkMode;
      });
      stopDockPreview = await listen<DockPreviewEvent>("lume://terminal-dock-preview", ({ payload }) => {
        const relevant = payload.preview &&
          (payload.movingLabel === label || payload.preview.targetLabel === label);
        if (relevant) {
          dockMovingLabel = payload.movingLabel;
          dockPreview = payload.preview;
        } else if (payload.movingLabel === dockMovingLabel || payload.movingLabel === label) {
          dockMovingLabel = null;
          dockPreview = null;
        }
      });
      stopMoved = await currentWindow.onMoved(({ payload }) => {
        if (settling) return;
        dragging = true;
        dragMoved = true;
        queueNativeMove(payload.x, payload.y);
      });
    })();
    return () => {
      disposed = true;
      stopListening?.();
      stopMoved?.();
      stopPreferences?.();
      stopDockPreview?.();
      colorScheme.removeEventListener("change", syncSystemTheme);
      if (dragFinalizeTimer) clearTimeout(dragFinalizeTimer);
    };
  });

  async function refresh() {
    const sessions = await loadSessions();
    session = sessions.find((item) => item.id === windowState?.sessionId) ?? null;
  }

  function sourceLabel(item: AgentSession) {
    if (item.source === "web") {
      if (item.sourceApp === "chrome") return "Chrome";
      if (item.sourceApp === "edge") return "Edge";
      if (item.sourceApp === "brave") return "Brave";
      return "Web";
    }
    return { cli: "CLI", vscode: "VS Code", desktop: "Desktop" }[item.source] ?? tr("Source", "Origem");
  }

  function sourceIcon(item: AgentSession) {
    if (item.source === "cli") return "terminal" as const;
    if (item.source === "vscode") return "vscode" as const;
    if (item.source === "web") return item.sourceApp ?? ("browsers" as const);
    return "unknown" as const;
  }

  function queueNativeMove(x: number, y: number) {
    const next = { x, y };
    pendingMove = next;
    lastMove = next;
    finalizeRequested = false;
    if (dragFinalizeTimer) clearTimeout(dragFinalizeTimer);
    dragFinalizeTimer = setTimeout(() => {
      finalizeRequested = true;
      void flushNativeMoves();
    }, 520);
    void flushNativeMoves();
  }

  async function flushNativeMoves() {
    if (moveSyncRunning) return;
    moveSyncRunning = true;
    try {
      while (pendingMove) {
        const next = pendingMove;
        pendingMove = null;
        windowState = await syncTerminalWindowPosition(label, next.x, next.y, false);
      }
      if (finalizeRequested && lastMove) {
        const finalPosition = lastMove;
        finalizeRequested = false;
        settling = true;
        dragging = false;
        dockPreview = null;
        windowState = await syncTerminalWindowPosition(
          label,
          finalPosition.x,
          finalPosition.y,
          true,
        );
        setTimeout(() => {
          settling = false;
        }, 240);
      }
    } catch (error) {
      dragging = false;
      settling = false;
      finalizeRequested = false;
      pendingMove = null;
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      moveSyncRunning = false;
      if (pendingMove || finalizeRequested) void flushNativeMoves();
    }
  }

  async function beginDrag(event: PointerEvent) {
    if (event.button !== 0 || !windowState) return;
    if ((event.target as HTMLElement).closest("button, textarea")) return;
    dragging = true;
    dragMoved = false;
    pendingMove = null;
    lastMove = null;
    finalizeRequested = false;
    settling = false;
    if (dragFinalizeTimer) clearTimeout(dragFinalizeTimer);
    try {
      await currentWindow.startDragging();
      setTimeout(() => {
        if (!dragMoved) dragging = false;
      }, 600);
    } catch (error) {
      dragging = false;
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function detach() {
    windowState = await undockTerminalWindow(label);
  }

  async function beginResize(event: PointerEvent, direction: ResizeDirection) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    dragging = false;
    finalizeRequested = false;
    pendingMove = null;
    dockPreview = null;
    if (dragFinalizeTimer) clearTimeout(dragFinalizeTimer);
    try {
      await currentWindow.startResizeDragging(direction);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function closeTerminal() {
    message = null;
    try {
      await closeTerminalWindow(label);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function terminateAgent() {
    if (!session?.processId || session.source !== "cli" || terminating) return;
    terminating = true;
    message = null;
    try {
      await terminateSession(session.id);
      terminateConfirm = false;
      await refresh();
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      terminating = false;
    }
  }

  async function sendPrompt() {
    if (!session || !prompt.trim() || sending || !canSubmit || !readyForPrompt) return;
    sending = true;
    message = null;
    try {
      await submitPrompt(session.id, prompt.trim());
      prompt = "";
      session = { ...session, status: "running", statusLabel: "Prompt sent by Lume", lastResponse: undefined };
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      sending = false;
    }
  }

  async function permission(action: PermissionAction) {
    if (!session?.pendingPermission) return;
    if (action === "open_source") {
      await openSessionSource(session.id);
      return;
    }
    await decidePermission(session.id, session.pendingPermission.id, action);
    await refresh();
  }

  async function openOrigin() {
    if (!session) return;
    message = null;
    try {
      await openSessionSource(session.id);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  function actionLabel(action: PermissionAction) {
    return {
      allow_once: tr("Allow", "Permitir"),
      allow_session: tr("For session", "Na sessão"),
      deny: tr("Deny", "Recusar"),
      open_source: tr("Open source", "Abrir origem"),
    }[action];
  }
</script>

<main class:dark={effectiveDark} class="terminal-window">
  {#if session}
    <section
      class:dragging
      class:settling
      class:dock-moving={dockMovingLabel === label && Boolean(dockPreview)}
      class:dock-target={dockPreview?.targetLabel === label}
      class:dock-left={dockPreview?.side === "left"}
      class:dock-right={dockPreview?.side === "right"}
      class:dock-top={dockPreview?.side === "top"}
      class:dock-bottom={dockPreview?.side === "bottom"}
      class="terminal-card"
    >
      {#if dockPreview?.targetLabel === label}
        <div class="dock-silhouette" aria-hidden="true"><span>{tr("Dock", "Acoplar")}</span></div>
      {/if}
      <header
        role="banner"
        onpointerdown={(event) => void beginDrag(event)}
      >
        <LumeLogo size={25} />
        <span class="agent-icon"><BrandIcon name={session.agent} size={16} /></span>
        <div class="identity">
          <strong>{session.agentLabel}</strong>
          <small>{session.project}</small>
        </div>
        <span class="source-badge">
          <BrandIcon name={sourceIcon(session)} size={10} />
          {sourceLabel(session)}
        </span>
        {#if windowState?.docked}
          <button class="dock-button" type="button" onclick={detach} aria-label={tr("Undock terminal", "Desacoplar terminal")} title={tr("Undock", "Desacoplar")}>
            <svg viewBox="0 0 20 20"><path d="M7 6 5.5 7.5a3 3 0 0 0 4.2 4.2l1.2-1.2M13 14l1.5-1.5a3 3 0 0 0-4.2-4.2L9.1 9.5" /></svg>
          </button>
        {/if}
        {#if session.source === "cli" && session.processId}
          <button class="terminate-button" type="button" onclick={() => (terminateConfirm = !terminateConfirm)} aria-label={tr("Stop agent", "Encerrar agente")} title={tr("Stop agent", "Encerrar agente")}>
            <svg viewBox="0 0 20 20"><path d="M10 3v7M5.5 5.5a6 6 0 1 0 9 0" /></svg>
          </button>
        {/if}
        <button class="close-button" type="button" onclick={closeTerminal} aria-label={tr("Close terminal", "Fechar terminal")}>
          <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8" /></svg>
        </button>
      </header>

      <div class="terminal-output">
        <p><span>$</span> {session.agentLabel.toLowerCase()} <i>{session.project}</i></p>
        <p class="status status-{session.status}"><span>&gt;</span> {displayText(language, session.statusLabel)}</p>
        {#if session.lastResponse}
          <div class="final-response">
            <strong>{tr("Final response", "Resposta final")}</strong>
            <p>{session.lastResponse}</p>
          </div>
        {/if}
        {#if session.pendingPermission}
          <div class="permission">
            <strong>{displayText(language, session.pendingPermission.summary)}</strong>
            <code>{session.pendingPermission.resource}</code>
            <div>
              {#each session.permissionProfile.availableActions as action}
                <button class:danger={action === "deny"} type="button" onclick={() => permission(action)}>
                  {actionLabel(action)}
                </button>
              {/each}
            </div>
          </div>
        {:else if !canSubmit}
          <p class="hint">{tr("This source is monitored here, but prompts are sent from the source.", "Esta origem é acompanhada aqui, mas o envio continua nela.")}</p>
        {:else if windowState?.docked}
          <p class="hint docked">{tr("Docked · drag this terminal to move the group.", "Acoplado · arraste este terminal para mover o conjunto.")}</p>
        {:else}
          <p class="hint">{tr("Move it close to another mini terminal to dock.", "Aproxime de outro mini terminal para acoplar.")}</p>
        {/if}
        {#if terminateConfirm}
          <div class="terminate-confirm">
            <span>{tr("Stop this agent and its commands?", "Encerrar este agente e os comandos dele?")}</span>
            <div>
              <button type="button" onclick={() => (terminateConfirm = false)}>{tr("Cancel", "Cancelar")}</button>
              <button class="danger" disabled={terminating} type="button" onclick={() => void terminateAgent()}>{terminating ? tr("Stopping…", "Encerrando…") : tr("Stop", "Encerrar")}</button>
            </div>
          </div>
        {/if}
      </div>

      <form
        class="terminal-composer"
        onsubmit={(event) => {
          event.preventDefault();
          void sendPrompt();
        }}
      >
        <textarea
          bind:value={prompt}
          disabled={!canSubmit || !readyForPrompt || sending}
          rows="2"
          aria-label={tr(`Prompt for ${session.agentLabel}`, `Prompt para ${session.agentLabel}`)}
          placeholder={!canSubmit ? tr("Prompt unavailable for this source", "Envio indisponível nesta origem") : readyForPrompt ? tr(`Prompt for ${session.agentLabel}…`, `Prompt para ${session.agentLabel}…`) : tr("Agent is running…", "Agente em execução…")}
        ></textarea>
        {#if canSubmit}
          <button disabled={!prompt.trim() || !readyForPrompt || sending} type="submit" aria-label={tr("Send prompt", "Enviar prompt")}>
            <svg viewBox="0 0 20 20"><path d="m4 10 12-6-4 12-2-4zM10 12l2-2" /></svg>
          </button>
        {:else}
          <button type="button" onclick={openOrigin} aria-label={tr("Open source", "Abrir origem")}>
            <svg viewBox="0 0 20 20"><path d="M7 5h8v8M14.5 5.5 6 14" /></svg>
          </button>
        {/if}
      </form>
      {#if message}<p class="message">{message}</p>{/if}
      <button class="resize-handle resize-nw" type="button" tabindex="-1" aria-label={tr("Resize from top-left corner", "Redimensionar pelo canto superior esquerdo")} onpointerdown={(event) => void beginResize(event, "NorthWest")}></button>
      <button class="resize-handle resize-ne" type="button" tabindex="-1" aria-label={tr("Resize from top-right corner", "Redimensionar pelo canto superior direito")} onpointerdown={(event) => void beginResize(event, "NorthEast")}></button>
      <button class="resize-handle resize-sw" type="button" tabindex="-1" aria-label={tr("Resize from bottom-left corner", "Redimensionar pelo canto inferior esquerdo")} onpointerdown={(event) => void beginResize(event, "SouthWest")}></button>
      <button class="resize-handle resize-se" type="button" tabindex="-1" aria-label={tr("Resize from bottom-right corner", "Redimensionar pelo canto inferior direito")} onpointerdown={(event) => void beginResize(event, "SouthEast")}></button>
    </section>
  {:else}
    <section class="terminal-card loading"><LumeLogo size={34} /><span>{tr("Connecting to session…", "Conectando à sessão…")}</span></section>
  {/if}
</main>

<style>
  .terminal-window { width: 100%; height: 100%; }
  .terminal-card { position: relative; width: 100%; height: 100%; display: flex; flex-direction: column; overflow: hidden; border: 1px solid rgba(103, 126, 116, 0.2); border-radius: 17px; color: #26342e; background: rgba(248, 251, 249, 0.97); box-shadow: 0 10px 34px rgba(20, 36, 29, 0.2); backdrop-filter: blur(24px) saturate(120%); transition: border-color 150ms ease, box-shadow 180ms ease, background-color 180ms ease, transform 180ms cubic-bezier(0.22, 1, 0.36, 1); }
  .terminal-card > header { min-height: 48px; padding: 7px 8px 7px 9px; display: flex; align-items: center; gap: 7px; border-bottom: 1px solid rgba(97, 119, 109, 0.11); cursor: grab; touch-action: none; }
  .terminal-card.dragging > header { cursor: grabbing; }
  .terminal-card.resizing { user-select: none; }
  .terminal-card.dock-moving { border-color: rgba(72, 142, 111, 0.5); box-shadow: 0 12px 38px rgba(31, 91, 66, 0.24), 0 0 0 2px rgba(75, 157, 120, 0.08); transform: scale(0.992); }
  .terminal-card.dock-target { border-color: rgba(65, 151, 111, 0.7); box-shadow: 0 12px 38px rgba(31, 91, 66, 0.22), 0 0 0 3px rgba(75, 157, 120, 0.14); }
  .terminal-card.settling { border-color: rgba(69, 139, 108, 0.42); box-shadow: 0 12px 38px rgba(31, 91, 66, 0.2); }
  .dock-silhouette { position: absolute; z-index: 30; inset: 5px; overflow: hidden; border: 1px solid rgba(71, 155, 117, 0.62); border-radius: 13px; background: rgba(76, 161, 121, 0.075); box-shadow: inset 0 0 0 1px rgba(225, 249, 238, 0.48); pointer-events: none; animation: dock-breathe 900ms ease-in-out infinite alternate; }
  .dock-silhouette::before { position: absolute; border: 1px solid rgba(65, 149, 111, 0.68); border-radius: 9px; content: ""; background: linear-gradient(135deg, rgba(77, 164, 121, 0.32), rgba(77, 164, 121, 0.12)); box-shadow: 0 6px 18px rgba(38, 105, 76, 0.14); }
  .dock-silhouette span { position: absolute; z-index: 1; padding: 3px 6px; border-radius: 999px; color: #39755a; background: rgba(232, 246, 239, 0.9); font-size: 7px; font-weight: 800; letter-spacing: 0.07em; text-transform: uppercase; }
  .dock-left .dock-silhouette::before, .dock-right .dock-silhouette::before { top: 12%; bottom: 12%; width: 31%; }
  .dock-left .dock-silhouette::before { left: 7px; }
  .dock-right .dock-silhouette::before { right: 7px; }
  .dock-left .dock-silhouette span { top: 50%; left: 12px; transform: translateY(-50%); }
  .dock-right .dock-silhouette span { top: 50%; right: 12px; transform: translateY(-50%); }
  .dock-top .dock-silhouette::before, .dock-bottom .dock-silhouette::before { right: 12%; left: 12%; height: 31%; }
  .dock-top .dock-silhouette::before { top: 7px; }
  .dock-bottom .dock-silhouette::before { bottom: 7px; }
  .dock-top .dock-silhouette span { top: 12px; left: 50%; transform: translateX(-50%); }
  .dock-bottom .dock-silhouette span { bottom: 12px; left: 50%; transform: translateX(-50%); }
  @keyframes dock-breathe { from { opacity: 0.7; } to { opacity: 1; } }
  .agent-icon { width: 26px; height: 26px; display: grid; place-items: center; border-radius: 8px; background: rgba(80, 105, 94, 0.06); }
  .identity { min-width: 0; flex: 1; display: grid; gap: 1px; }
  .identity strong { color: #26342e; font-size: 11px; }
  .identity small { overflow: hidden; color: #829089; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .source-badge { padding: 3px 5px; display: inline-flex; align-items: center; gap: 3px; border-radius: 999px; color: #718079; background: rgba(80, 104, 94, 0.075); font-size: 7px; font-weight: 760; letter-spacing: 0.04em; text-transform: uppercase; }
  header button { width: 25px; height: 25px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 7px; color: #73817b; background: transparent; cursor: pointer; }
  header button:hover { color: #43574e; background: rgba(72, 99, 87, 0.07); }
  .dock-button { color: #4a7564; }
  .terminate-button { color: #9d615c; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.7; }
  .terminal-output { min-height: 0; flex: 1; padding: 10px 12px 7px; overflow-y: auto; color: #55635d; background: linear-gradient(180deg, rgba(61, 87, 75, 0.025), transparent); font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace; font-size: 9px; }
  .terminal-output p { margin: 0 0 6px; line-height: 1.45; }
  .terminal-output p > span { color: #36a269; font-weight: 800; }
  .terminal-output i { color: #8a9690; font-style: normal; }
  .status-running, .status-running span { color: #4e7faf; }
  .status-permission_required, .status-permission_required span { color: #b06b25; }
  .status-waiting_for_input, .status-waiting_for_input span { color: #b0812d; }
  .status-completed, .status-completed span { color: #55a473; }
  .status-failed, .status-failed span { color: #ad4f4f; }
  .final-response { margin: 8px 0; padding: 8px 9px; border: 1px solid rgba(74, 102, 89, 0.1); border-radius: 8px; background: rgba(67, 99, 84, 0.035); }
  .final-response strong { display: block; margin-bottom: 5px; color: #648075; font: 760 8px Inter, sans-serif; letter-spacing: 0.05em; text-transform: uppercase; }
  .final-response p { max-height: 180px; margin: 0; overflow-y: auto; color: #475750; line-height: 1.55; overflow-wrap: anywhere; white-space: pre-wrap; scrollbar-width: thin; }
  .permission { margin: 7px 0 2px; padding-left: 9px; display: grid; gap: 6px; border-left: 2px solid #c87d32; }
  .permission strong { color: #5a4633; font: 700 9px/1.35 Inter, sans-serif; }
  .permission code { padding: 5px 6px; overflow: hidden; border-radius: 6px; color: #5f6b66; background: rgba(74, 99, 88, 0.055); font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .permission > div { display: flex; gap: 4px; }
  .permission button { min-height: 23px; padding: 0 7px; border: 1px solid rgba(82, 101, 93, 0.15); border-radius: 6px; color: #4b5d55; background: rgba(255, 255, 255, 0.58); font: 700 8px Inter, sans-serif; cursor: pointer; }
  .permission button.danger { color: #a64d4d; }
  .hint { color: #89948f; font-size: 8px; }
  .hint.docked { color: #4f7566; }
  .terminate-confirm { margin: 8px 0 2px; padding: 7px 8px; display: flex; align-items: center; gap: 6px; border: 1px solid rgba(166, 77, 77, 0.14); border-radius: 8px; background: rgba(166, 77, 77, 0.04); font: 700 8px/1.35 Inter, sans-serif; }
  .terminate-confirm > span { min-width: 0; flex: 1; color: #7d5d58; }
  .terminate-confirm > div { display: flex; gap: 4px; }
  .terminate-confirm button { min-height: 22px; padding: 0 6px; border: 1px solid rgba(84, 101, 93, 0.14); border-radius: 6px; color: #596861; background: rgba(255, 255, 255, 0.48); font: 700 7px Inter, sans-serif; cursor: pointer; }
  .terminate-confirm button.danger { color: #a54c4c; border-color: rgba(166, 77, 77, 0.2); }
  .terminal-composer { min-height: 63px; padding: 7px 8px 8px 10px; display: flex; align-items: flex-end; gap: 6px; border-top: 1px solid rgba(97, 119, 109, 0.11); }
  textarea { min-width: 0; height: 46px; flex: 1; padding: 7px 8px; resize: none; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 9px; outline: none; color: #34443d; background: rgba(255, 255, 255, 0.5); font: 9px/1.4 Inter, sans-serif; }
  textarea:focus { border-color: rgba(52, 151, 103, 0.42); box-shadow: 0 0 0 3px rgba(52, 151, 103, 0.07); }
  textarea:disabled { opacity: 0.58; }
  .terminal-composer button { width: 29px; height: 29px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 8px; color: white; background: #318e62; cursor: pointer; }
  .terminal-composer button:disabled { opacity: 0.35; cursor: default; }
  .message { margin: -4px 11px 6px; color: #ad4f4f; font-size: 8px; }
  .resize-handle { position: absolute; z-index: 20; width: 18px; height: 18px; padding: 0; border: 0; outline: 0; background: transparent; touch-action: none; }
  .resize-handle::after { position: absolute; width: 6px; height: 6px; content: ""; opacity: 0; transition: opacity 120ms ease; }
  .resize-handle:hover::after { opacity: 0.7; }
  .resize-nw { top: 0; left: 0; cursor: nwse-resize; }
  .resize-nw::after { top: 3px; left: 3px; border-top: 1px solid #668276; border-left: 1px solid #668276; }
  .resize-ne { top: 0; right: 0; cursor: nesw-resize; }
  .resize-ne::after { top: 3px; right: 3px; border-top: 1px solid #668276; border-right: 1px solid #668276; }
  .resize-sw { bottom: 0; left: 0; cursor: nesw-resize; }
  .resize-sw::after { bottom: 3px; left: 3px; border-bottom: 1px solid #668276; border-left: 1px solid #668276; }
  .resize-se { right: 0; bottom: 0; cursor: nwse-resize; }
  .resize-se::after { right: 3px; bottom: 3px; border-right: 1px solid #668276; border-bottom: 1px solid #668276; }
  .loading { align-items: center; justify-content: center; gap: 9px; color: #78857f; font-size: 9px; }

  .terminal-window.dark { color-scheme: dark; }
  .terminal-window.dark .terminal-card { color: #dbe7e1; border-color: rgba(190, 209, 200, 0.13); background: rgba(20, 29, 25, 0.97); }
  .terminal-window.dark .terminal-card.dock-moving,
  .terminal-window.dark .terminal-card.dock-target,
  .terminal-window.dark .terminal-card.settling { border-color: rgba(91, 186, 143, 0.5); box-shadow: 0 12px 38px rgba(8, 21, 15, 0.48), 0 0 0 2px rgba(91, 186, 143, 0.08); }
  .terminal-window.dark .dock-silhouette { border-color: rgba(96, 193, 149, 0.5); background: rgba(72, 157, 116, 0.06); box-shadow: inset 0 0 0 1px rgba(154, 220, 188, 0.08); }
  .terminal-window.dark .dock-silhouette::before { border-color: rgba(99, 197, 152, 0.52); background: linear-gradient(135deg, rgba(79, 174, 128, 0.22), rgba(69, 149, 111, 0.08)); }
  .terminal-window.dark .dock-silhouette span { color: #a8d9c2; background: rgba(27, 51, 40, 0.92); }
  .terminal-window.dark .terminal-card > header,
  .terminal-window.dark .terminal-composer { border-color: rgba(190, 209, 200, 0.09); }
  .terminal-window.dark .identity strong { color: #e2ebe6; }
  .terminal-window.dark .identity small,
  .terminal-window.dark .hint { color: #93a19a; }
  .terminal-window.dark .agent-icon,
  .terminal-window.dark .source-badge { background: rgba(205, 222, 213, 0.07); }
  .terminal-window.dark .source-badge { color: #a7b5ae; }
  .terminal-window.dark .terminal-output { color: #b8c6bf; background: linear-gradient(180deg, rgba(114, 151, 134, 0.035), transparent); }
  .terminal-window.dark .final-response { border-color: rgba(205, 222, 213, 0.08); background: rgba(218, 234, 226, 0.035); }
  .terminal-window.dark .final-response strong { color: #91a89d; }
  .terminal-window.dark .final-response p { color: #c3d0ca; }
  .terminal-window.dark textarea { color: #d0ddd6; border-color: rgba(205, 222, 213, 0.12); background: rgba(220, 234, 227, 0.045); }
  .terminal-window.dark .permission strong { color: #dfc6ac; }
  .terminal-window.dark .permission code,
  .terminal-window.dark .permission button { color: #bdcbc4; background: rgba(218, 232, 225, 0.055); }
  @media (prefers-reduced-motion: reduce) {
    .terminal-card { transition-duration: 0.01ms; }
    .dock-silhouette { animation: none; }
  }
</style>
