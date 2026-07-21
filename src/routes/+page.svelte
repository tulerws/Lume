<script lang="ts">
  import { onMount, tick } from "svelte";
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import { fade, fly, slide } from "svelte/transition";
  import { getVersion } from "@tauri-apps/api/app";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import {
    availableMonitors,
    getCurrentWindow,
    LogicalSize,
    primaryMonitor,
  } from "@tauri-apps/api/window";
  import BrandIcon from "$lib/BrandIcon.svelte";
  import LumeLogo from "$lib/LumeLogo.svelte";
  import TerminalWindow from "$lib/TerminalWindow.svelte";
  import type {
    AgentKind,
    AgentSession,
    CompanionStatus,
    HistoryEntry,
    IntegrationStatus,
    PermissionAction,
    Preferences,
    SessionStatus,
    TerminalWindowState,
  } from "$lib/domain";
  import { demoSessions } from "$lib/demo";
  import {
    configureIntegration,
    configureVscode,
    decidePermission,
    defaultPreferences,
    loadHistory,
    loadIntegrationStatuses,
    loadPreferences,
    loadSessions,
    loadTerminalWindows,
    openSessionSource,
    openTerminalWindow,
    loadVscodeStatus,
    moveOverlay,
    revealBrowserCompanion,
    launchAgentSession,
    savePreferences,
    submitPrompt,
  } from "$lib/lume";

  type View = "sessions" | "board" | "history" | "settings";
  type ShellStatus = SessionStatus | "idle";
  type MonitorOption = { id: string; label: string };
  type UpdateState =
    | "idle"
    | "checking"
    | "up_to_date"
    | "available"
    | "downloading"
    | "ready"
    | "error";

  const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const currentWindowLabel = isTauri ? getCurrentWindow().label : "main";
  const isTerminalWindow = currentWindowLabel.startsWith("terminal-");
  const compactSize = { width: 78, height: 46 };
  const expandedWidth = 392;
  const expandedMaxHeight = 560;

  let expanded = $state(!isTauri);
  let contentVisible = $state(!isTauri);
  let morphing = $state<"opening" | "closing" | null>(null);
  let morphProgress = $state(isTauri ? 0 : 1);
  let expandedHeight = $state(expandedMaxHeight);
  let view = $state<View>("sessions");
  let sessions = $state<AgentSession[]>(isTauri ? [] : structuredClone(demoSessions));
  let history = $state<HistoryEntry[]>([]);
  let preferences = $state<Preferences>({ ...defaultPreferences });
  let monitors = $state<MonitorOption[]>([]);
  let integrations = $state<IntegrationStatus[]>([]);
  let vscodeStatus = $state<CompanionStatus>({
    installed: false,
    configured: false,
    detail: "Verificando…",
  });
  let selectedId = $state<string | null>(null);
  let permissionError = $state<string | null>(null);
  let savingSettings = $state(false);
  let configuringIntegration = $state<IntegrationStatus["kind"] | null>(null);
  let configuringVscode = $state(false);
  let launcherOpen = $state(false);
  let launching = $state<IntegrationStatus["kind"] | null>(null);
  let launchError = $state<string | null>(null);
  let browserCompanionPath = $state<string | null>(null);
  let settingsMessage = $state<string | null>(null);
  let settingsMessageIsError = $state(false);
  let composerSessionId = $state<string | null>(null);
  let composerPrompt = $state("");
  let composerMessage = $state<string | null>(null);
  let composerSending = $state(false);
  let terminalWindows = $state<TerminalWindowState[]>([]);
  let openingTerminal = $state<string | null>(null);
  let terminalMessage = $state<string | null>(null);
  let overlayPosition = $state({ x: 0, y: 12 });
  let overlayReady = $state(false);
  let monitorBounds = $state({ width: 1920, height: 1080, scale: 1 });
  let dragging = $state(false);
  let appVersion = $state("0.3.0");
  let updateState = $state<UpdateState>("idle");
  let availableVersion = $state<string | null>(null);
  let updateDetail = $state("As atualizações são verificadas automaticamente.");
  let updateProgress = $state<number | null>(null);
  let pendingUpdate: Update | null = null;
  let suppressCompactToggle = false;
  let dragState: {
    pointerId: number;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
  } | null = null;
  let moveFrame: number | null = null;

  function currentExpandedSize() {
    return { width: expandedWidth, height: expandedHeight };
  }

  function observePanelSize(node: HTMLElement) {
    let resizeFrame: number | null = null;

    const syncHeight = (resizeWindow: boolean) => {
      const nextHeight = Math.min(
        expandedMaxHeight,
        Math.max(compactSize.height, Math.ceil(node.offsetHeight + 16)),
      );
      if (nextHeight === expandedHeight) return;
      expandedHeight = nextHeight;
      if (resizeWindow && isTauri && expanded && !morphing) {
        void getCurrentWindow()
          .setSize(new LogicalSize(expandedWidth, nextHeight))
          .catch(() => undefined);
      }
    };

    const observer = new ResizeObserver(() => {
      if (!expanded || morphing) return;
      if (resizeFrame !== null) cancelAnimationFrame(resizeFrame);
      resizeFrame = requestAnimationFrame(() => {
        resizeFrame = null;
        syncHeight(true);
      });
    });

    observer.observe(node);
    syncHeight(false);

    return {
      destroy() {
        observer.disconnect();
        if (resizeFrame !== null) cancelAnimationFrame(resizeFrame);
      },
    };
  }

  const activeCount = $derived(
    sessions.filter((session) =>
      ["running", "permission_required", "waiting_for_input"].includes(session.status),
    ).length,
  );

  const shellStatus = $derived.by<ShellStatus>(() => {
    if (sessions.length === 0) return "idle";
    if (sessions.some((session) => session.status === "permission_required")) {
      return "permission_required";
    }
    if (sessions.some((session) => session.status === "failed")) return "failed";
    if (sessions.some((session) => session.status === "running")) return "running";
    if (sessions.some((session) => session.status === "waiting_for_input")) {
      return "waiting_for_input";
    }
    return "completed";
  });

  onMount(() => {
    if (isTerminalWindow) return;
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let stopTerminalListening: (() => void) | undefined;
    let pollTimer: ReturnType<typeof setInterval> | undefined;
    let updateTimer: ReturnType<typeof setInterval> | undefined;

    void initializeUpdater();
    updateTimer = setInterval(() => void checkForUpdates(), 6 * 60 * 60 * 1_000);

    void (async () => {
      const [nextSessions, nextPreferences, nextIntegrations, nextVscodeStatus] = await Promise.all([
        loadSessions(),
        loadPreferences(),
        loadIntegrationStatuses(),
        loadVscodeStatus(),
      ]);
      if (disposed) return;
      sessions = nextSessions;
      preferences = nextPreferences;
      integrations = nextIntegrations;
      vscodeStatus = nextVscodeStatus;
      selectedId =
        sessions.find((session) => session.status === "permission_required")?.id ?? null;
      await loadMonitorOptions();
      await positionWindow();

      if (isTauri) {
        stopListening = await listen("lume://sessions-changed", () => {
          void refreshSessions(true);
        });
        stopTerminalListening = await listen("lume://terminal-windows-changed", () => {
          void refreshTerminalWindows();
        });
        pollTimer = setInterval(() => void refreshSessions(false), 5_000);
      }
    })();

    return () => {
      disposed = true;
      stopListening?.();
      stopTerminalListening?.();
      if (pollTimer) clearInterval(pollTimer);
      if (updateTimer) clearInterval(updateTimer);
      if (pendingUpdate) void pendingUpdate.close();
    };
  });

  async function initializeUpdater() {
    if (!isTauri) {
      updateState = "up_to_date";
      return;
    }

    try {
      appVersion = await getVersion();
    } catch {
      // Mantém a versão do pacote como fallback.
    }
    await checkForUpdates();
  }

  async function checkForUpdates() {
    if (
      !isTauri ||
      updateState === "checking" ||
      updateState === "available" ||
      updateState === "downloading" ||
      updateState === "ready"
    ) return;
    updateState = "checking";
    updateDetail = "Procurando uma nova versão…";
    updateProgress = null;

    try {
      const nextUpdate = await check({ timeout: 15_000 });
      if (pendingUpdate && pendingUpdate !== nextUpdate) await pendingUpdate.close();
      pendingUpdate = nextUpdate;
      availableVersion = nextUpdate?.version ?? null;
      if (nextUpdate) {
        updateState = "available";
        updateDetail = `A versão ${nextUpdate.version} está pronta para baixar.`;
      } else {
        updateState = "up_to_date";
        updateDetail = "Você está usando a versão mais recente.";
      }
    } catch {
      updateState = "error";
      updateDetail = "Não foi possível verificar agora. Tente novamente em instantes.";
    }
  }

  async function installAvailableUpdate() {
    if (!pendingUpdate || updateState === "downloading") return;
    updateState = "downloading";
    updateDetail = "Baixando e preparando a atualização…";
    updateProgress = 0;
    let downloaded = 0;
    let total: number | undefined;

    try {
      await pendingUpdate.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength;
          return;
        }
        if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          updateProgress = total ? Math.min(99, Math.round((downloaded / total) * 100)) : null;
          return;
        }
        updateProgress = 100;
      });
      updateState = "ready";
      updateDetail = "Atualização instalada. Reiniciando o Lume…";
      await relaunch();
    } catch {
      updateState = "error";
      updateDetail = "A atualização não pôde ser instalada. Tente novamente.";
      updateProgress = null;
    }
  }

  async function refreshSessions(withSound: boolean) {
    const next = await loadSessions();
    if (withSound && preferences.soundEnabled) {
      const previous = new Map(sessions.map((session) => [session.id, session.status]));
      for (const session of next) {
        if (previous.get(session.id) === session.status) continue;
        if (session.status === "completed") playTone("completed");
        if (session.status === "failed") playTone("failed");
      }
    }
    sessions = next;
  }

  async function loadMonitorOptions() {
    if (!isTauri) return;
    try {
      const found = await availableMonitors();
      monitors = found.map((monitor, index) => ({
        id: monitor.name ?? `monitor-${index}`,
        label: monitor.name ?? `Monitor ${index + 1}`,
      }));
    } catch {
      monitors = [];
    }
  }

  async function positionWindow(resetPosition = false) {
    if (!isTauri) return;
    try {
      const currentWindow = getCurrentWindow();
      const target = expanded ? currentExpandedSize() : compactSize;
      await currentWindow.setSize(new LogicalSize(target.width, target.height));

      const found = await availableMonitors();
      const configured = preferences.monitorId
        ? found.find((monitor, index) =>
            (monitor.name ?? `monitor-${index}`) === preferences.monitorId,
          )
        : undefined;
      const monitor = configured ?? (await primaryMonitor());
      if (!monitor) return;
      const scale = monitor.scaleFactor || 1;
      monitorBounds = { width: monitor.size.width, height: monitor.size.height, scale };
      if (!overlayReady || resetPosition) {
        overlayPosition = {
          x:
            preferences.overlayX ??
            Math.max(0, Math.round((monitor.size.width - target.width * scale) / 2)),
          y:
            preferences.overlayY ??
            (navigator.userAgent.toLowerCase().includes("linux") ? Math.round(44 * scale) : 12),
        };
        overlayReady = true;
      }
      overlayPosition = clampOverlayPosition(overlayPosition.x, overlayPosition.y, target);
      await moveOverlay(overlayPosition.x, overlayPosition.y, false);
    } catch {
      // Alguns compositores Wayland ignoram posicionamento solicitado pelo cliente.
    }
  }

  async function toggleExpanded() {
    if (suppressCompactToggle) {
      suppressCompactToggle = false;
      return;
    }
    if (morphing) return;
    if (!expanded) {
      morphing = "opening";
      expanded = true;
      contentVisible = false;
      await tick();
      await animateWindowSize(true);
      contentVisible = true;
      morphing = null;
      return;
    }
    morphing = "closing";
    contentVisible = false;
    await animateWindowSize(false);
    expanded = false;
    morphing = null;
    selectedId = null;
    view = "sessions";
    launcherOpen = false;
  }

  async function animateWindowSize(opening: boolean) {
    const from = opening ? compactSize : currentExpandedSize();
    const to = opening ? currentExpandedSize() : compactSize;
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const duration = reducedMotion ? 1 : opening ? 360 : 320;
    if (!isTauri) {
      morphProgress = opening ? 1 : 0;
      return;
    }
    const currentWindow = getCurrentWindow();
    const startedAt = performance.now();
    await new Promise<void>((resolve) => {
      const frame = async (now: number) => {
        const linear = Math.min(1, (now - startedAt) / duration);
        const eased = linear < 0.5
          ? 4 * linear * linear * linear
          : 1 - Math.pow(-2 * linear + 2, 3) / 2;
        morphProgress = opening ? eased : 1 - eased;
        if (opening && eased > 0.48) contentVisible = true;
        const width = Math.round(from.width + (to.width - from.width) * eased);
        const height = Math.round(from.height + (to.height - from.height) * eased);
        try {
          await currentWindow.setSize(new LogicalSize(width, height));
        } catch {
          resolve();
          return;
        }
        if (linear < 1) {
          requestAnimationFrame((next) => void frame(next));
        } else {
          resolve();
        }
      };
      requestAnimationFrame((now) => void frame(now));
    });
  }

  function clampOverlayPosition(
    x: number,
    y: number,
    target = expanded ? currentExpandedSize() : compactSize,
  ) {
    return {
      x: Math.max(0, Math.min(x, monitorBounds.width - target.width * monitorBounds.scale)),
      y: Math.max(0, Math.min(y, monitorBounds.height - target.height * monitorBounds.scale)),
    };
  }

  function beginOverlayDrag(event: PointerEvent, compact = false) {
    if (!isTauri || event.button !== 0 || morphing) return;
    if (!compact && (event.target as HTMLElement).closest("button, input, select, textarea")) {
      return;
    }
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    dragState = {
      pointerId: event.pointerId,
      startX: event.screenX,
      startY: event.screenY,
      originX: overlayPosition.x,
      originY: overlayPosition.y,
    };
    dragging = false;
  }

  function moveOverlayDrag(event: PointerEvent) {
    if (!dragState || dragState.pointerId !== event.pointerId) return;
    const dx = event.screenX - dragState.startX;
    const dy = event.screenY - dragState.startY;
    if (!dragging && Math.hypot(dx, dy) < 3) return;
    dragging = true;
    event.preventDefault();
    overlayPosition = clampOverlayPosition(dragState.originX + dx, dragState.originY + dy);
    if (moveFrame !== null) cancelAnimationFrame(moveFrame);
    moveFrame = requestAnimationFrame(() => {
      moveFrame = null;
      void moveOverlay(overlayPosition.x, overlayPosition.y, false);
    });
  }

  async function endOverlayDrag(event: PointerEvent, compact = false) {
    if (!dragState || dragState.pointerId !== event.pointerId) return;
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    dragState = null;
    if (!dragging) return;
    if (compact) suppressCompactToggle = true;
    preferences = {
      ...preferences,
      overlayX: Math.round(overlayPosition.x),
      overlayY: Math.round(overlayPosition.y),
    };
    await moveOverlay(overlayPosition.x, overlayPosition.y, true);
    dragging = false;
  }

  function openSession(session: AgentSession) {
    selectedId = selectedId === session.id ? null : session.id;
    if (selectedId !== session.id) composerSessionId = null;
    permissionError = null;
  }

  function canSubmitToSession(session: AgentSession) {
    return (
      session.source === "web" ||
      (session.agent === "codex" &&
        session.permissionProfile.canRespondFromLume &&
        Boolean(session.nativeSessionId))
    );
  }

  function canContinueSession(session: AgentSession) {
    return ["completed", "failed", "waiting_for_input"].includes(session.status);
  }

  function toggleSessionComposer(session: AgentSession) {
    composerSessionId = composerSessionId === session.id ? null : session.id;
    composerPrompt = "";
    composerMessage = null;
  }

  async function sendSessionPrompt(session: AgentSession) {
    const prompt = composerPrompt.trim();
    if (!prompt || composerSending) return;
    composerSending = true;
    composerMessage = null;
    try {
      if (isTauri) await submitPrompt(session.id, prompt);
      sessions = sessions.map((item) =>
        item.id === session.id
          ? { ...item, status: "running", statusLabel: "Prompt enviado pelo Lume" }
          : item,
      );
      composerPrompt = "";
      composerSessionId = null;
    } catch (error) {
      composerMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      composerSending = false;
    }
  }

  async function refreshTerminalWindows() {
    terminalWindows = await loadTerminalWindows();
  }

  async function showTerminal(session: AgentSession) {
    if (openingTerminal) return;
    openingTerminal = session.id;
    terminalMessage = null;
    try {
      if (isTauri) {
        await openTerminalWindow(session.id);
        await refreshTerminalWindows();
      } else {
        terminalMessage = `O mini terminal de ${session.agentLabel} abre em uma janela separada no app.`;
      }
    } catch (error) {
      terminalMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      openingTerminal = null;
    }
  }

  function terminalIsOpen(session: AgentSession) {
    return terminalWindows.some((terminal) => terminal.sessionId === session.id);
  }

  async function handlePermission(session: AgentSession, action: PermissionAction) {
    const permission = session.pendingPermission;
    if (!permission) return;
    permissionError = null;

    if (action === "open_source") {
      try {
        await openSessionSource(session.id);
      } catch (error) {
        permissionError = String(error).replace(/^Error:\s*/, "");
      }
      return;
    }

    try {
      if (isTauri) {
        await decidePermission(session.id, permission.id, action);
        await refreshSessions(false);
      } else {
        sessions = sessions.map((item) =>
          item.id === session.id
            ? {
                ...item,
                status: "running",
                statusLabel:
                  action === "deny" ? "Permissão recusada" : "Continuando a tarefa",
                pendingPermission: undefined,
              }
            : item,
        );
      }
      selectedId = null;
    } catch (error) {
      permissionError = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function openView(nextView: View) {
    view = nextView;
    selectedId = null;
    permissionError = null;
    launcherOpen = false;
    composerSessionId = null;
    composerMessage = null;
    terminalMessage = null;
    if (nextView === "board") await refreshTerminalWindows();
    if (nextView === "history") history = await loadHistory();
    if (nextView === "settings") {
      settingsMessage = null;
      [integrations, vscodeStatus] = await Promise.all([
        loadIntegrationStatuses(),
        loadVscodeStatus(),
      ]);
    }
  }

  async function toggleLauncher() {
    launcherOpen = !launcherOpen;
    launchError = null;
    if (launcherOpen && integrations.length === 0) {
      integrations = await loadIntegrationStatuses();
    }
  }

  async function startSession(agent: IntegrationStatus["kind"], resume: boolean) {
    if (!isTauri) {
      launcherOpen = false;
      return;
    }
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: resume ? "Projeto da sessão a retomar" : "Projeto da nova sessão",
    });
    if (!selected || Array.isArray(selected)) return;

    launching = agent;
    launchError = null;
    try {
      await launchAgentSession(
        agent,
        selected,
        resume,
        undefined,
        preferences.launchTarget,
      );
      launcherOpen = false;
    } catch (error) {
      launchError = String(error).replace(/^Error:\s*/, "");
    } finally {
      launching = null;
    }
  }

  async function toggleIntegration(integration: IntegrationStatus) {
    if (!integration.installed) return;
    const enabling = !integration.configured;
    configuringIntegration = integration.kind;
    settingsMessage = null;
    try {
      await configureIntegration(integration.kind, enabling);
      integrations = await loadIntegrationStatuses();
      settingsMessageIsError = false;
      settingsMessage = enabling
        ? integration.kind === "codex"
          ? "Codex conectado. Abra /hooks no Codex e confie no hook Lume uma vez."
          : `${integration.label} conectado ao Lume.`
        : `${integration.label} desconectado.`;
    } catch (error) {
      settingsMessageIsError = true;
      settingsMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      configuringIntegration = null;
    }
  }

  async function toggleVscode() {
    if (!vscodeStatus.installed) return;
    const enabling = !vscodeStatus.configured;
    configuringVscode = true;
    settingsMessage = null;
    try {
      await configureVscode(enabling);
      vscodeStatus = await loadVscodeStatus();
      settingsMessageIsError = false;
      settingsMessage = enabling
        ? "Companion instalado no VS Code."
        : "Companion removido do VS Code.";
    } catch (error) {
      settingsMessageIsError = true;
      settingsMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      configuringVscode = false;
    }
  }

  async function openBrowserCompanion() {
    try {
      browserCompanionPath = await revealBrowserCompanion();
    } catch {
      browserCompanionPath = "Não foi possível abrir a pasta da extensão.";
    }
  }

  async function updatePreference<K extends keyof Preferences>(
    key: K,
    value: Preferences[K],
  ) {
    const previous = preferences;
    preferences = { ...preferences, [key]: value };
    savingSettings = true;
    try {
      await savePreferences(preferences);
      if (key === "monitorId") await positionWindow();
    } catch (error) {
      preferences = previous;
      settingsMessageIsError = true;
      settingsMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      savingSettings = false;
    }
  }

  function playTone(kind: "completed" | "failed") {
    try {
      const AudioContextClass = window.AudioContext;
      const context = new AudioContextClass();
      const gain = context.createGain();
      gain.gain.setValueAtTime(0.0001, context.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.045, context.currentTime + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, context.currentTime + 0.42);
      gain.connect(context.destination);

      const notes = kind === "completed" ? [620, 820] : [330, 250];
      notes.forEach((frequency, index) => {
        const oscillator = context.createOscillator();
        oscillator.type = "sine";
        oscillator.frequency.value = frequency;
        oscillator.connect(gain);
        oscillator.start(context.currentTime + index * 0.1);
        oscillator.stop(context.currentTime + 0.3 + index * 0.1);
      });
      setTimeout(() => void context.close(), 600);
    } catch {
      // Áudio é opcional e pode estar bloqueado até a primeira interação.
    }
  }

  function actionLabel(action: PermissionAction) {
    return {
      allow_once: "Permitir uma vez",
      allow_session: "Nesta sessão",
      deny: "Recusar",
      open_source: "Abrir origem",
    }[action];
  }

  function sourceLabel(session: AgentSession) {
    if (session.source === "web") {
      if (session.sourceApp === "chrome") return "Chrome";
      if (session.sourceApp === "edge") return "Edge";
      if (session.sourceApp === "brave") return "Brave";
      return "Web";
    }
    return { cli: "CLI", vscode: "VS Code", desktop: "Desktop" }[session.source];
  }

  function sourceIcon(session: AgentSession) {
    if (session.source === "cli") return "terminal" as const;
    if (session.source === "vscode") return "vscode" as const;
    if (session.source === "web") return session.sourceApp ?? ("browsers" as const);
    return "unknown" as const;
  }

  function relativeTime(timestamp: number) {
    const seconds = Math.max(0, Math.round((Date.now() - timestamp) / 1_000));
    if (seconds < 60) return "agora";
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `há ${minutes} min`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `há ${hours} h`;
    return new Intl.DateTimeFormat("pt-BR", { day: "2-digit", month: "short" }).format(
      timestamp,
    );
  }

  function eventLabel(event: HistoryEntry["event"]) {
    return {
      completed: "Finalizado",
      failed: "Erro",
      permission_allowed: "Permitido",
      permission_denied: "Recusado",
    }[event];
  }
