<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { AgentSession, DockPreviewEvent, PermissionAction, Preferences, SessionActivity, TerminalWindowState } from "$lib/domain";
  import BrandIcon from "$lib/BrandIcon.svelte";
  import LumeLogo from "$lib/LumeLogo.svelte";
  import LumeMascot from "$lib/LumeMascot.svelte";
  import { displayText, localize, type Language } from "$lib/i18n";
  import {
    beginLayeredTerminalResize,
    beginTerminalNativeDrag,
    cancelTerminalWindowMove,
    closeTerminalWindow,
    decidePermission,
    finishLayeredTerminalResize,
    loadDisplayBackend,
    loadPreferences,
    loadSessions,
    loadTerminalWindowState,
    moveTerminalWindow,
    openSessionSource,
    resizeTerminalWindow,
    submitPrompt,
    terminateSession,
    undockTerminalWindow,
    type DisplayBackend,
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
  let pendingMove: { x: number; y: number } | null = null;
  let lastMove: { x: number; y: number } | null = null;
  let moveSyncRunning = false;
  let finalizeRequested = false;
  let displayBackend = $state<DisplayBackend>("native");
  let nativeDragActive = false;
  let dragState: {
    pointerId: number;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
    scale: number;
  } | null = null;
  let resizing = $state(false);
  let resizeEndTimer: ReturnType<typeof setTimeout> | undefined;
  let resizeDragState: {
    pointerId: number;
    direction: ResizeDirection;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
    originWidth: number;
    originHeight: number;
    scale: number;
  } | null = null;
  let resizePreparing: Promise<void> | null = null;
  let pendingResize: { x: number; y: number; width: number; height: number } | null = null;
  let resizeSyncRunning = false;
  let settling = $state(false);
  let dockMovingLabel = $state<string | null>(null);
  let dockPreview = $state<NonNullable<DockPreviewEvent["preview"]> | null>(null);
  let terminateConfirm = $state(false);
  let terminating = $state(false);
  let activeTab = $state<"chat" | "changes">("chat");
  let outputElement = $state<HTMLDivElement | null>(null);
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
  const activities = $derived(session?.activities ?? []);
  const changedFiles = $derived.by(() => {
    const files = [
      ...(session?.activities.flatMap((activity) => activity.files) ?? []),
      ...(session?.results.flatMap((result) => result.files) ?? []),
    ];
    return [...new Set(files)];
  });
  const fileActivities = $derived(
    session?.activities.filter((activity) => activity.kind === "file" && activity.detail) ?? [],
  );
  type ChatTurn = {
    id: string;
    prompt?: SessionActivity;
    items: SessionActivity[];
    files: string[];
  };
  const chatTurns = $derived.by<ChatTurn[]>(() => {
    const turns: ChatTurn[] = [];
    const ensureTurn = (id: string): ChatTurn => {
      const turn: ChatTurn = { id, items: [], files: [] };
      turns.push(turn);
      return turn;
    };
    let current: ChatTurn | undefined;
    for (const activity of activities) {
      if (activity.kind === "prompt") {
        current = ensureTurn(activity.id);
        current.prompt = activity;
        continue;
      }
      current ??= ensureTurn(`turn:${activity.id}`);
      current.items.push(activity);
      for (const file of activity.files) {
        if (!current.files.includes(file)) current.files.push(file);
      }
    }
    for (const result of session?.results ?? []) {
      const resultTurn =
        [...turns]
          .reverse()
          .find((turn) => !turn.prompt || turn.prompt.createdAt <= result.createdAt) ??
        ensureTurn(`result:${result.id}`);
      current = resultTurn;
      for (const file of result.files) {
        if (!resultTurn.files.includes(file)) resultTurn.files.push(file);
      }
      if (
        result.response &&
        !resultTurn.items.some(
          (item) => item.kind === "message" && item.detail === result.response,
        )
      ) {
        resultTurn.items.push({
          id: `response:${result.id}`,
          kind: "message",
          title: "Resposta do agente",
          detail: result.response,
          status: "completed",
          createdAt: result.createdAt,
          files: [],
        });
      }
    }
    if (
      session?.lastResponse &&
      !turns.some((turn) =>
        turn.items.some(
          (item) => item.kind === "message" && item.detail === session?.lastResponse,
        ),
      )
    ) {
      current ??= ensureTurn(`response:${session.id}`);
      current.items.push({
        id: `response:${session.id}:${session.updatedAt}`,
        kind: "message",
        title: "Resposta do agente",
        detail: session.lastResponse,
        status: "completed",
        createdAt: session.updatedAt,
        files: [],
      });
    }
    return turns;
  });

  onMount(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let stopWindowChanges: (() => void) | undefined;
    let stopResized: (() => void) | undefined;
    let stopPreferences: (() => void) | undefined;
    let stopDockPreview: (() => void) | undefined;
    let stopNativeDragEnded: (() => void) | undefined;
    const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    const syncSystemTheme = (event: MediaQueryListEvent | MediaQueryList) => {
      systemDark = event.matches;
    };
    syncSystemTheme(colorScheme);
    colorScheme.addEventListener("change", syncSystemTheme);
    void (async () => {
      const [nextWindowState, nextPreferences, nextDisplayBackend] = await Promise.all([
        loadTerminalWindowState(label),
        loadPreferences(),
        loadDisplayBackend(),
      ]);
      windowState = nextWindowState;
      language = nextPreferences.language;
      darkMode = nextPreferences.darkMode;
      displayBackend = nextDisplayBackend;
      await refresh();
      if (disposed) return;
      stopListening = await listen("lume://sessions-changed", () => void refresh());
      stopWindowChanges = await listen("lume://terminal-windows-changed", async () => {
        try {
          windowState = await loadTerminalWindowState(label);
        } catch {
          // The window may be closing.
        }
      });
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
      stopNativeDragEnded = await listen<{ label: string }>(
        "lume://terminal-native-drag-ended",
        async ({ payload }) => {
          if (payload.label !== label) return;
          nativeDragActive = false;
          dragging = false;
          dockMovingLabel = null;
          dockPreview = null;
          try {
            windowState = await loadTerminalWindowState(label);
          } catch {
            // The window may be closing.
          }
        },
      );
      stopResized = await currentWindow.onResized(() => {
        if (settling) return;
        if (resizeDragState) return;
        resizing = true;
        if (resizeEndTimer) clearTimeout(resizeEndTimer);
        resizeEndTimer = setTimeout(async () => {
          resizing = false;
          try {
            windowState = await loadTerminalWindowState(label);
          } catch {
            // The window may be closing.
          }
        }, 180);
      });
    })();
    return () => {
      disposed = true;
      stopListening?.();
      stopWindowChanges?.();
      stopResized?.();
      stopPreferences?.();
      stopDockPreview?.();
      stopNativeDragEnded?.();
      colorScheme.removeEventListener("change", syncSystemTheme);
      if (resizeEndTimer) clearTimeout(resizeEndTimer);
    };
  });

  async function refresh() {
    const shouldFollow = !outputElement ||
      outputElement.scrollHeight - outputElement.scrollTop - outputElement.clientHeight < 32;
    const sessions = await loadSessions();
    session = sessions.find((item) => item.id === windowState?.sessionId) ?? null;
    if (shouldFollow) {
      await tick();
      outputElement?.scrollTo({ top: outputElement.scrollHeight });
    }
  }

  function activityMark(activity: SessionActivity) {
    return {
      prompt: "›",
      message: "◆",
      analysis: "···",
      plan: "≡",
      command: "$",
      file: "±",
      test: "✓",
      tool: "⌁",
      permission: "!",
    }[activity.kind] ?? "·";
  }

  function activityTime(createdAt: number) {
    return new Intl.DateTimeFormat(language, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }).format(new Date(createdAt));
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

  function queueMove(x: number, y: number) {
    const next = { x, y };
    pendingMove = next;
    lastMove = next;
    void flushMoves();
  }

  async function flushMoves() {
    if (moveSyncRunning) return;
    moveSyncRunning = true;
    try {
      while (pendingMove) {
        const next = pendingMove;
        pendingMove = null;
        windowState = await moveTerminalWindow(label, next.x, next.y, false);
      }
      if (finalizeRequested && lastMove) {
        const finalPosition = lastMove;
        finalizeRequested = false;
        settling = true;
        dockPreview = null;
        dockMovingLabel = null;
        windowState = await moveTerminalWindow(
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
      dockMovingLabel = null;
      dockPreview = null;
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      moveSyncRunning = false;
      if (pendingMove || finalizeRequested) void flushMoves();
    }
  }

  function beginDrag(event: PointerEvent) {
    if (event.button !== 0 || !windowState) return;
    if ((event.target as HTMLElement).closest("button, textarea")) return;
    if (displayBackend === "xwayland-fallback") {
      event.preventDefault();
      dragging = true;
      nativeDragActive = true;
      dockMovingLabel = null;
      dockPreview = null;
      void beginTerminalNativeDrag(label)
        .then(() => currentWindow.startDragging())
        .catch((error) => {
          nativeDragActive = false;
          dragging = false;
          message = String(error).replace(/^Error:\s*/, "");
        });
      return;
    }
    if (displayBackend === "native-gnome") {
      event.preventDefault();
      dragging = true;
      nativeDragActive = true;
      dockMovingLabel = null;
      dockPreview = null;
      void currentWindow
        .startDragging()
        .catch((error) => {
          message = String(error).replace(/^Error:\s*/, "");
        })
        .finally(() => {
          setTimeout(() => {
            nativeDragActive = false;
            dragging = false;
          }, 300);
        });
      return;
    }
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    dragging = true;
    dragMoved = false;
    pendingMove = null;
    lastMove = null;
    finalizeRequested = false;
    settling = false;
    dragState = {
      pointerId: event.pointerId,
      startX: event.screenX,
      startY: event.screenY,
      originX: windowState.x,
      originY: windowState.y,
      scale: windowState.scale,
    };
  }

  function moveDrag(event: PointerEvent) {
    if (!dragState || dragState.pointerId !== event.pointerId) return;
    const dx = (event.screenX - dragState.startX) * dragState.scale;
    const dy = (event.screenY - dragState.startY) * dragState.scale;
    if (!dragMoved && Math.hypot(dx, dy) < 2) return;
    event.preventDefault();
    dragMoved = true;
    queueMove(
      Math.round(dragState.originX + dx),
      Math.round(dragState.originY + dy),
    );
  }

  function endDrag(event: PointerEvent) {
    if (nativeDragActive) return;
    if (!dragState || dragState.pointerId !== event.pointerId) return;
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    dragState = null;
    dragging = false;
    if (!dragMoved || !lastMove) {
      dockMovingLabel = null;
      dockPreview = null;
      return;
    }
    finalizeRequested = true;
    void flushMoves();
  }

  function cancelDrag(event: PointerEvent) {
    if (!dragState || dragState.pointerId !== event.pointerId) return;
    dragState = null;
    dragging = false;
    finalizeRequested = false;
    pendingMove = null;
    dockMovingLabel = null;
    dockPreview = null;
    void (async () => {
      while (moveSyncRunning) await new Promise((resolve) => setTimeout(resolve, 0));
      await cancelTerminalWindowMove(label);
    })().catch(() => undefined);
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
    dockMovingLabel = null;
    resizing = true;
    if (windowState?.layered) {
      const target = event.currentTarget as HTMLElement;
      target.setPointerCapture(event.pointerId);
      resizeDragState = {
        pointerId: event.pointerId,
        direction,
        startX: event.screenX,
        startY: event.screenY,
        originX: windowState.x,
        originY: windowState.y,
        originWidth: windowState.width,
        originHeight: windowState.height,
        scale: windowState.scale,
      };
      pendingResize = null;
      resizePreparing = beginLayeredTerminalResize(label)
        .then((state) => {
          windowState = state;
        })
        .catch((error) => {
          message = String(error).replace(/^Error:\s*/, "");
          resizeDragState = null;
          resizing = false;
        })
        .finally(() => {
          resizePreparing = null;
          if (pendingResize) void flushResize();
        });
      return;
    }
    try {
      await currentWindow.startResizeDragging(direction);
    } catch (error) {
      resizing = false;
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  function moveResize(event: PointerEvent) {
    if (!resizeDragState || resizeDragState.pointerId !== event.pointerId) return;
    event.preventDefault();
    event.stopPropagation();
    const scale = resizeDragState.scale;
    const dx = (event.screenX - resizeDragState.startX) * scale;
    const dy = (event.screenY - resizeDragState.startY) * scale;
    const east = resizeDragState.direction.endsWith("East");
    const south = resizeDragState.direction.startsWith("South");
    const desiredWidth = resizeDragState.originWidth + (east ? dx : -dx) / scale;
    const desiredHeight = resizeDragState.originHeight + (south ? dy : -dy) / scale;
    const width = Math.max(300, Math.min(760, Math.round(desiredWidth)));
    const height = Math.max(240, Math.min(640, Math.round(desiredHeight)));
    pendingResize = {
      x: east
        ? resizeDragState.originX
        : resizeDragState.originX + Math.round((resizeDragState.originWidth - width) * scale),
      y: south
        ? resizeDragState.originY
        : resizeDragState.originY + Math.round((resizeDragState.originHeight - height) * scale),
      width,
      height,
    };
    void flushResize();
  }

  async function flushResize() {
    if (resizeSyncRunning || resizePreparing) return;
    resizeSyncRunning = true;
    try {
      while (pendingResize) {
        const next = pendingResize;
        pendingResize = null;
        windowState = await resizeTerminalWindow(
          label,
          next.x,
          next.y,
          next.width,
          next.height,
        );
      }
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
      pendingResize = null;
    } finally {
      resizeSyncRunning = false;
      if (pendingResize) void flushResize();
    }
  }

  async function endResize(event: PointerEvent) {
    if (!resizeDragState || resizeDragState.pointerId !== event.pointerId) return;
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    resizeDragState = null;
    if (resizePreparing) await resizePreparing;
    while (resizeSyncRunning || pendingResize) {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    try {
      windowState = await finishLayeredTerminalResize(label);
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
      class:resizing
      class:settling
      class:dock-moving={dockMovingLabel === label && Boolean(dockPreview)}
      class:dock-target={dockPreview?.targetLabel === label}
      class:dock-left={dockPreview?.side === "left"}
      class:dock-right={dockPreview?.side === "right"}
      class:dock-top={dockPreview?.side === "top"}
      class:dock-bottom={dockPreview?.side === "bottom"}
      class:joined-left={windowState?.connectedSides.includes("left")}
      class:joined-right={windowState?.connectedSides.includes("right")}
      class:joined-top={windowState?.connectedSides.includes("top")}
      class:joined-bottom={windowState?.connectedSides.includes("bottom")}
      class="terminal-card"
    >
      {#if dockPreview?.targetLabel === label}
        <div class="dock-silhouette" aria-hidden="true"><span>{tr("Dock", "Acoplar")}</span></div>
      {/if}
      <header
        role="banner"
        onpointerdown={beginDrag}
        onpointermove={moveDrag}
        onpointerup={endDrag}
        onpointercancel={cancelDrag}
      >
        <LumeMascot status={session.status} size={25} />
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

      <nav class="hub-tabs" aria-label={tr("Session details", "Detalhes da sessão")}>
        <button class:active={activeTab === "chat"} type="button" onclick={() => (activeTab = "chat")}>
          {tr("Chat", "Chat")} <span>{chatTurns.length}</span>
        </button>
        <button class:active={activeTab === "changes"} type="button" onclick={() => (activeTab = "changes")}>
          {tr("Changes", "Alterações")} <span>{changedFiles.length}</span>
        </button>
      </nav>

      <div class="terminal-output" bind:this={outputElement}>
        <p><span>$</span> {session.agentLabel.toLowerCase()} <i>{session.project}</i></p>
        <p class="status status-{session.status}"><span>&gt;</span> {displayText(language, session.statusLabel)}</p>
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
        {/if}

        {#if activeTab === "chat"}
          <div class="chat-feed">
            {#each chatTurns as turn (turn.id)}
              <article class="chat-turn">
                {#if turn.prompt?.detail}
                  <div class="chat-message user-message">
                    <header>
                      <strong>{tr("You", "Você")}</strong>
                      <time>{activityTime(turn.prompt.createdAt)}</time>
                    </header>
                    <pre>{turn.prompt.detail}</pre>
                  </div>
                {/if}
                {#each turn.items as item (item.id)}
                  {#if item.kind === "message" && item.detail}
                    <div class="chat-message agent-message">
                      <header>
                        <strong>{session.agentLabel}</strong>
                        <time>{activityTime(item.createdAt)}</time>
                      </header>
                      <pre>{item.detail}</pre>
                    </div>
                  {:else if item.kind !== "file"}
                    <details class="turn-trace">
                      <summary>
                        <span>{activityMark(item)}</span>
                        {displayText(language, item.title)}
                        <time>{activityTime(item.createdAt)}</time>
                      </summary>
                      {#if item.detail}<pre>{item.detail}</pre>{/if}
                    </details>
                  {/if}
                {/each}
                {#if turn.files.length}
                  <div class="turn-files">
                    <strong>{tr("Files changed in this prompt", "Arquivos alterados neste prompt")}</strong>
                    <div>
                      {#each turn.files as file}<code><span>±</span>{file}</code>{/each}
                    </div>
                  </div>
                {/if}
              </article>
            {:else}
              <p class="empty-state">{tr("Messages and agent activity will appear here in real time.", "As mensagens e a atividade do agente aparecerão aqui em tempo real.")}</p>
            {/each}
          </div>
        {:else}
          <section class="changes-panel">
            <strong>{tr("All changed files", "Todos os arquivos alterados")}</strong>
            {#if changedFiles.length}
              <div class="change-list">
                {#each changedFiles as file}<code><span>±</span>{file}</code>{/each}
              </div>
            {:else}
              <p class="empty-state">{tr("No file changes were reported in this session.", "Nenhuma alteração de arquivo foi informada nesta sessão.")}</p>
            {/if}
            {#each fileActivities as activity (activity.id)}
              <details class="change-diff">
                <summary>{displayText(language, activity.title)}</summary>
                <pre>{activity.detail}</pre>
              </details>
            {/each}
          </section>
        {/if}

        {#if !canSubmit}
          <p class="hint">{tr("This source is monitored here, but prompts are sent from the source.", "Esta origem é acompanhada aqui, mas o envio continua nela.")}</p>
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
      <button class="resize-handle resize-nw" type="button" tabindex="-1" aria-label={tr("Resize from top-left corner", "Redimensionar pelo canto superior esquerdo")} onpointerdown={(event) => void beginResize(event, "NorthWest")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
      <button class="resize-handle resize-ne" type="button" tabindex="-1" aria-label={tr("Resize from top-right corner", "Redimensionar pelo canto superior direito")} onpointerdown={(event) => void beginResize(event, "NorthEast")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
      <button class="resize-handle resize-sw" type="button" tabindex="-1" aria-label={tr("Resize from bottom-left corner", "Redimensionar pelo canto inferior esquerdo")} onpointerdown={(event) => void beginResize(event, "SouthWest")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
      <button class="resize-handle resize-se" type="button" tabindex="-1" aria-label={tr("Resize from bottom-right corner", "Redimensionar pelo canto inferior direito")} onpointerdown={(event) => void beginResize(event, "SouthEast")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
    </section>
  {:else}
    <section class="terminal-card loading"><LumeLogo size={34} /><span>{tr("Connecting to session…", "Conectando à sessão…")}</span></section>
  {/if}
</main>

<style>
  .terminal-window { width: 100%; height: 100%; }
  .terminal-card { position: relative; width: 100%; height: 100%; display: flex; flex-direction: column; overflow: hidden; container-type: inline-size; --chat-font-size: 9px; --chat-small-font-size: 8px; --chat-tiny-font-size: 7px; border: 1px solid rgba(103, 126, 116, 0.2); border-radius: 17px; color: #26342e; background: #f8fbf9; box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.32); transition: border-color 150ms ease, box-shadow 180ms ease, background-color 180ms ease, transform 180ms cubic-bezier(0.22, 1, 0.36, 1); }
  @supports (width: 1cqw) {
    .terminal-card { --chat-font-size: clamp(9px, calc(7px + 0.55cqw), 12px); --chat-small-font-size: clamp(8px, calc(6.2px + 0.5cqw), 10.5px); --chat-tiny-font-size: clamp(7px, calc(5.8px + 0.4cqw), 9px); }
  }
  .terminal-card > header { min-height: 48px; padding: 7px 8px 7px 9px; display: flex; align-items: center; gap: 7px; border-bottom: 1px solid rgba(97, 119, 109, 0.11); cursor: grab; touch-action: none; }
  .terminal-card.dragging > header { cursor: grabbing; }
  .terminal-card.resizing { user-select: none; }
  .terminal-card.dock-moving { border-color: rgba(72, 142, 111, 0.58); box-shadow: inset 0 0 0 2px rgba(75, 157, 120, 0.12); transform: scale(0.992); }
  .terminal-card.dock-target { border-color: rgba(65, 151, 111, 0.78); box-shadow: inset 0 0 0 3px rgba(75, 157, 120, 0.18); }
  .terminal-card.settling { border-color: rgba(69, 139, 108, 0.48); box-shadow: inset 0 0 0 2px rgba(75, 157, 120, 0.11); }
  .terminal-card.joined-left { border-left-width: 0; border-top-left-radius: 0; border-bottom-left-radius: 0; }
  .terminal-card.joined-right { border-right-width: 0; border-top-right-radius: 0; border-bottom-right-radius: 0; }
  .terminal-card.joined-top { border-top-width: 0; border-top-left-radius: 0; border-top-right-radius: 0; }
  .terminal-card.joined-bottom { border-bottom-width: 0; border-bottom-left-radius: 0; border-bottom-right-radius: 0; }
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
  header button { position: relative; z-index: 25; width: 25px; height: 25px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 7px; color: #73817b; background: transparent; cursor: pointer; }
  header button:hover { color: #43574e; background: rgba(72, 99, 87, 0.07); }
  .dock-button { color: #4a7564; }
  .terminate-button { color: #9d615c; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.7; }
  .hub-tabs { min-height: 29px; padding: 4px 9px 0; display: flex; gap: 3px; border-bottom: 1px solid rgba(97, 119, 109, 0.09); }
  .hub-tabs button { min-width: 0; padding: 0 7px 4px; border: 0; border-bottom: 2px solid transparent; color: #8b9791; background: transparent; font: 700 8px Inter, sans-serif; cursor: pointer; }
  .hub-tabs button.active { color: #39785d; border-bottom-color: #3b9c70; }
  .hub-tabs span { min-width: 14px; height: 14px; padding: 0 4px; display: inline-grid; place-items: center; border-radius: 999px; color: #72827a; background: rgba(76, 101, 90, 0.075); font-size: 7px; }
  .terminal-output { min-height: 0; flex: 1; padding: 10px 12px 7px; overflow-y: auto; color: #55635d; background: linear-gradient(180deg, rgba(61, 87, 75, 0.025), transparent); font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace; font-size: var(--chat-font-size); }
  .terminal-output p { margin: 0 0 6px; line-height: 1.45; }
  .terminal-output p > span { color: #36a269; font-weight: 800; }
  .terminal-output i { color: #8a9690; font-style: normal; }
  .status-running, .status-running span { color: #4e7faf; }
  .status-permission_required, .status-permission_required span { color: #b06b25; }
  .status-waiting_for_input, .status-waiting_for_input span { color: #b0812d; }
  .status-completed, .status-completed span { color: #55a473; }
  .status-failed, .status-failed span { color: #ad4f4f; }
  .chat-feed { margin: 9px 0 7px; display: grid; gap: 10px; }
  .chat-turn { padding-bottom: 10px; display: grid; gap: 6px; border-bottom: 1px solid rgba(81, 105, 94, 0.09); }
  .chat-message { max-width: 94%; padding: 7px 8px; border: 1px solid rgba(77, 104, 91, 0.09); border-radius: 9px; background: rgba(69, 99, 84, 0.035); }
  .chat-message.user-message { margin-left: auto; border-bottom-right-radius: 3px; background: rgba(50, 145, 99, 0.075); }
  .chat-message.agent-message { margin-right: auto; border-bottom-left-radius: 3px; }
  .chat-message header { display: flex; align-items: center; gap: 6px; }
  .chat-message header strong { min-width: 0; flex: 1; color: #4f685c; font: 750 var(--chat-small-font-size) Inter, sans-serif; }
  .chat-message header time { color: #9aa59f; font-size: var(--chat-tiny-font-size); }
  .chat-message pre { margin: 5px 0 0; color: #4b5c54; font: var(--chat-font-size)/1.5 "SFMono-Regular", Consolas, "Liberation Mono", monospace; overflow-wrap: anywhere; white-space: pre-wrap; }
  .turn-trace { padding: 4px 6px; border-radius: 6px; background: rgba(72, 101, 88, 0.03); }
  .turn-trace summary { display: flex; align-items: center; gap: 5px; color: #71817a; font: 700 var(--chat-tiny-font-size) Inter, sans-serif; cursor: pointer; }
  .turn-trace summary > span { width: 14px; color: #4f806a; text-align: center; }
  .turn-trace summary time { margin-left: auto; color: #a0aaa5; font-weight: 500; }
  .turn-trace pre { max-height: 180px; margin: 5px 0 0 19px; overflow: auto; color: #5c6b64; font: var(--chat-small-font-size)/1.5 "SFMono-Regular", Consolas, "Liberation Mono", monospace; overflow-wrap: anywhere; white-space: pre-wrap; }
  .turn-files { padding: 7px 8px; border-left: 2px solid #4b9b73; border-radius: 0 7px 7px 0; background: rgba(55, 142, 98, 0.045); }
  .turn-files > strong { display: block; margin-bottom: 5px; color: #4f775f; font: 750 var(--chat-tiny-font-size) Inter, sans-serif; text-transform: uppercase; }
  .turn-files > div { display: grid; gap: 3px; }
  .turn-files code { display: flex; gap: 5px; color: #496258; font-size: var(--chat-small-font-size); overflow-wrap: anywhere; white-space: normal; }
  .turn-files code span { color: #45906a; }
  .privacy-note { padding-top: 6px; border-top: 1px solid rgba(81, 105, 94, 0.08); color: #9aa49f; font: var(--chat-tiny-font-size)/1.45 Inter, sans-serif; }
  .empty-state { margin: 7px 0 10px !important; color: #909c96; font: var(--chat-small-font-size)/1.5 Inter, sans-serif; }
  .changes-panel { margin-top: 9px; display: grid; gap: 7px; }
  .changes-panel > strong { color: #6a7c73; font: 760 var(--chat-small-font-size) Inter, sans-serif; letter-spacing: 0.04em; text-transform: uppercase; }
  .change-list { display: grid; gap: 4px; }
  .change-list code { padding: 5px 6px; display: flex; align-items: flex-start; gap: 6px; border-radius: 6px; color: #4f6158; background: rgba(70, 101, 86, 0.045); font-size: var(--chat-small-font-size); overflow-wrap: anywhere; white-space: normal; }
  .change-list span { color: #45906a; }
  .change-diff { padding: 5px 6px; border: 1px solid rgba(78, 104, 92, 0.09); border-radius: 7px; }
  .change-diff summary { color: #53685e; font: 720 var(--chat-small-font-size) Inter, sans-serif; cursor: pointer; }
  .change-diff pre { max-height: 240px; margin: 6px 0 0; overflow: auto; color: #5b6b63; font: var(--chat-tiny-font-size)/1.45 "SFMono-Regular", Consolas, "Liberation Mono", monospace; white-space: pre; }
  .permission { margin: 7px 0 2px; padding-left: 9px; display: grid; gap: 6px; border-left: 2px solid #c87d32; }
  .permission strong { color: #5a4633; font: 700 var(--chat-font-size)/1.35 Inter, sans-serif; }
  .permission code { padding: 5px 6px; overflow: hidden; border-radius: 6px; color: #5f6b66; background: rgba(74, 99, 88, 0.055); font-size: var(--chat-small-font-size); text-overflow: ellipsis; white-space: nowrap; }
  .permission > div { display: flex; gap: 4px; }
  .permission button { min-height: 23px; padding: 0 7px; border: 1px solid rgba(82, 101, 93, 0.15); border-radius: 6px; color: #4b5d55; background: rgba(255, 255, 255, 0.58); font: 700 8px Inter, sans-serif; cursor: pointer; }
  .permission button.danger { color: #a64d4d; }
  .hint { color: #89948f; font-size: var(--chat-small-font-size); }
  .hint.docked { color: #4f7566; }
  .terminate-confirm { margin: 8px 0 2px; padding: 7px 8px; display: flex; align-items: center; gap: 6px; border: 1px solid rgba(166, 77, 77, 0.14); border-radius: 8px; background: rgba(166, 77, 77, 0.04); font: 700 8px/1.35 Inter, sans-serif; }
  .terminate-confirm > span { min-width: 0; flex: 1; color: #7d5d58; }
  .terminate-confirm > div { display: flex; gap: 4px; }
  .terminate-confirm button { min-height: 22px; padding: 0 6px; border: 1px solid rgba(84, 101, 93, 0.14); border-radius: 6px; color: #596861; background: rgba(255, 255, 255, 0.48); font: 700 7px Inter, sans-serif; cursor: pointer; }
  .terminate-confirm button.danger { color: #a54c4c; border-color: rgba(166, 77, 77, 0.2); }
  .terminal-composer { min-height: 63px; padding: 7px 8px 8px 10px; display: flex; align-items: flex-end; gap: 6px; border-top: 1px solid rgba(97, 119, 109, 0.11); }
  textarea { min-width: 0; height: 46px; flex: 1; padding: 7px 8px; resize: none; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 9px; outline: none; color: #34443d; background: rgba(255, 255, 255, 0.5); font: var(--chat-font-size)/1.4 Inter, sans-serif; }
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
  .terminal-window.dark .terminal-card { color: #dbe7e1; border-color: rgba(190, 209, 200, 0.13); background: #141d19; }
  .terminal-window.dark .terminal-card.dock-moving,
  .terminal-window.dark .terminal-card.dock-target,
  .terminal-window.dark .terminal-card.settling { border-color: rgba(91, 186, 143, 0.5); box-shadow: inset 0 0 0 2px rgba(91, 186, 143, 0.1), inset 0 -10px 24px rgba(8, 21, 15, 0.18); }
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
  .terminal-window.dark .hub-tabs { border-color: rgba(190, 209, 200, 0.07); }
  .terminal-window.dark .hub-tabs button { color: #84938c; }
  .terminal-window.dark .hub-tabs button.active { color: #83c6a6; border-bottom-color: #59ad84; }
  .terminal-window.dark .hub-tabs span { background: rgba(205, 222, 213, 0.07); }
  .terminal-window.dark .terminal-output { color: #b8c6bf; background: linear-gradient(180deg, rgba(114, 151, 134, 0.035), transparent); }
  .terminal-window.dark .chat-turn { border-color: rgba(205, 222, 213, 0.07); }
  .terminal-window.dark .chat-message { border-color: rgba(205, 222, 213, 0.08); background: rgba(218, 234, 226, 0.035); }
  .terminal-window.dark .chat-message.user-message { background: rgba(76, 169, 124, 0.09); }
  .terminal-window.dark .chat-message header strong,
  .terminal-window.dark .chat-message pre,
  .terminal-window.dark .turn-trace pre,
  .terminal-window.dark .turn-files code { color: #bdcbc4; }
  .terminal-window.dark .turn-trace { background: rgba(218, 234, 226, 0.025); }
  .terminal-window.dark .turn-files { border-color: #5bad83; background: rgba(91, 177, 137, 0.055); }
  .terminal-window.dark .turn-files > strong { color: #8bc6a8; }
  .terminal-window.dark .change-diff summary,
  .terminal-window.dark .change-diff pre,
  .terminal-window.dark .change-list code { color: #bdcbc4; }
  .terminal-window.dark .privacy-note { border-color: rgba(205, 222, 213, 0.07); color: #78877f; }
  .terminal-window.dark textarea { color: #d0ddd6; border-color: rgba(205, 222, 213, 0.12); background: rgba(220, 234, 227, 0.045); }
  .terminal-window.dark .permission strong { color: #dfc6ac; }
  .terminal-window.dark .permission code,
  .terminal-window.dark .permission button { color: #bdcbc4; background: rgba(218, 232, 225, 0.055); }
  @media (prefers-reduced-motion: reduce) {
    .terminal-card { transition-duration: 0.01ms; }
    .dock-silhouette { animation: none; }
  }
</style>
