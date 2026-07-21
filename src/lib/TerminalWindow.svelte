<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { AgentSession, PermissionAction, TerminalWindowState } from "$lib/domain";
  import BrandIcon from "$lib/BrandIcon.svelte";
  import LumeLogo from "$lib/LumeLogo.svelte";
  import {
    closeTerminalWindow,
    decidePermission,
    loadSessions,
    loadTerminalWindowState,
    moveTerminalWindow,
    openSessionSource,
    resizeTerminalWindow,
    submitPrompt,
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
  let resizing = $state(false);
  let moveFrame: number | null = null;
  let resizeFrame: number | null = null;
  let dragState: {
    pointerId: number;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
  } | null = null;
  let resizeState: {
    pointerId: number;
    direction: ResizeDirection;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
    originWidth: number;
    originHeight: number;
  } | null = null;

  const canSubmit = $derived(
    Boolean(
      session &&
        (session.source === "web" ||
          (session.agent === "codex" &&
            session.nativeSessionId)),
    ),
  );
  const readyForPrompt = $derived(
    Boolean(session && ["completed", "failed", "waiting_for_input"].includes(session.status)),
  );

  onMount(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void (async () => {
      windowState = await loadTerminalWindowState(label);
      await refresh();
      if (disposed) return;
      stopListening = await listen("lume://sessions-changed", () => void refresh());
    })();
    return () => {
      disposed = true;
      stopListening?.();
      if (moveFrame !== null) cancelAnimationFrame(moveFrame);
      if (resizeFrame !== null) cancelAnimationFrame(resizeFrame);
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
    return { cli: "CLI", vscode: "VS Code", desktop: "Desktop" }[item.source] ?? "Origem";
  }

  function sourceIcon(item: AgentSession) {
    if (item.source === "cli") return "terminal" as const;
    if (item.source === "vscode") return "vscode" as const;
    if (item.source === "web") return item.sourceApp ?? ("browsers" as const);
    return "unknown" as const;
  }

  function beginDrag(event: PointerEvent) {
    if (event.button !== 0 || !windowState) return;
    if ((event.target as HTMLElement).closest("button, textarea")) return;
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    dragState = {
      pointerId: event.pointerId,
      startX: event.screenX,
      startY: event.screenY,
      originX: windowState.x,
      originY: windowState.y,
    };
    dragging = false;
  }

  function drag(event: PointerEvent) {
    if (!dragState || event.pointerId !== dragState.pointerId) return;
    const dx = event.screenX - dragState.startX;
    const dy = event.screenY - dragState.startY;
    if (!dragging && Math.hypot(dx, dy) < 3) return;
    dragging = true;
    const x = dragState.originX + dx;
    const y = dragState.originY + dy;
    if (moveFrame !== null) cancelAnimationFrame(moveFrame);
    moveFrame = requestAnimationFrame(() => {
      moveFrame = null;
      void moveTerminalWindow(label, x, y, false).then((next) => (windowState = next));
    });
  }

  async function endDrag(event: PointerEvent) {
    if (!dragState || event.pointerId !== dragState.pointerId) return;
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    const dx = event.screenX - dragState.startX;
    const dy = event.screenY - dragState.startY;
    const x = dragState.originX + dx;
    const y = dragState.originY + dy;
    dragState = null;
    if (!dragging) return;
    windowState = await moveTerminalWindow(label, x, y, true);
    dragging = false;
  }

  async function detach() {
    windowState = await undockTerminalWindow(label);
  }

  function beginResize(event: PointerEvent, direction: ResizeDirection) {
    if (!windowState || event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    resizeState = {
      pointerId: event.pointerId,
      direction,
      startX: event.screenX,
      startY: event.screenY,
      originX: windowState.x,
      originY: windowState.y,
      originWidth: windowState.width,
      originHeight: windowState.height,
    };
    resizing = true;
  }

  function resizedPlacement(event: PointerEvent) {
    if (!resizeState) return null;
    const dx = event.screenX - resizeState.startX;
    const dy = event.screenY - resizeState.startY;
    const fromWest = resizeState.direction.endsWith("West");
    const fromNorth = resizeState.direction.startsWith("North");
    const width = Math.min(760, Math.max(300, resizeState.originWidth + (fromWest ? -dx : dx)));
    const height = Math.min(640, Math.max(240, resizeState.originHeight + (fromNorth ? -dy : dy)));
    return {
      x: fromWest ? resizeState.originX + resizeState.originWidth - width : resizeState.originX,
      y: fromNorth ? resizeState.originY + resizeState.originHeight - height : resizeState.originY,
      width,
      height,
    };
  }

  function resize(event: PointerEvent) {
    if (!resizeState || event.pointerId !== resizeState.pointerId) return;
    const next = resizedPlacement(event);
    if (!next) return;
    if (resizeFrame !== null) cancelAnimationFrame(resizeFrame);
    resizeFrame = requestAnimationFrame(() => {
      resizeFrame = null;
      void resizeTerminalWindow(label, next.x, next.y, next.width, next.height).then(
        (state) => (windowState = state),
      ).catch((error) => (message = String(error).replace(/^Error:\s*/, "")));
    });
  }

  async function endResize(event: PointerEvent) {
    if (!resizeState || event.pointerId !== resizeState.pointerId) return;
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    const next = resizedPlacement(event);
    resizeState = null;
    if (resizeFrame !== null) cancelAnimationFrame(resizeFrame);
    resizeFrame = null;
    try {
      if (next) {
        windowState = await resizeTerminalWindow(label, next.x, next.y, next.width, next.height);
      }
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      resizing = false;
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

  async function sendPrompt() {
    if (!session || !prompt.trim() || sending || !canSubmit || !readyForPrompt) return;
    sending = true;
    message = null;
    try {
      await submitPrompt(session.id, prompt.trim());
      prompt = "";
      session = { ...session, status: "running", statusLabel: "Prompt enviado pelo Lume", lastResponse: undefined };
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
      allow_once: "Permitir",
      allow_session: "Na sessão",
      deny: "Recusar",
      open_source: "Abrir origem",
    }[action];
  }
</script>

<main class="terminal-window">
  {#if session}
    <section class:dragging class:resizing class="terminal-card">
      <header
        role="banner"
        onpointerdown={beginDrag}
        onpointermove={drag}
        onpointerup={endDrag}
        onpointercancel={endDrag}
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
          <button class="dock-button" type="button" onclick={detach} aria-label="Desacoplar terminal" title="Desacoplar">
            <svg viewBox="0 0 20 20"><path d="M7 6 5.5 7.5a3 3 0 0 0 4.2 4.2l1.2-1.2M13 14l1.5-1.5a3 3 0 0 0-4.2-4.2L9.1 9.5" /></svg>
          </button>
        {/if}
        <button class="close-button" type="button" onclick={closeTerminal} aria-label="Fechar terminal">
          <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8" /></svg>
        </button>
      </header>

      <div class="terminal-output">
        <p><span>$</span> {session.agentLabel.toLowerCase()} <i>{session.project}</i></p>
        <p class="status status-{session.status}"><span>&gt;</span> {session.statusLabel}</p>
        {#if session.lastResponse}
          <div class="final-response">
            <strong>Resposta final</strong>
            <p>{session.lastResponse}</p>
          </div>
        {/if}
        {#if session.pendingPermission}
          <div class="permission">
            <strong>{session.pendingPermission.summary}</strong>
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
          <p class="hint">Esta origem é acompanhada aqui, mas o envio continua nela.</p>
        {:else if windowState?.docked}
          <p class="hint docked">Acoplado · arraste este terminal para mover o conjunto.</p>
        {:else}
          <p class="hint">Aproxime de outro mini terminal para acoplar.</p>
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
          aria-label="Prompt para {session.agentLabel}"
          placeholder={!canSubmit ? "Envio indisponível nesta origem" : readyForPrompt ? `Prompt para ${session.agentLabel}…` : "Agente em execução…"}
        ></textarea>
        {#if canSubmit}
          <button disabled={!prompt.trim() || !readyForPrompt || sending} type="submit" aria-label="Enviar prompt">
            <svg viewBox="0 0 20 20"><path d="m4 10 12-6-4 12-2-4zM10 12l2-2" /></svg>
          </button>
        {:else}
          <button type="button" onclick={openOrigin} aria-label="Abrir origem">
            <svg viewBox="0 0 20 20"><path d="M7 5h8v8M14.5 5.5 6 14" /></svg>
          </button>
        {/if}
      </form>
      {#if message}<p class="message">{message}</p>{/if}
      <button class="resize-handle resize-nw" type="button" tabindex="-1" aria-label="Redimensionar pelo canto superior esquerdo" onpointerdown={(event) => beginResize(event, "NorthWest")} onpointermove={resize} onpointerup={endResize} onpointercancel={endResize}></button>
      <button class="resize-handle resize-ne" type="button" tabindex="-1" aria-label="Redimensionar pelo canto superior direito" onpointerdown={(event) => beginResize(event, "NorthEast")} onpointermove={resize} onpointerup={endResize} onpointercancel={endResize}></button>
      <button class="resize-handle resize-sw" type="button" tabindex="-1" aria-label="Redimensionar pelo canto inferior esquerdo" onpointerdown={(event) => beginResize(event, "SouthWest")} onpointermove={resize} onpointerup={endResize} onpointercancel={endResize}></button>
      <button class="resize-handle resize-se" type="button" tabindex="-1" aria-label="Redimensionar pelo canto inferior direito" onpointerdown={(event) => beginResize(event, "SouthEast")} onpointermove={resize} onpointerup={endResize} onpointercancel={endResize}></button>
    </section>
  {:else}
    <section class="terminal-card loading"><LumeLogo size={34} /><span>Conectando à sessão…</span></section>
  {/if}
</main>

<style>
  .terminal-window { width: 100%; height: 100%; }
  .terminal-card { position: relative; width: 100%; height: 100%; display: flex; flex-direction: column; overflow: hidden; border: 1px solid rgba(103, 126, 116, 0.2); border-radius: 17px; color: #26342e; background: rgba(248, 251, 249, 0.97); box-shadow: 0 10px 34px rgba(20, 36, 29, 0.2); backdrop-filter: blur(24px) saturate(120%); }
  .terminal-card > header { min-height: 48px; padding: 7px 8px 7px 9px; display: flex; align-items: center; gap: 7px; border-bottom: 1px solid rgba(97, 119, 109, 0.11); cursor: grab; touch-action: none; }
  .terminal-card.dragging > header { cursor: grabbing; }
  .terminal-card.resizing { user-select: none; }
  .agent-icon { width: 26px; height: 26px; display: grid; place-items: center; border-radius: 8px; background: rgba(80, 105, 94, 0.06); }
  .identity { min-width: 0; flex: 1; display: grid; gap: 1px; }
  .identity strong { color: #26342e; font-size: 11px; }
  .identity small { overflow: hidden; color: #829089; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .source-badge { padding: 3px 5px; display: inline-flex; align-items: center; gap: 3px; border-radius: 999px; color: #718079; background: rgba(80, 104, 94, 0.075); font-size: 7px; font-weight: 760; letter-spacing: 0.04em; text-transform: uppercase; }
  header button { width: 25px; height: 25px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 7px; color: #73817b; background: transparent; cursor: pointer; }
  header button:hover { color: #43574e; background: rgba(72, 99, 87, 0.07); }
  .dock-button { color: #4a7564; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.7; }
  .terminal-output { min-height: 0; flex: 1; padding: 10px 12px 7px; overflow-y: auto; color: #55635d; background: linear-gradient(180deg, rgba(61, 87, 75, 0.025), transparent); font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace; font-size: 9px; }
  .terminal-output p { margin: 0 0 6px; line-height: 1.45; }
  .terminal-output p > span { color: #36a269; font-weight: 800; }
  .terminal-output i { color: #8a9690; font-style: normal; }
  .status-running, .status-running span { color: #4e7faf; }
  .status-permission_required, .status-permission_required span { color: #b06b25; }
  .status-waiting_for_input, .status-waiting_for_input span { color: #b0812d; }
  .status-completed, .status-completed span { color: #7d8782; }
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
  .terminal-composer { min-height: 63px; padding: 7px 8px 8px 10px; display: flex; align-items: flex-end; gap: 6px; border-top: 1px solid rgba(97, 119, 109, 0.11); }
  textarea { min-width: 0; height: 46px; flex: 1; padding: 7px 8px; resize: none; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 9px; outline: none; color: #34443d; background: rgba(255, 255, 255, 0.5); font: 9px/1.4 Inter, sans-serif; }
  textarea:focus { border-color: rgba(52, 151, 103, 0.42); box-shadow: 0 0 0 3px rgba(52, 151, 103, 0.07); }
  textarea:disabled { opacity: 0.58; }
  .terminal-composer button { width: 29px; height: 29px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 8px; color: white; background: #318e62; cursor: pointer; }
  .terminal-composer button:disabled { opacity: 0.35; cursor: default; }
  .message { margin: -4px 11px 6px; color: #ad4f4f; font-size: 8px; }
  .resize-handle { position: absolute; z-index: 20; width: 18px; height: 18px; padding: 0; border: 0; outline: 0; background: transparent; touch-action: none; }
  .resize-handle::after { position: absolute; width: 6px; height: 6px; content: ""; opacity: 0; transition: opacity 120ms ease; }
  .resize-handle:hover::after, .terminal-card.resizing .resize-handle::after { opacity: 0.7; }
  .resize-nw { top: 0; left: 0; cursor: nwse-resize; }
  .resize-nw::after { top: 3px; left: 3px; border-top: 1px solid #668276; border-left: 1px solid #668276; }
  .resize-ne { top: 0; right: 0; cursor: nesw-resize; }
  .resize-ne::after { top: 3px; right: 3px; border-top: 1px solid #668276; border-right: 1px solid #668276; }
  .resize-sw { bottom: 0; left: 0; cursor: nesw-resize; }
  .resize-sw::after { bottom: 3px; left: 3px; border-bottom: 1px solid #668276; border-left: 1px solid #668276; }
  .resize-se { right: 0; bottom: 0; cursor: nwse-resize; }
  .resize-se::after { right: 3px; bottom: 3px; border-right: 1px solid #668276; border-bottom: 1px solid #668276; }
  .loading { align-items: center; justify-content: center; gap: 9px; color: #78857f; font-size: 9px; }

  @media (prefers-color-scheme: dark) {
    .terminal-card { color: #dbe7e1; border-color: rgba(190, 209, 200, 0.13); background: rgba(20, 29, 25, 0.97); }
    .terminal-card > header, .terminal-composer { border-color: rgba(190, 209, 200, 0.09); }
    .identity strong { color: #e2ebe6; }
    .identity small, .hint { color: #93a19a; }
    .agent-icon, .source-badge { background: rgba(205, 222, 213, 0.07); }
    .source-badge { color: #a7b5ae; }
    .terminal-output { color: #b8c6bf; background: linear-gradient(180deg, rgba(114, 151, 134, 0.035), transparent); }
    .final-response { border-color: rgba(205, 222, 213, 0.08); background: rgba(218, 234, 226, 0.035); }
    .final-response strong { color: #91a89d; }
    .final-response p { color: #c3d0ca; }
    textarea { color: #d0ddd6; border-color: rgba(205, 222, 213, 0.12); background: rgba(220, 234, 227, 0.045); }
    .permission strong { color: #dfc6ac; }
    .permission code, .permission button { color: #bdcbc4; background: rgba(218, 232, 225, 0.055); }
  }
</style>
