<script lang="ts">
  import { onMount, tick } from "svelte";
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import { fade, fly, slide } from "svelte/transition";
  import { getVersion } from "@tauri-apps/api/app";
  import { emit, listen } from "@tauri-apps/api/event";
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
  import LumeMascot from "$lib/LumeMascot.svelte";
  import TerminalWindow from "$lib/TerminalWindow.svelte";
  import { displayText, localize } from "$lib/i18n";
  import type {
    AgentKind,
    AgentSession,
    CompanionStatus,
    ExternalAgentPlugin,
    HistoryEntry,
    IntegrationDiagnostic,
    IntegrationStatus,
    PermissionAction,
    Preferences,
    ResultNote,
    SessionStatus,
    TerminalWindowState,
    WhiteboardLayout,
  } from "$lib/domain";
  import { demoSessions } from "$lib/demo";
  import {
    configureIntegration,
    configureVscode,
    diagnoseIntegration,
    decidePermission,
    defaultPreferences,
    deleteResultNote,
    loadHistory,
    loadResultNotes,
    loadIntegrationStatuses,
    loadPreferences,
    loadSessions,
    loadTerminalWindows,
    loadExternalPlugins,
    openSessionSource,
    openTerminalWindow,
    loadVscodeStatus,
    moveOverlay,
    installExternalPlugin,
    removeExternalPlugin,
    revealBrowserCompanion,
    launchAgentSession,
    savePreferences,
    saveResultNote,
    restoreTerminalLayout,
    submitPrompt,
    terminateSession,
    revealPluginDirectory,
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
  const isLinux = typeof navigator !== "undefined" &&
    navigator.userAgent.toLowerCase().includes("linux");
  const currentWindowLabel = isTauri ? getCurrentWindow().label : "main";
  const isTerminalWindow = currentWindowLabel.startsWith("terminal-");
  const compactSize = { width: 78, height: 46 };
  const expandedWidth = 392;
  const expandedMaxHeight = 560;
  const edgeAnchorThreshold = 18;

  let expanded = $state(!isTauri);
  let contentVisible = $state(!isTauri);
  let morphing = $state<"opening" | "closing" | null>(null);
  let morphProgress = $state(isTauri ? 0 : 1);
  let expandedHeight = $state(expandedMaxHeight);
  let view = $state<View>("sessions");
  let sessions = $state<AgentSession[]>(isTauri ? [] : structuredClone(demoSessions));
  let history = $state<HistoryEntry[]>([]);
  let resultNotes = $state<ResultNote[]>([]);
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
  let diagnosingIntegration = $state<IntegrationStatus["kind"] | null>(null);
  let integrationDiagnostics = $state<Partial<Record<IntegrationStatus["kind"], IntegrationDiagnostic>>>({});
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
  let terminateConfirmId = $state<string | null>(null);
  let terminatingSessionId = $state<string | null>(null);
  let sessionActionMessage = $state<string | null>(null);
  let copiedResultId = $state<string | null>(null);
  let savingNoteId = $state<string | null>(null);
  let noteMessage = $state<string | null>(null);
  let selectedProfileKey = $state<string | null>(null);
  let terminalWindows = $state<TerminalWindowState[]>([]);
  let openingTerminal = $state<string | null>(null);
  let terminalMessage = $state<string | null>(null);
  let layoutName = $state("");
  let selectedLayoutId = $state<string | null>(null);
  let restoringLayout = $state(false);
  let externalPlugins = $state<ExternalAgentPlugin[]>([]);
  let installingPlugin = $state(false);
  let pluginMessage = $state<string | null>(null);
  let paletteOpen = $state(false);
  let paletteQuery = $state("");
  let paletteIndex = $state(0);
  let overlayPosition = $state({ x: 0, y: 12 });
  let compactAnchorPosition: { x: number; y: number } | null = null;
  let overlayReady = $state(false);
  let monitorBounds = $state({ width: 1920, height: 1080, scale: 1 });
  let dragging = $state(false);
  let mascotAwake = $state(false);
  let mascotSleepTimer: ReturnType<typeof setTimeout> | undefined;
  let appVersion = $state("0.4.0");
  let updateState = $state<UpdateState>("idle");
  let availableVersion = $state<string | null>(null);
  let updateDetail = $state("Updates are checked automatically.");
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
  let pendingOverlayMove: { x: number; y: number } | null = null;
  let overlayMoveTask: Promise<void> | null = null;
  let pendingMorphGeometry: { width: number; height: number; x: number; y: number } | null = null;
  let morphGeometryTask: Promise<void> | null = null;
  let systemDark = $state(false);

  function tr(english: string, portuguese: string) {
    return localize(preferences.language, english, portuguese);
  }

  function shown(value: string) {
    return displayText(preferences.language, value);
  }

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
      const previousSize = currentExpandedSize();
      const anchor = compactAnchorPosition ?? compactPositionFromExpanded(overlayPosition, previousSize);
      expandedHeight = nextHeight;
      if (resizeWindow && isTauri && expanded && !morphing) {
        compactAnchorPosition = anchor;
        const target = currentExpandedSize();
        const position = expandedPositionFromCompact(anchor, target);
        overlayPosition = position;
        void Promise.all([
          getCurrentWindow().setSize(new LogicalSize(target.width, target.height)),
          moveOverlay(position.x, position.y, false, preferences.monitorId),
        ]).catch(() => undefined);
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

  const effectiveDark = $derived(preferences.darkMode ?? systemDark);
  $effect(() => {
    document.documentElement.dataset.theme = effectiveDark ? "dark" : "light";
  });
  const activeCount = $derived(
    sessions.filter((session) =>
      ["running", "permission_required", "waiting_for_input"].includes(session.status),
    ).length,
  );
  const recentResults = $derived.by(() =>
    sessions
      .flatMap((session) => session.results.map((result) => ({ session, result })))
      .sort((left, right) => right.result.createdAt - left.result.createdAt),
  );
  const detectedProjects = $derived.by(() => {
    const projects = new Map<string, string>();
    for (const [key, profile] of Object.entries(preferences.projectProfiles)) {
      if (profile.label) projects.set(key, profile.label);
    }
    for (const session of sessions) {
      projects.set(projectKey(session.workingDirectory ?? session.project), session.project);
    }
    return Array.from(projects, ([key, label]) => ({ key, label })).sort((left, right) =>
      left.label.localeCompare(right.label),
    );
  });
  const selectedProject = $derived(
    detectedProjects.find((project) => project.key === selectedProfileKey),
  );
  const selectedProjectProfile = $derived(
    selectedProfileKey ? preferences.projectProfiles[selectedProfileKey] : undefined,
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
    const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    const syncSystemTheme = (event: MediaQueryListEvent | MediaQueryList) => {
      systemDark = event.matches;
    };
    syncSystemTheme(colorScheme);
    colorScheme.addEventListener("change", syncSystemTheme);
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let stopTerminalListening: (() => void) | undefined;
    let stopPaletteListening: (() => void) | undefined;
    let pollTimer: ReturnType<typeof setInterval> | undefined;
    let updateTimer: ReturnType<typeof setInterval> | undefined;

    updateTimer = setInterval(() => void checkForUpdates(), 6 * 60 * 60 * 1_000);

    void (async () => {
      const [nextSessions, nextPreferences, nextIntegrations, nextVscodeStatus, nextPlugins] = await Promise.all([
        loadSessions(),
        loadPreferences(),
        loadIntegrationStatuses(),
        loadVscodeStatus(),
        loadExternalPlugins(),
      ]);
      if (disposed) return;
      sessions = nextSessions;
      preferences = nextPreferences;
      selectedProfileKey = detectedProjects[0]?.key ?? null;
      void initializeUpdater();
      integrations = nextIntegrations;
      vscodeStatus = nextVscodeStatus;
      externalPlugins = nextPlugins;
      selectedLayoutId = preferences.whiteboardLayouts[0]?.id ?? null;
      layoutName = preferences.whiteboardLayouts[0]?.name ?? "";
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
        stopPaletteListening = await listen("lume://open-command-palette", () => {
          void showCommandPalette();
        });
        pollTimer = setInterval(() => void refreshSessions(false), 5_000);
      }
    })();

    return () => {
      disposed = true;
      stopListening?.();
      stopTerminalListening?.();
      stopPaletteListening?.();
      colorScheme.removeEventListener("change", syncSystemTheme);
      if (pollTimer) clearInterval(pollTimer);
      if (updateTimer) clearInterval(updateTimer);
      if (mascotSleepTimer) clearTimeout(mascotSleepTimer);
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
      // Keep the package version as fallback.
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
    updateDetail = tr("Checking for a new version…", "Procurando uma nova versão…");
    updateProgress = null;

    try {
      const nextUpdate = await check({ timeout: 15_000 });
      if (pendingUpdate && pendingUpdate !== nextUpdate) await pendingUpdate.close();
      pendingUpdate = nextUpdate;
      availableVersion = nextUpdate?.version ?? null;
      if (nextUpdate) {
        updateState = "available";
        updateDetail = tr(
          `Version ${nextUpdate.version} is ready to download.`,
          `A versão ${nextUpdate.version} está pronta para baixar.`,
        );
      } else {
        updateState = "up_to_date";
        updateDetail = tr("You are using the latest version.", "Você está usando a versão mais recente.");
      }
    } catch {
      updateState = "error";
      updateDetail = tr(
        "Could not check for updates right now. Try again shortly.",
        "Não foi possível verificar agora. Tente novamente em instantes.",
      );
    }
  }

  async function installAvailableUpdate() {
    if (!pendingUpdate || updateState === "downloading") return;
    updateState = "downloading";
    updateDetail = tr("Downloading and preparing the update…", "Baixando e preparando a atualização…");
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
      updateDetail = tr("Update installed. Restarting Lume…", "Atualização instalada. Reiniciando o Lume…");
      await relaunch();
    } catch {
      updateState = "error";
      updateDetail = tr(
        "The update could not be installed. Try again.",
        "A atualização não pôde ser instalada. Tente novamente.",
      );
      updateProgress = null;
    }
  }

  async function refreshSessions(withSound: boolean) {
    const next = await loadSessions();
    if (withSound && preferences.soundEnabled) {
      const previous = new Map(sessions.map((session) => [session.id, session.status]));
      for (const session of next) {
        if (!projectSoundEnabled(session)) continue;
        const previousStatus = previous.get(session.id);
        if (previousStatus === session.status) continue;
        if (session.status === "permission_required") playTone("permission");
        if (
          session.status === "completed" &&
          (previousStatus === "running" || previousStatus === "permission_required")
        ) playTone("completed");
        if (session.status === "failed" && previousStatus) playTone("failed");
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
            (isLinux ? Math.round(44 * scale) : 12),
        };
        overlayReady = true;
      }
      overlayPosition = clampOverlayPosition(overlayPosition.x, overlayPosition.y, target);
      await moveOverlay(overlayPosition.x, overlayPosition.y, false, preferences.monitorId);
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
      compactAnchorPosition = { ...overlayPosition };
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
    compactAnchorPosition = null;
    selectedId = null;
    view = "sessions";
    launcherOpen = false;
    await tick();
    morphing = null;
    void remapCompactSurface();
  }

  async function animateWindowSize(opening: boolean) {
    const expandedTarget = currentExpandedSize();
    const compactTargetPosition = compactAnchorPosition ??
      compactPositionFromExpanded(overlayPosition, expandedTarget);
    const expandedTargetPosition = expandedPositionFromCompact(
      compactTargetPosition,
      expandedTarget,
    );
    const from = opening ? compactSize : expandedTarget;
    const to = opening ? expandedTarget : compactSize;
    const fromPosition = opening ? compactTargetPosition : expandedTargetPosition;
    const toPosition = opening ? expandedTargetPosition : compactTargetPosition;
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const duration = reducedMotion ? 1 : opening ? 360 : 320;
    if (!isTauri) {
      morphProgress = opening ? 1 : 0;
      return;
    }
    const currentWindow = getCurrentWindow();
    const startedAt = performance.now();
    await new Promise<void>((resolve) => {
      const frame = (now: number) => {
        const linear = Math.min(1, (now - startedAt) / duration);
        const eased = linear < 0.5
          ? 4 * linear * linear * linear
          : 1 - Math.pow(-2 * linear + 2, 3) / 2;
        morphProgress = opening ? eased : 1 - eased;
        if (opening && eased > 0.48) contentVisible = true;
        const width = Math.round(from.width + (to.width - from.width) * eased);
        const height = Math.round(from.height + (to.height - from.height) * eased);
        const x = Math.round(fromPosition.x + (toPosition.x - fromPosition.x) * eased);
        const y = Math.round(fromPosition.y + (toPosition.y - fromPosition.y) * eased);
        queueMorphGeometry(width, height, x, y);
        overlayPosition = { x, y };
        if (linear < 1) {
          requestAnimationFrame(frame);
        } else {
          resolve();
        }
      };
      requestAnimationFrame(frame);
    });
    while (morphGeometryTask) await morphGeometryTask;
    await Promise.allSettled([
      currentWindow.setSize(new LogicalSize(to.width, to.height)),
      moveOverlay(toPosition.x, toPosition.y, false, preferences.monitorId),
    ]);
    overlayPosition = { ...toPosition };
    morphProgress = opening ? 1 : 0;
  }

  function queueMorphGeometry(width: number, height: number, x: number, y: number) {
    pendingMorphGeometry = { width, height, x, y };
    if (morphGeometryTask) return;
    const currentWindow = getCurrentWindow();
    morphGeometryTask = (async () => {
      while (pendingMorphGeometry) {
        const geometry = pendingMorphGeometry;
        pendingMorphGeometry = null;
        await Promise.all([
          currentWindow.setSize(new LogicalSize(geometry.width, geometry.height)),
          moveOverlay(geometry.x, geometry.y, false, preferences.monitorId),
        ]);
      }
    })()
      .catch(() => undefined)
      .finally(() => {
        morphGeometryTask = null;
        if (pendingMorphGeometry) {
          queueMorphGeometry(
            pendingMorphGeometry.width,
            pendingMorphGeometry.height,
            pendingMorphGeometry.x,
            pendingMorphGeometry.y,
          );
        }
      });
  }

  async function remapCompactSurface() {
    if (!isTauri || !isLinux) return;
    const currentWindow = getCurrentWindow();
    try {
      await currentWindow.hide();
      await currentWindow.setSize(new LogicalSize(compactSize.width, compactSize.height));
      await moveOverlay(
        overlayPosition.x,
        overlayPosition.y,
        false,
        preferences.monitorId,
      );
    } finally {
      await currentWindow.show().catch(() => undefined);
    }
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

  function expandedPositionFromCompact(
    compactPosition: { x: number; y: number },
    target = currentExpandedSize(),
  ) {
    const compactWidth = compactSize.width * monitorBounds.scale;
    const compactHeight = compactSize.height * monitorBounds.scale;
    const targetWidth = target.width * monitorBounds.scale;
    const targetHeight = target.height * monitorBounds.scale;
    const rightDistance = monitorBounds.width - compactPosition.x - compactWidth;
    const bottomDistance = monitorBounds.height - compactPosition.y - compactHeight;
    const edgeThreshold = edgeAnchorThreshold * monitorBounds.scale;
    const x = rightDistance <= edgeThreshold
      ? compactPosition.x - (targetWidth - compactWidth)
      : compactPosition.x;
    const y = bottomDistance <= edgeThreshold
      ? compactPosition.y - (targetHeight - compactHeight)
      : compactPosition.y;
    return clampOverlayPosition(x, y, target);
  }

  function compactPositionFromExpanded(
    expandedPosition: { x: number; y: number },
    source = currentExpandedSize(),
  ) {
    const compactWidth = compactSize.width * monitorBounds.scale;
    const compactHeight = compactSize.height * monitorBounds.scale;
    const sourceWidth = source.width * monitorBounds.scale;
    const sourceHeight = source.height * monitorBounds.scale;
    const rightDistance = monitorBounds.width - expandedPosition.x - sourceWidth;
    const bottomDistance = monitorBounds.height - expandedPosition.y - sourceHeight;
    const edgeThreshold = edgeAnchorThreshold * monitorBounds.scale;
    const x = rightDistance <= edgeThreshold
      ? expandedPosition.x + (sourceWidth - compactWidth)
      : expandedPosition.x;
    const y = bottomDistance <= edgeThreshold
      ? expandedPosition.y + (sourceHeight - compactHeight)
      : expandedPosition.y;
    return clampOverlayPosition(x, y, compactSize);
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

  function wakeMascot() {
    if (shellStatus !== "idle") return;
    mascotAwake = true;
    if (mascotSleepTimer) clearTimeout(mascotSleepTimer);
    mascotSleepTimer = setTimeout(() => {
      mascotAwake = false;
      mascotSleepTimer = undefined;
    }, 1_600);
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
      queueOverlayMove(overlayPosition.x, overlayPosition.y);
    });
  }

  function queueOverlayMove(x: number, y: number) {
    pendingOverlayMove = { x, y };
    if (overlayMoveTask) return;
    overlayMoveTask = (async () => {
      while (pendingOverlayMove) {
        const next = pendingOverlayMove;
        pendingOverlayMove = null;
        await moveOverlay(next.x, next.y, false, preferences.monitorId);
      }
    })()
      .catch(() => undefined)
      .finally(() => {
        overlayMoveTask = null;
        if (pendingOverlayMove) queueOverlayMove(pendingOverlayMove.x, pendingOverlayMove.y);
      });
  }

  async function waitForOverlayMoves() {
    while (overlayMoveTask) await overlayMoveTask;
  }

  async function endOverlayDrag(event: PointerEvent, compact = false) {
    if (!dragState || dragState.pointerId !== event.pointerId) return;
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    dragState = null;
    if (!dragging) return;
    dragging = false;
    if (compact) suppressCompactToggle = true;
    const persistedPosition = expanded
      ? compactPositionFromExpanded(overlayPosition)
      : overlayPosition;
    compactAnchorPosition = expanded ? persistedPosition : null;
    preferences = {
      ...preferences,
      overlayX: Math.round(persistedPosition.x),
      overlayY: Math.round(persistedPosition.y),
    };
    queueOverlayMove(overlayPosition.x, overlayPosition.y);
    await waitForOverlayMoves();
    await savePreferences(preferences);
  }

  function openSession(session: AgentSession) {
    selectedId = selectedId === session.id ? null : session.id;
    if (selectedId !== session.id) composerSessionId = null;
    permissionError = null;
    terminateConfirmId = null;
    sessionActionMessage = null;
  }

  function canSubmitToSession(session: AgentSession) {
    return sessionCapabilities(session).canPrompt;
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
          ? { ...item, status: "running", statusLabel: "Prompt enviado pelo Lume", lastResponse: undefined }
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

  function canTerminateSession(session: AgentSession) {
    return sessionCapabilities(session).canTerminate;
  }

  function sessionCapabilities(session: AgentSession) {
    return {
      canPrompt:
        session.source === "web" ||
        (session.agent !== "unknown" && Boolean(session.nativeSessionId)),
      canApprove: Boolean(
        session.pendingPermission && session.permissionProfile.canRespondFromLume,
      ),
      canTerminate: session.source === "cli" && Boolean(session.processId),
      canOpenSource: session.source === "web" || session.source === "vscode",
      canReadResults: session.results.length > 0 || Boolean(session.lastResponse),
    };
  }

  async function copyResult(resultId: string, response: string) {
    try {
      await navigator.clipboard.writeText(response);
      copiedResultId = resultId;
      setTimeout(() => {
        if (copiedResultId === resultId) copiedResultId = null;
      }, 1_500);
    } catch {
      copiedResultId = null;
    }
  }

  async function keepResultAsNote(session: AgentSession, resultId: string) {
    if (savingNoteId) return;
    savingNoteId = resultId;
    noteMessage = null;
    try {
      const note = await saveResultNote(session.id, resultId, `${session.agentLabel} · ${session.project}`);
      resultNotes = [note, ...resultNotes.filter((item) => item.id !== note.id)];
      noteMessage = tr("Result saved as a local note.", "Resultado salvo como nota local.");
    } catch (error) {
      noteMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      savingNoteId = null;
    }
  }

  async function removeResultNote(id: string) {
    try {
      await deleteResultNote(id);
      resultNotes = resultNotes.filter((note) => note.id !== id);
    } catch (error) {
      noteMessage = String(error).replace(/^Error:\s*/, "");
    }
  }

  function continueFromResult(session: AgentSession) {
    view = "sessions";
    selectedId = session.id;
    composerSessionId = session.id;
    composerPrompt = "";
    composerMessage = null;
  }

  async function terminateAgent(session: AgentSession) {
    if (!canTerminateSession(session) || terminatingSessionId) return;
    if (terminateConfirmId !== session.id) {
      terminateConfirmId = session.id;
      sessionActionMessage = null;
      return;
    }
    terminatingSessionId = session.id;
    sessionActionMessage = null;
    try {
      if (isTauri) await terminateSession(session.id);
      terminateConfirmId = null;
      await refreshSessions(false);
    } catch (error) {
      sessionActionMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      terminatingSessionId = null;
    }
  }

  async function refreshTerminalWindows() {
    terminalWindows = await loadTerminalWindows();
  }

  async function openTerminal(session: AgentSession) {
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

  async function saveCurrentLayout() {
    await refreshTerminalWindows();
    if (terminalWindows.length === 0) {
      terminalMessage = tr("Open at least one terminal before saving a layout.", "Abra ao menos um terminal antes de salvar um layout.");
      return;
    }
    const name = layoutName.trim() || tr("My layout", "Meu layout");
    const id = selectedLayoutId && preferences.whiteboardLayouts.some((layout) => layout.id === selectedLayoutId)
      ? selectedLayoutId
      : `layout-${Date.now().toString(36)}`;
    const layout: WhiteboardLayout = {
      id,
      name,
      terminals: terminalWindows.flatMap((terminal) => {
        const session = sessions.find((item) => item.id === terminal.sessionId);
        return session
          ? [{
              agent: session.agent,
              agentLabel: session.agentLabel,
              project: session.project,
              source: session.source,
              x: terminal.x,
              y: terminal.y,
              width: terminal.width,
              height: terminal.height,
              groupId: terminal.groupId,
              monitorId: terminal.monitorId,
            }]
          : [];
      }),
    };
    const layouts = preferences.whiteboardLayouts.some((item) => item.id === id)
      ? preferences.whiteboardLayouts.map((item) => item.id === id ? layout : item)
      : [...preferences.whiteboardLayouts, layout];
    await updatePreference("whiteboardLayouts", layouts);
    selectedLayoutId = id;
    layoutName = name;
    terminalMessage = tr("Whiteboard layout saved.", "Layout do whiteboard salvo.");
  }

  async function restoreSavedLayout(layout: WhiteboardLayout) {
    if (restoringLayout) return;
    restoringLayout = true;
    terminalMessage = null;
    const used = new Set<string>();
    const entries: Array<{
      sessionId: string;
      x: number;
      y: number;
      width: number;
      height: number;
      groupId?: string;
      monitorId?: string;
    }> = [];
    try {
      for (const slot of layout.terminals) {
        const session = sessions.find((item) =>
          !used.has(item.id) &&
          item.agent === slot.agent &&
          (item.agent !== "unknown" || item.agentLabel === slot.agentLabel) &&
          item.project === slot.project &&
          item.source === slot.source,
        );
        if (!session) continue;
        used.add(session.id);
        await openTerminalWindow(session.id);
        entries.push({
          sessionId: session.id,
          x: slot.x,
          y: slot.y,
          width: slot.width,
          height: slot.height,
          groupId: slot.groupId,
          monitorId: slot.monitorId,
        });
      }
      if (entries.length === 0) {
        terminalMessage = tr("No open session matches this layout.", "Nenhuma sessão aberta corresponde a este layout.");
        return;
      }
      terminalWindows = await restoreTerminalLayout(entries);
      selectedLayoutId = layout.id;
      layoutName = layout.name;
      terminalMessage = tr(`Restored ${entries.length} terminals.`, `${entries.length} terminais restaurados.`);
    } catch (error) {
      terminalMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      restoringLayout = false;
    }
  }

  async function deleteSavedLayout(id: string) {
    const profiles = Object.fromEntries(
      Object.entries(preferences.projectProfiles).map(([key, profile]) => [
        key,
        profile.whiteboardLayoutId === id
          ? { ...profile, whiteboardLayoutId: undefined }
          : profile,
      ]),
    );
    preferences = {
      ...preferences,
      whiteboardLayouts: preferences.whiteboardLayouts.filter((layout) => layout.id !== id),
      projectProfiles: profiles,
    };
    await savePreferences(preferences);
    selectedLayoutId = preferences.whiteboardLayouts[0]?.id ?? null;
    layoutName = preferences.whiteboardLayouts.find((layout) => layout.id === selectedLayoutId)?.name ?? "";
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
                statusLabel: action === "deny" ? "Permission denied" : "Continuing task",
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
    paletteOpen = false;
    selectedId = null;
    permissionError = null;
    launcherOpen = false;
    composerSessionId = null;
    composerMessage = null;
    terminalMessage = null;
    if (nextView === "board") await refreshTerminalWindows();
    if (nextView === "history") {
      [history, resultNotes] = await Promise.all([loadHistory(), loadResultNotes()]);
    }
    if (nextView === "settings") {
      selectedProfileKey ??= detectedProjects[0]?.key ?? null;
      settingsMessage = null;
      [integrations, vscodeStatus, externalPlugins] = await Promise.all([
        loadIntegrationStatuses(),
        loadVscodeStatus(),
        loadExternalPlugins(),
      ]);
    }
  }

  type PaletteCommand = { id: string; label: string; detail: string; run: () => void | Promise<void> };

  function paletteCommands(): PaletteCommand[] {
    const commands: PaletteCommand[] = [
      { id: "sessions", label: tr("Sessions", "Sessões"), detail: tr("Show active agents", "Mostrar agentes ativos"), run: () => openView("sessions") },
      { id: "whiteboard", label: "Whiteboard", detail: tr("Open floating terminals", "Abrir terminais flutuantes"), run: () => openView("board") },
      { id: "history", label: tr("History and notes", "Histórico e notas"), detail: tr("Open completed results", "Abrir resultados finalizados"), run: () => openView("history") },
      { id: "settings", label: tr("Settings", "Ajustes"), detail: tr("Configure Lume", "Configurar o Lume"), run: () => openView("settings") },
      { id: "new-session", label: tr("New agent session", "Nova sessão de agente"), detail: tr("Open the agent launcher", "Abrir o iniciador de agentes"), run: async () => { await openView("sessions"); launcherOpen = true; } },
    ];
    for (const session of sessions) {
      commands.push({
        id: `session-${session.id}`,
        label: `${session.agentLabel} · ${session.project}`,
        detail: shown(session.statusLabel),
        run: async () => {
          await openView("sessions");
          selectedId = session.id;
        },
      });
    }
    const query = paletteQuery.trim().toLowerCase();
    return query
      ? commands.filter((command) => `${command.label} ${command.detail}`.toLowerCase().includes(query))
      : commands;
  }

  async function showCommandPalette() {
    if (!expanded) await toggleExpanded();
    paletteQuery = "";
    paletteIndex = 0;
    paletteOpen = true;
    await tick();
    document.querySelector<HTMLInputElement>("[data-command-palette]")?.focus();
  }

  async function runPaletteCommand(command: PaletteCommand) {
    paletteOpen = false;
    await command.run();
  }

  function handlePaletteKey(event: KeyboardEvent) {
    const commands = paletteCommands();
    if (event.key === "Escape") {
      paletteOpen = false;
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      paletteIndex = commands.length ? (paletteIndex + 1) % commands.length : 0;
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      paletteIndex = commands.length ? (paletteIndex - 1 + commands.length) % commands.length : 0;
      return;
    }
    if (event.key === "Enter" && commands[paletteIndex]) {
      event.preventDefault();
      void runPaletteCommand(commands[paletteIndex]);
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
      title: resume
        ? tr("Project of the session to resume", "Projeto da sessão a retomar")
        : tr("Project for the new session", "Projeto da nova sessão"),
    });
    if (!selected || Array.isArray(selected)) return;

    launching = agent;
    launchError = null;
    try {
      const profile = preferences.projectProfiles[projectKey(selected)];
      await launchAgentSession(
        agent,
        selected,
        resume,
        undefined,
        profile?.launchTarget ?? preferences.launchTarget,
        resume ? undefined : profile?.permissionMode,
        resume ? undefined : profile?.approvalPolicy,
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
          ? tr(
              "Codex connected. Open /hooks in Codex and trust the Lume hook once.",
              "Codex conectado. Abra /hooks no Codex e confie no hook Lume uma vez.",
            )
          : tr(`${integration.label} connected to Lume.`, `${integration.label} conectado ao Lume.`)
        : tr(`${integration.label} disconnected.`, `${integration.label} desconectado.`);
    } catch (error) {
      settingsMessageIsError = true;
      settingsMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      configuringIntegration = null;
    }
  }

  async function runIntegrationDiagnostic(integration: IntegrationStatus) {
    diagnosingIntegration = integration.kind;
    settingsMessage = null;
    try {
      integrationDiagnostics = {
        ...integrationDiagnostics,
        [integration.kind]: await diagnoseIntegration(integration.kind),
      };
    } catch (error) {
      settingsMessageIsError = true;
      settingsMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      diagnosingIntegration = null;
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
        ? tr("Companion installed in VS Code.", "Companion instalado no VS Code.")
        : tr("Companion removed from VS Code.", "Companion removido do VS Code.");
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
      browserCompanionPath = tr(
        "Could not open the extension folder.",
        "Não foi possível abrir a pasta da extensão.",
      );
    }
  }

  async function addExternalPlugin() {
    if (!isTauri || installingPlugin) return;
    const selected = await openDialog({
      multiple: false,
      directory: false,
      title: tr("Install agent detector", "Instalar detector de agente"),
      filters: [{ name: "Lume plugin", extensions: ["json"] }],
    });
    if (!selected || Array.isArray(selected)) return;
    installingPlugin = true;
    pluginMessage = null;
    try {
      const plugin = await installExternalPlugin(selected);
      externalPlugins = await loadExternalPlugins();
      pluginMessage = tr(`${plugin.name} is now monitored.`, `${plugin.name} agora é monitorado.`);
    } catch (error) {
      pluginMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      installingPlugin = false;
    }
  }

  async function uninstallExternalPlugin(id: string) {
    try {
      await removeExternalPlugin(id);
      externalPlugins = await loadExternalPlugins();
      pluginMessage = tr("Detector removed.", "Detector removido.");
    } catch (error) {
      pluginMessage = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function openPluginFolder() {
    try {
      pluginMessage = await revealPluginDirectory();
    } catch (error) {
      pluginMessage = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function updatePreference<K extends keyof Preferences>(
    key: K,
    value: Preferences[K],
  ) {
    const previous = preferences;
    preferences = { ...preferences, [key]: value };
    if (key === "language") {
      if (updateState === "up_to_date") {
        updateDetail = tr("You are using the latest version.", "Você está usando a versão mais recente.");
      } else if (updateState === "idle") {
        updateDetail = tr("Updates are checked automatically.", "As atualizações são verificadas automaticamente.");
      }
    }
    savingSettings = true;
    try {
      await savePreferences(preferences);
      if (isTauri) void emit("lume://preferences-changed", preferences);
      if (key === "monitorId") await positionWindow();
    } catch (error) {
      preferences = previous;
      settingsMessageIsError = true;
      settingsMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      savingSettings = false;
    }
  }

  function projectKey(value: string) {
    const normalized = value.trim().replaceAll("\\", "/").replace(/\/+$/, "");
    const identity = /^[a-z]:/i.test(normalized) ? normalized.toLowerCase() : normalized;
    let hash = 0x811c9dc5;
    for (let index = 0; index < identity.length; index += 1) {
      hash ^= identity.charCodeAt(index);
      hash = Math.imul(hash, 0x01000193);
    }
    return `project-${(hash >>> 0).toString(16).padStart(8, "0")}`;
  }

  function projectSoundEnabled(session: AgentSession) {
    const profile = preferences.projectProfiles[projectKey(session.workingDirectory ?? session.project)];
    return profile?.soundEnabled ?? true;
  }

  async function updateSelectedProjectProfile(
    patch: Partial<Preferences["projectProfiles"][string]>,
  ) {
    if (!selectedProfileKey || !selectedProject) return;
    const current = preferences.projectProfiles[selectedProfileKey] ?? {
      label: selectedProject.label,
      soundEnabled: true,
      launchTarget: undefined,
      monitorId: undefined,
      overlayX: undefined,
      overlayY: undefined,
      permissionMode: undefined,
      approvalPolicy: undefined,
      whiteboardLayoutId: undefined,
      preferredAgents: [],
    };
    await updatePreference("projectProfiles", {
      ...preferences.projectProfiles,
      [selectedProfileKey]: { ...current, ...patch },
    });
  }

  async function captureProfilePosition() {
    const position = expanded
      ? compactPositionFromExpanded(overlayPosition)
      : overlayPosition;
    await updateSelectedProjectProfile({
      overlayX: Math.round(position.x),
      overlayY: Math.round(position.y),
    });
  }

  async function togglePreferredAgent(agent: AgentKind) {
    const current = selectedProjectProfile?.preferredAgents ?? [];
    await updateSelectedProjectProfile({
      preferredAgents: current.includes(agent)
        ? current.filter((item) => item !== agent)
        : [...current, agent],
    });
  }

  async function applySelectedProjectProfile() {
    const profile = selectedProjectProfile;
    if (!profile) return;
    preferences = {
      ...preferences,
      monitorId: profile.monitorId ?? preferences.monitorId,
      overlayX: profile.overlayX ?? preferences.overlayX,
      overlayY: profile.overlayY ?? preferences.overlayY,
    };
    await savePreferences(preferences);
    if (isTauri) void emit("lume://preferences-changed", preferences);
    await positionWindow(true);
    const layout = preferences.whiteboardLayouts.find(
      (item) => item.id === profile.whiteboardLayoutId,
    );
    if (layout) {
      view = "board";
      await restoreSavedLayout(layout);
    }
    settingsMessageIsError = false;
    settingsMessage = tr("Project profile applied.", "Perfil do projeto aplicado.");
  }

  function launcherIntegrations() {
    const preferred = selectedProjectProfile?.preferredAgents ?? [];
    return integrations
      .filter((integration) => integration.installed)
      .slice()
      .sort((left, right) => {
        const leftIndex = preferred.indexOf(left.kind);
        const rightIndex = preferred.indexOf(right.kind);
        if (leftIndex === rightIndex) return left.label.localeCompare(right.label);
        if (leftIndex < 0) return 1;
        if (rightIndex < 0) return -1;
        return leftIndex - rightIndex;
      });
  }

  function playTone(kind: "completed" | "failed" | "permission") {
    try {
      const AudioContextClass = window.AudioContext;
      const context = new AudioContextClass();
      const gain = context.createGain();
      gain.gain.setValueAtTime(0.0001, context.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.045, context.currentTime + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, context.currentTime + 0.42);
      gain.connect(context.destination);

      const notes = kind === "completed"
        ? [620, 820]
        : kind === "permission"
          ? [520, 690, 520]
          : [330, 250];
      notes.forEach((frequency, index) => {
        const oscillator = context.createOscillator();
        oscillator.type = "sine";
        oscillator.frequency.value = frequency;
        oscillator.connect(gain);
        oscillator.start(context.currentTime + index * 0.09);
        oscillator.stop(context.currentTime + 0.25 + index * 0.09);
      });
      setTimeout(() => void context.close(), 600);
    } catch {
      // Audio is optional and may be blocked until the first interaction.
    }
  }

  function actionLabel(action: PermissionAction) {
    return {
      allow_once: tr("Allow once", "Permitir uma vez"),
      allow_session: tr("For this session", "Nesta sessão"),
      deny: tr("Deny", "Recusar"),
      open_source: tr("Open source", "Abrir origem"),
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
    if (seconds < 60) return tr("now", "agora");
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return tr(`${minutes} min ago`, `há ${minutes} min`);
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return tr(`${hours} hr ago`, `há ${hours} h`);
    return new Intl.DateTimeFormat(preferences.language === "pt-BR" ? "pt-BR" : "en", { day: "2-digit", month: "short" }).format(
      timestamp,
    );
  }

  function eventLabel(event: HistoryEntry["event"]) {
    return {
      completed: tr("Completed", "Finalizado"),
      failed: tr("Error", "Erro"),
      permission_allowed: tr("Allowed", "Permitido"),
      permission_denied: tr("Denied", "Recusado"),
    }[event];
  }
</script>

<svelte:head>
  <title>Lume</title>
  <meta name="description" content="A discreet local monitor for AI agent sessions." />
</svelte:head>

{#if isTerminalWindow}
  <TerminalWindow />
{:else}
<main
  class:expanded
  class:dark={effectiveDark}
  class="overlay-shell"
  style={`--panel-gap-right: ${Math.round(8 * morphProgress)}px; --panel-gap-bottom: ${Math.round(16 * morphProgress)}px; --panel-radius: ${Math.round(23 - 2 * morphProgress)}px;`}
  onpointermove={wakeMascot}
  aria-label={tr("Lume, agent monitor", "Lume, monitor de agentes")}
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
      aria-label={tr(`Open Lume, ${activeCount} active agents`, `Abrir Lume, ${activeCount} agentes ativos`)}
    >
      <LumeMascot status={shellStatus} awake={mascotAwake || dragging} size={30} />
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
        <div class="brand-lockup">
          <LumeMascot status={shellStatus} awake={mascotAwake || dragging} size={32} />
          <div>
            <strong>Lume</strong>
            <span>{activeCount === 1 ? tr("1 active agent", "1 agente ativo") : tr(`${activeCount} active agents`, `${activeCount} agentes ativos`)}</span>
          </div>
        </div>
        <div class="header-actions">
          <button class="palette-button" type="button" title={preferences.globalShortcut} onclick={showCommandPalette} aria-label={tr("Open command palette", "Abrir paleta de comandos")}>
            <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="8.5" cy="8.5" r="4.5" /><path d="m12 12 4 4" /></svg>
          </button>
          {#if view === "sessions"}
            <button class:active={launcherOpen} class="add-button" type="button" onclick={toggleLauncher} aria-label={tr("Open or resume session", "Abrir ou retomar sessão")}>
              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 5v10M5 10h10" /></svg>
            </button>
          {/if}
          <button class="collapse-button" type="button" onclick={toggleExpanded} aria-label={tr("Collapse", "Recolher")}>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5.5 8 4.5 4 4.5-4" /></svg>
          </button>
        </div>
      </header>

      {#if launcherOpen}
        <div class="launcher-popover" transition:fly={{ y: -5, duration: 170, easing: cubicOut }}>
          <span class="launcher-title">{tr("Open session", "Abrir sessão")}</span>
          {#each launcherIntegrations() as integration}
            <div class="launcher-row">
              <span class="agent-avatar agent-{integration.kind}"><BrandIcon name={integration.kind} size={17} /></span>
              <strong>{integration.label}</strong>
              <button disabled={launching !== null} type="button" onclick={() => startSession(integration.kind, false)}>{tr("New", "Nova")}</button>
              <button disabled={launching !== null} type="button" onclick={() => startSession(integration.kind, true)}>{tr("Resume", "Retomar")}</button>
            </div>
          {:else}
            <p>{tr("No compatible CLI was found.", "Nenhuma CLI compatível foi encontrada.")}</p>
          {/each}
          {#if launchError}<p class="launcher-error">{launchError}</p>{/if}
        </div>
      {/if}

      {#if paletteOpen}
        <div class="command-palette-layer" transition:fade={{ duration: 120 }}>
          <button class="command-palette-backdrop" type="button" aria-label={tr("Close command palette", "Fechar paleta de comandos")} onclick={() => (paletteOpen = false)}></button>
          <div class="command-palette" role="dialog" aria-label={tr("Command palette", "Paleta de comandos")}>
            <div class="command-search">
              <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="8.5" cy="8.5" r="4.5" /><path d="m12 12 4 4" /></svg>
              <input
                data-command-palette
                value={paletteQuery}
                placeholder={tr("Search sessions and commands…", "Buscar sessões e comandos…")}
                oninput={(event) => { paletteQuery = event.currentTarget.value; paletteIndex = 0; }}
                onkeydown={handlePaletteKey}
              />
              <kbd>Esc</kbd>
            </div>
            <div class="command-results">
              {#each paletteCommands() as command, index (command.id)}
                <button class:active={paletteIndex === index} type="button" onmouseenter={() => (paletteIndex = index)} onclick={() => runPaletteCommand(command)}>
                  <span><strong>{command.label}</strong><small>{command.detail}</small></span>
                  <kbd>↵</kbd>
                </button>
              {:else}
                <p>{tr("No matching command.", "Nenhum comando encontrado.")}</p>
              {/each}
            </div>
          </div>
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
                      {#if session.permissionProfile.approvalsReviewer === "auto_review" && session.permissionProfile.mode !== "full_access"}
                        <span class="access-badge auto-review">{tr("Approve for me", "Aprovar por mim")}</span>
                      {/if}
                      {#if session.permissionProfile.mode === "full_access"}
                        <span class="access-badge full-access">{tr("Full access", "Acesso total")}</span>
                      {/if}
                    </span>
                    <span class="project-name">{session.project}</span>
                    <span class="status-line status-{session.status}">
                      {#if session.status === "running"}
                        <span class="running-dots" aria-hidden="true"><i></i><i></i><i></i></span>
                      {:else}
                        <i></i>
                      {/if}
                      {shown(session.statusLabel)}
                    </span>
                    {#if session.lastResponse && selectedId !== session.id}
                      <span class="response-preview">
                        <b>{tr("Final response", "Resposta final")}</b>
                        <span>{session.lastResponse}</span>
                      </span>
                    {/if}
                  </span>
                  <svg class="chevron" viewBox="0 0 20 20" aria-hidden="true">
                    <path d="m8 5 5 5-5 5" />
                  </svg>
                </button>

                {#if selectedId === session.id}
                  {@const capabilities = sessionCapabilities(session)}
                  <div class="session-details" transition:slide={{ duration: 190, easing: cubicOut }}>
                    <div class="capability-bar">
                      {#if capabilities.canOpenSource}
                        <button type="button" onclick={() => openSessionSource(session.id)}>{tr("Open source", "Abrir origem")}</button>
                      {/if}
                      {#if capabilities.canReadResults && session.lastResponse}
                        <button type="button" onclick={() => copyResult(`${session.id}-latest`, session.lastResponse ?? "")}>{copiedResultId === `${session.id}-latest` ? tr("Copied", "Copiado") : tr("Copy result", "Copiar resultado")}</button>
                      {/if}
                    </div>

                    {#if session.lastResponse}
                      <div class="final-response">
                        <span class="eyebrow">{tr("Final response", "Resposta final")}</span>
                        <p>{session.lastResponse}</p>
                      </div>
                    {/if}

                    {#if session.pendingPermission}
                      <div class="permission-block risk-{session.pendingPermission.risk}">
                        <strong>{shown(session.pendingPermission.summary)}</strong>
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
                    {/if}

                    {#if canContinueSession(session) && canSubmitToSession(session)}
                      <button
                        class:active={composerSessionId === session.id}
                        class="continue-trigger"
                        type="button"
                        onclick={() => toggleSessionComposer(session)}
                      >
                        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 10h11M11 6l4 4-4 4" /></svg>
                        {session.status === "waiting_for_input" ? tr("Send prompt through Lume", "Enviar prompt pelo Lume") : tr("Continue through Lume", "Continuar pelo Lume")}
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
                            aria-label={tr(`New prompt for ${session.agentLabel}`, `Novo prompt para ${session.agentLabel}`)}
                            placeholder={tr("Enter the next prompt…", "Digite o próximo prompt…")}
                            rows="2"
                          ></textarea>
                          <button disabled={!composerPrompt.trim() || composerSending} type="submit" aria-label={tr("Send prompt", "Enviar prompt")}>
                            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m4 10 12-6-4 12-2-4zM10 12l2-2" /></svg>
                          </button>
                        </form>
                        {#if composerMessage}<p class="inline-error">{composerMessage}</p>{/if}
                      {/if}
                    {/if}

                    {#if canTerminateSession(session)}
                      <div class:confirming={terminateConfirmId === session.id} class="terminate-agent-control">
                        {#if terminateConfirmId === session.id}
                          <span>{tr("Stop the agent and its running commands?", "Encerrar o agente e os comandos em execução?")}</span>
                          <button type="button" onclick={() => (terminateConfirmId = null)}>{tr("Cancel", "Cancelar")}</button>
                          <button class="danger" disabled={terminatingSessionId === session.id} type="button" onclick={() => void terminateAgent(session)}>
                            {terminatingSessionId === session.id ? tr("Stopping…", "Encerrando…") : tr("Stop", "Encerrar")}
                          </button>
                        {:else}
                          <button type="button" onclick={() => void terminateAgent(session)}>
                            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 3v7M5.5 5.5a6 6 0 1 0 9 0" /></svg>
                            {tr("Stop agent", "Encerrar agente")}
                          </button>
                        {/if}
                      </div>
                      {#if sessionActionMessage && (terminateConfirmId === session.id || terminatingSessionId === session.id)}
                        <p class="inline-error">{sessionActionMessage}</p>
                      {/if}
                    {/if}
                  </div>
                {/if}
              </article>
            {:else}
              <div class="empty-state" transition:fade>
                <span class="quiet-orbit" aria-hidden="true"><i></i></span>
                <strong>{tr("No active sessions", "Nenhuma sessão ativa")}</strong>
                <p>{tr("New sessions will appear here automatically.", "Novas sessões aparecerão aqui automaticamente.")}</p>
              </div>
            {/each}
          </div>
        {:else if view === "board"}
          <div class="whiteboard" in:fade={{ duration: 150 }}>
            <div class="board-intro">
              <strong>{tr("A separate space for each chat", "Um espaço separado para cada chat")}</strong>
              <p>{tr("Open independent mini terminals and move them close to dock them.", "Abra mini terminais independentes e aproxime um do outro para acoplá-los.")}</p>
            </div>

            <div class="layout-toolbar">
              <select
                aria-label={tr("Saved whiteboard layout", "Layout salvo do whiteboard")}
                value={selectedLayoutId ?? ""}
                onchange={(event) => {
                  selectedLayoutId = event.currentTarget.value || null;
                  layoutName = preferences.whiteboardLayouts.find((layout) => layout.id === selectedLayoutId)?.name ?? "";
                }}
              >
                <option value="">{tr("New layout", "Novo layout")}</option>
                {#each preferences.whiteboardLayouts as layout (layout.id)}
                  <option value={layout.id}>{layout.name}</option>
                {/each}
              </select>
              <input bind:value={layoutName} maxlength="48" placeholder={tr("Layout name", "Nome do layout")} />
              <button type="button" onclick={saveCurrentLayout}>{tr("Save", "Salvar")}</button>
              {#if selectedLayoutId}
                {@const selectedLayout = preferences.whiteboardLayouts.find((layout) => layout.id === selectedLayoutId)}
                <button disabled={!selectedLayout || restoringLayout} type="button" onclick={() => selectedLayout && restoreSavedLayout(selectedLayout)}>
                  {restoringLayout ? "…" : tr("Restore", "Restaurar")}
                </button>
                <button class="layout-delete" type="button" aria-label={tr("Delete layout", "Excluir layout")} onclick={() => selectedLayoutId && deleteSavedLayout(selectedLayoutId)}>×</button>
              {/if}
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
                    disabled={openingTerminal !== null || terminalIsOpen(session)}
                    type="button"
                    title={terminalIsOpen(session) ? tr("Close the terminal with X to open it again", "Feche o terminal pelo X para abri-lo novamente") : tr("Open separate terminal", "Abrir terminal separado")}
                    onclick={() => openTerminal(session)}
                  >
                    {openingTerminal === session.id ? tr("Opening…", "Abrindo…") : tr("Open", "Abrir")}
                  </button>
                </div>
              {:else}
                <p class="board-empty">{tr("Sessions will appear here when detected.", "As sessões aparecerão aqui quando forem detectadas.")}</p>
              {/each}
            </div>
            {#if terminalMessage}<p class="board-message" transition:fade>{terminalMessage}</p>{/if}
            <div class="dock-guide">
              <svg viewBox="0 0 32 20" aria-hidden="true"><rect x="2" y="3" width="12" height="14" rx="3" /><rect x="18" y="3" width="12" height="14" rx="3" /><path d="M14 10h4" /></svg>
              <span>{tr("Each window has its own prompt. Dock them side by side or above and below.", "Cada janela tem seu próprio prompt. Acople pelas laterais ou por cima e por baixo.")}</span>
            </div>
          </div>
        {:else if view === "history"}
          <div class="history-list" in:fade={{ duration: 150 }}>
            <div class="results-intro">
              <strong>{tr("Final responses from your agents", "Respostas finais dos seus agentes")}</strong>
              <p>{tr("Kept only while Lume is running.", "Mantidas apenas enquanto o Lume está aberto.")}</p>
            </div>
            {#if resultNotes.length > 0}
              <div class="settings-section-label history-label">{tr("Saved notes", "Notas salvas")}</div>
              <div class="saved-notes">
                {#each resultNotes as note (note.id)}
                  <article class="saved-note">
                    <span><strong>{note.title}</strong><small>{note.project} · {relativeTime(note.createdAt)}</small></span>
                    <p>{note.body}</p>
                    {#if note.files.length || note.tests.length}
                      <div class="artifact-summary">
                        {#if note.files.length}<span>{note.files.length} {tr("files", "arquivos")}</span>{/if}
                        {#if note.tests.length}<span>{note.tests.length} {tr("checks", "verificações")}</span>{/if}
                      </div>
                    {/if}
                    <button type="button" onclick={() => removeResultNote(note.id)}>{tr("Delete", "Excluir")}</button>
                  </article>
                {/each}
              </div>
            {/if}
            <div class="results-list">
              {#each recentResults as item (item.result.id)}
                {@const capabilities = sessionCapabilities(item.session)}
                <article class="result-card">
                  <div class="result-heading">
                    <span class="agent-avatar agent-{item.session.agent}"><BrandIcon name={item.session.agent} size={15} /></span>
                    <span><strong>{item.session.agentLabel}</strong><small>{item.session.project} · {relativeTime(item.result.createdAt)}</small></span>
                  </div>
                  <p>{item.result.response}</p>
                  {#if item.result.files?.length || item.result.tests?.length}
                    <div class="result-artifacts">
                      {#if item.result.files?.length}
                        <span><strong>{tr("Files", "Arquivos")}</strong>{item.result.files.join(" · ")}</span>
                      {/if}
                      {#if item.result.tests?.length}
                        <span><strong>{tr("Checks", "Verificações")}</strong>{item.result.tests.join(" · ")}</span>
                      {/if}
                    </div>
                  {/if}
                  <div class="result-actions">
                    <button type="button" onclick={() => copyResult(item.result.id, item.result.response)}>{copiedResultId === item.result.id ? tr("Copied", "Copiado") : tr("Copy", "Copiar")}</button>
                    <button disabled={savingNoteId === item.result.id} type="button" onclick={() => keepResultAsNote(item.session, item.result.id)}>{savingNoteId === item.result.id ? "…" : tr("Save note", "Salvar nota")}</button>
                    {#if capabilities.canPrompt && canContinueSession(item.session)}
                      <button type="button" onclick={() => continueFromResult(item.session)}>{tr("Continue", "Continuar")}</button>
                    {/if}
                    {#if capabilities.canOpenSource}
                      <button type="button" onclick={() => openSessionSource(item.session.id)}>{tr("Open source", "Abrir origem")}</button>
                    {/if}
                  </div>
                </article>
              {:else}
                <p class="results-empty">{tr("Completed agent responses will appear here.", "As respostas de agentes finalizados aparecerão aqui.")}</p>
              {/each}
            </div>
            {#if noteMessage}<p class="board-message">{noteMessage}</p>{/if}
            <div class="settings-section-label history-label">{tr("Activity", "Atividade")}</div>
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
                <strong>{tr("No activity yet", "Nenhuma atividade")}</strong>
                <p>{tr("Completions, errors, and decisions will appear here.", "Conclusões, erros e decisões aparecerão aqui.")}</p>
              </div>
            {/each}
            <p class="privacy-note">{tr("Commands, paths, and permission contents are not stored.", "Comandos, caminhos e conteúdos de permissões não são guardados.")}</p>
          </div>
        {:else}
          <div class="settings" in:fade={{ duration: 150 }}>
            <div class="settings-section-label">{tr("Agents", "Agentes")}</div>
            {#each integrations as integration}
              {@const diagnostic = integrationDiagnostics[integration.kind]}
              <div class="integration-row">
                <span class="agent-avatar agent-{integration.kind}"><BrandIcon name={integration.kind} size={18} /></span>
                <div>
                  <strong>{integration.label}</strong>
                  <span>{shown(integration.detail)}</span>
                </div>
                <div class="integration-actions">
                  <button
                    class="diagnose-button"
                    disabled={diagnosingIntegration !== null}
                    type="button"
                    onclick={() => runIntegrationDiagnostic(integration)}
                  >{diagnosingIntegration === integration.kind ? "…" : tr("Test", "Testar")}</button>
                  <button
                    class:connected={integration.configured}
                    disabled={!integration.installed || configuringIntegration === integration.kind}
                    type="button"
                    onclick={() => toggleIntegration(integration)}
                  >
                    {configuringIntegration === integration.kind
                      ? "…"
                      : integration.configured
                        ? tr("Connected", "Conectado")
                        : tr("Connect", "Conectar")}
                  </button>
                </div>
              </div>
              {#if diagnostic}
                <div class:healthy={diagnostic.healthy} class="diagnostic-card" transition:slide={{ duration: 150, easing: cubicOut }}>
                  {#each diagnostic.checks as check (check.id)}
                    <div class="diagnostic-check status-{check.status}">
                      <i aria-hidden="true"></i>
                      <span><strong>{shown(check.label)}</strong><small>{check.id === "activity" && diagnostic.lastEventAt ? relativeTime(diagnostic.lastEventAt) : shown(check.detail)}</small></span>
                    </div>
                  {/each}
                </div>
              {/if}
            {/each}
            <div class="settings-section-label preferences-label">{tr("External detectors", "Detectores externos")}</div>
            {#each externalPlugins as plugin (plugin.id)}
              <div class="integration-row external-plugin-row">
                <span class="agent-avatar agent-unknown"><BrandIcon name="unknown" size={17} /></span>
                <div><strong>{plugin.name}</strong><span>{plugin.executable} · {plugin.id}</span></div>
                <button type="button" onclick={() => uninstallExternalPlugin(plugin.id)}>{tr("Remove", "Remover")}</button>
              </div>
            {:else}
              <p class="profile-empty">{tr("Install a JSON manifest to monitor another CLI process.", "Instale um manifesto JSON para monitorar outro processo CLI.")}</p>
            {/each}
            <div class="plugin-actions">
              <button disabled={installingPlugin} type="button" onclick={addExternalPlugin}>{installingPlugin ? "…" : tr("Install manifest", "Instalar manifesto")}</button>
              <button type="button" onclick={openPluginFolder}>{tr("Open folder", "Abrir pasta")}</button>
            </div>
            {#if pluginMessage}<p class="browser-path">{pluginMessage}</p>{/if}
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
                <span>{shown(vscodeStatus.detail)}</span>
              </div>
              <button
                class:connected={vscodeStatus.configured}
                disabled={!vscodeStatus.installed || configuringVscode}
                type="button"
                onclick={toggleVscode}
              >{configuringVscode ? "…" : vscodeStatus.configured ? tr("Connected", "Conectado") : tr("Connect", "Conectar")}</button>
            </div>
            <div class="integration-row browser-row">
              <span class="agent-avatar agent-browser"><BrandIcon name="browsers" size={21} /></span>
              <div>
                <strong>Chrome, Edge & Brave</strong>
                <span>{tr("Load the folder as an unpacked extension.", "Carregue a pasta como extensão descompactada.")}</span>
              </div>
              <button type="button" onclick={openBrowserCompanion}>{tr("Open folder", "Abrir pasta")}</button>
            </div>
            {#if browserCompanionPath}
              <p class="browser-path" transition:fade>{browserCompanionPath}</p>
            {/if}
            <div class="settings-section-label preferences-label">{tr("Preferences", "Preferências")}</div>
            <label class="field-row">
              <span><strong>{tr("Language", "Idioma")}</strong><small>{tr("Lume interface language.", "Idioma da interface do Lume.")}</small></span>
              <select
                value={preferences.language}
                onchange={(event) =>
                  updatePreference("language", event.currentTarget.value as Preferences["language"])}
              >
                <option value="en">English</option>
                <option value="pt-BR">Português</option>
              </select>
            </label>
            <div class="setting-row">
              <div><strong>{tr("Dark mode", "Modo escuro")}</strong><span>{tr("Switch between the light and dark appearance.", "Alterne entre a aparência clara e escura.")}</span></div>
              <label class="switch">
                <input
                  type="checkbox"
                  checked={effectiveDark}
                  onchange={(event) =>
                    updatePreference("darkMode", event.currentTarget.checked)}
                />
                <span></span>
              </label>
            </div>
            <div class="setting-row">
              <div><strong>{tr("Start with the system", "Iniciar com o sistema")}</strong><span>{tr("Lume stays available in the system tray.", "Lume fica disponível na bandeja.")}</span></div>
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
              <div><strong>{tr("Subtle sounds", "Sons sutis")}</strong><span>{tr("Only when a task finishes, fails, or requests permission.", "Apenas ao finalizar, encontrar erro ou pedir permissão.")}</span></div>
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
              <div><strong>{tr("Show over fullscreen", "Sobre tela cheia")}</strong><span>{tr("Keep disabled to avoid videos and games.", "Desativado evita vídeos e jogos.")}</span></div>
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
              <span><strong>{tr("Monitor", "Monitor")}</strong><small>{tr("The primary display is used by default.", "O principal é usado por padrão.")}</small></span>
              <select
                value={preferences.monitorId ?? ""}
                onchange={(event) =>
                  updatePreference("monitorId", event.currentTarget.value || undefined)}
              >
                <option value="">{tr("Primary", "Principal")}</option>
                {#each monitors as monitor}
                  <option value={monitor.id}>{monitor.label}</option>
                {/each}
              </select>
            </label>
            <label class="field-row">
              <span><strong>{tr("History", "Histórico")}</strong><small>{tr("Local, sanitized summaries.", "Resumos locais e sanitizados.")}</small></span>
              <select
                value={preferences.historyRetentionDays}
                onchange={(event) =>
                  updatePreference(
                    "historyRetentionDays",
                    Number(event.currentTarget.value),
                  )}
              >
                <option value={7}>{tr("7 days", "7 dias")}</option>
                <option value={30}>{tr("30 days", "30 dias")}</option>
                <option value={90}>{tr("90 days", "90 dias")}</option>
              </select>
            </label>
            <div class="launch-setting">
              <span><strong>{tr("Open sessions in", "Abrir sessões em")}</strong><small>{tr("Use your usual tool.", "Use sua ferramenta habitual.")}</small></span>
              <div class="segmented" aria-label={tr("Session destination", "Destino das sessões")}>
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
            <div class="field-row shortcut-row">
              <span><strong>{tr("Command palette", "Paleta de comandos")}</strong><small>{tr("Available even when Lume is hidden.", "Disponível mesmo com o Lume oculto.")}</small></span>
              <kbd>{preferences.globalShortcut}</kbd>
            </div>
            <div class="settings-section-label preferences-label">{tr("Project profiles", "Perfis por projeto")}</div>
            {#if detectedProjects.length > 0}
              <label class="field-row">
                <span><strong>{tr("Project", "Projeto")}</strong><small>{tr("Overrides only for this project.", "Ajustes somente para este projeto.")}</small></span>
                <select
                  value={selectedProfileKey ?? ""}
                  onchange={(event) => (selectedProfileKey = event.currentTarget.value)}
                >
                  {#each detectedProjects as project (project.key)}
                    <option value={project.key}>{project.label}</option>
                  {/each}
                </select>
              </label>
              <div class="setting-row">
                <div><strong>{tr("Project sounds", "Sons do projeto")}</strong><span>{tr("Allow completion, error, and permission sounds.", "Permite sons de conclusão, erro e permissão.")}</span></div>
                <label class="switch">
                  <input
                    type="checkbox"
                    checked={selectedProjectProfile?.soundEnabled ?? true}
                    onchange={(event) =>
                      updateSelectedProjectProfile({ soundEnabled: event.currentTarget.checked })}
                  />
                  <span></span>
                </label>
              </div>
              <div class="launch-setting project-launch-setting">
                <span><strong>{tr("Session destination", "Destino das sessões")}</strong><small>{tr("Override the global destination.", "Substitui o destino global.")}</small></span>
                <div class="segmented" aria-label={tr("Project session destination", "Destino das sessões do projeto")}>
                  {#each [["", tr("Global", "Global")], ["auto", "Auto"], ["terminal", "Terminal"], ["vscode", "VS Code"]] as option}
                    <button
                      class:active={(selectedProjectProfile?.launchTarget ?? "") === option[0]}
                      type="button"
                      onclick={() =>
                        updateSelectedProjectProfile({ launchTarget: option[0] ? option[0] as Preferences["launchTarget"] : undefined })}
                    >{option[1]}</button>
                  {/each}
                </div>
              </div>
              <label class="field-row">
                <span><strong>{tr("Profile monitor", "Monitor do perfil")}</strong><small>{tr("Where this project should appear.", "Onde este projeto deve aparecer.")}</small></span>
                <select value={selectedProjectProfile?.monitorId ?? ""} onchange={(event) => updateSelectedProjectProfile({ monitorId: event.currentTarget.value || undefined })}>
                  <option value="">{tr("Global", "Global")}</option>
                  {#each monitors as monitor}
                    <option value={monitor.id}>{monitor.label}</option>
                  {/each}
                </select>
              </label>
              <div class="setting-row">
                <div><strong>{tr("Capsule position", "Posição da cápsula")}</strong><span>{selectedProjectProfile?.overlayX !== undefined ? `${selectedProjectProfile.overlayX}, ${selectedProjectProfile.overlayY}` : tr("Use the global position", "Usar a posição global")}</span></div>
                <button class="profile-action" type="button" onclick={captureProfilePosition}>{tr("Use current", "Usar atual")}</button>
              </div>
              <label class="field-row">
                <span><strong>{tr("Permission preset", "Preset de permissão")}</strong><small>{tr("Applied only when Lume starts a new session.", "Aplicado apenas ao iniciar uma nova sessão pelo Lume.")}</small></span>
                <select value={selectedProjectProfile?.permissionMode ?? ""} onchange={(event) => updateSelectedProjectProfile({ permissionMode: (event.currentTarget.value || undefined) as Preferences["projectProfiles"][string]["permissionMode"] })}>
                  <option value="">{tr("Agent default", "Padrão do agente")}</option>
                  <option value="plan">Plan</option>
                  <option value="read_only">{tr("Read only", "Somente leitura")}</option>
                  <option value="workspace_write">Workspace write</option>
                  <option value="full_access">{tr("Full access — no sandbox", "Acesso total — sem sandbox")}</option>
                </select>
              </label>
              <label class="field-row">
                <span><strong>{tr("Approval policy", "Política de aprovação")}</strong><small>{tr("Supported by Codex launch profiles.", "Suportada nos perfis de abertura do Codex.")}</small></span>
                <select value={selectedProjectProfile?.approvalPolicy ?? ""} onchange={(event) => updateSelectedProjectProfile({ approvalPolicy: (event.currentTarget.value || undefined) as Preferences["projectProfiles"][string]["approvalPolicy"] })}>
                  <option value="">{tr("Agent default", "Padrão do agente")}</option>
                  <option value="untrusted">Untrusted</option>
                  <option value="on-request">On request</option>
                  <option value="never">Never</option>
                </select>
              </label>
              <label class="field-row">
                <span><strong>Whiteboard</strong><small>{tr("Default saved layout for this project.", "Layout salvo padrão deste projeto.")}</small></span>
                <select value={selectedProjectProfile?.whiteboardLayoutId ?? ""} onchange={(event) => updateSelectedProjectProfile({ whiteboardLayoutId: event.currentTarget.value || undefined })}>
                  <option value="">{tr("No layout", "Sem layout")}</option>
                  {#each preferences.whiteboardLayouts as layout (layout.id)}
                    <option value={layout.id}>{layout.name}</option>
                  {/each}
                </select>
              </label>
              <div class="launch-setting preferred-agents-setting">
                <span><strong>{tr("Preferred agents", "Agentes preferidos")}</strong><small>{tr("Shown first in the launcher.", "Aparecem primeiro no iniciador.")}</small></span>
                <div class="agent-preferences">
                  {#each integrations as integration (integration.kind)}
                    <button class:active={(selectedProjectProfile?.preferredAgents ?? []).includes(integration.kind)} type="button" onclick={() => togglePreferredAgent(integration.kind)}>
                      <BrandIcon name={integration.kind} size={14} />{integration.label}
                    </button>
                  {/each}
                </div>
              </div>
              <button class="apply-profile-button" type="button" onclick={applySelectedProjectProfile}>{tr("Apply project profile", "Aplicar perfil do projeto")}</button>
            {:else}
              <p class="profile-empty">{tr("Profiles appear after a project is detected.", "Os perfis aparecem depois que um projeto é detectado.")}</p>
            {/if}
            <div class="settings-section-label preferences-label">{tr("About", "Sobre")}</div>
            <div class="update-card" aria-live="polite">
              <div class="update-main">
                <LumeLogo size={30} />
                <div class="update-copy">
                  <strong>Lume</strong>
                  <span>{tr("Version", "Versão")} {appVersion}</span>
                </div>
                {#if updateState === "available"}
                  <button class="update-available" type="button" onclick={installAvailableUpdate}>
                    {tr("Update to", "Atualizar para")} {availableVersion}
                  </button>
                {:else}
                  <button
                    type="button"
                    disabled={updateState === "checking" || updateState === "downloading" || updateState === "ready"}
                    onclick={checkForUpdates}
                  >
                    {updateState === "checking"
                      ? tr("Checking…", "Verificando…")
                      : updateState === "downloading"
                        ? updateProgress === null
                          ? tr("Downloading…", "Baixando…")
                          : `${updateProgress}%`
                        : updateState === "ready"
                          ? tr("Restarting…", "Reiniciando…")
                          : tr("Check", "Verificar")}
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
            <span class:visible={savingSettings} class="save-state">{tr("Saving…", "Salvando…")}</span>
          </div>
        {/if}
      </div>

      <footer>
        <button
          class:active={view === "sessions"}
          type="button"
          onclick={() => openView("sessions")}
          aria-label={tr("Sessions", "Sessões")}
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <circle cx="6" cy="10" r="2.5" /><circle cx="14" cy="10" r="2.5" />
          </svg>
          <span>{tr("Sessions", "Sessões")}</span>
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
          <span>{tr("Board", "Mesa")}</span>
        </button>
        <button
          class:active={view === "history"}
          type="button"
          onclick={() => openView("history")}
          aria-label={tr("Results", "Resultados")}
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M4.5 5.5h11M4.5 10h11M4.5 14.5h7" />
          </svg>
          <span>{tr("Results", "Resultados")}</span>
        </button>
        <button
          class:active={view === "settings"}
          class:has-update={updateState === "available"}
          type="button"
          onclick={() => openView("settings")}
          aria-label={tr("Settings", "Configurações")}
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <circle cx="10" cy="10" r="3" />
            <path d="M10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M15.3 4.7l-1.4 1.4M6.1 13.9l-1.4 1.4" />
          </svg>
          <span>{tr("Settings", "Ajustes")}</span>
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
  input,
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
    background: rgba(249, 251, 250, 0.985);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.34), inset 0 -5px 12px rgba(43, 64, 55, 0.035);
    cursor: pointer;
    touch-action: none;
    transition: border-color 160ms ease, background-color 160ms ease;
  }

  .lume-orb:hover {
    border-color: rgba(79, 116, 99, 0.3);
  }

  .lume-orb:active { background: rgba(245, 249, 247, 0.99); }
  .lume-orb.dragging { cursor: grabbing; }

  .status-permission_required { color: #ae6b24; }
  .status-failed { color: #a84d4d; }
  .status-completed { color: #4f966b; }
  .status-idle { color: #829089; }

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
  }

  .panel-content,
  .panel footer,
  .panel .brand-lockup > div,
  .panel .header-actions,
  .panel .launcher-popover {
    transition: opacity 150ms ease;
  }
  .panel:not(.content-visible) .panel-content,
  .panel:not(.content-visible) footer,
  .panel:not(.content-visible) .brand-lockup > div,
  .panel:not(.content-visible) .header-actions,
  .panel:not(.content-visible) .launcher-popover {
    opacity: 0;
    pointer-events: none;
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

  .add-button,
  .palette-button,
  .collapse-button {
    border: 0;
    color: #697872;
    background: transparent;
    cursor: pointer;
  }

  .collapse-button {
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border-radius: 10px;
  }

  .header-actions { display: flex; align-items: center; gap: 2px; }
  .add-button, .palette-button { width: 32px; height: 32px; display: grid; place-items: center; border-radius: 10px; }
  .add-button:hover,
  .add-button.active { color: #486d5e; background: rgba(80, 103, 94, 0.07); }

  .add-button:hover,
  .palette-button:hover,
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
  .command-palette-layer { position: absolute; z-index: 12; inset: 0 0 16px; display: grid; place-items: start center; padding-top: 66px; }
  .command-palette-backdrop { position: absolute; inset: 0; width: 100%; border: 0; background: rgba(21, 31, 27, 0.2); backdrop-filter: blur(3px); cursor: default; }
  .command-palette { position: relative; width: calc(100% - 30px); overflow: hidden; border: 1px solid rgba(89, 111, 101, 0.16); border-radius: 15px; background: rgba(250, 252, 251, 0.98); box-shadow: 0 18px 45px rgba(24, 38, 32, 0.24); }
  .command-search { height: 43px; padding: 0 10px; display: flex; align-items: center; gap: 8px; border-bottom: 1px solid rgba(91, 112, 102, 0.1); }
  .command-search svg { width: 15px; height: 15px; flex: 0 0 auto; fill: none; stroke: #6f8179; stroke-width: 1.5; }
  .command-search input { min-width: 0; flex: 1; border: 0; outline: 0; color: #304039; background: transparent; font: inherit; font-size: 10px; }
  .command-palette kbd, .shortcut-row kbd { padding: 3px 5px; border: 1px solid rgba(92, 112, 103, 0.13); border-radius: 5px; color: #7b8983; background: rgba(80, 105, 94, 0.045); font-family: inherit; font-size: 7px; }
  .command-results { max-height: 265px; padding: 5px; overflow-y: auto; }
  .command-results > button { width: 100%; min-height: 42px; padding: 6px 8px; display: flex; align-items: center; gap: 8px; border: 0; border-radius: 9px; color: inherit; background: transparent; text-align: left; cursor: pointer; }
  .command-results > button.active { background: rgba(78, 109, 95, 0.075); }
  .command-results > button span { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .command-results strong { overflow: hidden; color: #34443d; font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .command-results small, .command-results > p { margin: 0; color: #89958f; font-size: 8px; }
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
  .session-title-row { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 6px; }
  .session-title-row strong { color: #27342f; font-size: 11px; }
  .source-label { display: inline-flex; align-items: center; gap: 3px; padding: 2px 5px; border-radius: 999px; color: #718079; background: rgba(80, 104, 94, 0.075); font-size: 8px; font-weight: 720; letter-spacing: 0.045em; line-height: 1.25; text-transform: uppercase; }
  .access-badge { padding: 2px 5px; border: 1px solid transparent; border-radius: 999px; font-size: 7px; font-weight: 760; letter-spacing: 0.025em; line-height: 1.25; white-space: nowrap; }
  .access-badge.auto-review { border-color: rgba(80, 120, 170, 0.12); color: #5579a3; background: rgba(80, 120, 170, 0.08); }
  .access-badge.full-access { border-color: rgba(177, 115, 65, 0.13); color: #9b663d; background: rgba(177, 115, 65, 0.09); }
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
  .status-line.status-completed { color: #4f966b; }
  .status-line.status-completed > i { background: #59aa78; box-shadow: 0 0 0 3px rgba(89, 170, 120, 0.1); }
  .status-line.status-failed > i { background: #b95454; }
  .status-line.status-waiting_for_input { color: #a87925; }
  .status-line.status-waiting_for_input > i { background: #c99a3f; }
  .response-preview { min-width: 0; margin-top: 4px; padding: 6px 7px; display: grid; gap: 2px; border-left: 2px solid rgba(77, 117, 99, 0.22); border-radius: 0 7px 7px 0; color: #697771; background: rgba(73, 102, 89, 0.035); }
  .response-preview b { color: #668075; font-size: 7px; letter-spacing: 0.055em; text-transform: uppercase; }
  .response-preview span { overflow: hidden; display: -webkit-box; font-size: 9px; line-height: 1.35; line-clamp: 2; overflow-wrap: anywhere; -webkit-box-orient: vertical; -webkit-line-clamp: 2; }

  @keyframes status-dot-bounce {
    0%, 60%, 100% { opacity: 0.48; transform: translateY(1px); }
    30% { opacity: 1; transform: translateY(-2px); }
  }

  .chevron { width: 13px; height: 13px; color: #98a19d; transition: transform 180ms ease; }
  .selected .chevron { transform: rotate(90deg); }

  .session-details { padding: 0 2px 13px 43px; }
  .capability-bar { margin: -2px 0 9px; display: flex; align-items: center; gap: 5px; }
  .capability-bar button { height: 24px; padding: 0 7px; border: 1px solid rgba(83, 108, 97, 0.11); border-radius: 7px; color: #63786e; background: rgba(77, 105, 92, 0.035); font-size: 8px; font-weight: 700; cursor: pointer; }
  .capability-bar button:hover { background: rgba(77, 105, 92, 0.08); }

  .permission-block { padding-left: 11px; border-left: 2px solid #d49350; display: grid; gap: 6px; }
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

  .final-response { margin: 0 0 10px; padding: 9px 10px; border: 1px solid rgba(78, 105, 93, 0.1); border-radius: 10px; background: rgba(73, 102, 89, 0.035); }
  .final-response .eyebrow { display: block; margin-bottom: 5px; color: #668075; font-size: 8px; font-weight: 780; letter-spacing: 0.055em; text-transform: uppercase; }
  .final-response p { max-height: 150px; margin: 0; overflow-y: auto; color: #43524c; font-size: 10px; line-height: 1.5; overflow-wrap: anywhere; white-space: pre-wrap; scrollbar-width: thin; }

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
  .terminate-agent-control { margin-top: 11px; display: flex; align-items: center; gap: 6px; }
  .terminate-agent-control > button { padding: 0; display: inline-flex; align-items: center; gap: 5px; border: 0; color: #9a5c59; background: transparent; font-size: 9px; font-weight: 720; cursor: pointer; }
  .terminate-agent-control > button svg { width: 13px; height: 13px; }
  .terminate-agent-control.confirming { padding: 7px 8px; border: 1px solid rgba(166, 77, 77, 0.13); border-radius: 9px; background: rgba(166, 77, 77, 0.035); }
  .terminate-agent-control.confirming span { min-width: 0; flex: 1; color: #755b57; font-size: 9px; line-height: 1.35; }
  .terminate-agent-control.confirming button { min-height: 24px; padding: 0 7px; border: 1px solid rgba(91, 107, 100, 0.13); border-radius: 7px; color: #627068; background: rgba(255, 255, 255, 0.42); font-size: 8px; font-weight: 700; cursor: pointer; }
  .terminate-agent-control.confirming button.danger { border-color: rgba(166, 77, 77, 0.2); color: #a54c4c; }
  .terminate-agent-control button:disabled { opacity: 0.45; cursor: default; }

  .whiteboard { max-height: 431px; min-height: 0; padding: 7px 16px 15px; display: flex; flex-direction: column; overflow: hidden; }
  .board-intro { padding: 8px 1px 14px; border-bottom: 1px solid rgba(105, 123, 115, 0.1); }
  .board-intro strong { color: #2d3a35; font-size: 12px; }
  .board-intro p { margin: 4px 0 0; color: #7f8a85; font-size: 9px; line-height: 1.45; }
  .layout-toolbar { padding: 8px 0 4px; display: grid; grid-template-columns: minmax(72px, 1fr) minmax(70px, 1fr) auto auto auto; gap: 4px; }
  .layout-toolbar select, .layout-toolbar input { min-width: 0; height: 27px; padding: 0 6px; border: 1px solid rgba(87, 109, 99, 0.13); border-radius: 7px; outline: 0; color: #52625b; background: rgba(255, 255, 255, 0.42); font: inherit; font-size: 8px; }
  .layout-toolbar button { height: 27px; padding: 0 6px; border: 1px solid rgba(87, 109, 99, 0.13); border-radius: 7px; color: #567165; background: rgba(255, 255, 255, 0.4); font-size: 8px; cursor: pointer; }
  .layout-toolbar .layout-delete { color: #a45a58; }
  .terminal-picker { min-height: 0; padding: 9px 0 6px; flex: 1 1 auto; overflow-x: hidden; overflow-y: auto; overscroll-behavior: contain; scrollbar-gutter: stable; scrollbar-width: thin; scrollbar-color: #cad2ce transparent; }
  .terminal-picker-row { min-height: 59px; display: flex; align-items: center; gap: 8px; border-bottom: 1px solid rgba(105, 123, 115, 0.09); }
  .terminal-picker-row:last-child { border-bottom: 0; }
  .terminal-picker-copy { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .terminal-picker-copy strong { color: #35423d; font-size: 10px; }
  .terminal-picker-copy small { overflow: hidden; color: #89938f; font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .terminal-picker-row > button { min-width: 52px; height: 28px; padding: 0 9px; border: 1px solid rgba(82, 105, 95, 0.16); border-radius: 9px; color: #4d6f61; background: rgba(255, 255, 255, 0.38); font-size: 9px; font-weight: 720; cursor: pointer; transition: transform 140ms ease, background 140ms ease; }
  .terminal-picker-row > button:hover:not(:disabled) { transform: translateY(-1px); background: white; }
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
  .results-intro { padding: 8px 1px 12px; border-bottom: 1px solid rgba(105, 123, 115, 0.1); }
  .results-intro strong { color: #2d3a35; font-size: 12px; }
  .results-intro p { margin: 4px 0 0; color: #7f8a85; font-size: 9px; }
  .results-list { display: grid; gap: 8px; padding: 10px 0 3px; }
  .result-card { padding: 9px 10px; border: 1px solid rgba(91, 115, 104, 0.1); border-radius: 11px; background: rgba(75, 105, 91, 0.03); }
  .result-heading { display: flex; align-items: center; gap: 7px; }
  .result-heading .agent-avatar { width: 25px; height: 25px; border-radius: 8px; }
  .result-heading > span:last-child { min-width: 0; display: grid; gap: 1px; }
  .result-heading strong { color: #34443d; font-size: 9px; }
  .result-heading small { overflow: hidden; color: #87928d; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .result-card > p { max-height: 78px; margin: 8px 0; overflow: hidden; display: -webkit-box; color: #52615b; font-size: 9px; line-height: 1.45; line-clamp: 4; overflow-wrap: anywhere; white-space: pre-wrap; -webkit-box-orient: vertical; -webkit-line-clamp: 4; }
  .result-artifacts { margin: 0 0 8px; display: grid; gap: 4px; }
  .result-artifacts span { overflow: hidden; color: #78867f; font-size: 8px; line-height: 1.35; text-overflow: ellipsis; white-space: nowrap; }
  .result-artifacts strong { margin-right: 5px; color: #60766c; font-size: 7px; text-transform: uppercase; }
  .result-actions { display: flex; align-items: center; gap: 5px; }
  .result-actions button { height: 24px; padding: 0 7px; border: 1px solid rgba(84, 109, 98, 0.12); border-radius: 7px; color: #5e756b; background: rgba(255, 255, 255, 0.36); font-size: 8px; font-weight: 700; cursor: pointer; }
  .result-actions button:hover { background: rgba(255, 255, 255, 0.72); }
  .results-empty { margin: 8px 2px 4px; color: #89938f; font-size: 9px; }
  .saved-notes { display: grid; gap: 6px; }
  .saved-note { position: relative; padding: 9px 34px 9px 10px; border: 1px solid rgba(83, 112, 99, 0.12); border-radius: 10px; background: rgba(244, 239, 198, 0.16); }
  .saved-note > span { display: grid; gap: 1px; }
  .saved-note strong { color: #4c5d55; font-size: 9px; }
  .saved-note small { color: #8a958f; font-size: 8px; }
  .saved-note p { max-height: 42px; margin: 6px 0; overflow: hidden; color: #65736d; font-size: 8px; line-height: 1.4; }
  .saved-note > button { position: absolute; top: 7px; right: 7px; padding: 3px; border: 0; color: #9a7771; background: transparent; font-size: 7px; cursor: pointer; }
  .artifact-summary { display: flex; gap: 5px; }
  .artifact-summary span { padding: 2px 4px; border-radius: 5px; color: #73837b; background: rgba(77, 105, 92, 0.06); font-size: 7px; }
  .history-label { margin-top: 7px; }
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
  .integration-row > div:not(.integration-actions) { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .integration-row strong { color: #35423d; font-size: 10px; }
  .integration-row div span { overflow: hidden; color: #89938f; font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .integration-row button { min-width: 63px; height: 27px; padding: 0 8px; border: 1px solid rgba(82, 105, 95, 0.14); border-radius: 8px; color: #577064; background: transparent; font-size: 9px; font-weight: 680; cursor: pointer; transition: background 150ms ease, color 150ms ease, transform 150ms ease; }
  .settings-feedback { margin: -2px 16px 9px; color: #65736c; font-size: 9px; line-height: 1.45; }
  .settings-feedback.error { color: #a34f4f; }
  .integration-row button:hover:not(:disabled) { transform: translateY(-1px); background: rgba(82, 112, 99, 0.06); }
  .integration-row button.connected { border-color: transparent; color: #6d7e76; }
  .integration-row button:disabled { cursor: default; opacity: 0.5; }
  .integration-actions { display: flex; align-items: center; gap: 4px; }
  .integration-actions .diagnose-button { min-width: 42px; padding: 0 6px; border-color: transparent; color: #78877f; }
  .diagnostic-card { margin: -1px 0 7px 38px; padding: 7px 8px; display: grid; gap: 6px; border: 1px solid rgba(93, 113, 104, 0.1); border-radius: 9px; background: rgba(75, 103, 90, 0.03); }
  .diagnostic-check { min-width: 0; display: flex; align-items: flex-start; gap: 7px; }
  .diagnostic-check > i { width: 6px; height: 6px; margin-top: 3px; flex: 0 0 auto; border-radius: 50%; background: #789487; }
  .diagnostic-check.status-warning > i { background: #c3933c; }
  .diagnostic-check.status-error > i { background: #bd5c59; }
  .diagnostic-check > span { min-width: 0; display: grid; gap: 1px; }
  .diagnostic-check strong { color: #4c5c55; font-size: 8px; }
  .diagnostic-check small { overflow: hidden; color: #89958f; font-size: 8px; line-height: 1.35; text-overflow: ellipsis; white-space: nowrap; }
  .browser-row button { min-width: 68px; }
  .browser-path { margin: 7px 2px 0; overflow-wrap: anywhere; color: #89938f; font-size: 9px; line-height: 1.4; }
  .plugin-actions { padding-top: 8px; display: flex; gap: 5px; }
  .plugin-actions button, .profile-action, .apply-profile-button { min-height: 27px; padding: 0 8px; border: 1px solid rgba(82, 105, 95, 0.14); border-radius: 8px; color: #577064; background: transparent; font-size: 8px; font-weight: 680; cursor: pointer; }
  .external-plugin-row > button { min-width: 54px; }
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
  .project-launch-setting .segmented { grid-template-columns: repeat(4, 1fr); }
  .preferred-agents-setting { border-bottom: 1px solid rgba(105, 123, 115, 0.1); }
  .agent-preferences { display: flex; flex-wrap: wrap; gap: 5px; }
  .agent-preferences button { min-height: 29px; padding: 0 8px; display: inline-flex; align-items: center; gap: 5px; border: 1px solid rgba(83, 107, 97, 0.12); border-radius: 8px; color: #74817b; background: transparent; font-size: 8px; cursor: pointer; }
  .agent-preferences button.active { color: #3f6656; border-color: rgba(72, 114, 96, 0.24); background: rgba(72, 114, 96, 0.07); }
  .apply-profile-button { width: 100%; margin-top: 10px; color: #f6fbf8; border-color: #527c6c; background: #527c6c; }
  .shortcut-row kbd { font-size: 8px; white-space: nowrap; }
  .profile-empty { margin: 5px 1px 2px; color: #89938f; font-size: 9px; line-height: 1.45; }
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

  .overlay-shell.dark { color-scheme: dark; }
  .overlay-shell.dark .lume-orb,
  .overlay-shell.dark .panel,
  .overlay-shell.dark .launcher-popover { color: #dfe8e3; border-color: rgba(190, 209, 200, 0.13); background: rgba(27, 34, 31, 0.96); }
  .overlay-shell.dark .brand-lockup strong,
  .overlay-shell.dark .session-title-row strong,
  .overlay-shell.dark .board-intro strong,
  .overlay-shell.dark .terminal-picker-copy strong,
  .overlay-shell.dark .history-row strong,
  .overlay-shell.dark .setting-row strong,
  .overlay-shell.dark .integration-row strong,
  .overlay-shell.dark .field-row strong,
  .overlay-shell.dark .launch-setting strong { color: #e3ebe7; }
  .overlay-shell.dark .update-copy strong { color: #e3ebe7; }
  .overlay-shell.dark .launcher-row strong { color: #dfe8e3; }
  .overlay-shell.dark .panel-header,
  .overlay-shell.dark footer,
  .overlay-shell.dark .session-row,
  .overlay-shell.dark .history-row,
  .overlay-shell.dark .setting-row,
  .overlay-shell.dark .field-row { border-color: rgba(190, 209, 200, 0.09); }
  .overlay-shell.dark .session-row:hover,
  .overlay-shell.dark .session-row.selected { background: rgba(198, 218, 208, 0.045); }
  .overlay-shell.dark .project-name,
  .overlay-shell.dark .board-intro p,
  .overlay-shell.dark .terminal-picker-copy small,
  .overlay-shell.dark .history-row span,
  .overlay-shell.dark .settings-feedback { color: #adbab4; }
  .overlay-shell.dark .settings-feedback.error { color: #d68d8d; }
  .overlay-shell.dark .update-card { border-color: rgba(190, 209, 200, 0.09); background: rgba(216, 229, 223, 0.035); }
  .overlay-shell.dark .update-card p.error { color: #d68d8d; }
  .overlay-shell.dark .diagnostic-card,
  .overlay-shell.dark .result-card { border-color: rgba(190, 209, 200, 0.09); background: rgba(216, 229, 223, 0.035); }
  .overlay-shell.dark .diagnostic-check strong,
  .overlay-shell.dark .result-heading strong,
  .overlay-shell.dark .results-intro strong { color: #dce7e1; }
  .overlay-shell.dark .diagnostic-check small,
  .overlay-shell.dark .result-heading small,
  .overlay-shell.dark .results-intro p,
  .overlay-shell.dark .result-card > p { color: #aebdb5; }
  .overlay-shell.dark .result-actions button,
  .overlay-shell.dark .capability-bar button { color: #b9c8c0; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .empty-state strong { color: #c5d0cb; }
  .overlay-shell.dark code,
  .overlay-shell.dark .segmented { color: #bdc8c3; background: rgba(216, 229, 223, 0.06); }
  .overlay-shell.dark .permission-block > strong { color: #e2d0bd; }
  .overlay-shell.dark .permission-actions button,
  .overlay-shell.dark .field-row select,
  .overlay-shell.dark .inline-composer textarea { color: #c5d0cb; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .final-response,
  .overlay-shell.dark .response-preview { border-color: rgba(203, 221, 212, 0.08); background: rgba(210, 230, 220, 0.035); }
  .overlay-shell.dark .final-response .eyebrow,
  .overlay-shell.dark .response-preview b { color: #8ca69a; }
  .overlay-shell.dark .final-response p,
  .overlay-shell.dark .response-preview span { color: #c2d0c9; }
  .overlay-shell.dark .source-label { color: #9daca5; background: rgba(205, 222, 213, 0.08); }
  .overlay-shell.dark .access-badge.auto-review { border-color: rgba(123, 165, 211, 0.16); color: #9ab9d9; background: rgba(92, 137, 187, 0.12); }
  .overlay-shell.dark .access-badge.full-access { border-color: rgba(216, 157, 105, 0.17); color: #d4a77f; background: rgba(186, 122, 71, 0.12); }
  .overlay-shell.dark .board-intro,
  .overlay-shell.dark .terminal-picker-row,
  .overlay-shell.dark .dock-guide { border-color: rgba(190, 209, 200, 0.09); }
  .overlay-shell.dark .terminal-picker-row > button { color: #b7c4be; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .terminal-picker-row > button:hover:not(:disabled) { background: rgba(222, 233, 228, 0.09); }
  .overlay-shell.dark .permission-actions button:hover { background: rgba(222, 233, 228, 0.09); }
  .overlay-shell.dark .segmented button.active { color: #dfe8e3; background: rgba(214, 229, 221, 0.1); }
  .overlay-shell.dark .command-palette { border-color: rgba(207, 223, 215, 0.13); background: rgba(27, 34, 31, 0.985); }
  .overlay-shell.dark .command-search { border-color: rgba(207, 223, 215, 0.09); }
  .overlay-shell.dark .command-search input,
  .overlay-shell.dark .command-results strong { color: #dce7e1; }
  .overlay-shell.dark .command-results > button.active { background: rgba(213, 229, 221, 0.07); }
  .overlay-shell.dark .layout-toolbar select,
  .overlay-shell.dark .layout-toolbar input,
  .overlay-shell.dark .layout-toolbar button,
  .overlay-shell.dark .plugin-actions button,
  .overlay-shell.dark .profile-action,
  .overlay-shell.dark .agent-preferences button { color: #bdcbc4; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .saved-note { border-color: rgba(207, 223, 215, 0.1); background: rgba(226, 211, 121, 0.04); }
  .overlay-shell.dark .saved-note strong { color: #d7e2dc; }
  .overlay-shell.dark .saved-note p,
  .overlay-shell.dark .result-artifacts span { color: #aab8b1; }
</style>