</script>

<svelte:head>
  <title>Lume</title>
  <meta name="description" content="Monitor local e discreto para sessões de agentes de IA." />
</svelte:head>

{#if isTerminalWindow}
  <TerminalWindow />
{:else}
<main
  class:expanded
  class="overlay-shell"
  style={`--panel-gap-right: ${Math.round(8 * morphProgress)}px; --panel-gap-bottom: ${Math.round(16 * morphProgress)}px; --panel-radius: ${Math.round(23 - 2 * morphProgress)}px;`}
  aria-label="Lume, monitor de agentes"
>
  {#if !expanded}
    <button
      class="lume-orb status-{shellStatus}"
      class:dragging
      type="button"
      onclick={toggleExpanded}
      onpointerdown={(event) => beginOverlayDrag(event, true)}
      onpointermove={moveOverlayDrag}
      onpointerup={(event) => endOverlayDrag(event, true)}
      onpointercancel={(event) => endOverlayDrag(event, true)}
      aria-label="Abrir Lume, {activeCount} agentes ativos"
    >
      <span class="lume-brand-icon status-{shellStatus}" aria-hidden="true">
        <LumeLogo size={28} />
        <span class="logo-state">
        {#if shellStatus === "completed"}
          <svg class="status-glyph" viewBox="0 0 20 20"><path d="m5.5 10.2 3 3 6-6.2" /></svg>
        {:else if shellStatus === "failed"}
          <svg class="status-glyph" viewBox="0 0 20 20"><path d="M10 5.2v6.2M10 14.7h.01" /></svg>
        {:else}
          <i></i>
        {/if}
        </span>
      </span>
      <span class="agent-count">{activeCount}</span>
    </button>
  {:else}
    <section use:observePanelSize class:content-visible={contentVisible} class:morphing class="panel">
      <header
        role="banner"
        class:dragging
        class="panel-header"
        onpointerdown={beginOverlayDrag}
        onpointermove={moveOverlayDrag}
        onpointerup={endOverlayDrag}
        onpointercancel={endOverlayDrag}
      >
        {#if view === "sessions"}
          <div class="brand-lockup">
            <LumeLogo size={31} />
            <div>
              <strong>Lume</strong>
              <span>{activeCount === 1 ? "1 sessão ativa" : `${activeCount} sessões ativas`}</span>
            </div>
          </div>
        {:else}
          <button class="back-button" type="button" onclick={() => openView("sessions")}>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m12.5 5-5 5 5 5" /></svg>
            <span>{view === "board" ? "Whiteboard" : view === "history" ? "Histórico" : "Configurações"}</span>
          </button>
        {/if}
        <div class="header-actions">
          {#if view === "sessions"}
            <button class:active={launcherOpen} class="add-button" type="button" onclick={toggleLauncher} aria-label="Abrir ou retomar sessão">
              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 5v10M5 10h10" /></svg>
            </button>
          {/if}
          <button class="collapse-button" type="button" onclick={toggleExpanded} aria-label="Recolher">
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5.5 8 4.5 4 4.5-4" /></svg>
          </button>
        </div>
      </header>

      {#if launcherOpen}
        <div class="launcher-popover" transition:fly={{ y: -5, duration: 170, easing: cubicOut }}>
          <span class="launcher-title">Abrir sessão</span>
          {#each integrations.filter((integration) => integration.installed) as integration}
            <div class="launcher-row">
              <span class="agent-avatar agent-{integration.kind}"><BrandIcon name={integration.kind} size={17} /></span>
              <strong>{integration.label}</strong>
              <button disabled={launching !== null} type="button" onclick={() => startSession(integration.kind, false)}>Nova</button>
              <button disabled={launching !== null} type="button" onclick={() => startSession(integration.kind, true)}>Retomar</button>
            </div>
          {:else}
            <p>Nenhuma CLI compatível foi encontrada.</p>
          {/each}
          {#if launchError}<p class="launcher-error">{launchError}</p>{/if}
        </div>
      {/if}

      <div class="panel-content">
        {#if view === "sessions"}
          <div class="session-list">
            {#each sessions as session (session.id)}
              <article
                animate:flip={{ duration: 220 }}
                class:attention={session.status === "permission_required"}
                class:selected={selectedId === session.id}
                class="session-row"
              >
                <button class="session-summary" type="button" onclick={() => openSession(session)}>
                  <span class="agent-avatar agent-{session.agent}"><BrandIcon name={session.agent} size={20} /></span>
                  <span class="session-copy">
                    <span class="session-title-row">
                      <strong>{session.agentLabel}</strong>
                      <span class="source-label">
                        <BrandIcon name={sourceIcon(session)} size={session.source === "web" ? 11 : 9} />
                        {sourceLabel(session)}
                      </span>
                    </span>
                    <span class="project-name">{session.project}</span>
                    <span class="status-line status-{session.status}">
                      {#if session.status === "running"}
                        <span class="running-dots" aria-hidden="true"><i></i><i></i><i></i></span>
                      {:else}
                        <i></i>
                      {/if}
                      {session.statusLabel}
                    </span>
                  </span>
                  <svg class="chevron" viewBox="0 0 20 20" aria-hidden="true">
                    <path d="m8 5 5 5-5 5" />
                  </svg>
                </button>

                {#if selectedId === session.id}
                  <div class="session-details" transition:slide={{ duration: 190, easing: cubicOut }}>
                    <div class="access-profile">
                      <span>{session.permissionProfile.label}</span>
                      <small>{session.permissionProfile.approvalPolicy}</small>
                    </div>

                    {#if session.pendingPermission}
                      <div class="permission-block risk-{session.pendingPermission.risk}">
                        <span class="eyebrow">Permissão solicitada</span>
                        <strong>{session.pendingPermission.summary}</strong>
                        <code>{session.pendingPermission.resource}</code>
                        <div class="permission-actions">
                          {#each session.permissionProfile.availableActions as action}
                            <button
                              class:primary={action === "allow_once"}
                              class:danger={action === "deny"}
                              type="button"
                              onclick={() => handlePermission(session, action)}
                            >
                              {actionLabel(action)}
                            </button>
                          {/each}
                        </div>
                        {#if permissionError}
                          <p class="inline-error" transition:fade>{permissionError}</p>
                        {/if}
                      </div>
                    {:else if !session.permissionProfile.canRespondFromLume}
                      <p class="integration-note">
                        O Lume acompanha esta origem; as ações continuam nela.
                      </p>
                    {/if}

                    {#if canContinueSession(session) && canSubmitToSession(session)}
                      <button
                        class:active={composerSessionId === session.id}
                        class="continue-trigger"
                        type="button"
                        onclick={() => toggleSessionComposer(session)}
                      >
                        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 10h11M11 6l4 4-4 4" /></svg>
                        Continuar pelo Lume
                      </button>
                      {#if composerSessionId === session.id}
                        <form
                          class="inline-composer"
                          onsubmit={(event) => {
                            event.preventDefault();
                            void sendSessionPrompt(session);
                          }}
                          transition:slide={{ duration: 160, easing: cubicOut }}
                        >
                          <textarea
                            bind:value={composerPrompt}
                            aria-label="Novo prompt para {session.agentLabel}"
                            placeholder="Digite o próximo prompt…"
                            rows="2"
                          ></textarea>
                          <button disabled={!composerPrompt.trim() || composerSending} type="submit" aria-label="Enviar prompt">
                            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m4 10 12-6-4 12-2-4zM10 12l2-2" /></svg>
                          </button>
                        </form>
                        {#if composerMessage}<p class="inline-error">{composerMessage}</p>{/if}
                      {/if}
                    {/if}
                  </div>
                {/if}
              </article>
            {:else}
              <div class="empty-state" transition:fade>
                <span class="quiet-orbit" aria-hidden="true"><i></i></span>
                <strong>Nenhuma sessão ativa</strong>
                <p>Novas sessões aparecerão aqui automaticamente.</p>
              </div>
            {/each}
          </div>
        {:else if view === "board"}
          <div class="whiteboard" in:fade={{ duration: 150 }}>
            <div class="board-intro">
              <span class="eyebrow">Terminais flutuantes</span>
              <strong>Um espaço separado para cada chat</strong>
              <p>Abra mini terminais independentes e aproxime um do outro para acoplá-los.</p>
            </div>

            <div class="terminal-picker">
              {#each sessions as session (session.id)}
                <div class="terminal-picker-row">
                  <span class="agent-avatar agent-{session.agent}"><BrandIcon name={session.agent} size={18} /></span>
                  <span class="terminal-picker-copy">
                    <strong>{session.agentLabel}</strong>
                    <small>{session.project}</small>
                  </span>
                  <span class="source-label">
                    <BrandIcon name={sourceIcon(session)} size={session.source === "web" ? 11 : 9} />
                    {sourceLabel(session)}
                  </span>
                  <button
                    class:opened={terminalIsOpen(session)}
                    disabled={openingTerminal !== null}
                    type="button"
                    onclick={() => showTerminal(session)}
                  >
                    {openingTerminal === session.id ? "Abrindo…" : terminalIsOpen(session) ? "Mostrar" : "Abrir"}
                  </button>
                </div>
              {:else}
                <p class="board-empty">As sessões aparecerão aqui quando forem detectadas.</p>
              {/each}
            </div>
            {#if terminalMessage}<p class="board-message" transition:fade>{terminalMessage}</p>{/if}
            <div class="dock-guide">
              <svg viewBox="0 0 32 20" aria-hidden="true"><rect x="2" y="3" width="12" height="14" rx="3" /><rect x="18" y="3" width="12" height="14" rx="3" /><path d="M14 10h4" /></svg>
              <span>Cada janela tem seu próprio prompt. Acople pelas laterais ou por cima e por baixo.</span>
            </div>
          </div>
        {:else if view === "history"}
          <div class="history-list" in:fade={{ duration: 150 }}>
            {#each history as entry (entry.id)}
              <div class="history-row">
                <span class="history-dot event-{entry.event}" aria-hidden="true"></span>
                <div>
                  <span><strong>{entry.agentLabel}</strong> · {entry.project}</span>
                  <small>{eventLabel(entry.event)} · {relativeTime(entry.createdAt)}</small>
                </div>
              </div>
            {:else}
              <div class="empty-state">
                <strong>Histórico vazio</strong>
                <p>Conclusões, erros e decisões aparecerão aqui.</p>
              </div>
            {/each}
            <p class="privacy-note">Comandos, caminhos e conteúdos de permissões não são guardados.</p>
          </div>
        {:else}
          <div class="settings" in:fade={{ duration: 150 }}>
            <div class="settings-section-label">Agentes</div>
            {#each integrations as integration}
              <div class="integration-row">
                <span class="agent-avatar agent-{integration.kind}"><BrandIcon name={integration.kind} size={18} /></span>
                <div>
                  <strong>{integration.label}</strong>
                  <span>{integration.detail}</span>
                </div>
                <button
                  class:connected={integration.configured}
                  disabled={!integration.installed || configuringIntegration === integration.kind}
                  type="button"
                  onclick={() => toggleIntegration(integration)}
                >
                  {configuringIntegration === integration.kind
                    ? "…"
                    : integration.configured
                      ? "Conectado"
                      : "Conectar"}
                </button>
              </div>
            {/each}
            {#if settingsMessage}
              <p class:error={settingsMessageIsError} class="settings-feedback" transition:fade>
                {settingsMessage}
              </p>
            {/if}
            <div class="settings-section-label preferences-label">Interface</div>
            <div class="integration-row">
              <span class="agent-avatar agent-vscode"><BrandIcon name="vscode" size={19} /></span>
              <div>
                <strong>VS Code Companion</strong>
                <span>{vscodeStatus.detail}</span>
              </div>
              <button
                class:connected={vscodeStatus.configured}
                disabled={!vscodeStatus.installed || configuringVscode}
                type="button"
                onclick={toggleVscode}
              >{configuringVscode ? "…" : vscodeStatus.configured ? "Conectado" : "Conectar"}</button>
            </div>
            <div class="integration-row browser-row">
              <span class="agent-avatar agent-browser"><BrandIcon name="browsers" size={21} /></span>
              <div>
                <strong>Chrome, Edge e Brave</strong>
                <span>Carregue a pasta como extensão descompactada.</span>
              </div>
              <button type="button" onclick={openBrowserCompanion}>Abrir pasta</button>
            </div>
            {#if browserCompanionPath}
              <p class="browser-path" transition:fade>{browserCompanionPath}</p>
            {/if}
            <div class="settings-section-label preferences-label">Preferências</div>
            <div class="setting-row">
              <div><strong>Iniciar com o sistema</strong><span>Lume fica disponível na bandeja.</span></div>
              <label class="switch">
                <input
                  type="checkbox"
                  checked={preferences.autostart}
                  onchange={(event) =>
                    updatePreference("autostart", event.currentTarget.checked)}
                />
                <span></span>
              </label>
            </div>
            <div class="setting-row">
              <div><strong>Sons sutis</strong><span>Apenas ao finalizar ou encontrar erro.</span></div>
              <label class="switch">
                <input
                  type="checkbox"
                  checked={preferences.soundEnabled}
                  onchange={(event) =>
                    updatePreference("soundEnabled", event.currentTarget.checked)}
                />
                <span></span>
              </label>
            </div>
            <div class="setting-row">
              <div><strong>Sobre tela cheia</strong><span>Desativado evita vídeos e jogos.</span></div>
              <label class="switch">
                <input
                  type="checkbox"
                  checked={preferences.showOverFullscreen}
                  onchange={(event) =>
                    updatePreference("showOverFullscreen", event.currentTarget.checked)}
                />
                <span></span>
              </label>
            </div>
            <label class="field-row">
              <span><strong>Monitor</strong><small>O principal é usado por padrão.</small></span>
              <select
                value={preferences.monitorId ?? ""}
                onchange={(event) =>
                  updatePreference("monitorId", event.currentTarget.value || undefined)}
              >
                <option value="">Principal</option>
                {#each monitors as monitor}
                  <option value={monitor.id}>{monitor.label}</option>
                {/each}
              </select>
            </label>
            <label class="field-row">
              <span><strong>Histórico</strong><small>Resumos locais e sanitizados.</small></span>
              <select
                value={preferences.historyRetentionDays}
                onchange={(event) =>
                  updatePreference(
                    "historyRetentionDays",
                    Number(event.currentTarget.value),
                  )}
              >
                <option value={7}>7 dias</option>
                <option value={30}>30 dias</option>
                <option value={90}>90 dias</option>
              </select>
            </label>
            <div class="launch-setting">
              <span><strong>Abrir sessões em</strong><small>Use sua ferramenta habitual.</small></span>
              <div class="segmented" aria-label="Destino das sessões">
                {#each [["auto", "Auto"], ["terminal", "Terminal"], ["vscode", "VS Code"]] as option}
                  <button
                    class:active={preferences.launchTarget === option[0]}
                    type="button"
                    onclick={() =>
                      updatePreference("launchTarget", option[0] as Preferences["launchTarget"])}
                  >{option[1]}</button>
                {/each}
              </div>
            </div>
            <div class="settings-section-label preferences-label">Sobre</div>
            <div class="update-card" aria-live="polite">
              <div class="update-main">
                <LumeLogo size={30} />
                <div class="update-copy">
                  <strong>Lume</strong>
                  <span>Versão {appVersion}</span>
                </div>
                {#if updateState === "available"}
                  <button class="update-available" type="button" onclick={installAvailableUpdate}>
                    Atualizar para {availableVersion}
                  </button>
                {:else}
                  <button
                    type="button"
                    disabled={updateState === "checking" || updateState === "downloading" || updateState === "ready"}
                    onclick={checkForUpdates}
                  >
                    {updateState === "checking"
                      ? "Verificando…"
                      : updateState === "downloading"
                        ? updateProgress === null
                          ? "Baixando…"
                          : `${updateProgress}%`
                        : updateState === "ready"
                          ? "Reiniciando…"
                          : "Verificar"}
                  </button>
                {/if}
              </div>
              <p class:error={updateState === "error"}>{updateDetail}</p>
              {#if updateState === "downloading" || updateState === "ready"}
                <div class:indeterminate={updateProgress === null} class="update-progress" aria-hidden="true">
                  <span style:width={`${updateProgress ?? 24}%`}></span>
                </div>
              {/if}
            </div>
            <span class:visible={savingSettings} class="save-state">Salvando…</span>
          </div>
        {/if}
      </div>

      <footer>
        <button
          class:active={view === "sessions"}
          type="button"
          onclick={() => openView("sessions")}
          aria-label="Sessões"
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <circle cx="6" cy="10" r="2.5" /><circle cx="14" cy="10" r="2.5" />
          </svg>
          <span>Sessões</span>
        </button>
        <button
          class:active={view === "board"}
          type="button"
          onclick={() => openView("board")}
          aria-label="Whiteboard"
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <circle cx="5" cy="6" r="2" /><circle cx="15" cy="6" r="2" /><circle cx="10" cy="15" r="2" />
            <path d="m6.7 7 2.2 6M13.3 7l-2.2 6M7 6h6" />
          </svg>
          <span>Mesa</span>
        </button>
        <button
          class:active={view === "history"}
          type="button"
          onclick={() => openView("history")}
          aria-label="Histórico"
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M4.5 5.5h11M4.5 10h11M4.5 14.5h7" />
          </svg>
          <span>Histórico</span>
        </button>
        <button
          class:active={view === "settings"}
          class:has-update={updateState === "available"}
          type="button"
          onclick={() => openView("settings")}
          aria-label="Configurações"
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <circle cx="10" cy="10" r="3" />
            <path d="M10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M15.3 4.7l-1.4 1.4M6.1 13.9l-1.4 1.4" />
          </svg>
          <span>Ajustes</span>
        </button>
      </footer>
    </section>
  {/if}
</main>
{/if}

<style>
  .overlay-shell {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: flex-start;
    justify-content: flex-start;
  }

  .overlay-shell.expanded {
    padding: 0 var(--panel-gap-right) var(--panel-gap-bottom) 0;
  }

  button,
  select,
  textarea {
    -webkit-tap-highlight-color: transparent;
  }

  .lume-orb {
    width: 78px;
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: 1px solid rgba(103, 122, 114, 0.2);
    border-radius: 999px;
    color: #4e7567;
    background: rgba(249, 251, 250, 0.94);
    box-shadow: 0 7px 26px rgba(28, 43, 37, 0.17);
    backdrop-filter: blur(22px) saturate(120%);
    cursor: pointer;
    touch-action: none;
    transition: transform 160ms ease, box-shadow 160ms ease;
  }

  .lume-orb:hover {
    transform: translateY(1px) scale(1.02);
    box-shadow: 0 9px 30px rgba(28, 43, 37, 0.2);
  }

  .lume-orb:active { transform: scale(0.97); }
  .lume-orb.dragging { cursor: grabbing; transform: scale(0.985); }

  .lume-brand-icon { position: relative; width: 28px; height: 28px; flex: 0 0 auto; }
  .logo-state { position: absolute; right: -3px; bottom: -2px; width: 12px; height: 12px; display: grid; place-items: center; border: 2px solid rgba(249, 251, 250, 0.96); border-radius: 50%; color: white; background: currentColor; }
  .logo-state i { width: 4px; height: 4px; border-radius: 50%; background: white; }
  .logo-state .status-glyph { width: 9px; height: 9px; stroke: white; stroke-width: 2.4; }
  .status-permission_required .logo-state { background: #ae6b24; }
  .status-failed .logo-state { background: #a84d4d; }
  .status-completed .logo-state { background: #678476; }
  .status-running .logo-state { background: #4f7f6c; }
  .status-waiting_for_input .logo-state { background: #627f9d; }
  .status-idle .logo-state { background: #829089; }
  .status-permission_required { color: #ae6b24; }
  .status-failed { color: #a84d4d; }
  .status-completed { color: #708079; }
  .status-idle { color: #829089; }

  .status-permission_required.lume-orb .lume-brand-icon {
    animation: attention 1.8s ease-in-out infinite;
  }

  @keyframes attention {
    50% { box-shadow: 0 0 0 5px rgba(183, 111, 36, 0.12); }
  }

  .agent-count {
    min-width: 19px;
    height: 19px;
    padding: 0 5px;
    display: grid;
    place-items: center;
    border-radius: 999px;
    color: #f8faf9;
    background: #30473e;
    font-size: 10px;
    font-weight: 760;
  }

  .panel {
    position: relative;
    width: 100%;
    height: auto;
    max-height: 544px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid rgba(105, 124, 116, 0.18);
    border-radius: var(--panel-radius);
    color: #26322e;
    background: rgba(249, 251, 250, 0.965);
    backdrop-filter: blur(28px) saturate(125%);
  }

  .panel-content,
  .panel footer,
  .panel .brand-lockup > div,
  .panel .header-actions,
  .panel .back-button,
  .panel .launcher-popover {
    transition: opacity 150ms ease, transform 190ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .panel:not(.content-visible) .panel-content,
  .panel:not(.content-visible) footer,
  .panel:not(.content-visible) .brand-lockup > div,
  .panel:not(.content-visible) .header-actions,
  .panel:not(.content-visible) .back-button,
  .panel:not(.content-visible) .launcher-popover {
    opacity: 0;
    pointer-events: none;
    transform: translateY(-4px) scale(0.985);
  }

  .panel-header {
    flex: 0 0 auto;
    min-height: 61px;
    padding: 12px 13px 10px 16px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid rgba(101, 120, 112, 0.11);
    cursor: grab;
    touch-action: none;
  }

  .panel-header.dragging { cursor: grabbing; }

  .brand-lockup { display: flex; align-items: center; gap: 10px; color: #4e7567; }
  .brand-lockup div { display: grid; gap: 1px; }
  .brand-lockup strong { color: #202d28; font-size: 13px; letter-spacing: -0.01em; }
  .brand-lockup div span { color: #75817c; font-size: 10px; }

  .back-button,
  .add-button,
  .collapse-button {
    border: 0;
    color: #697872;
    background: transparent;
    cursor: pointer;
  }

  .back-button {
    height: 32px;
    padding: 0 6px 0 2px;
    display: flex;
    align-items: center;
    gap: 5px;
    border-radius: 9px;
    color: #2e3c36;
    font-size: 12px;
    font-weight: 680;
  }

  .collapse-button {
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border-radius: 10px;
  }

  .header-actions { display: flex; align-items: center; gap: 2px; }
  .add-button { width: 32px; height: 32px; display: grid; place-items: center; border-radius: 10px; }
  .add-button:hover,
  .add-button.active { color: #486d5e; background: rgba(80, 103, 94, 0.07); }

  .back-button:hover,
  .add-button:hover,
  .collapse-button:hover { background: rgba(80, 103, 94, 0.07); }

  svg {
    width: 17px;
    height: 17px;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-width: 1.65;
  }

  .panel-content { position: relative; max-height: 431px; min-height: 0; flex: 0 1 auto; overflow: hidden; }
  .launcher-popover { position: absolute; z-index: 4; top: 53px; right: 13px; width: 250px; padding: 10px 11px; border: 1px solid rgba(99, 119, 110, 0.14); border-radius: 14px; background: rgba(250, 252, 251, 0.985); box-shadow: 0 14px 38px rgba(27, 42, 35, 0.18); backdrop-filter: blur(22px); }
  .launcher-title { display: block; padding: 1px 3px 7px; color: #8c9691; font-size: 9px; font-weight: 750; letter-spacing: 0.06em; text-transform: uppercase; }
  .launcher-row { min-height: 45px; display: flex; align-items: center; gap: 7px; border-top: 1px solid rgba(105, 123, 115, 0.08); }
  .launcher-row .agent-avatar { width: 25px; height: 25px; border-radius: 8px; font-size: 9px; }
  .launcher-row strong { min-width: 0; flex: 1; color: #35423d; font-size: 10px; }
  .launcher-row button { height: 25px; padding: 0 7px; border: 0; border-radius: 7px; color: #60736a; background: rgba(78, 105, 93, 0.055); font-size: 9px; font-weight: 700; cursor: pointer; }
  .launcher-row button:hover { background: rgba(78, 105, 93, 0.1); }
  .launcher-row button:disabled { opacity: 0.45; }
  .launcher-popover > p { margin: 8px 3px; color: #89938f; font-size: 10px; }
  .launcher-popover .launcher-error { color: #a54c4c; }
  .session-list,
  .history-list,
  .settings { max-height: 431px; min-height: 0; overflow-x: hidden; overflow-y: auto; overscroll-behavior: contain; scrollbar-gutter: stable; scrollbar-width: thin; scrollbar-color: #cad2ce transparent; }

  .session-list::-webkit-scrollbar,
  .history-list::-webkit-scrollbar,
  .settings::-webkit-scrollbar,
  .terminal-picker::-webkit-scrollbar { width: 5px; background: transparent; }
  .session-list::-webkit-scrollbar-button,
  .history-list::-webkit-scrollbar-button,
  .settings::-webkit-scrollbar-button,
  .terminal-picker::-webkit-scrollbar-button { width: 0; height: 0; display: none; }
  .session-list::-webkit-scrollbar-track,
  .history-list::-webkit-scrollbar-track,
  .settings::-webkit-scrollbar-track,
  .terminal-picker::-webkit-scrollbar-track { background: transparent; }
  .session-list::-webkit-scrollbar-thumb,
  .history-list::-webkit-scrollbar-thumb,
  .settings::-webkit-scrollbar-thumb,
  .terminal-picker::-webkit-scrollbar-thumb { border-radius: 999px; background: #cad2ce; }

  .session-list { padding: 5px 14px 8px; }

  .session-row {
    border-bottom: 1px solid rgba(105, 123, 115, 0.1);
    transition: background 160ms ease;
  }

  .session-row:last-child { border-bottom: 0; }
  .session-row:hover,
  .session-row.selected { margin: 0 -6px; padding: 0 6px; border-radius: 12px; background: rgba(76, 104, 92, 0.045); }
  .session-row.attention { background: linear-gradient(90deg, rgba(183, 111, 36, 0.07), transparent 75%); }

  .session-summary {
    width: 100%;
    min-height: 76px;
    padding: 10px 1px;
    display: flex;
    align-items: center;
    gap: 11px;
    border: 0;
    color: inherit;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .agent-avatar {
    width: 32px;
    height: 32px;
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    border-radius: 10px;
    font-size: 11px;
    font-weight: 780;
    transition: transform 160ms ease;
  }

  .session-summary:hover .agent-avatar { transform: scale(1.04); }
  .agent-codex { color: #202523; background: #edf0ee; }
  .agent-claude { color: #d97757; background: #f7ece6; }
  .agent-gemini { color: #6e73ca; background: #eef0fb; }
  .agent-vscode { color: #287aa9; background: #edf6fb; }
  .agent-browser { color: #52615a; background: #f1f3f2; }
  .agent-unknown { color: #48534f; background: #e2e7e4; }

  .session-copy { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .session-title-row { display: flex; align-items: center; gap: 6px; }
  .session-title-row strong { color: #27342f; font-size: 11px; }
  .source-label { display: inline-flex; align-items: center; gap: 3px; padding: 2px 5px; border-radius: 999px; color: #718079; background: rgba(80, 104, 94, 0.075); font-size: 8px; font-weight: 720; letter-spacing: 0.045em; line-height: 1.25; text-transform: uppercase; }
  .project-name { overflow: hidden; color: #56645e; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }

  .status-line { display: flex; align-items: center; gap: 5px; color: #7a8580; font-size: 10px; }
  .status-line > i { width: 5px; height: 5px; border-radius: 50%; background: #82908a; }
  .status-line.status-running { color: #4e7faf; }
  .running-dots { height: 8px; display: inline-flex; align-items: center; gap: 2px; }
  .running-dots i { width: 3px; height: 3px; border-radius: 50%; background: #5388bd; animation: status-dot-bounce 900ms ease-in-out infinite; }
  .running-dots i:nth-child(2) { animation-delay: 120ms; }
  .running-dots i:nth-child(3) { animation-delay: 240ms; }
  .status-line.status-permission_required { color: #a46522; }
  .status-line.status-permission_required > i { background: #cb8235; box-shadow: 0 0 0 3px rgba(203, 130, 53, 0.1); }
  .status-line.status-completed > i { background: #78a18f; }
  .status-line.status-failed > i { background: #b95454; }
  .status-line.status-waiting_for_input > i { background: #6681a4; }

  @keyframes status-dot-bounce {
    0%, 60%, 100% { opacity: 0.48; transform: translateY(1px); }
    30% { opacity: 1; transform: translateY(-2px); }
  }

  .chevron { width: 13px; height: 13px; color: #98a19d; transition: transform 180ms ease; }
  .selected .chevron { transform: rotate(90deg); }

  .session-details { padding: 0 2px 13px 43px; }
  .access-profile { margin: -2px 0 10px; display: grid; gap: 1px; }
  .access-profile span { color: #46554f; font-size: 10px; font-weight: 700; }
  .access-profile small { color: #89938f; font-size: 9px; }

  .permission-block { padding-left: 11px; border-left: 2px solid #d49350; display: grid; gap: 6px; }
  .permission-block .eyebrow { color: #a06323; font-size: 9px; font-weight: 780; letter-spacing: 0.05em; text-transform: uppercase; }
  .permission-block > strong { color: #4d3b2a; font-size: 11px; font-weight: 650; line-height: 1.4; }
  code { padding: 7px 8px; overflow: hidden; border-radius: 7px; color: #46524d; background: rgba(70, 82, 77, 0.055); font-family: "SFMono-Regular", Consolas, monospace; font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }

  .permission-actions { margin-top: 2px; display: flex; flex-wrap: wrap; gap: 5px; }
  .permission-actions button {
    min-height: 27px;
    padding: 0 9px;
    border: 1px solid rgba(82, 101, 93, 0.16);
    border-radius: 8px;
    color: #4d5b55;
    background: rgba(255, 255, 255, 0.58);
    font-size: 10px;
    font-weight: 700;
    cursor: pointer;
    transition: transform 130ms ease, background 130ms ease;
  }
  .permission-actions button:hover { transform: translateY(-1px); background: white; }
  .permission-actions button:active { transform: scale(0.97); }
  .permission-actions button.primary { border-color: #456d5d; color: white; background: #456d5d; }
  .permission-actions button.danger { color: #a54c4c; }
  .inline-error { margin: 1px 0 0; color: #a54c4c; font-size: 9px; }
  .integration-note { margin: 0; color: #7c8983; font-size: 10px; line-height: 1.45; }

  .continue-trigger { margin-top: 9px; padding: 0; display: inline-flex; align-items: center; gap: 5px; border: 0; color: #557266; background: transparent; font-size: 9px; font-weight: 720; cursor: pointer; }
  .continue-trigger svg { width: 13px; height: 13px; transition: transform 150ms ease; }
  .continue-trigger:hover svg,
  .continue-trigger.active svg { transform: translateX(2px); }
  .inline-composer { margin-top: 8px; display: flex; align-items: flex-end; gap: 6px; }
  .inline-composer textarea { resize: none; outline: none; font: inherit; }
  .inline-composer textarea { min-width: 0; min-height: 52px; flex: 1; padding: 8px 9px; border: 1px solid rgba(85, 109, 99, 0.14); border-radius: 10px; color: #34423c; background: rgba(255, 255, 255, 0.48); font-size: 10px; line-height: 1.4; }
  .inline-composer textarea:focus { border-color: rgba(70, 111, 94, 0.42); box-shadow: 0 0 0 3px rgba(74, 118, 99, 0.06); }
  .inline-composer button { width: 30px; height: 30px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 9px; color: white; background: #496f60; cursor: pointer; transition: transform 140ms ease, opacity 140ms ease; }
  .inline-composer button:hover:not(:disabled) { transform: translateY(-1px); }
  .inline-composer button:disabled { opacity: 0.35; cursor: default; }

  .whiteboard { max-height: 431px; min-height: 0; padding: 7px 16px 15px; display: flex; flex-direction: column; overflow: hidden; }
  .board-intro { padding: 8px 1px 14px; border-bottom: 1px solid rgba(105, 123, 115, 0.1); }
  .board-intro .eyebrow { display: block; margin-bottom: 4px; color: #7a8c84; font-size: 8px; font-weight: 760; letter-spacing: 0.065em; text-transform: uppercase; }
  .board-intro strong { color: #2d3a35; font-size: 12px; }
  .board-intro p { margin: 4px 0 0; color: #7f8a85; font-size: 9px; line-height: 1.45; }
  .terminal-picker { min-height: 0; padding: 9px 0 6px; flex: 1 1 auto; overflow-x: hidden; overflow-y: auto; overscroll-behavior: contain; scrollbar-gutter: stable; scrollbar-width: thin; scrollbar-color: #cad2ce transparent; }
  .terminal-picker-row { min-height: 59px; display: flex; align-items: center; gap: 8px; border-bottom: 1px solid rgba(105, 123, 115, 0.09); }
  .terminal-picker-row:last-child { border-bottom: 0; }
  .terminal-picker-copy { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .terminal-picker-copy strong { color: #35423d; font-size: 10px; }
  .terminal-picker-copy small { overflow: hidden; color: #89938f; font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .terminal-picker-row > button { min-width: 52px; height: 28px; padding: 0 9px; border: 1px solid rgba(82, 105, 95, 0.16); border-radius: 9px; color: #4d6f61; background: rgba(255, 255, 255, 0.38); font-size: 9px; font-weight: 720; cursor: pointer; transition: transform 140ms ease, background 140ms ease; }
  .terminal-picker-row > button:hover:not(:disabled) { transform: translateY(-1px); background: white; }
  .terminal-picker-row > button.opened { color: #73827b; background: transparent; }
  .terminal-picker-row > button:disabled { opacity: 0.5; cursor: default; }
  .board-empty { margin: 22px 0; color: #89938f; font-size: 9px; line-height: 1.45; }
  .board-message { margin: 1px 0 4px; color: #5f756b; font-size: 9px; }
  .dock-guide { margin-top: 9px; padding: 11px 9px; display: flex; align-items: center; gap: 9px; border-top: 1px solid rgba(105, 123, 115, 0.1); color: #8a9590; font-size: 8px; line-height: 1.4; }
  .dock-guide svg { width: 34px; height: 22px; flex: 0 0 auto; fill: none; stroke: #6f8f81; stroke-width: 1.3; }

  .empty-state { height: 100%; min-height: 260px; display: flex; flex-direction: column; align-items: center; justify-content: center; color: #73807a; text-align: center; }
  .empty-state strong { margin-top: 10px; color: #44524c; font-size: 11px; }
  .empty-state p { max-width: 210px; margin: 4px 0 0; font-size: 10px; line-height: 1.45; }
  .quiet-orbit { width: 31px; height: 31px; display: grid; place-items: center; border: 1px solid #aab6b0; border-radius: 50%; }
  .quiet-orbit i { width: 7px; height: 7px; border-radius: 50%; background: #799186; }

  .history-list { padding: 6px 16px 16px; }
  .history-row { min-height: 60px; display: flex; align-items: center; gap: 11px; border-bottom: 1px solid rgba(105, 123, 115, 0.1); }
  .history-dot { width: 7px; height: 7px; flex: 0 0 auto; border-radius: 50%; background: #6f9b88; }
  .history-dot.event-failed,
  .history-dot.event-permission_denied { background: #b95555; }
  .history-dot.event-permission_allowed { background: #6683a5; }
  .history-row div { min-width: 0; display: grid; gap: 3px; }
  .history-row span { overflow: hidden; color: #58665f; font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
  .history-row strong { color: #2d3a35; font-size: 10px; }
  .history-row small { color: #8a9490; font-size: 9px; }
  .privacy-note { margin: 14px 12px 0; color: #8c9691; font-size: 9px; line-height: 1.45; text-align: center; }

  .settings { padding: 5px 16px 20px; }
  .settings-section-label { padding: 9px 0 5px; color: #929c97; font-size: 9px; font-weight: 750; letter-spacing: 0.07em; text-transform: uppercase; }
  .settings-section-label.preferences-label { padding-top: 17px; }
  .integration-row { min-height: 55px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid rgba(105, 123, 115, 0.1); }
  .integration-row .agent-avatar { width: 28px; height: 28px; border-radius: 9px; font-size: 10px; }
  .integration-row > div { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .integration-row strong { color: #35423d; font-size: 10px; }
  .integration-row div span { overflow: hidden; color: #89938f; font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .integration-row button { min-width: 63px; height: 27px; padding: 0 8px; border: 1px solid rgba(82, 105, 95, 0.14); border-radius: 8px; color: #577064; background: transparent; font-size: 9px; font-weight: 680; cursor: pointer; transition: background 150ms ease, color 150ms ease, transform 150ms ease; }
  .settings-feedback { margin: -2px 16px 9px; color: #65736c; font-size: 9px; line-height: 1.45; }
  .settings-feedback.error { color: #a34f4f; }
  .integration-row button:hover:not(:disabled) { transform: translateY(-1px); background: rgba(82, 112, 99, 0.06); }
  .integration-row button.connected { border-color: transparent; color: #6d7e76; }
  .integration-row button:disabled { cursor: default; opacity: 0.5; }
  .browser-row button { min-width: 68px; }
  .browser-path { margin: 7px 2px 0; overflow-wrap: anywhere; color: #89938f; font-size: 9px; line-height: 1.4; }
  .setting-row,
  .field-row { min-height: 67px; display: flex; align-items: center; justify-content: space-between; gap: 14px; border-bottom: 1px solid rgba(105, 123, 115, 0.1); }
  .setting-row > div,
  .field-row > span,
  .launch-setting > span { display: grid; gap: 3px; }
  .setting-row strong,
  .field-row strong,
  .launch-setting strong { color: #35423d; font-size: 10px; }
  .setting-row div span,
  .field-row small,
  .launch-setting small { color: #89938f; font-size: 9px; font-weight: 400; }

  .switch { position: relative; width: 33px; height: 19px; flex: 0 0 auto; }
  .switch input { position: absolute; opacity: 0; pointer-events: none; }
  .switch span { position: absolute; inset: 0; border-radius: 999px; background: #ccd3cf; cursor: pointer; transition: background 180ms ease; }
  .switch span::after { content: ""; position: absolute; width: 15px; height: 15px; top: 2px; left: 2px; border-radius: 50%; background: white; box-shadow: 0 1px 3px rgba(29, 43, 37, 0.22); transition: transform 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
  .switch input:checked + span { background: #527c6c; }
  .switch input:checked + span::after { transform: translateX(14px); }
  .switch input:focus-visible + span { outline: 2px solid #83958d; outline-offset: 2px; }

  .field-row select { max-width: 120px; padding: 6px 23px 6px 8px; border: 1px solid rgba(92, 111, 103, 0.16); border-radius: 8px; color: #4b5a54; background: rgba(255, 255, 255, 0.52); font-size: 10px; }
  .launch-setting { padding: 14px 0 10px; display: grid; gap: 11px; }
  .segmented { padding: 2px; display: grid; grid-template-columns: repeat(3, 1fr); border-radius: 9px; background: rgba(83, 104, 95, 0.07); }
  .segmented button { height: 29px; border: 0; border-radius: 7px; color: #74817b; background: transparent; font-size: 9px; font-weight: 680; cursor: pointer; transition: color 150ms ease, background 150ms ease, box-shadow 150ms ease; }
  .segmented button.active { color: #35473f; background: rgba(255, 255, 255, 0.82); box-shadow: 0 1px 4px rgba(37, 53, 46, 0.1); }
  .update-card { padding: 12px; border: 1px solid rgba(92, 111, 103, 0.11); border-radius: 13px; background: rgba(84, 111, 99, 0.035); }
  .update-main { display: flex; align-items: center; gap: 9px; }
  .update-copy { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .update-copy strong { color: #35423d; font-size: 10px; }
  .update-copy span,
  .update-card p { color: #89938f; font-size: 9px; }
  .update-card p { margin: 9px 0 0; line-height: 1.4; }
  .update-card p.error { color: #a34f4f; }
  .update-main button { min-width: 63px; height: 27px; padding: 0 8px; border: 1px solid rgba(82, 105, 95, 0.14); border-radius: 8px; color: #577064; background: transparent; font-size: 9px; font-weight: 680; cursor: pointer; transition: background 150ms ease, transform 150ms ease; }
  .update-main button.update-available { color: #f7fbf9; border-color: #527c6c; background: #527c6c; }
  .update-main button:hover:not(:disabled) { transform: translateY(-1px); background: rgba(82, 112, 99, 0.09); }
  .update-main button.update-available:hover { background: #476f60; }
  .update-main button:disabled { cursor: default; opacity: 0.58; }
  .update-progress { height: 2px; margin-top: 9px; overflow: hidden; border-radius: 999px; background: rgba(82, 112, 99, 0.1); }
  .update-progress span { height: 100%; display: block; border-radius: inherit; background: #5f8ac7; transition: width 180ms ease; }
  .update-progress.indeterminate span { animation: update-slide 1.15s ease-in-out infinite alternate; }
  @keyframes update-slide { from { transform: translateX(-70%); } to { transform: translateX(320%); } }
  .save-state { display: block; color: #87928d; font-size: 9px; text-align: right; opacity: 0; transition: opacity 120ms ease; }
  .save-state.visible { opacity: 1; }

  footer {
    flex: 0 0 auto;
    min-height: 52px;
    padding: 6px 10px 8px;
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    align-items: center;
    border-top: 1px solid rgba(101, 120, 112, 0.11);
  }
  footer button { height: 36px; display: flex; align-items: center; justify-content: center; gap: 5px; border: 0; border-radius: 10px; color: #88928e; background: transparent; font-size: 9px; font-weight: 650; cursor: pointer; transition: color 150ms ease, background 150ms ease; }
  footer button:hover { color: #52615a; background: rgba(76, 100, 90, 0.045); }
  footer button.active { color: #476c5d; }
  footer button svg { width: 15px; height: 15px; }
  footer button.has-update { position: relative; }
  footer button.has-update::after { content: ""; position: absolute; top: 5px; right: 14px; width: 5px; height: 5px; border: 2px solid rgba(248, 250, 249, 0.95); border-radius: 50%; background: #5f8ac7; }

  @media (prefers-reduced-motion: reduce) {
    *, *::before, *::after { animation-duration: 0.01ms !important; animation-iteration-count: 1 !important; transition-duration: 0.01ms !important; }
  }

  @media (prefers-color-scheme: dark) {
    .lume-orb,
    .panel,
    .launcher-popover { color: #dfe8e3; border-color: rgba(190, 209, 200, 0.13); background: rgba(27, 34, 31, 0.96); }
    .brand-lockup strong,
    .back-button,
    .session-title-row strong,
    .board-intro strong,
    .terminal-picker-copy strong,
    .history-row strong,
    .setting-row strong,
    .integration-row strong,
    .field-row strong,
    .launch-setting strong { color: #e3ebe7; }
    .update-copy strong { color: #e3ebe7; }
    .launcher-row strong { color: #dfe8e3; }
    .panel-header,
    footer,
    .session-row,
    .history-row,
    .setting-row,
    .field-row { border-color: rgba(190, 209, 200, 0.09); }
    .session-row:hover,
    .session-row.selected { background: rgba(198, 218, 208, 0.045); }
    .project-name,
    .board-intro p,
    .terminal-picker-copy small,
    .history-row span,
    .settings-feedback { color: #adbab4; }
    .settings-feedback.error { color: #d68d8d; }
    .update-card { border-color: rgba(190, 209, 200, 0.09); background: rgba(216, 229, 223, 0.035); }
    .update-card p.error { color: #d68d8d; }
    .access-profile span,
    .empty-state strong { color: #c5d0cb; }
    code,
    .segmented { color: #bdc8c3; background: rgba(216, 229, 223, 0.06); }
    .permission-block > strong { color: #e2d0bd; }
    .permission-actions button,
    .field-row select,
    .inline-composer textarea { color: #c5d0cb; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
    .source-label { color: #9daca5; background: rgba(205, 222, 213, 0.08); }
    .board-intro,
    .terminal-picker-row,
    .dock-guide { border-color: rgba(190, 209, 200, 0.09); }
    .terminal-picker-row > button { color: #b7c4be; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
    .terminal-picker-row > button:hover:not(:disabled) { background: rgba(222, 233, 228, 0.09); }
    .permission-actions button:hover { background: rgba(222, 233, 228, 0.09); }
    .segmented button.active { color: #dfe8e3; background: rgba(214, 229, 221, 0.1); }
  }
</style>
