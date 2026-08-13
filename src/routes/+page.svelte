<script lang="ts">
  import { dev } from "$app/environment";
  import { onMount, tick } from "svelte";
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import { fade, fly, slide } from "svelte/transition";
  import { getVersion } from "@tauri-apps/api/app";
  import { emit, listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import QRCode from "qrcode";
  import {
    availableMonitors,
    getCurrentWindow,
    LogicalSize,
    primaryMonitor,
  } from "@tauri-apps/api/window";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import BrandIcon from "$lib/BrandIcon.svelte";
  import LumeLogo from "$lib/LumeLogo.svelte";
  import LumeMascot from "$lib/LumeMascot.svelte";
  import LumeSelect from "$lib/LumeSelect.svelte";
  import TerminalWindow from "$lib/TerminalWindow.svelte";
  import WorkflowBridgeWindow from "$lib/WorkflowBridgeWindow.svelte";
  import { displayText, localize } from "$lib/i18n";
  import {
    clipboardHasImage,
    clipboardMayContainImage,
    collectClipboardImages,
    createImagePreview,
    prepareClipboardImage,
  } from "$lib/imageAttachments";
  import { sessionCapabilities } from "$lib/sessionCapabilities";
  import { resolveTerminalSession } from "$lib/sessionIdentity";
  import { stripInternalAgentMetadata } from "$lib/markdown.js";
  import type {
    AgentKind,
    AgentSession,
    CompanionStatus,
    ExternalAgentPlugin,
    HistoryEntry,
    IntegrationDiagnostic,
    IntegrationStatus,
    MobileGatewayStatus,
    MobilePairingOffer,
    MobileScope,
    PairedDevice,
    PermissionAction,
    Preferences,
    PromptAttachmentInput,
    QuestionAnswer,
    ResumableSession,
    ResultNote,
    SessionStatus,
    TerminalWindowState,
    WhiteboardLayout,
    WorkflowRun,
  } from "$lib/domain";
  import { demoSessions } from "$lib/demo";
  import {
    configureIntegration,
    configureVscode,
    beginMobilePairing,
    answerQuestion,
    diagnoseIntegration,
    disableMobileGateway,
    decidePermission,
    defaultPreferences,
    deleteResultNote,
    loadDisplayBackend,
    loadHistory,
    loadResultNotes,
    loadResumableSessions,
    loadIntegrationStatuses,
    loadMobileGatewayStatus,
    loadOverlayPosition,
    loadPairedDevices,
    loadPreferences,
    loadWorkflowRun,
    loadSessions,
    loadTerminalWindows,
    loadExternalPlugins,
    openSessionSource,
    openTerminalWindow,
    loadVscodeStatus,
    moveOverlay,
    resizeOverlaySurface,
    installExternalPlugin,
    interruptPrompt,
    removeExternalPlugin,
    readLocalImageDataUrl,
    rebindWorkflowSession,
    renameSession,
    revealBrowserCompanion,
    revokePairedDevice,
    launchAgentSession,
    savePreferences,
    saveResultNote,
    restoreTerminalLayout,
    setTerminalWorkflowEnabled,
    setTerminalWindowsVisible,
    steerQueuedPrompt,
    submitPrompt,
    takePendingShortcutAction,
    terminateSession,
    enableMobileGateway,
    setPairedDeviceScopes,
    revealPluginDirectory,
    type DisplayBackend,
  } from "$lib/lume";

  type View = "sessions" | "board" | "history" | "settings";
  type ShellStatus = SessionStatus | "idle";
  type ShortcutAction = "open" | "palette" | "new-session" | "whiteboard";
  type CompanionUpdateEvent = { mobileVersion: string };
  type ShortcutPreferenceKey =
    | "openShortcut"
    | "globalShortcut"
    | "newSessionShortcut"
    | "whiteboardShortcut";
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
  const runtimePlatform = typeof navigator === "undefined"
    ? ""
    : `${navigator.userAgent} ${navigator.platform}`.toLowerCase();
  const isLinux = runtimePlatform.includes("linux") || runtimePlatform.includes("x11");
  const currentWindowLabel = isTauri ? getCurrentWindow().label : "main";
  const isTerminalWindow = currentWindowLabel.startsWith("terminal-");
  const isWorkflowBridgeWindow = currentWindowLabel.startsWith("workflow-bridge-");
  const compactSize = { width: 78, height: 44 };
  const expandedWidth = 392;
  const expandedMaxHeight = 560;
  const devMobileDeviceId = "lume-mobile-dev-preview";
  const devMobileDevice: PairedDevice = {
    id: devMobileDeviceId,
    name: "Lume Mobile",
    createdAt: 0,
    scopes: ["monitor", "prompt", "approve"],
  };
  const expandedPanelMaxHeight = 544;
  const edgeAnchorThreshold = 18;

  let expanded = $state(!isTauri);
  let contentVisible = $state(!isTauri);
  let morphing = $state<"opening" | "closing" | null>(null);
  let morphProgress = $state(isTauri ? 0 : 1);
  let morphWidth = $state(compactSize.width);
  let morphHeight = $state(compactSize.height);
  let measuringPanel = $state(false);
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
  let questionSelections = $state<Record<string, string>>({});
  let savingSettings = $state(false);
  let configuringIntegration = $state<IntegrationStatus["kind"] | null>(null);
  let diagnosingIntegration = $state<IntegrationStatus["kind"] | null>(null);
  let integrationDiagnostics = $state<Partial<Record<IntegrationStatus["kind"], IntegrationDiagnostic>>>({});
  let configuringVscode = $state(false);
  let launcherOpen = $state(false);
  let launching = $state<IntegrationStatus["kind"] | null>(null);
  let launchError = $state<string | null>(null);
  let resumeAgent = $state<IntegrationStatus["kind"] | null>(null);
  let resumableSessions = $state<ResumableSession[]>([]);
  let loadingResumeAgent = $state<IntegrationStatus["kind"] | null>(null);
  let browserCompanionPath = $state<string | null>(null);
  let settingsMessage = $state<string | null>(null);
  let settingsMessageIsError = $state(false);
  let resetConfirming = $state(false);
  let resettingSettings = $state(false);
  let composerSessionId = $state<string | null>(null);
  let composerPrompt = $state("");
  let composerAttachments = $state<PromptAttachmentInput[]>([]);
  let composerMessage = $state<string | null>(null);
  let composerSending = $state(false);
  let steeringQueuedActivityId = $state<string | null>(null);
  let terminateConfirmId = $state<string | null>(null);
  let terminatingSessionId = $state<string | null>(null);
  let interruptingSessionId = $state<string | null>(null);
  let sessionActionMessage = $state<string | null>(null);
  let renamingSessionId = $state<string | null>(null);
  let renameDraft = $state("");
  let renameError = $state<string | null>(null);
  let renamingSession = $state(false);
  let copiedResultId = $state<string | null>(null);
  let savingNoteId = $state<string | null>(null);
  let noteMessage = $state<string | null>(null);
  let selectedProfileKey = $state<string | null>(null);
  let terminalWindows = $state<TerminalWindowState[]>([]);
  let workflowModeChanging = $state(false);
  let workflowSettingsOpen = $state(false);
  let workflowSettingsSaving = $state(false);
  let workflowRunStates = $state<Record<string, WorkflowRun | null>>({});
  let workflowRebindingStepId = $state<string | null>(null);
  let openingTerminal = $state<string | null>(null);
  let terminalMessage = $state<string | null>(null);
  let layoutName = $state("");
  let selectedLayoutId = $state<string | null>(null);
  let restoringLayout = $state(false);
  let externalPlugins = $state<ExternalAgentPlugin[]>([]);
  let installingPlugin = $state(false);
  let pluginMessage = $state<string | null>(null);
  let paletteOpen = $state(false);
  let shortcutEditorKey = $state<ShortcutPreferenceKey | null>(null);
  let shortcutDraft = $state("");
  let shortcutEditorError = $state<string | null>(null);
  let paletteQuery = $state("");
  let paletteIndex = $state(0);
  let overlayPosition = $state({ x: 0, y: 12 });
  let compactAnchorPosition: { x: number; y: number } | null = null;
  let overlayReady = $state(false);
  let monitorBounds = $state({ x: 0, y: 0, width: 1920, height: 1080, scale: 1 });
  let displayBackend = $state<DisplayBackend>("native");
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
    lastX: number;
    lastY: number;
    originX: number;
    originY: number;
    scale: number;
    target: HTMLElement;
    compact: boolean;
  } | null = null;
  let moveFrame: number | null = null;
  let pendingOverlayMove: { x: number; y: number } | null = null;
  let overlayMoveTask: Promise<void> | null = null;
  let systemDark = $state(false);
  let mobileStatus = $state<MobileGatewayStatus | null>(null);
  let pairedDevices = $state<PairedDevice[]>(dev ? [devMobileDevice] : []);
  let newMobileDevice = $state<PairedDevice | null>(null);
  const knownMobileDeviceIds = new Set<string>();
  let mobileDevicesInitialized = false;
  let pairingOffer = $state<MobilePairingOffer | null>(null);
  let pairingQr = $state<string | null>(null);
  let mobileBusy = $state(false);
  let mobileMessage = $state<string | null>(null);
  let mobileMessageIsError = $state(false);
  const mobileApkUrl = "https://github.com/tulerws/Lume/releases/latest/download/Lume-Mobile.apk";

  function tr(english: string, portuguese: string) {
    return localize(preferences.language, english, portuguese);
  }

  function withDevMobileDevice(devices: PairedDevice[]) {
    return dev && !devices.some((device) => device.id === devMobileDeviceId)
      ? [devMobileDevice, ...devices]
      : devices;
  }

  function applyPairedDevices(devices: PairedDevice[], announceNew: boolean) {
    const realDevices = devices.filter((device) => device.id !== devMobileDeviceId);
    const addedDevice = announceNew
      ? [...realDevices]
          .sort((left, right) => right.createdAt - left.createdAt)
          .find((device) => !knownMobileDeviceIds.has(device.id))
      : undefined;
    for (const device of realDevices) knownMobileDeviceIds.add(device.id);
    pairedDevices = withDevMobileDevice(realDevices);
    mobileDevicesInitialized = true;
    if (addedDevice) newMobileDevice = addedDevice;
  }

  function shown(value: string) {
    return displayText(preferences.language, value);
  }

  function sessionDisplayName(session: AgentSession) {
    return session.sessionName?.trim() || session.project?.trim() || session.agentLabel;
  }

  function sessionDirectoryName(session: AgentSession) {
    const directory = session.workingDirectory?.trim().replace(/[\\/]+$/, "");
    return directory?.split(/[\\/]/).pop() || session.project?.trim() || session.agentLabel;
  }

  function currentExpandedSize() {
    return { width: expandedWidth, height: expandedHeight };
  }

  function applyExpandedHeight(nextHeight: number, resizeWindow: boolean) {
    const clampedHeight = Math.min(
      expandedMaxHeight,
      Math.max(compactSize.height, Math.ceil(nextHeight)),
    );
    if (clampedHeight === expandedHeight) return;
    const previousSize = currentExpandedSize();
    const anchor = compactAnchorPosition ?? compactPositionFromExpanded(overlayPosition, previousSize);
    expandedHeight = clampedHeight;
    if (resizeWindow && isTauri && expanded && !morphing) {
      compactAnchorPosition = anchor;
      const target = currentExpandedSize();
      const position = expandedPositionFromCompact(anchor, target);
      overlayPosition = position;
      void Promise.all([
        setOverlaySurfaceSize(target.width, target.height),
        moveOverlay(position.x, position.y, false, preferences.monitorId),
      ]).catch(() => undefined);
    }
  }

  function observePanelSize(node: HTMLElement) {
    let resizeFrame: number | null = null;

    const syncHeight = (resizeWindow: boolean) => {
      applyExpandedHeight(node.offsetHeight, resizeWindow);
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

  let launcherSurfaceSync = 0;
  async function syncLauncherSurface(open: boolean) {
    const sync = ++launcherSurfaceSync;
    await tick();
    if (sync !== launcherSurfaceSync || !expanded || morphing) return;
    const panel = document.querySelector<HTMLElement>(".panel");
    if (!panel) return;
    applyExpandedHeight(open ? expandedMaxHeight : panel.offsetHeight, true);
  }

  $effect(() => {
    const open = launcherOpen;
    if (expanded && !morphing) void syncLauncherSurface(open);
  });

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
    if (isTerminalWindow || isWorkflowBridgeWindow) return;
    const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    const syncSystemTheme = (event: MediaQueryListEvent | MediaQueryList) => {
      systemDark = event.matches;
    };
    const finishOverlayDragFromWindow = (event: PointerEvent) => {
      if (!dragState || dragState.pointerId !== event.pointerId) return;
      void endOverlayDrag(event, dragState.compact);
    };
    syncSystemTheme(colorScheme);
    colorScheme.addEventListener("change", syncSystemTheme);
    window.addEventListener("keydown", handleAppShortcut);
    window.addEventListener("pointerdown", bringOverlayToFront, true);
    window.addEventListener("pointerup", finishOverlayDragFromWindow, true);
    window.addEventListener("pointercancel", finishOverlayDragFromWindow, true);
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let stopTerminalListening: (() => void) | undefined;
    let stopShortcutListening: (() => void) | undefined;
    let stopCompanionUpdateListening: (() => void) | undefined;
    let stopMobileDeviceListening: (() => void) | undefined;
    let pollTimer: ReturnType<typeof setInterval> | undefined;
    let updateTimer: ReturnType<typeof setInterval> | undefined;
    let resumeRefreshTimer: ReturnType<typeof setTimeout> | undefined;
    const refreshAfterResume = () => {
      void refreshSessions(false);
      if (resumeRefreshTimer) clearTimeout(resumeRefreshTimer);
      resumeRefreshTimer = setTimeout(() => void refreshSessions(false), 2_500);
    };
    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") refreshAfterResume();
    };

    window.addEventListener("focus", refreshAfterResume);
    window.addEventListener("pageshow", refreshAfterResume);
    document.addEventListener("visibilitychange", refreshWhenVisible);

    updateTimer = setInterval(() => void checkForUpdates(), 6 * 60 * 60 * 1_000);

    void (async () => {
      const [nextPreferences, nextDisplayBackend] = await Promise.all([
        loadPreferences(),
        loadDisplayBackend(),
      ]);
      if (disposed) return;
      preferences = nextPreferences;
      displayBackend = nextDisplayBackend;
      try {
        overlayPosition = await loadOverlayPosition();
        overlayReady = true;
      } catch {
        if (preferences.overlayX !== undefined && preferences.overlayY !== undefined) {
          overlayPosition = { x: preferences.overlayX, y: preferences.overlayY };
          overlayReady = true;
        }
      }
      await loadMonitorOptions();
      await positionWindow();
      overlayReady = true;

      const [nextSessions, nextIntegrations, nextVscodeStatus, nextPlugins] = await Promise.all([
        loadSessions(),
        loadIntegrationStatuses(),
        loadVscodeStatus(),
        loadExternalPlugins(),
      ]);
      if (disposed) return;
      sessions = nextSessions;
      selectedProfileKey = detectedProjects[0]?.key ?? null;
      void initializeUpdater();
      integrations = nextIntegrations;
      vscodeStatus = nextVscodeStatus;
      externalPlugins = nextPlugins;
      selectedLayoutId = preferences.whiteboardLayouts[0]?.id ?? null;
      layoutName = preferences.whiteboardLayouts[0]?.name ?? "";
      selectedId =
        sessions.find((session) => session.status === "permission_required")?.id ?? null;

      if (isTauri) {
        stopMobileDeviceListening = await listen<PairedDevice>(
          "lume://mobile-device-paired",
          ({ payload }) => {
            applyPairedDevices([
              ...pairedDevices.filter(
                (device) =>
                  device.id !== devMobileDeviceId && device.id !== payload.id,
              ),
              payload,
            ], true);
          },
        );
        try {
          applyPairedDevices(await loadPairedDevices(), false);
        } catch {
          // Mobile settings still expose the connection error when opened.
        }
        stopListening = await listen("lume://sessions-changed", () => {
          void refreshSessions(true);
        });
        stopTerminalListening = await listen("lume://terminal-windows-changed", () => {
          void refreshTerminalWindows();
        });
        stopShortcutListening = await listen<ShortcutAction>("lume://shortcut", ({ payload }) => {
          void runShortcutAction(payload);
        });
        stopCompanionUpdateListening = await listen<CompanionUpdateEvent>(
          "lume://companion-update-check",
          ({ payload }) => {
            void handleCompanionUpdateRequest(payload);
          },
        );
        const pendingShortcut = await takePendingShortcutAction();
        if (pendingShortcut) void runShortcutAction(pendingShortcut);
        pollTimer = setInterval(() => void refreshSessions(false), 15_000);
      }
    })();

    return () => {
      disposed = true;
      stopListening?.();
      stopTerminalListening?.();
      stopShortcutListening?.();
      stopCompanionUpdateListening?.();
      stopMobileDeviceListening?.();
      colorScheme.removeEventListener("change", syncSystemTheme);
      window.removeEventListener("focus", refreshAfterResume);
      window.removeEventListener("pageshow", refreshAfterResume);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
      window.removeEventListener("keydown", handleAppShortcut);
      window.removeEventListener("pointerdown", bringOverlayToFront, true);
      window.removeEventListener("pointerup", finishOverlayDragFromWindow, true);
      window.removeEventListener("pointercancel", finishOverlayDragFromWindow, true);
      if (pollTimer) clearInterval(pollTimer);
      if (updateTimer) clearInterval(updateTimer);
      if (resumeRefreshTimer) clearTimeout(resumeRefreshTimer);
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

  async function checkForUpdates(): Promise<Update | null> {
    if (
      !isTauri ||
      updateState === "checking" ||
      updateState === "downloading" ||
      updateState === "ready"
    ) return pendingUpdate;
    updateState = "checking";
    updateDetail = tr("Checking for a new version…", "Procurando uma nova versão…");
    updateProgress = null;

    try {
      const nextUpdate = await check({
        timeout: 15_000,
        headers: {
          "Cache-Control": "no-cache",
          Pragma: "no-cache",
        },
      });
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
      return nextUpdate;
    } catch {
      updateState = "error";
      updateDetail = tr(
        "Could not check for updates right now. Try again shortly.",
        "Não foi possível verificar agora. Tente novamente em instantes.",
      );
      return null;
    }
  }

  async function handleUpdateButton() {
    if (updateState === "checking" || updateState === "downloading" || updateState === "ready") {
      return;
    }
    await checkForUpdates();
  }

  async function handleInstallUpdate() {
    if (updateState !== "available") return;
    const latestUpdate = await checkForUpdates();
    if (latestUpdate && updateState === "available") await installAvailableUpdate();
  }

  async function handleCompanionUpdateRequest(payload: CompanionUpdateEvent) {
    for (let attempt = 0; updateState === "checking" && attempt < 40; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    if (!["downloading", "ready"].includes(updateState)) {
      await checkForUpdates();
    }
    if (!["available", "downloading", "ready"].includes(updateState)) return;
    if (!expanded) await toggleExpanded();
    await openView("settings");
    await tick();
    document.querySelector<HTMLElement>("[data-update-card]")?.scrollIntoView({
      behavior: "smooth",
      block: "center",
    });
    updateDetail = tr(
      `Lume Mobile ${payload.mobileVersion} found this desktop update.`,
      `O Lume Mobile ${payload.mobileVersion} encontrou esta atualização do desktop.`,
    );
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
      const target = expanded ? currentExpandedSize() : compactSize;
      await setOverlaySurfaceSize(target.width, target.height);

      const found = await availableMonitors();
      const configured = preferences.monitorId
        ? found.find((monitor, index) =>
            (monitor.name ?? `monitor-${index}`) === preferences.monitorId,
          )
        : undefined;
      const monitor = configured ?? (await primaryMonitor());
      if (!monitor) return;
      const scale = monitor.scaleFactor || 1;
      monitorBounds = {
        x: monitor.position.x,
        y: monitor.position.y,
        width: monitor.size.width,
        height: monitor.size.height,
        scale,
      };
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
    if (!overlayReady) {
      await positionWindow();
      if (!overlayReady) return;
    }
    if (morphing) return;
    const opening = !expanded;
    let expandedTarget = currentExpandedSize();

    void setTerminalWindowsVisible(opening).catch((error) => {
      terminalMessage = String(error).replace(/^Error:\s*/, "");
    });
    morphing = opening ? "opening" : "closing";
    contentVisible = false;
    if (opening) {
      expanded = true;
      measuringPanel = true;
      morphProgress = 1;
      await tick();
      const panel = document.querySelector<HTMLElement>(".panel");
      if (panel) {
        expandedHeight = Math.min(
          expandedMaxHeight,
          Math.max(compactSize.height, Math.ceil(panel.offsetHeight)),
        );
      }
      expandedTarget = currentExpandedSize();
      measuringPanel = false;
      morphProgress = 0;
      morphWidth = compactSize.width;
      morphHeight = compactSize.height;
      await tick();
    } else {
      morphWidth = expandedTarget.width;
      morphHeight = expandedTarget.height;
    }

    const compactTargetPosition = opening
      ? { ...overlayPosition }
      : compactAnchorPosition ??
        compactPositionFromExpanded(overlayPosition, expandedTarget);
    const expandedTargetPosition = expandedPositionFromCompact(
      compactTargetPosition,
      expandedTarget,
    );

    compactAnchorPosition = compactTargetPosition;
    await animateCapsule(
      opening,
      expandedTarget,
      compactTargetPosition,
      expandedTargetPosition,
    );

    if (opening) {
      contentVisible = true;
    } else {
      expanded = false;
      morphProgress = 0;
      await tick();
      compactAnchorPosition = null;
      selectedId = null;
      view = "sessions";
      launcherOpen = false;
    }
    morphing = null;
  }

  async function animateCapsule(
    opening: boolean,
    expandedTarget: { width: number; height: number },
    compactTargetPosition: { x: number; y: number },
    expandedTargetPosition: { x: number; y: number },
  ) {
    const from = opening ? compactSize : expandedTarget;
    const to = opening ? expandedTarget : compactSize;
    const fromPosition = opening ? compactTargetPosition : expandedTargetPosition;
    const toPosition = opening ? expandedTargetPosition : compactTargetPosition;

    if (!isTauri) {
      morphProgress = opening ? 1 : 0;
      overlayPosition = { ...toPosition };
      return;
    }

    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const duration = reducedMotion ? 1 : opening ? 300 : 260;
    const startedAt = performance.now();

    await new Promise<void>((resolve) => {
      const frame = async (now: number) => {
        const linear = Math.min(1, (now - startedAt) / duration);
        const eased = morphEase(linear);
        morphProgress = opening ? eased : 1 - eased;
        if (opening && eased >= 0.38) contentVisible = true;

        await applyCapsuleGeometry({
          width: Math.round(from.width + (to.width - from.width) * eased),
          height: Math.round(from.height + (to.height - from.height) * eased),
          x: Math.round(fromPosition.x + (toPosition.x - fromPosition.x) * eased),
          y: Math.round(fromPosition.y + (toPosition.y - fromPosition.y) * eased),
        });

        if (linear < 1) {
          requestAnimationFrame((next) => void frame(next));
        } else {
          resolve();
        }
      };
      requestAnimationFrame((now) => void frame(now));
    });

    await applyCapsuleGeometry({
      width: to.width,
      height: to.height,
      x: toPosition.x,
      y: toPosition.y,
    });
    morphProgress = opening ? 1 : 0;
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  }

  async function applyCapsuleGeometry(geometry: {
    width: number;
    height: number;
    x: number;
    y: number;
  }) {
    morphWidth = geometry.width;
    morphHeight = geometry.height;
    await tick();

    const tasks: Promise<unknown>[] = [
      setOverlaySurfaceSize(geometry.width, geometry.height, true),
    ];
    if (geometry.x !== overlayPosition.x || geometry.y !== overlayPosition.y) {
      tasks.push(moveOverlay(geometry.x, geometry.y, false, preferences.monitorId));
    }
    await Promise.allSettled(tasks);
    overlayPosition = { x: geometry.x, y: geometry.y };
  }

  async function setOverlaySurfaceSize(
    width: number,
    height: number,
    syncLinuxSurface = false,
  ) {
    const size = new LogicalSize(width, height);
    const tasks: Promise<unknown>[] = [getCurrentWindow().setSize(size)];
    if (isLinux && (displayBackend !== "native-gnome" || syncLinuxSurface)) {
      tasks.push(
        getCurrentWebview().setSize(size),
        resizeOverlaySurface(width, height),
      );
    }
    await Promise.allSettled(tasks);
  }

  function morphEase(value: number) {
    return value < 0.5
      ? 4 * value * value * value
      : 1 - Math.pow(-2 * value + 2, 3) / 2;
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
    if (!isTauri || !overlayReady || event.button !== 0 || morphing) return;
    bringOverlayToFront();
    if (!compact && (event.target as HTMLElement).closest("button, input, select, textarea")) {
      return;
    }
    if (!compact && event.detail === 2) {
      event.preventDefault();
      void toggleExpanded();
      return;
    }
    if (displayBackend === "native-gnome") {
      dragging = true;
      void getCurrentWindow()
        .startDragging()
        .catch(() => undefined)
        .finally(() => {
          setTimeout(() => {
            dragging = false;
          }, 120);
        });
      return;
    }
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    dragState = {
      pointerId: event.pointerId,
      startX: event.screenX,
      startY: event.screenY,
      lastX: event.screenX,
      lastY: event.screenY,
      originX: overlayPosition.x,
      originY: overlayPosition.y,
      scale: monitorBounds.scale,
      target,
      compact,
    };
    dragging = false;
  }

  async function bringOverlayToFront() {
    if (!isTauri) return;
    await getCurrentWindow().setFocus().catch(() => undefined);
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
    const stepX = (event.screenX - dragState.lastX) * dragState.scale;
    const stepY = (event.screenY - dragState.lastY) * dragState.scale;
    const pointerJumpLimit = Math.max(120, 160 * dragState.scale);
    if (Math.hypot(stepX, stepY) > pointerJumpLimit) {
      dragState.startX = event.screenX;
      dragState.startY = event.screenY;
      dragState.lastX = event.screenX;
      dragState.lastY = event.screenY;
      dragState.originX = overlayPosition.x;
      dragState.originY = overlayPosition.y;
      return;
    }
    dragState.lastX = event.screenX;
    dragState.lastY = event.screenY;
    const dx = (event.screenX - dragState.startX) * dragState.scale;
    const dy = (event.screenY - dragState.startY) * dragState.scale;
    if (!dragging && Math.hypot(dx, dy) < 3) return;
    dragging = true;
    event.preventDefault();
    overlayPosition = clampOverlayPosition(
      dragState.originX + dx,
      dragState.originY + dy,
    );
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
    const target = dragState.target;
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

  function beginSessionRename(session: AgentSession) {
    renamingSessionId = session.id;
    renameDraft = sessionDisplayName(session);
    renameError = null;
  }

  function cancelSessionRename() {
    renamingSessionId = null;
    renameDraft = "";
    renameError = null;
  }

  async function saveSessionRename(session: AgentSession) {
    if (renamingSession) return;
    const requested = renameDraft.trim();
    if (!requested) {
      renameError = tr("Enter a name for this session.", "Digite um nome para esta sessão.");
      return;
    }
    renamingSession = true;
    renameError = null;
    try {
      const finalName = isTauri
        ? await renameSession(session.id, requested)
        : uniqueLocalSessionName(requested, session.id);
      sessions = sessions.map((item) =>
        item.id === session.id ? { ...item, sessionName: finalName } : item
      );
      cancelSessionRename();
    } catch (error) {
      renameError = String(error).replace(/^Error:\s*/, "");
    } finally {
      renamingSession = false;
    }
  }

  function uniqueLocalSessionName(requested: string, sessionId: string) {
    const used = new Set(
      sessions
        .filter((session) => session.id !== sessionId)
        .map((session) => sessionDisplayName(session).toLocaleLowerCase())
    );
    if (!used.has(requested.toLocaleLowerCase())) return requested;
    for (let suffix = 2; ; suffix += 1) {
      const candidate = `${requested} (${suffix})`;
      if (!used.has(candidate.toLocaleLowerCase())) return candidate;
    }
  }

  function canSubmitToSession(session: AgentSession) {
    return sessionCapabilities(session).canPrompt;
  }

  function canContinueSession(session: AgentSession) {
    return (
      ["completed", "failed", "waiting_for_input"].includes(session.status)
      || (
        session.status === "running"
        && sessionCapabilities(session).promptDeliveries.includes("steer")
      )
    );
  }

  function pendingQueuedPrompts(session: AgentSession) {
    return session.activities
      .filter((activity) =>
        activity.kind === "queued_prompt" && activity.status === "waiting"
      )
      .sort((left, right) => left.createdAt - right.createdAt);
  }

  function toggleSessionComposer(session: AgentSession) {
    composerSessionId = composerSessionId === session.id ? null : session.id;
    composerPrompt = "";
    composerAttachments = [];
    composerMessage = null;
  }

  async function sendSessionPrompt(session: AgentSession) {
    const prompt = composerPrompt.trim();
    if ((!prompt && composerAttachments.length === 0) || composerSending) return;
    composerSending = true;
    composerMessage = null;
    try {
      const delivery = session.status === "running" ? "queue" : "new_turn";
      if (isTauri) {
        await submitPrompt(session.id, prompt, composerAttachments, delivery);
      }
      sessions = sessions.map((item) =>
        item.id === session.id
          ? {
              ...item,
              status: "running",
              statusLabel: delivery === "queue" ? "Prompt queued" : "Prompt sent by Lume",
              lastResponse: delivery === "new_turn" ? undefined : item.lastResponse,
            }
          : item,
      );
      composerPrompt = "";
      composerAttachments = [];
      composerSessionId = delivery === "queue" ? session.id : null;
      if (isTauri) await refreshSessions(false);
    } catch (error) {
      composerMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      composerSending = false;
    }
  }

  async function steerSessionQueuedPrompt(session: AgentSession) {
    const queuedPrompt = pendingQueuedPrompts(session)[0];
    if (!queuedPrompt || steeringQueuedActivityId) return;
    steeringQueuedActivityId = queuedPrompt.id;
    composerMessage = null;
    try {
      if (isTauri) {
        await steerQueuedPrompt(session.id, queuedPrompt.id);
        await refreshSessions(false);
      }
      composerMessage = tr(
        "Queued prompt steered into the current task.",
        "Prompt da fila enviado para a tarefa atual.",
      );
    } catch (error) {
      composerMessage = String(error).replace(/^Error:\s*/, "");
      if (isTauri) await refreshSessions(false).catch(() => undefined);
    } finally {
      steeringQueuedActivityId = null;
    }
  }

  function handleSessionComposerKeydown(event: KeyboardEvent, session: AgentSession) {
    if (
      event.key !== "Tab"
      || event.shiftKey
      || event.isComposing
      || pendingQueuedPrompts(session).length === 0
    ) return;
    event.preventDefault();
    void steerSessionQueuedPrompt(session);
  }

  async function inlineImagePreview(path: string) {
    return createImagePreview(
      await readLocalImageDataUrl(path),
      preferences.language,
    );
  }

  function removeComposerImage(index: number) {
    composerAttachments = composerAttachments.filter((_, current) => current !== index);
  }

  async function pasteSessionImages(
    event: ClipboardEvent,
    session: AgentSession,
  ) {
    if (!clipboardHasImage(event) && !clipboardMayContainImage(event)) return;
    event.preventDefault();
    composerMessage = null;
    const capabilities = sessionCapabilities(session);
    if (
      composerSending ||
      !canContinueSession(session) ||
      !capabilities.canPrompt ||
      !capabilities.canAttachImages
    ) {
      composerMessage = tr(
        "Images can only be attached when this session is ready for a prompt.",
        "Imagens só podem ser anexadas quando esta sessão estiver pronta para um prompt.",
      );
      return;
    }
    try {
      const { files, paths } = await collectClipboardImages(event, preferences.language);
      const available = 4 - composerAttachments.length;
      const prepared: PromptAttachmentInput[] = [];
      for (const [index, file] of files.slice(0, available).entries()) {
        prepared.push(await prepareClipboardImage(file, index, preferences.language));
      }
      for (const path of paths.slice(0, available - prepared.length)) {
        prepared.push({
          name: path.split(/[\\/]/).pop() || "image",
          mimeType: "",
          path,
          previewDataUrl: await inlineImagePreview(path),
        });
      }
      composerAttachments = [...composerAttachments, ...prepared];
    } catch (error) {
      composerMessage = String(error).replace(/^Error:\s*/, "");
    }
  }

  function canTerminateSession(session: AgentSession) {
    return sessionCapabilities(session).canTerminate;
  }

  function canInterruptSession(session: AgentSession) {
    return sessionCapabilities(session).canInterrupt;
  }

  async function interruptSessionPrompt(session: AgentSession) {
    if (!canInterruptSession(session) || interruptingSessionId) return;
    interruptingSessionId = session.id;
    sessionActionMessage = null;
    try {
      if (isTauri) await interruptPrompt(session.id);
      await refreshSessions(false);
    } catch (error) {
      sessionActionMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      interruptingSessionId = null;
    }
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
      const note = await saveResultNote(session.id, resultId, sessionDisplayName(session));
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

  async function togglePanelWorkflowMode(enabled: boolean) {
    if (workflowModeChanging) return;
    workflowModeChanging = true;
    terminalMessage = null;
    try {
      await updatePreference("workflowEnabled", enabled);
      if (isTauri) terminalWindows = await setTerminalWorkflowEnabled(enabled);
    } catch (error) {
      terminalMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      workflowModeChanging = false;
    }
  }

  async function toggleWorkflowSettings() {
    workflowSettingsOpen = !workflowSettingsOpen;
    if (!workflowSettingsOpen || !isTauri) return;
    await refreshTerminalWindows();
    const entries = await Promise.all(preferences.workflowGroups.map(async (group) => {
      try {
        return [group.id, await loadWorkflowRun(group.id)] as const;
      } catch {
        return [group.id, null] as const;
      }
    }));
    workflowRunStates = Object.fromEntries(entries);
  }

  function workflowSessionKey(session: AgentSession) {
    return session.nativeSessionId?.trim() || session.id;
  }

  function missingWorkflowSteps() {
    const connected = new Set(sessions.map(workflowSessionKey));
    return preferences.workflowGroups
      .filter((group) => {
        const run = workflowRunStates[group.id];
        return Boolean(run && !["completed", "cancelled"].includes(run.status));
      })
      .flatMap((group) => {
        const runStepIds = new Set(workflowRunStates[group.id]?.steps.map((step) => step.stepId));
        return group.steps
          .filter((step) => runStepIds.has(step.id) && !connected.has(step.sessionNativeId))
          .map((step, index) => ({ group, step, index }));
      });
  }

  function workflowReplacementSessions(groupId: string, stepId: string) {
    const group = preferences.workflowGroups.find((item) => item.id === groupId);
    const occupied = new Set(
      group?.steps
        .filter((step) => step.id !== stepId)
        .map((step) => step.sessionNativeId) ?? [],
    );
    return sessions.filter((session) => !occupied.has(workflowSessionKey(session)));
  }

  async function updateWorkflowSetting<K extends keyof Preferences["workflowSettings"]>(
    key: K,
    value: Preferences["workflowSettings"][K],
  ) {
    if (workflowSettingsSaving) return;
    workflowSettingsSaving = true;
    try {
      await updatePreference("workflowSettings", {
        ...preferences.workflowSettings,
        [key]: value,
      });
    } finally {
      workflowSettingsSaving = false;
    }
  }

  async function replaceWorkflowSession(
    workflowId: string,
    stepId: string,
    sessionNativeId: string,
  ) {
    if (!sessionNativeId || workflowRebindingStepId) return;
    workflowRebindingStepId = stepId;
    terminalMessage = null;
    try {
      await rebindWorkflowSession(workflowId, stepId, sessionNativeId);
      preferences = await loadPreferences();
      if (isTauri) void emit("lume://preferences-changed", preferences);
      terminalMessage = tr(
        "Workflow agent replaced. Retry the step if it was interrupted.",
        "Agente do workflow substituído. Tente a etapa novamente se ela foi interrompida.",
      );
    } catch (error) {
      terminalMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      workflowRebindingStepId = null;
    }
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
        terminalMessage = tr(
          `${sessionDisplayName(session)} opens in a separate window.`,
          `${sessionDisplayName(session)} abre em uma janela separada.`,
        );
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
        const session = resolveTerminalSession(terminal, sessions);
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
    return terminalWindows.some(
      (terminal) => resolveTerminalSession(terminal, sessions)?.id === session.id,
    );
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

  async function handleQuestionOption(
    session: AgentSession,
    questionId: string,
    value: string,
  ) {
    const request = session.pendingQuestion;
    if (!request) return;
    const selections = {
      ...questionSelections,
      [`${request.id}:${questionId}`]: value,
    };
    questionSelections = selections;
    const answers: QuestionAnswer[] = request.questions
      .map((question) => ({
        questionId: question.id,
        answers: selections[`${request.id}:${question.id}`]
          ? [selections[`${request.id}:${question.id}`]]
          : [],
      }))
      .filter((answer) => answer.answers.length > 0);
    if (answers.length !== request.questions.length) return;
    permissionError = null;
    try {
      await answerQuestion(session.id, request.id, answers);
      questionSelections = {};
      await refreshSessions(false);
    } catch (error) {
      permissionError = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function refreshMobileSettings() {
    if (!isTauri) return;
    try {
      const [status, devices] = await Promise.all([
        loadMobileGatewayStatus(),
        loadPairedDevices(),
      ]);
      mobileStatus = status;
      applyPairedDevices(devices, mobileDevicesInitialized);
    } catch (error) {
      mobileMessageIsError = true;
      mobileMessage = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function toggleMobileAccess() {
    if (!isTauri || mobileBusy) return;
    mobileBusy = true;
    mobileMessage = null;
    pairingOffer = null;
    pairingQr = null;
    try {
      mobileStatus = mobileStatus?.networkReachable
        ? await disableMobileGateway()
        : await enableMobileGateway();
      if (mobileStatus.networkReachable) {
        pairingOffer = await beginMobilePairing();
        pairingQr = await QRCode.toDataURL(pairingOffer.payload, {
          width: 208,
          margin: 4,
          errorCorrectionLevel: "M",
          color: { dark: "#14241d", light: "#ffffff" },
        });
      }
      mobileMessageIsError = false;
    } catch (error) {
      mobileMessageIsError = true;
      mobileMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      mobileBusy = false;
    }
  }

  async function createMobilePairing() {
    if (!isTauri || mobileBusy) return;
    mobileBusy = true;
    mobileMessage = null;
    try {
      pairingOffer = await beginMobilePairing();
      pairingQr = await QRCode.toDataURL(pairingOffer.payload, {
        width: 208,
        margin: 4,
        errorCorrectionLevel: "M",
        color: { dark: "#14241d", light: "#ffffff" },
      });
      mobileMessageIsError = false;
    } catch (error) {
      mobileMessageIsError = true;
      mobileMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      mobileBusy = false;
    }
  }

  async function removePairedDevice(id: string) {
    if (mobileBusy) return;
    mobileBusy = true;
    mobileMessage = null;
    try {
      await revokePairedDevice(id);
      applyPairedDevices(await loadPairedDevices(), false);
      if (newMobileDevice?.id === id) newMobileDevice = null;
      mobileMessageIsError = false;
      mobileMessage = tr("Device access revoked.", "Acesso do dispositivo revogado.");
    } catch (error) {
      mobileMessageIsError = true;
      mobileMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      mobileBusy = false;
    }
  }

  async function togglePairedDeviceScope(device: PairedDevice, scope: MobileScope) {
    if (mobileBusy || scope === "monitor") return;
    const scopes = device.scopes.includes(scope)
      ? device.scopes.filter((value) => value !== scope)
      : [...device.scopes, scope];
    if (dev && device.id === devMobileDeviceId) {
      pairedDevices = pairedDevices.map((value) =>
        value.id === devMobileDeviceId ? { ...value, scopes } : value,
      );
      mobileMessageIsError = false;
      mobileMessage = tr("Preview permission updated.", "Permissão de demonstração atualizada.");
      return;
    }
    mobileBusy = true;
    mobileMessage = null;
    try {
      await setPairedDeviceScopes(device.id, scopes);
      applyPairedDevices(await loadPairedDevices(), false);
      mobileMessageIsError = false;
      mobileMessage = tr("Device permissions updated.", "Permissões do dispositivo atualizadas.");
    } catch (error) {
      mobileMessageIsError = true;
      mobileMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      mobileBusy = false;
    }
  }

  async function copyMobileValue(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      mobileMessageIsError = false;
      mobileMessage = tr("Copied.", "Copiado.");
    } catch {
      mobileMessageIsError = true;
      mobileMessage = tr("Could not copy this value.", "Não foi possível copiar este valor.");
    }
  }

  async function reviewNewMobileDevice() {
    const deviceId = newMobileDevice?.id;
    if (!deviceId) return;
    await openView("settings");
    await tick();
    const section = document.querySelector<HTMLDetailsElement>("[data-mobile-access-section]");
    if (section) section.open = true;
    await tick();
    const deviceCard = [...document.querySelectorAll<HTMLElement>("[data-mobile-device-id]")]
      .find((element) => element.dataset.mobileDeviceId === deviceId);
    (deviceCard ?? section)?.scrollIntoView({ behavior: "smooth", block: "center" });
    newMobileDevice = null;
  }

  async function openView(nextView: View) {
    if (
      nextView === "settings" &&
      isTauri &&
      expanded &&
      !morphing &&
      expandedHeight < expandedPanelMaxHeight
    ) {
      const previousSize = currentExpandedSize();
      const anchor =
        compactAnchorPosition ?? compactPositionFromExpanded(overlayPosition, previousSize);
      expandedHeight = expandedPanelMaxHeight;
      compactAnchorPosition = anchor;
      const target = currentExpandedSize();
      const position = expandedPositionFromCompact(anchor, target);
      await Promise.allSettled([
        setOverlaySurfaceSize(target.width, target.height),
        moveOverlay(position.x, position.y, false, preferences.monitorId),
      ]);
      overlayPosition = position;
    }
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
      await refreshMobileSettings();
    }
  }

  type PaletteCommand = { id: string; label: string; detail: string; run: () => void | Promise<void> };

  function paletteCommands(): PaletteCommand[] {
    const commands: PaletteCommand[] = [
      { id: "sessions", label: tr("Sessions", "Sessões"), detail: tr("Show active agents", "Mostrar agentes ativos"), run: () => openView("sessions") },
      { id: "whiteboard", label: tr("Terminals", "Terminais"), detail: tr("Open floating terminals", "Abrir terminais flutuantes"), run: () => openView("board") },
      { id: "history", label: tr("History and notes", "Histórico e notas"), detail: tr("Open completed results", "Abrir resultados finalizados"), run: () => openView("history") },
      { id: "settings", label: tr("Settings", "Ajustes"), detail: tr("Configure Lume", "Configurar o Lume"), run: () => openView("settings") },
      { id: "new-session", label: tr("New agent session", "Nova sessão de agente"), detail: tr("Open the agent launcher", "Abrir o iniciador de agentes"), run: async () => { await openView("sessions"); launcherOpen = true; } },
    ];
    for (const session of sessions) {
      commands.push({
        id: `session-${session.id}`,
        label: sessionDisplayName(session),
        detail: shown(session.statusLabel),
        run: async () => {
          await openView("sessions");
          selectedId = session.id;
        },
      });
      if (!terminalIsOpen(session)) {
        commands.push({
          id: `terminal-${session.id}`,
          label: tr(`Open ${sessionDisplayName(session)} terminal`, `Abrir terminal ${sessionDisplayName(session)}`),
          detail: `${session.project} · ${tr("Chat and changed files", "Chat e arquivos alterados")}`,
          run: async () => {
            await openView("board");
            await openTerminal(session);
          },
        });
      }
      if (canSubmitToSession(session)) {
        commands.push({
          id: `prompt-${session.id}`,
          label: tr(`Send prompt to ${sessionDisplayName(session)}`, `Enviar prompt para ${sessionDisplayName(session)}`),
          detail: session.project,
          run: async () => {
            await openView("sessions");
            selectedId = session.id;
            composerSessionId = session.id;
            composerPrompt = "";
            await tick();
          },
        });
      }
    }
    const query = paletteQuery.trim().toLowerCase();
    return query
      ? commands.filter((command) => `${command.label} ${command.detail}`.toLowerCase().includes(query))
      : commands;
  }

  async function runShortcutAction(action: ShortcutAction) {
    if (action === "palette") {
      await showCommandPalette();
      return;
    }
    if (!expanded) await toggleExpanded();
    if (action === "new-session") {
      await openView("sessions");
      launcherOpen = true;
      return;
    }
    if (action === "whiteboard") {
      await openView("board");
    }
  }

  function shortcutFromEvent(event: KeyboardEvent): string | null {
    if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return null;
    const modifiers = [
      event.ctrlKey ? "Ctrl" : "",
      event.altKey ? "Alt" : "",
      event.shiftKey ? "Shift" : "",
      event.metaKey ? "Super" : "",
    ].filter(Boolean);
    if (modifiers.length === 0) return null;
    let key = event.code;
    if (key.startsWith("Key")) key = key.slice(3);
    else if (key.startsWith("Digit")) key = key.slice(5);
    if (!key || key === "Unidentified") return null;
    return [...modifiers, key].join("+");
  }

  function shortcutMatches(event: KeyboardEvent, shortcut: string) {
    return shortcutFromEvent(event)?.toLowerCase() === shortcut.toLowerCase();
  }

  function handleAppShortcut(event: KeyboardEvent) {
    if (event.defaultPrevented || event.repeat) return;
    const configured: Array<[ShortcutAction, string]> = [
      ["open", preferences.openShortcut],
      ["palette", preferences.globalShortcut],
      ["new-session", preferences.newSessionShortcut],
      ["whiteboard", preferences.whiteboardShortcut],
    ];
    let action = configured.find(([, shortcut]) => shortcutMatches(event, shortcut))?.[0];
    if (
      !action &&
      (event.ctrlKey || event.metaKey) &&
      event.shiftKey &&
      event.code === "KeyP"
    ) {
      action = "palette";
    }
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    void runShortcutAction(action);
  }

  function captureShortcut(event: KeyboardEvent) {
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      shortcutEditorKey = null;
      return;
    }
    const shortcut = shortcutFromEvent(event);
    if (!shortcut) return;
    shortcutDraft = shortcut;
    shortcutEditorError = null;
  }

  async function openShortcutEditor(key: ShortcutPreferenceKey) {
    shortcutEditorKey = key;
    shortcutDraft = preferences[key];
    shortcutEditorError = null;
    await tick();
    document.querySelector<HTMLElement>("[data-shortcut-capture]")?.focus();
  }

  async function saveShortcut() {
    if (!shortcutEditorKey || !shortcutDraft) return;
    const saved = await updatePreference(shortcutEditorKey, shortcutDraft);
    if (saved) shortcutEditorKey = null;
    else shortcutEditorError = settingsMessage;
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
    if (!launcherOpen) {
      resumeAgent = null;
      resumableSessions = [];
    }
    if (launcherOpen && integrations.length === 0) {
      integrations = await loadIntegrationStatuses();
    }
  }

  async function startSession(agent: IntegrationStatus["kind"]) {
    if (!isTauri) {
      launcherOpen = false;
      return;
    }
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: tr("Project for the new session", "Projeto da nova sessão"),
    });
    if (!selected || Array.isArray(selected)) return;

    launching = agent;
    launchError = null;
    try {
      const profile = preferences.projectProfiles[projectKey(selected)];
      await launchAgentSession(
        agent,
        selected,
        false,
        undefined,
        profile?.launchTarget ?? preferences.launchTarget,
        profile?.permissionMode,
        profile?.approvalPolicy,
      );
      launcherOpen = false;
    } catch (error) {
      launchError = String(error).replace(/^Error:\s*/, "");
    } finally {
      launching = null;
    }
  }

  async function toggleResumeSessions(agent: IntegrationStatus["kind"]) {
    if (resumeAgent === agent) {
      resumeAgent = null;
      resumableSessions = [];
      return;
    }
    resumeAgent = agent;
    resumableSessions = [];
    loadingResumeAgent = agent;
    launchError = null;
    try {
      resumableSessions = await loadResumableSessions(agent);
    } catch (error) {
      launchError = String(error).replace(/^Error:\s*/, "");
    } finally {
      loadingResumeAgent = null;
    }
  }

  async function resumeStoredSession(stored: ResumableSession) {
    launching = stored.agent;
    launchError = null;
    try {
      const profile = preferences.projectProfiles[projectKey(stored.workingDirectory)];
      await launchAgentSession(
        stored.agent,
        stored.workingDirectory,
        true,
        stored.id,
        profile?.launchTarget ?? preferences.launchTarget,
      );
      launcherOpen = false;
      resumeAgent = null;
      resumableSessions = [];
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
      if (integration.installed) {
        await configureIntegration(integration.kind, true);
        integrations = await loadIntegrationStatuses();
      }
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
  ): Promise<boolean> {
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
      return true;
    } catch (error) {
      preferences = previous;
      settingsMessageIsError = true;
      settingsMessage = String(error).replace(/^Error:\s*/, "");
      return false;
    } finally {
      savingSettings = false;
    }
  }

  async function resetSettings() {
    if (!resetConfirming) {
      resetConfirming = true;
      settingsMessage = null;
      return;
    }
    resettingSettings = true;
    settingsMessage = null;
    try {
      preferences = { ...defaultPreferences };
      await savePreferences(preferences);
      if (isTauri) void emit("lume://preferences-changed", preferences);
      selectedLayoutId = null;
      layoutName = "";
      resetConfirming = false;
      await positionWindow();
    } catch (error) {
      settingsMessageIsError = true;
      settingsMessage = String(error).replace(/^Error:\s*/, "");
    } finally {
      resettingSettings = false;
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

  function integrationAgentKind(kind: IntegrationStatus["kind"]): AgentKind {
    return kind === "claude" ? "claude_code" : kind;
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
        const leftIndex = preferred.indexOf(integrationAgentKind(left.kind));
        const rightIndex = preferred.indexOf(integrationAgentKind(right.kind));
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
      const peakVolume = 0.09 * Math.max(0, Math.min(100, preferences.soundVolume)) / 100;
      gain.gain.setValueAtTime(0.0001, context.currentTime);
      gain.gain.exponentialRampToValueAtTime(
        Math.max(0.0001, peakVolume),
        context.currentTime + 0.015,
      );
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
{:else if isWorkflowBridgeWindow}
  <WorkflowBridgeWindow />
{:else}
<main
  class:expanded
  class:dark={effectiveDark}
  class:morphing={morphing !== null}
  class="overlay-shell"
  style={`--panel-radius: ${Math.round(23 - 2 * morphProgress)}px; --morph-width: ${morphWidth}px; --morph-height: ${morphHeight}px;`}
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
      <LumeMascot status={shellStatus} awake={mascotAwake || dragging} size={32} />
      <span class="agent-count">{activeCount}</span>
    </button>
  {:else}
    <section use:observePanelSize class:content-visible={contentVisible} class:morphing class:measuring={measuringPanel} class:palette-open={paletteOpen} class:launcher-open={launcherOpen} class:workflow-settings-open={workflowSettingsOpen} class="panel">
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

      {#if newMobileDevice}
        <aside class="mobile-device-banner" transition:slide={{ duration: 180, easing: cubicOut }}>
          <span class="mobile-device-banner-icon" aria-hidden="true">
            <svg viewBox="0 0 20 20"><rect x="5.5" y="2.5" width="9" height="15" rx="2" /><path d="M8.5 5h3M9 14.5h2" /></svg>
          </span>
          <span>
            <strong>{tr("New phone connected", "Novo celular conectado")}</strong>
            <small>{newMobileDevice.name}</small>
          </span>
          <button type="button" onclick={reviewNewMobileDevice}>
            {tr("Review permissions", "Ver permissões")}
          </button>
        </aside>
      {/if}

      {#if launcherOpen}
        <div class="launcher-popover" transition:fade={{ duration: 100 }}>
          <div class="launcher-popover-scroll">
            <span class="launcher-title">{tr("Open session", "Abrir sessão")}</span>
            {#each launcherIntegrations() as integration}
              <div class:expanded={resumeAgent === integration.kind} class="launcher-agent">
                <div class="launcher-row">
                  <span class="agent-avatar agent-{integration.kind}"><BrandIcon name={integration.kind} size={17} /></span>
                  <strong>{integration.label}</strong>
                  <button disabled={launching !== null} type="button" onclick={() => startSession(integration.kind)}>{tr("New", "Nova")}</button>
                  {#if integration.kind !== "gemini"}
                    <button
                      class:active={resumeAgent === integration.kind}
                      disabled={launching !== null || loadingResumeAgent !== null}
                      type="button"
                      onclick={() => toggleResumeSessions(integration.kind)}
                    >{loadingResumeAgent === integration.kind ? "…" : tr("Resume", "Retomar")}</button>
                  {/if}
                </div>
                {#if resumeAgent === integration.kind}
                  <div class="resume-session-list">
                    {#each resumableSessions as stored (stored.id)}
                      <button
                        class="resume-session"
                        disabled={launching !== null}
                        type="button"
                        title={stored.workingDirectory}
                        onclick={() => resumeStoredSession(stored)}
                      >
                        <span>
                          <strong>{stored.name}</strong>
                          <small>{stored.source} · {relativeTime(stored.updatedAt)}</small>
                        </span>
                        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M6 5h8v8M13.5 5.5 5 14" /></svg>
                      </button>
                    {:else}
                      {#if loadingResumeAgent !== integration.kind}
                        <p>{tr("No resumable sessions were found.", "Nenhuma sessão retomável foi encontrada.")}</p>
                      {/if}
                    {/each}
                  </div>
                {/if}
              </div>
            {:else}
              <p>{tr("No compatible CLI was found.", "Nenhuma CLI compatível foi encontrada.")}</p>
            {/each}
            {#if launchError}<p class="launcher-error">{launchError}</p>{/if}
          </div>
        </div>
      {/if}

      {#if workflowSettingsOpen}
        <button class="workflow-settings-dismiss" type="button" aria-label={tr("Close workflow settings", "Fechar configurações do workflow")} onclick={() => (workflowSettingsOpen = false)}></button>
        <section class="workflow-settings-popover" transition:fade={{ duration: 100 }}>
          <div class="workflow-settings-scroll">
          <header>
            <div>
              <strong>{tr("Workflow settings", "Configurações do workflow")}</strong>
              <small>{tr("Global safety limits", "Limites globais de segurança")}</small>
            </div>
            <button type="button" aria-label={tr("Close", "Fechar")} onclick={() => (workflowSettingsOpen = false)}>×</button>
          </header>

          <div class="workflow-setting-grid">
            <label>
              <span>{tr("Transitions", "Transições")}</span>
              <input type="number" min="1" max="100" value={preferences.workflowSettings.maxTransitions} disabled={workflowSettingsSaving} onchange={(event) => void updateWorkflowSetting("maxTransitions", Number(event.currentTarget.value))} />
            </label>
            <label>
              <span>{tr("Attempts per step", "Tentativas por etapa")}</span>
              <input type="number" min="1" max="10" value={preferences.workflowSettings.maxAttemptsPerStep} disabled={workflowSettingsSaving} onchange={(event) => void updateWorkflowSetting("maxAttemptsPerStep", Number(event.currentTarget.value))} />
            </label>
            <label>
              <span>{tr("Timeout (minutes)", "Timeout (minutos)")}</span>
              <input type="number" min="0" max="1440" value={preferences.workflowSettings.stepTimeoutMinutes} disabled={workflowSettingsSaving} onchange={(event) => void updateWorkflowSetting("stepTimeoutMinutes", Number(event.currentTarget.value))} />
            </label>
            <label>
              <span>{tr("Context tokens", "Tokens de contexto")}</span>
              <input type="number" min="1000" max="100000" step="1000" value={preferences.workflowSettings.maxContextTokens} disabled={workflowSettingsSaving} onchange={(event) => void updateWorkflowSetting("maxContextTokens", Number(event.currentTarget.value))} />
            </label>
          </div>

          <label class="workflow-setting-toggle">
            <span>{tr("Approve sensitive handoffs", "Aprovar handoffs sensíveis")}</span>
            <input type="checkbox" checked={preferences.workflowSettings.requireApprovalForSensitiveContext} disabled={workflowSettingsSaving} onchange={(event) => void updateWorkflowSetting("requireApprovalForSensitiveContext", event.currentTarget.checked)} />
            <i aria-hidden="true"></i>
          </label>
          <label class="workflow-setting-toggle">
            <span>{tr("Protect agent rate limits", "Proteger limites dos agentes")}</span>
            <input type="checkbox" checked={preferences.workflowSettings.pauseOnRateLimit} disabled={workflowSettingsSaving} onchange={(event) => void updateWorkflowSetting("pauseOnRateLimit", event.currentTarget.checked)} />
            <i aria-hidden="true"></i>
          </label>
          {#if preferences.workflowSettings.pauseOnRateLimit}
            <label class="workflow-reserve-setting">
              <span>{tr("Minimum remaining", "Reserva mínima")}</span>
              <input type="range" min="0" max="50" step="5" value={preferences.workflowSettings.minimumRateLimitRemainingPercent} disabled={workflowSettingsSaving} onchange={(event) => void updateWorkflowSetting("minimumRateLimitRemainingPercent", Number(event.currentTarget.value))} />
              <strong>{preferences.workflowSettings.minimumRateLimitRemainingPercent}%</strong>
            </label>
          {/if}

          {#if missingWorkflowSteps().length > 0}
            <div class="workflow-missing-sessions">
              <strong>{tr("Missing agents", "Agentes ausentes")}</strong>
              {#each missingWorkflowSteps() as missing (`${missing.group.id}:${missing.step.id}`)}
                <label>
                    <span title={`${missing.step.customRoleLabel || missing.step.role} · ${tr("Step", "Etapa")} ${missing.index + 1}`}>{missing.step.customRoleLabel || missing.step.role} · {missing.index + 1}</span>
                  <LumeSelect
                    ariaLabel={tr("Replacement agent session", "Sessão substituta do agente")}
                    value=""
                    minWidth={168}
                    options={[
                      { value: "", label: workflowRebindingStepId === missing.step.id ? tr("Replacing…", "Substituindo…") : tr("Replace session…", "Substituir sessão…") },
                      ...workflowReplacementSessions(missing.group.id, missing.step.id).map((session) => ({
                        value: workflowSessionKey(session),
                        label: sessionDisplayName(session),
                        description: `${session.agentLabel} · ${session.project}`,
                      })),
                    ]}
                    onValueChange={(value) => void replaceWorkflowSession(missing.group.id, missing.step.id, value)}
                  />
                </label>
              {/each}
            </div>
          {/if}
          </div>
        </section>
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

      {#if shortcutEditorKey}
        <div class="shortcut-editor-layer" transition:fade={{ duration: 120 }}>
          <button class="shortcut-editor-backdrop" type="button" aria-label={tr("Close shortcut editor", "Fechar editor de atalho")} onclick={() => (shortcutEditorKey = null)}></button>
          <div class="shortcut-editor" role="dialog" aria-modal="true" aria-labelledby="shortcut-editor-title">
            <strong id="shortcut-editor-title">{tr("Press a new shortcut", "Pressione um novo atalho")}</strong>
            <small>{tr("Use one or more modifier keys with another key.", "Use uma ou mais teclas modificadoras com outra tecla.")}</small>
            <button data-shortcut-capture class="shortcut-capture" type="button" onkeydown={captureShortcut}>
              <kbd>{shortcutDraft || tr("Press keys…", "Pressione as teclas…")}</kbd>
            </button>
            {#if shortcutEditorError}<p>{shortcutEditorError}</p>{/if}
            <div class="shortcut-editor-actions">
              <button type="button" onclick={() => (shortcutEditorKey = null)}>{tr("Cancel", "Cancelar")}</button>
              <button class="primary" disabled={!shortcutDraft || savingSettings} type="button" onclick={() => void saveShortcut()}>{tr("Save", "Salvar")}</button>
            </div>
          </div>
        </div>
      {/if}

      <div class="panel-content">
        {#if view === "sessions"}
          <div class="session-list">
            {#each sessions as session (session.id)}
              {@const visibleLastResponse = stripInternalAgentMetadata(session.lastResponse)}
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
                      <strong>{sessionDisplayName(session)}</strong>
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
                    <span class="project-name" title={session.workingDirectory}>
                      {sessionDirectoryName(session)}
                    </span>
                    <span class="status-line status-{session.status}">
                      {#if session.status === "running"}
                        <span class="running-dots" aria-hidden="true"><i></i><i></i><i></i></span>
                      {:else}
                        <i></i>
                      {/if}
                      {shown(session.statusLabel)}
                    </span>
                    {#if visibleLastResponse && selectedId !== session.id}
                      <span class="response-preview">
                        <b>{tr("Final response", "Resposta final")}</b>
                        <span>{visibleLastResponse}</span>
                      </span>
                    {/if}
                  </span>
                  <svg class="chevron" viewBox="0 0 20 20" aria-hidden="true">
                    <path d="m8 5 5 5-5 5" />
                  </svg>
                </button>

                {#if selectedId === session.id}
                  {@const capabilities = sessionCapabilities(session)}
                  {@const queuedPrompts = pendingQueuedPrompts(session)}
                  <div class="session-details" transition:slide={{ duration: 190, easing: cubicOut }}>
                    {#if visibleLastResponse}
                      <div class="final-response">
                        <span class="eyebrow">{tr("Final response", "Resposta final")}</span>
                        <button
                          class="final-response-copy"
                          type="button"
                          onclick={() => copyResult(`${session.id}-latest`, visibleLastResponse)}
                          aria-label={copiedResultId === `${session.id}-latest` ? tr("Copied", "Copiado") : tr("Copy final response", "Copiar resposta final")}
                          title={copiedResultId === `${session.id}-latest` ? tr("Copied", "Copiado") : tr("Copy", "Copiar")}
                        >
                          {#if copiedResultId === `${session.id}-latest`}
                            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5 10 3 3 7-7" /></svg>
                          {:else}
                            <svg viewBox="0 0 20 20" aria-hidden="true"><rect x="7" y="6" width="8" height="9" rx="1.5" /><path d="M12 6V4.5A1.5 1.5 0 0 0 10.5 3h-6A1.5 1.5 0 0 0 3 4.5v7A1.5 1.5 0 0 0 4.5 13H7" /></svg>
                          {/if}
                        </button>
                        <p>{visibleLastResponse}</p>
                      </div>
                    {/if}

                    <div class="session-action-bar" aria-label={tr("Session actions", "Ações da sessão")}>
                      <button
                        class="session-action-button"
                        type="button"
                        data-label={tr("Rename session", "Renomear sessão")}
                        aria-label={tr("Rename session", "Renomear sessão")}
                        onclick={() => beginSessionRename(session)}
                      >
                        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m4 14-.5 2.5L6 16l9-9-2-2-9 9Z"></path><path d="m11.5 6.5 2 2"></path></svg>
                      </button>
                      {#if capabilities.canOpenSource}
                        <button
                          class="session-action-button"
                          type="button"
                          data-label={tr("Open source", "Abrir origem")}
                          aria-label={tr("Open source", "Abrir origem")}
                          onclick={() => openSessionSource(session.id)}
                        >
                          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M7 5h8v8M14.5 5.5 6 14"></path><path d="M13 15H5V7"></path></svg>
                        </button>
                      {/if}
                      {#if canContinueSession(session) && canSubmitToSession(session)}
                        <button
                          class:active={composerSessionId === session.id}
                          class="session-action-button"
                          type="button"
                          data-label={session.status === "waiting_for_input" ? tr("Send prompt", "Enviar prompt") : tr("Continue", "Continuar")}
                          aria-label={session.status === "waiting_for_input" ? tr("Send prompt", "Enviar prompt") : tr("Continue", "Continuar")}
                          onclick={() => toggleSessionComposer(session)}
                        >
                          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 10h11M11 6l4 4-4 4"></path></svg>
                        </button>
                      {/if}
                      {#if canInterruptSession(session)}
                        <button
                          class="session-action-button warning"
                          disabled={interruptingSessionId === session.id}
                          type="button"
                          data-label={interruptingSessionId === session.id ? tr("Interrupting…", "Interrompendo…") : tr("Interrupt prompt", "Interromper prompt")}
                          aria-label={interruptingSessionId === session.id ? tr("Interrupting…", "Interrompendo…") : tr("Interrupt prompt", "Interromper prompt")}
                          onclick={() => void interruptSessionPrompt(session)}
                        >
                          <svg viewBox="0 0 20 20" aria-hidden="true"><rect x="6" y="6" width="8" height="8" rx="1"></rect></svg>
                        </button>
                      {/if}
                      {#if canTerminateSession(session)}
                        <button
                          class="session-action-button danger"
                          disabled={terminatingSessionId === session.id}
                          type="button"
                          data-label={tr("Stop agent", "Encerrar agente")}
                          aria-label={tr("Stop agent", "Encerrar agente")}
                          onclick={() => void terminateAgent(session)}
                        >
                          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 3v7M5.5 5.5a6 6 0 1 0 9 0"></path></svg>
                        </button>
                      {/if}
                    </div>

                    {#if renamingSessionId === session.id}
                      <form class="session-name-editor" onsubmit={(event) => { event.preventDefault(); void saveSessionRename(session); }}>
                        <input
                          maxlength="80"
                          bind:value={renameDraft}
                          aria-label={tr("Session name", "Nome da sessão")}
                          onkeydown={(event) => {
                            if (event.key === "Escape") {
                              event.preventDefault();
                              cancelSessionRename();
                            }
                          }}
                        />
                        <button class="primary" disabled={renamingSession} type="submit">{tr("Save", "Salvar")}</button>
                        <button disabled={renamingSession} type="button" onclick={cancelSessionRename}>{tr("Cancel", "Cancelar")}</button>
                        {#if renameError}<small>{renameError}</small>{/if}
                      </form>
                    {/if}

                    {#if terminateConfirmId === session.id}
                      <div class="terminate-agent-control confirming">
                        <span>{tr("Stop the agent and its running commands?", "Encerrar o agente e os comandos em execução?")}</span>
                        <button type="button" onclick={() => (terminateConfirmId = null)}>{tr("Cancel", "Cancelar")}</button>
                        <button class="danger" disabled={terminatingSessionId === session.id} type="button" onclick={() => void terminateAgent(session)}>
                          {terminatingSessionId === session.id ? tr("Stopping…", "Encerrando…") : tr("Stop", "Encerrar")}
                        </button>
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
                    {#if session.pendingQuestion}
                      <div class="question-block">
                        <span class="eyebrow">{tr("Agent question", "Pergunta do agente")}</span>
                        {#each session.pendingQuestion.questions as question}
                          <section>
                            <strong>{shown(question.question)}</strong>
                            {#if question.options.length}
                              <div class="question-actions">
                                {#each question.options as option, index}
                                  <button
                                    class:selected={questionSelections[`${session.pendingQuestion.id}:${question.id}`] === option.label}
                                    type="button"
                                    onclick={() => void handleQuestionOption(session, question.id, option.label)}
                                  >
                                    <b>{index + 1}</b> {shown(option.label)}
                                  </button>
                                {/each}
                              </div>
                            {/if}
                            <small>{tr("Choose an option or type its number below.", "Escolha uma opção ou digite o número abaixo.")}</small>
                          </section>
                        {/each}
                        {#if permissionError}
                          <p class="inline-error" transition:fade>{permissionError}</p>
                        {/if}
                      </div>
                    {/if}

                    {#if canContinueSession(session) && canSubmitToSession(session) && composerSessionId === session.id}
                        <form
                          class="inline-composer"
                          onpaste={(event) => void pasteSessionImages(event, session)}
                          onsubmit={(event) => {
                            event.preventDefault();
                            void sendSessionPrompt(session);
                          }}
                          transition:slide={{ duration: 160, easing: cubicOut }}
                        >
                          {#if composerAttachments.length}
                            <div class="inline-attachments">
                              {#each composerAttachments as attachment, index}
                                <span title={attachment.name}>
                                  <img src={attachment.previewDataUrl} alt={attachment.name} />
                                  <button
                                    type="button"
                                    onclick={() => removeComposerImage(index)}
                                    aria-label={tr("Remove image", "Remover imagem")}
                                  >×</button>
                                </span>
                              {/each}
                            </div>
                          {/if}
                          {#if queuedPrompts[0]}
                            <button
                              class="inline-queue-tray"
                              disabled={steeringQueuedActivityId !== null}
                              type="button"
                              onclick={() => void steerSessionQueuedPrompt(session)}
                              aria-label={tr("Steer the next queued prompt now", "Enviar agora o próximo prompt da fila")}
                            >
                              <span class="queue-mark" aria-hidden="true">↳</span>
                              <span class="queue-copy">
                                <small>{queuedPrompts.length > 1 ? tr(`${queuedPrompts.length} queued prompts`, `${queuedPrompts.length} prompts na fila`) : tr("Queued next", "Próximo na fila")}</small>
                                <strong>{queuedPrompts[0].detail || tr("Prompt with attached images", "Prompt com imagens anexadas")}</strong>
                              </span>
                              <span class="queue-shortcut">
                                <kbd>Tab</kbd>
                                <small>{steeringQueuedActivityId === queuedPrompts[0].id ? tr("Steering…", "Enviando…") : tr("Steer now", "Enviar agora")}</small>
                              </span>
                            </button>
                          {/if}
                          <div class="inline-composer-controls">
                            <textarea
                              bind:value={composerPrompt}
                              onkeydown={(event) => handleSessionComposerKeydown(event, session)}
                              aria-label={tr(`New prompt for ${sessionDisplayName(session)}`, `Novo prompt para ${sessionDisplayName(session)}`)}
                              placeholder={tr("Paste an image or enter the next prompt…", "Cole uma imagem ou digite o próximo prompt…")}
                              rows="2"
                            ></textarea>
                            <button
                              disabled={(!composerPrompt.trim() && composerAttachments.length === 0) || composerSending}
                              type="submit"
                              aria-label={tr("Send prompt", "Enviar prompt")}
                            >
                              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m4 10 12-6-4 12-2-4zM10 12l2-2" /></svg>
                            </button>
                          </div>
                        </form>
                      {#if composerMessage}<p class="inline-error">{composerMessage}</p>{/if}
                    {/if}

                    {#if sessionActionMessage && selectedId === session.id}
                      <p class="inline-error">{sessionActionMessage}</p>
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
            <div class="layout-toolbar">
              <LumeSelect
                ariaLabel={tr("Saved whiteboard layout", "Layout salvo do whiteboard")}
                value={selectedLayoutId ?? ""}
                minWidth={142}
                options={[
                  { value: "", label: tr("New layout", "Novo layout") },
                  ...preferences.whiteboardLayouts.map((layout) => ({ value: layout.id, label: layout.name })),
                ]}
                onValueChange={(value) => {
                  selectedLayoutId = value || null;
                  layoutName = preferences.whiteboardLayouts.find((layout) => layout.id === selectedLayoutId)?.name ?? "";
                }}
              />
              {#if !selectedLayoutId}
                <input bind:value={layoutName} maxlength="48" placeholder={tr("Layout name", "Nome do layout")} />
              {/if}
              <button
                class="layout-action"
                type="button"
                title={selectedLayoutId ? tr("Update layout", "Atualizar layout") : tr("Save new layout", "Salvar novo layout")}
                aria-label={selectedLayoutId ? tr("Update layout", "Atualizar layout") : tr("Save new layout", "Salvar novo layout")}
                onclick={saveCurrentLayout}
              >
                <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 3.5h10l2 2v11H4v-13Z" /><path d="M7 3.5v5h6v-5M7 16.5v-5h6v5" /></svg>
              </button>
              {#if selectedLayoutId}
                {@const selectedLayout = preferences.whiteboardLayouts.find((layout) => layout.id === selectedLayoutId)}
                <button
                  class:loading={restoringLayout}
                  class="layout-action"
                  disabled={!selectedLayout || restoringLayout}
                  type="button"
                  title={tr("Restore layout", "Restaurar layout")}
                  aria-label={tr("Restore layout", "Restaurar layout")}
                  onclick={() => selectedLayout && restoreSavedLayout(selectedLayout)}
                >
                  {#if restoringLayout}
                    <span class="layout-spinner" aria-hidden="true"></span>
                  {:else}
                    <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5.5 6.5H2.8V3.8" /><path d="M3.2 6.2A7 7 0 1 1 3 13" /></svg>
                  {/if}
                </button>
                <button
                  class="layout-action layout-delete"
                  type="button"
                  title={tr("Delete layout", "Excluir layout")}
                  aria-label={tr("Delete layout", "Excluir layout")}
                  onclick={() => selectedLayoutId && deleteSavedLayout(selectedLayoutId)}
                >
                  <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 6h12M8 3.5h4L13 6H7l1-2.5ZM6 6l.7 10h6.6L14 6M8.5 9v4.5M11.5 9v4.5" /></svg>
                </button>
              {/if}
            </div>

            <div class:enabled={preferences.workflowEnabled} class="workflow-group-row workflow-global-mode">
              <span class="workflow-group-symbol" aria-hidden="true">
                <svg viewBox="0 0 24 20">
                  <path class="normal-link" d="M3 10h18" />
                  <path class="workflow-link" d="M12 2.5 3 17h18L12 2.5Z" />
                  <circle class="node-start" cx="3" cy="17" r="2.25" />
                  <circle class="node-center" cx="12" cy="2.5" r="2.25" />
                  <circle class="node-end" cx="21" cy="17" r="2.25" />
                </svg>
              </span>
              <span class="workflow-group-copy">
                <strong>{tr("Terminal mode", "Modo dos terminais")}</strong>
              </span>
              <span class:workflow-active={preferences.workflowEnabled} class="workflow-mode-switch" role="group" aria-label={tr("Terminal mode", "Modo dos terminais")}>
                <button
                  class:active={!preferences.workflowEnabled}
                  disabled={workflowModeChanging}
                  type="button"
                  onclick={() => void togglePanelWorkflowMode(false)}
                >{tr("Normal", "Normal")}</button>
                <button
                  class:active={preferences.workflowEnabled}
                  disabled={workflowModeChanging}
                  type="button"
                  onclick={() => void togglePanelWorkflowMode(true)}
                >Workflow</button>
              </span>
              <button
                class:active={workflowSettingsOpen}
                class="workflow-settings-trigger"
                type="button"
                title={tr("Workflow settings", "Configurações do workflow")}
                aria-label={tr("Workflow settings", "Configurações do workflow")}
                aria-expanded={workflowSettingsOpen}
                onclick={() => void toggleWorkflowSettings()}
              >
                <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 6h12M4 10h12M4 14h12" /></svg>
              </button>
            </div>

            <div class="terminal-picker">
              {#each sessions as session (session.id)}
                <div class="terminal-picker-row">
                  <span class="agent-avatar agent-{session.agent}"><BrandIcon name={session.agent} size={18} /></span>
                  <span class="terminal-picker-copy">
                    <strong>{sessionDisplayName(session)}</strong>
                    <small>{session.agentLabel} · {session.project}</small>
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
                    <p>{stripInternalAgentMetadata(note.body)}</p>
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
                {@const visibleResultResponse = stripInternalAgentMetadata(item.result.response)}
                <article class="result-card">
                  <div class="result-card-top">
                    <div class="result-heading">
                      <span class="agent-avatar agent-{item.session.agent}"><BrandIcon name={item.session.agent} size={15} /></span>
                      <span><strong>{sessionDisplayName(item.session)}</strong><small>{item.session.agentLabel} · {item.session.project} · {relativeTime(item.result.createdAt)}</small></span>
                    </div>
                    <div class="result-actions" aria-label={tr("Result actions", "Ações do resultado")}>
                      <button
                        class="result-action-button"
                        type="button"
                        data-label={copiedResultId === item.result.id ? tr("Copied", "Copiado") : tr("Copy", "Copiar")}
                        aria-label={copiedResultId === item.result.id ? tr("Copied", "Copiado") : tr("Copy", "Copiar")}
                        onclick={() => copyResult(item.result.id, visibleResultResponse)}
                      >
                        {#if copiedResultId === item.result.id}
                          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5 10 3 3 7-7" /></svg>
                        {:else}
                          <svg viewBox="0 0 20 20" aria-hidden="true"><rect x="7" y="6" width="8" height="9" rx="1.5" /><path d="M12 6V4.5A1.5 1.5 0 0 0 10.5 3h-6A1.5 1.5 0 0 0 3 4.5v7A1.5 1.5 0 0 0 4.5 13H7" /></svg>
                        {/if}
                      </button>
                      <button
                        class="result-action-button"
                        disabled={savingNoteId === item.result.id}
                        type="button"
                        data-label={savingNoteId === item.result.id ? tr("Saving…", "Salvando…") : tr("Save note", "Salvar nota")}
                        aria-label={savingNoteId === item.result.id ? tr("Saving…", "Salvando…") : tr("Save note", "Salvar nota")}
                        onclick={() => keepResultAsNote(item.session, item.result.id)}
                      >
                        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 3.5h10v13l-5-3-5 3v-13Z" /></svg>
                      </button>
                      {#if capabilities.canPrompt && canContinueSession(item.session)}
                        <button
                          class="result-action-button"
                          type="button"
                          data-label={tr("Continue", "Continuar")}
                          aria-label={tr("Continue", "Continuar")}
                          onclick={() => continueFromResult(item.session)}
                        >
                          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 10h11M11 6l4 4-4 4" /></svg>
                        </button>
                      {/if}
                      {#if capabilities.canOpenSource}
                        <button
                          class="result-action-button"
                          type="button"
                          data-label={tr("Open source", "Abrir origem")}
                          aria-label={tr("Open source", "Abrir origem")}
                          onclick={() => openSessionSource(item.session.id)}
                        >
                          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M7 5h8v8M14.5 5.5 6 14" /><path d="M13 15H5V7" /></svg>
                        </button>
                      {/if}
                    </div>
                  </div>
                  <p>{visibleResultResponse}</p>
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
                </article>
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
            <details class="settings-section" open>
              <summary class="settings-section-label">{tr("Agents", "Agentes")}</summary>
              <div class="settings-section-content">
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
              </div>
            </details>
            <details class="settings-section">
              <summary class="settings-section-label">{tr("External detectors", "Detectores externos")}</summary>
              <div class="settings-section-content">
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
              </div>
            </details>
            {#if settingsMessage}
              <p class:error={settingsMessageIsError} class="settings-feedback" transition:fade>
                {settingsMessage}
              </p>
            {/if}
            <details class="settings-section">
              <summary class="settings-section-label">Interface</summary>
              <div class="settings-section-content">
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
              </div>
            </details>
            <details class="settings-section">
              <summary class="settings-section-label">{tr("Preferences", "Preferências")}</summary>
              <div class="settings-section-content">
            <label class="field-row">
              <span><strong>{tr("Language", "Idioma")}</strong><small>{tr("Lume interface language.", "Idioma da interface do Lume.")}</small></span>
              <LumeSelect
                ariaLabel={tr("Language", "Idioma")}
                value={preferences.language}
                options={[{ value: "en", label: "English" }, { value: "pt-BR", label: "Português" }]}
                onValueChange={(value) => updatePreference("language", value as Preferences["language"])}
              />
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
              <div><strong>{tr("Desktop pop-up notifications", "Notificações pop-up no desktop")}</strong><span>{tr("Show task and permission alerts outside Lume.", "Mostre alertas de tarefas e permissões fora do Lume.")}</span></div>
              <label class="switch">
                <input
                  type="checkbox"
                  checked={preferences.popupNotificationsEnabled}
                  onchange={(event) =>
                    updatePreference("popupNotificationsEnabled", event.currentTarget.checked)}
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
            <div class:disabled={!preferences.soundEnabled} class="setting-row sound-volume-row">
              <div><strong>{tr("Sound volume", "Volume dos sons")}</strong><span>{tr("Adjust notification tones.", "Ajuste o volume dos alertas.")}</span></div>
              <label class="volume-control">
                <input
                  aria-label={tr("Sound volume", "Volume dos sons")}
                  disabled={!preferences.soundEnabled}
                  type="range"
                  min="0"
                  max="100"
                  step="5"
                  value={preferences.soundVolume}
                  oninput={(event) =>
                    updatePreference("soundVolume", Number(event.currentTarget.value))}
                />
                <output>{preferences.soundVolume}%</output>
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
              <LumeSelect
                ariaLabel={tr("Monitor", "Monitor")}
                value={preferences.monitorId ?? ""}
                options={[
                  { value: "", label: tr("Primary", "Principal") },
                  ...monitors.map((monitor) => ({ value: monitor.id, label: monitor.label })),
                ]}
                onValueChange={(value) => updatePreference("monitorId", value || undefined)}
              />
            </label>
            <label class="field-row">
              <span><strong>{tr("History", "Histórico")}</strong><small>{tr("Local, sanitized summaries.", "Resumos locais e sanitizados.")}</small></span>
              <LumeSelect
                ariaLabel={tr("History retention", "Retenção do histórico")}
                value={String(preferences.historyRetentionDays)}
                options={[
                  { value: "7", label: tr("7 days", "7 dias") },
                  { value: "30", label: tr("30 days", "30 dias") },
                  { value: "90", label: tr("90 days", "90 dias") },
                ]}
                onValueChange={(value) => updatePreference("historyRetentionDays", Number(value))}
              />
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
              </div>
            </details>
            <details class="settings-section">
              <summary class="settings-section-label">{tr("Keyboard shortcuts", "Atalhos de teclado")}</summary>
              <div class="settings-section-content">
                {#each [
                  ["openShortcut", tr("Open Lume", "Abrir o Lume"), tr("Shows and expands the overlay.", "Exibe e expande a sobreposição.")],
                  ["globalShortcut", tr("Command palette", "Paleta de comandos"), tr("Search actions and active agents.", "Busca ações e agentes ativos.")],
                  ["newSessionShortcut", tr("New session", "Nova sessão"), tr("Opens the agent launcher.", "Abre o iniciador de agentes.")],
                  ["whiteboardShortcut", "Whiteboard", tr("Opens the floating terminal hub.", "Abre o hub de terminais flutuantes.")],
                ] as shortcut}
                  <div class="field-row shortcut-row">
                    <span><strong>{shortcut[1]}</strong><small>{shortcut[2]}</small></span>
                    <button
                      class="shortcut-input"
                      type="button"
                      aria-label={shortcut[1]}
                      onclick={() => void openShortcutEditor(shortcut[0] as ShortcutPreferenceKey)}
                    >{preferences[shortcut[0] as ShortcutPreferenceKey]}</button>
                  </div>
                {/each}
              </div>
            </details>
            <details class="settings-section">
              <summary class="settings-section-label">{tr("Project profiles", "Perfis por projeto")}</summary>
              <div class="settings-section-content">
            {#if detectedProjects.length > 0}
              <label class="field-row">
                <span><strong>{tr("Project", "Projeto")}</strong><small>{tr("Overrides only for this project.", "Ajustes somente para este projeto.")}</small></span>
                <LumeSelect
                  ariaLabel={tr("Project", "Projeto")}
                  value={selectedProfileKey ?? ""}
                  minWidth={145}
                  options={detectedProjects.map((project) => ({ value: project.key, label: project.label }))}
                  onValueChange={(value) => (selectedProfileKey = value)}
                />
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
                <LumeSelect
                  ariaLabel={tr("Profile monitor", "Monitor do perfil")}
                  value={selectedProjectProfile?.monitorId ?? ""}
                  options={[
                    { value: "", label: tr("Global", "Global") },
                    ...monitors.map((monitor) => ({ value: monitor.id, label: monitor.label })),
                  ]}
                  onValueChange={(value) => updateSelectedProjectProfile({ monitorId: value || undefined })}
                />
              </label>
              <div class="setting-row">
                <div><strong>{tr("Capsule position", "Posição da cápsula")}</strong><span>{selectedProjectProfile?.overlayX !== undefined ? `${selectedProjectProfile.overlayX}, ${selectedProjectProfile.overlayY}` : tr("Use the global position", "Usar a posição global")}</span></div>
                <button class="profile-action" type="button" onclick={captureProfilePosition}>{tr("Use current", "Usar atual")}</button>
              </div>
              <label class="field-row">
                <span><strong>{tr("Permission preset", "Preset de permissão")}</strong><small>{tr("Applied only when Lume starts a new session.", "Aplicado apenas ao iniciar uma nova sessão pelo Lume.")}</small></span>
                <LumeSelect
                  ariaLabel={tr("Permission preset", "Preset de permissão")}
                  value={selectedProjectProfile?.permissionMode ?? ""}
                  minWidth={145}
                  options={[
                    { value: "", label: tr("Agent default", "Padrão do agente") },
                    { value: "plan", label: "Plan" },
                    { value: "read_only", label: tr("Read only", "Somente leitura") },
                    { value: "workspace_write", label: "Workspace write" },
                    { value: "full_access", label: tr("Full access", "Acesso total"), description: tr("No sandbox", "Sem sandbox") },
                  ]}
                  onValueChange={(value) => updateSelectedProjectProfile({ permissionMode: (value || undefined) as Preferences["projectProfiles"][string]["permissionMode"] })}
                />
              </label>
              <label class="field-row">
                <span><strong>{tr("Approval policy", "Política de aprovação")}</strong><small>{tr("Supported by Codex launch profiles.", "Suportada nos perfis de abertura do Codex.")}</small></span>
                <LumeSelect
                  ariaLabel={tr("Approval policy", "Política de aprovação")}
                  value={selectedProjectProfile?.approvalPolicy ?? ""}
                  options={[
                    { value: "", label: tr("Agent default", "Padrão do agente") },
                    { value: "untrusted", label: "Untrusted" },
                    { value: "on-request", label: "On request" },
                    { value: "never", label: "Never" },
                  ]}
                  onValueChange={(value) => updateSelectedProjectProfile({ approvalPolicy: (value || undefined) as Preferences["projectProfiles"][string]["approvalPolicy"] })}
                />
              </label>
              <label class="field-row">
                <span><strong>Whiteboard</strong><small>{tr("Default saved layout for this project.", "Layout salvo padrão deste projeto.")}</small></span>
                <LumeSelect
                  ariaLabel="Whiteboard"
                  value={selectedProjectProfile?.whiteboardLayoutId ?? ""}
                  options={[
                    { value: "", label: tr("No layout", "Sem layout") },
                    ...preferences.whiteboardLayouts.map((layout) => ({ value: layout.id, label: layout.name })),
                  ]}
                  onValueChange={(value) => updateSelectedProjectProfile({ whiteboardLayoutId: value || undefined })}
                />
              </label>
              <div class="launch-setting preferred-agents-setting">
                <span><strong>{tr("Preferred agents", "Agentes preferidos")}</strong><small>{tr("Shown first in the launcher.", "Aparecem primeiro no iniciador.")}</small></span>
                <div class="agent-preferences">
                  {#each integrations as integration (integration.kind)}
                    <button class:active={(selectedProjectProfile?.preferredAgents ?? []).includes(integrationAgentKind(integration.kind))} type="button" onclick={() => togglePreferredAgent(integrationAgentKind(integration.kind))}>
                      <BrandIcon name={integrationAgentKind(integration.kind)} size={14} />{integration.label}
                    </button>
                  {/each}
                </div>
              </div>
              <button class="apply-profile-button" type="button" onclick={applySelectedProjectProfile}>{tr("Apply project profile", "Aplicar perfil do projeto")}</button>
            {:else}
              <p class="profile-empty">{tr("Profiles appear after a project is detected.", "Os perfis aparecem depois que um projeto é detectado.")}</p>
            {/if}
              </div>
            </details>
            <details class="settings-section" data-mobile-access-section>
              <summary class="settings-section-label">{tr("Mobile access", "Acesso mobile")}</summary>
              <div class="settings-section-content">
                <div class="mobile-access-card">
              <div class="mobile-access-header">
                <div>
                  <strong>{tr("Local network gateway", "Gateway da rede local")}</strong>
                  <span>{tr("Only paired devices can read your sessions.", "Apenas dispositivos pareados podem ler suas sessões.")}</span>
                </div>
                <label class="switch">
                  <input
                    type="checkbox"
                    checked={mobileStatus?.networkReachable ?? false}
                    disabled={!isTauri || mobileBusy}
                    onchange={() => void toggleMobileAccess()}
                  />
                  <span></span>
                </label>
              </div>
              {#if !isTauri}
                <p class="mobile-message">{tr("Open the floating Lume desktop app to enable mobile access.", "Abra o aplicativo desktop flutuante do Lume para ativar o acesso mobile.")}</p>
              {/if}
              {#if mobileBusy}
                <p class="mobile-message">{tr("Starting the secure gateway…", "Iniciando o gateway seguro…")}</p>
              {/if}
              {#if mobileStatus?.networkReachable}
                <div class="mobile-address">
                  <span><strong>{tr("Encrypted local", "Local criptografado")}</strong><code>{mobileStatus.address}</code></span>
                  <button type="button" onclick={() => void copyMobileValue(mobileStatus?.address ?? "")}>{tr("Copy", "Copiar")}</button>
                </div>
                <div class="mobile-pair-action">
                  <span>
                    <strong>{tr("Scan once to connect", "Leia uma vez para conectar")}</strong>
                    <small>{tr("The installed app opens automatically. Otherwise, the PWA opens with the APK download option.", "O aplicativo instalado abre automaticamente. Caso contrário, o PWA abre com a opção de baixar o APK.")}</small>
                  </span>
                  <button disabled={mobileBusy} type="button" onclick={() => void createMobilePairing()}>{pairingOffer ? tr("New code", "Novo código") : tr("Show QR", "Mostrar QR")}</button>
                </div>
                {#if pairingOffer && pairingQr}
                  <div class="mobile-pairing" transition:fade={{ duration: 140 }}>
                    <img src={pairingQr} alt={tr("Lume mobile pairing QR code", "QR Code de pareamento mobile do Lume")} />
                    <span>
                      <strong>{tr("One-time code", "Código de uso único")}</strong>
                      <code>{pairingOffer.code}</code>
                      <small>{tr("Expires", "Expira")} {new Date(pairingOffer.expiresAt).toLocaleTimeString()} · {tr("Same Wi-Fi required", "Requer a mesma rede Wi-Fi")}</small>
                    </span>
                  </div>
                {/if}
                <div class="mobile-apk">
                  <span>
                    <strong>{tr("Optional direct APK download", "Download direto opcional do APK")}</strong>
                    <code>{mobileApkUrl}</code>
                    <small>{tr("The same download is offered after scanning the QR code.", "O mesmo download é oferecido depois da leitura do QR Code.")}</small>
                  </span>
                  <button type="button" onclick={() => void copyMobileValue(mobileApkUrl)}>{tr("Copy link", "Copiar link")}</button>
                </div>
              {/if}
              {#if mobileMessage}
                <p class:error={mobileMessageIsError} class="mobile-message">{mobileMessage}</p>
              {/if}
                </div>
                {#if pairedDevices.length}
                  <div class="paired-devices">
                    <div class="paired-devices-intro">
                      <strong>{tr("Phone permissions", "Permissões do telefone")}</strong>
                      </div>
                    {#each pairedDevices as device (device.id)}
                      <article class="paired-device-card" data-mobile-device-id={device.id}>
                        <div class="paired-device-header">
                          <span class="paired-device-info">
                            <strong>{device.name}</strong>
                            <small>{dev && device.id === devMobileDeviceId ? tr("Development preview", "Demonstração do ambiente de desenvolvimento") : device.lastSeenAt ? relativeTime(device.lastSeenAt) : tr("Not used yet", "Ainda não utilizado")}</small>
                          </span>
                          {#if dev && device.id === devMobileDeviceId}
                            <span class="preview-badge">{tr("Preview", "Demonstração")}</span>
                          {:else}
                            <button class="revoke-device" disabled={mobileBusy} type="button" onclick={() => void removePairedDevice(device.id)}>{tr("Revoke access", "Revogar acesso")}</button>
                          {/if}
                        </div>
                        <div class="device-permissions">
                          <div class="device-permission">
                            <span class="permission-copy">
                              <strong>{tr("View sessions", "Visualizar sessões")}</strong>
                            </span>
                            <span class="permission-state allowed">{tr("Always allowed", "Sempre permitido")}</span>
                          </div>
                          {#each [
                            {
                              scope: "prompt" as MobileScope,
                              label: tr("Send prompts", "Enviar prompts"),
                            },
                            {
                              scope: "approve" as MobileScope,
                              label: tr("Manage approvals", "Gerenciar aprovações"),
                            },
                            {
                              scope: "terminate" as MobileScope,
                              label: tr("Stop agents", "Encerrar agentes"),
                            },
                          ] as permission}
                            <div class:active={device.scopes.includes(permission.scope)} class="device-permission">
                              <span class="permission-copy">
                                <strong>{permission.label}</strong>
                              </span>
                              <span class="permission-choice">
                                <label class="switch">
                                  <input
                                    aria-label={`${permission.label}: ${device.scopes.includes(permission.scope) ? tr("Allowed", "Permitido") : tr("Not allowed", "Não permitido")}`}
                                    type="checkbox"
                                    checked={device.scopes.includes(permission.scope)}
                                    disabled={mobileBusy}
                                    onchange={() => void togglePairedDeviceScope(device, permission.scope)}
                                  />
                                  <span></span>
                                </label>
                              </span>
                            </div>
                          {/each}
                        </div>
                      </article>
                    {/each}
                  </div>
                {/if}
              </div>
            </details>
            <section class="settings-section settings-section-static">
              <div class="settings-section-label">{tr("About", "Sobre")}</div>
              <div class="settings-section-content">
                <div class="update-card" data-update-card aria-live="polite">
              <div class="update-main">
                <LumeLogo size={30} />
                <div class="update-copy">
                  <strong>Lume</strong>
                  <span>{tr("Version", "Versão")} {appVersion}</span>
                </div>
                {#if updateState === "available"}
                  <button class="update-available" type="button" onclick={handleInstallUpdate}>
                    {tr("Update to", "Atualizar para")} {availableVersion}
                  </button>
                {:else}
                  <button
                    type="button"
                    disabled={updateState === "checking" || updateState === "downloading" || updateState === "ready"}
                    onclick={handleUpdateButton}
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
              </div>
            </section>
            <details class="settings-section">
              <summary class="settings-section-label">{tr("Reset", "Redefinir")}</summary>
              <div class="settings-section-content">
                <div class:confirming={resetConfirming} class="reset-settings-control">
                  {#if resetConfirming}
                    <span>{tr("Reset all Lume settings to their defaults?", "Redefinir todas as configurações do Lume para o padrão?")}</span>
                    <button type="button" onclick={() => (resetConfirming = false)}>{tr("Cancel", "Cancelar")}</button>
                    <button class="danger" disabled={resettingSettings} type="button" onclick={() => void resetSettings()}>
                      {resettingSettings ? tr("Resetting…", "Redefinindo…") : tr("Reset", "Redefinir")}
                    </button>
                  {:else}
                    <button type="button" onclick={() => void resetSettings()}>
                      {tr("Reset", "Redefinir")}
                    </button>
                  {/if}
                </div>
              </div>
            </details>
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
          aria-label={tr("Terminals", "Terminais")}
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <circle cx="5" cy="6" r="2" /><circle cx="15" cy="6" r="2" /><circle cx="10" cy="15" r="2" />
            <path d="m6.7 7 2.2 6M13.3 7l-2.2 6M7 6h6" />
          </svg>
          <span>{tr("Terminals", "Terminais")}</span>
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
          class:has-mobile-device={newMobileDevice !== null}
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
    position: relative;
    width: 100%;
    height: 100%;
    display: flex;
    align-items: flex-start;
    justify-content: flex-start;
  }

  .overlay-shell.morphing {
    clip-path: inset(
      0 max(0px, calc(100% - var(--morph-width)))
      max(0px, calc(100% - var(--morph-height))) 0
      round var(--panel-radius)
    );
  }

  .overlay-shell:not(.expanded) {
    clip-path: inset(0 calc(100% - 78px) calc(100% - 44px) 0 round 23px);
  }

  button,
  input,
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
    background: #f9fbfa;
  }

  .panel.palette-open {
    min-height: 390px;
  }

  .panel.launcher-open,
  .panel.workflow-settings-open {
    overflow: visible;
  }

  .panel.morphing:not(.measuring) {
    width: var(--morph-width);
    height: var(--morph-height);
    flex: 0 0 auto;
    min-height: 0;
    max-height: none;
  }

  .panel.measuring {
    position: absolute;
    width: 392px;
    height: auto;
    max-height: 544px;
    visibility: hidden;
  }

  .panel-content,
  .panel footer,
  .panel .brand-lockup > div,
  .panel .header-actions,
  .panel .launcher-popover,
  .panel .mobile-device-banner {
    transition: opacity 150ms ease;
  }
  .panel:not(.content-visible) .panel-content,
  .panel:not(.content-visible) footer,
  .panel:not(.content-visible) .brand-lockup > div,
  .panel:not(.content-visible) .header-actions,
  .panel:not(.content-visible) .launcher-popover,
  .panel:not(.content-visible) .mobile-device-banner {
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

  .mobile-device-banner {
    min-height: 52px;
    padding: 8px 12px;
    display: grid;
    grid-template-columns: 30px minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
    border-bottom: 1px solid rgba(84, 143, 112, 0.16);
    background: rgba(91, 164, 126, 0.075);
  }
  .mobile-device-banner-icon {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: 10px;
    color: #4c8c6c;
    background: rgba(88, 167, 125, 0.12);
  }
  .mobile-device-banner-icon svg { width: 16px; height: 16px; }
  .mobile-device-banner > span:nth-child(2) { min-width: 0; display: grid; gap: 2px; }
  .mobile-device-banner strong { overflow: hidden; color: #315e49; font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .mobile-device-banner small { overflow: hidden; color: #6d887b; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .mobile-device-banner > button {
    min-height: 27px;
    padding: 0 8px;
    border: 1px solid rgba(75, 137, 105, 0.2);
    border-radius: 8px;
    color: #47745f;
    background: rgba(255, 255, 255, 0.52);
    font-size: 8px;
    font-weight: 750;
    cursor: pointer;
  }
  .mobile-device-banner > button:hover { background: rgba(255, 255, 255, 0.82); }

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
  .launcher-popover { position: absolute; z-index: 4; top: 53px; right: 13px; width: 320px; max-height: calc(100vh - 65px); overflow: hidden; isolation: isolate; border: 1px solid rgba(99, 119, 110, 0.14); border-radius: 14px; background: #fafcfb; background-clip: padding-box; }
  .launcher-popover-scroll { box-sizing: border-box; width: 100%; max-height: calc(100vh - 65px); padding: 10px 11px; overflow-x: hidden; overflow-y: auto; overscroll-behavior: contain; scrollbar-gutter: stable; }
  .launcher-title { display: block; padding: 1px 3px 7px; color: #8c9691; font-size: 9px; font-weight: 750; letter-spacing: 0.06em; text-transform: uppercase; }
  .launcher-row { min-height: 45px; display: flex; align-items: center; gap: 7px; border-top: 1px solid rgba(105, 123, 115, 0.08); }
  .launcher-row .agent-avatar { width: 25px; height: 25px; border-radius: 8px; font-size: 9px; }
  .launcher-row strong { min-width: 0; flex: 1; color: #35423d; font-size: 10px; }
  .launcher-row button { height: 25px; padding: 0 7px; border: 0; border-radius: 7px; color: #60736a; background: rgba(78, 105, 93, 0.055); font-size: 9px; font-weight: 700; cursor: pointer; }
  .launcher-row button:hover { background: rgba(78, 105, 93, 0.1); }
  .launcher-row button.active { color: #327a58; background: rgba(57, 139, 96, 0.1); }
  .launcher-row button:disabled { opacity: 0.45; }
  .resume-session-list { padding: 2px 0 7px 32px; display: grid; gap: 3px; overflow: visible; }
  .resume-session-list > p { margin: 8px 2px; color: #89938f; font-size: 9px; }
  .resume-session { width: 100%; min-height: 39px; padding: 5px 7px 5px 8px; display: flex; align-items: center; gap: 8px; overflow: hidden; border: 0; border-radius: 9px; color: #51665c; background: rgba(76, 104, 91, 0.045); text-align: left; cursor: pointer; }
  .resume-session:hover { background: rgba(61, 132, 96, 0.09); }
  .resume-session:disabled { opacity: 0.45; cursor: default; }
  .resume-session > span { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .resume-session strong, .resume-session small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .resume-session strong { color: #3e5048; font-size: 9px; }
  .resume-session small { color: #89958f; font-size: 7px; }
  .resume-session svg { width: 12px; height: 12px; flex: 0 0 auto; }
  .launcher-popover-scroll > p { margin: 8px 3px; color: #89938f; font-size: 10px; }
  .launcher-popover .launcher-error { color: #a54c4c; }
  .command-palette-layer { position: absolute; z-index: 12; inset: 0 0 16px; display: grid; place-items: start center; padding-top: 66px; }
  .command-palette-backdrop { position: absolute; inset: 0; width: 100%; border: 0; background: rgba(21, 31, 27, 0.2); backdrop-filter: blur(3px); cursor: default; }
  .command-palette { position: relative; width: calc(100% - 30px); overflow: hidden; border: 1px solid rgba(89, 111, 101, 0.16); border-radius: 15px; background: rgba(250, 252, 251, 0.98); box-shadow: 0 18px 45px rgba(24, 38, 32, 0.24); }
  .command-search { height: 43px; padding: 0 10px; display: flex; align-items: center; gap: 8px; border-bottom: 1px solid rgba(91, 112, 102, 0.1); }
  .command-search svg { width: 15px; height: 15px; flex: 0 0 auto; fill: none; stroke: #6f8179; stroke-width: 1.5; }
  .command-search input { min-width: 0; flex: 1; border: 0; outline: 0; color: #304039; background: transparent; font: inherit; font-size: 10px; }
  .command-palette kbd { padding: 3px 5px; border: 1px solid rgba(92, 112, 103, 0.13); border-radius: 5px; color: #7b8983; background: rgba(80, 105, 94, 0.045); font-family: inherit; font-size: 7px; }
  .command-results { max-height: 265px; padding: 5px; overflow-y: auto; }
  .command-results > button { width: 100%; min-height: 42px; padding: 6px 8px; display: flex; align-items: center; gap: 8px; border: 0; border-radius: 9px; color: inherit; background: transparent; text-align: left; cursor: pointer; }
  .command-results > button.active { background: rgba(78, 109, 95, 0.075); }
  .command-results > button span { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .command-results strong { overflow: hidden; color: #34443d; font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .command-results small, .command-results > p { margin: 0; color: #89958f; font-size: 8px; }
  .shortcut-editor-layer { position: absolute; z-index: 18; inset: 0; display: grid; place-items: center; padding: 18px; }
  .shortcut-editor-backdrop { position: absolute; inset: 0; width: 100%; border: 0; background: rgba(21, 31, 27, 0.24); backdrop-filter: blur(3px); cursor: default; }
  .shortcut-editor { position: relative; width: min(270px, 100%); padding: 16px; display: grid; gap: 8px; border: 1px solid rgba(89, 111, 101, 0.18); border-radius: 14px; background: rgba(250, 252, 251, 0.99); box-shadow: 0 18px 45px rgba(24, 38, 32, 0.26); }
  .shortcut-editor > strong { color: #34443d; font-size: 11px; }
  .shortcut-editor > small { color: #829089; font-size: 8px; line-height: 1.45; }
  .shortcut-editor > p { margin: 0; color: #a34d4d; font-size: 8px; line-height: 1.4; }
  .shortcut-capture { height: 44px; margin-top: 3px; border: 1px solid rgba(70, 113, 95, 0.28); border-radius: 10px; outline: 0; color: #3e6153; background: rgba(74, 122, 102, 0.07); cursor: text; }
  .shortcut-capture:focus { border-color: rgba(69, 130, 103, 0.58); box-shadow: 0 0 0 3px rgba(74, 122, 102, 0.1); }
  .shortcut-capture kbd { font: 750 10px Inter, sans-serif; }
  .shortcut-editor-actions { margin-top: 4px; display: flex; justify-content: flex-end; gap: 6px; }
  .shortcut-editor-actions button { min-width: 60px; height: 28px; padding: 0 9px; border: 1px solid rgba(82, 105, 95, 0.15); border-radius: 8px; color: #66776e; background: transparent; font-size: 8px; font-weight: 750; cursor: pointer; }
  .shortcut-editor-actions button.primary { color: #fff; border-color: #317e59; background: #317e59; }
  .shortcut-editor-actions button:disabled { opacity: 0.45; cursor: default; }
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
  .agent-codex,
  .agent-chatgpt { color: #202523; background: #edf0ee; }
  .agent-claude,
  .agent-claude_code { color: #d97757; background: #f7ece6; }
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
  .running-dots i { will-change: transform, opacity; }
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
  .session-action-bar { position: relative; margin: 0 0 10px; display: flex; align-items: center; gap: 5px; }
  .session-action-button { position: relative; width: 27px; height: 27px; padding: 0; display: grid; place-items: center; border: 1px solid rgba(83, 108, 97, 0.11); border-radius: 8px; color: #65786f; background: rgba(77, 105, 92, 0.035); cursor: pointer; transition: color 130ms ease, background 130ms ease, transform 130ms ease; }
  .session-action-button:hover:not(:disabled),
  .session-action-button.active { color: #3f745d; background: rgba(68, 125, 99, 0.09); transform: translateY(-1px); }
  .session-action-button.warning { color: #a2762f; }
  .session-action-button.danger { color: #9a5c59; }
  .session-action-button:disabled { opacity: 0.42; cursor: default; }
  .session-action-button svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.5; }
  .session-action-button::after { position: absolute; z-index: 25; bottom: calc(100% + 5px); left: 50%; max-width: 120px; padding: 4px 6px; content: attr(data-label); opacity: 0; pointer-events: none; border: 1px solid rgba(74, 96, 86, 0.12); border-radius: 6px; color: #52635b; background: rgba(249, 251, 250, 0.98); box-shadow: 0 5px 15px rgba(43, 58, 51, 0.12); font-size: 7px; font-weight: 700; line-height: 1.2; text-align: center; white-space: nowrap; transform: translate(-50%, 3px); transition: opacity 110ms ease, transform 110ms ease; }
  .session-action-button:hover::after,
  .session-action-button:focus-visible::after { opacity: 1; transform: translate(-50%, 0); }
  .session-name-editor { margin: 0 0 9px; display: flex; flex-wrap: wrap; gap: 5px; }
  .session-name-editor input { min-width: 0; flex: 1 1 130px; padding: 6px 8px; border: 1px solid rgba(75, 101, 89, 0.14); border-radius: 7px; color: #314139; background: rgba(255, 255, 255, 0.5); font: inherit; font-size: 10px; outline: none; }
  .session-name-editor input:focus { border-color: rgba(64, 132, 99, 0.42); box-shadow: 0 0 0 2px rgba(64, 132, 99, 0.08); }
  .session-name-editor button { padding: 5px 7px; border: 1px solid rgba(75, 101, 89, 0.12); border-radius: 7px; color: #66746d; background: rgba(75, 101, 89, 0.06); font-size: 9px; }
  .session-name-editor button.primary { color: #fff; background: #3e8e68; }
  .session-name-editor small { flex-basis: 100%; color: #ad5555; font-size: 8px; }
  .permission-block { padding-left: 11px; border-left: 2px solid #d49350; display: grid; gap: 6px; }
  .permission-block > strong { color: #4d3b2a; font-size: 11px; font-weight: 650; line-height: 1.4; }
  .question-block { padding: 9px; display: grid; gap: 8px; border: 1px solid rgba(48, 133, 176, 0.2); border-radius: 9px; background: rgba(48, 133, 176, 0.055); }
  .question-block section { min-width: 0; display: grid; gap: 5px; }
  .question-block section > strong { color: #344b52; font-size: 11px; line-height: 1.4; overflow-wrap: anywhere; }
  .question-block section > small { color: #6c8077; font-size: 9px; }
  .question-actions { display: flex; flex-wrap: wrap; gap: 5px; }
  .question-actions button { min-height: 26px; padding: 0 8px; border: 1px solid rgba(65, 112, 133, 0.16); border-radius: 7px; color: #425a61; background: rgba(255, 255, 255, 0.52); font-size: 9px; font-weight: 700; cursor: pointer; }
  .question-actions button.selected { border-color: rgba(44, 137, 178, 0.42); background: rgba(48, 145, 187, 0.12); }
  .question-actions button b { color: #2f83aa; }
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

  .final-response { position: relative; margin: 0 0 10px; padding: 9px 36px 9px 10px; border: 1px solid rgba(78, 105, 93, 0.1); border-radius: 10px; background: rgba(73, 102, 89, 0.035); }
  .final-response .eyebrow { display: block; margin-bottom: 5px; color: #668075; font-size: 8px; font-weight: 780; letter-spacing: 0.055em; text-transform: uppercase; }
  .final-response p { max-height: 150px; margin: 0; overflow-y: auto; color: #43524c; font-size: 10px; line-height: 1.5; overflow-wrap: anywhere; white-space: pre-wrap; scrollbar-width: thin; }
  .final-response-copy { position: absolute; top: 6px; right: 6px; width: 24px; height: 24px; padding: 0; display: grid; place-items: center; border: 0; border-radius: 7px; color: #6a7f75; background: transparent; cursor: pointer; }
  .final-response-copy:hover { color: #3f6253; background: rgba(72, 99, 87, 0.08); }
  .final-response-copy svg { width: 13px; height: 13px; }

  .inline-composer { margin-top: 8px; display: flex; flex-direction: column; gap: 6px; }
  .inline-queue-tray { min-width: 0; width: 100%; min-height: 36px; padding: 5px 7px; display: flex; align-items: center; gap: 7px; border: 1px solid rgba(80, 119, 160, 0.13); border-radius: 9px; color: #4f6d83; background: rgba(74, 119, 157, 0.055); text-align: left; cursor: pointer; }
  .inline-queue-tray:hover:not(:disabled) { border-color: rgba(67, 119, 164, 0.24); background: rgba(74, 119, 157, 0.09); }
  .inline-queue-tray:disabled { opacity: 0.58; cursor: default; }
  .inline-queue-tray .queue-mark { width: 18px; height: 18px; display: grid; flex: 0 0 auto; place-items: center; border-radius: 5px; color: #477fa9; background: rgba(66, 127, 174, 0.1); font: 800 11px Inter, sans-serif; }
  .inline-queue-tray .queue-copy { min-width: 0; flex: 1; display: grid; gap: 1px; }
  .inline-queue-tray .queue-copy small { color: #7790a1; font-size: 7px; font-weight: 760; letter-spacing: 0.035em; text-transform: uppercase; }
  .inline-queue-tray .queue-copy strong { overflow: hidden; color: #4c6576; font-size: 9px; font-weight: 620; text-overflow: ellipsis; white-space: nowrap; }
  .inline-queue-tray .queue-shortcut { display: flex; flex: 0 0 auto; align-items: center; gap: 4px; color: #748b9a; }
  .inline-queue-tray .queue-shortcut kbd { min-width: 24px; padding: 2px 4px; border: 1px solid rgba(75, 106, 127, 0.17); border-bottom-width: 2px; border-radius: 5px; color: #547286; background: rgba(255, 255, 255, 0.48); font: 750 7px Inter, sans-serif; text-align: center; }
  .inline-queue-tray .queue-shortcut small { font-size: 7px; font-weight: 650; white-space: nowrap; }
  .inline-composer-controls { display: flex; align-items: flex-end; gap: 6px; }
  .inline-composer textarea { resize: none; outline: none; font: inherit; }
  .inline-composer textarea { min-width: 0; min-height: 52px; flex: 1; padding: 8px 9px; border: 1px solid rgba(85, 109, 99, 0.14); border-radius: 10px; color: #34423c; background: rgba(255, 255, 255, 0.48); font-size: 10px; line-height: 1.4; }
  .inline-composer textarea:focus { border-color: rgba(70, 111, 94, 0.42); box-shadow: 0 0 0 3px rgba(74, 118, 99, 0.06); }
  .inline-composer-controls > button { width: 30px; height: 30px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 9px; color: white; background: #496f60; cursor: pointer; transition: transform 140ms ease, opacity 140ms ease; }
  .inline-composer-controls > button:hover:not(:disabled) { transform: translateY(-1px); }
  .inline-composer-controls > button:disabled { opacity: 0.35; cursor: default; }
  .inline-attachments { min-width: 0; display: flex; gap: 6px; overflow-x: auto; }
  .inline-attachments > span { position: relative; width: 44px; height: 44px; flex: 0 0 auto; overflow: hidden; border: 1px solid rgba(85, 109, 99, 0.14); border-radius: 9px; background: rgba(71, 98, 86, 0.05); }
  .inline-attachments img { width: 100%; height: 100%; display: block; object-fit: cover; }
  .inline-attachments button { position: absolute; top: 2px; right: 2px; width: 16px; height: 16px; padding: 0; display: grid; place-items: center; border: 1px solid rgba(255, 255, 255, 0.5); border-radius: 50%; color: white; background: rgba(27, 39, 34, 0.78); font-size: 11px; line-height: 1; cursor: pointer; }
  .terminate-agent-control { margin: 0 0 9px; display: flex; align-items: center; gap: 6px; }
  .terminate-agent-control.confirming { padding: 7px 8px; border: 1px solid rgba(166, 77, 77, 0.13); border-radius: 9px; background: rgba(166, 77, 77, 0.035); }
  .terminate-agent-control.confirming span { min-width: 0; flex: 1; color: #755b57; font-size: 9px; line-height: 1.35; }
  .terminate-agent-control.confirming button { min-height: 24px; padding: 0 7px; border: 1px solid rgba(91, 107, 100, 0.13); border-radius: 7px; color: #627068; background: rgba(255, 255, 255, 0.42); font-size: 8px; font-weight: 700; cursor: pointer; }
  .terminate-agent-control.confirming button.danger { border-color: rgba(166, 77, 77, 0.2); color: #a54c4c; }
  .terminate-agent-control button:disabled { opacity: 0.45; cursor: default; }
  .reset-settings-control { display: flex; align-items: center; gap: 6px; }
  .reset-settings-control > button { min-height: 27px; padding: 0 9px; border: 1px solid rgba(165, 76, 76, 0.45); border-radius: 8px; color: #a54c4c; background: transparent; font-size: 10px; font-weight: 750; cursor: pointer; transition: transform 130ms ease, background 130ms ease; }
  .reset-settings-control > button:hover:not(:disabled) { transform: translateY(-1px); background: rgba(165, 76, 76, 0.1); }
  .reset-settings-control.confirming { padding: 7px 8px; border: 1px solid rgba(165, 76, 76, 0.22); border-radius: 9px; background: rgba(165, 76, 76, 0.05); }
  .reset-settings-control.confirming span { min-width: 0; flex: 1; color: #8a4340; font-size: 9px; font-weight: 650; line-height: 1.35; }
  .reset-settings-control.confirming button { min-height: 24px; padding: 0 7px; border: 1px solid rgba(77, 91, 85, 0.3); border-radius: 7px; color: #4d5b55; background: transparent; font-size: 8px; font-weight: 750; cursor: pointer; transition: background 130ms ease; }
  .reset-settings-control.confirming button:hover:not(:disabled) { background: rgba(77, 91, 85, 0.1); }
  .reset-settings-control.confirming button.danger { border-color: rgba(165, 76, 76, 0.5); color: #a54c4c; }
  .reset-settings-control.confirming button.danger:hover:not(:disabled) { background: rgba(165, 76, 76, 0.12); }
  .reset-settings-control button:disabled { opacity: 0.45; cursor: default; }

  .whiteboard { position: relative; max-height: 431px; min-height: 0; padding: 7px 16px 15px; display: flex; flex-direction: column; overflow: hidden; }
  .layout-toolbar { padding: 8px 0 4px; display: flex; align-items: center; gap: 4px; }
  .layout-toolbar input { min-width: 0; height: 27px; padding: 0 6px; flex: 1 1 auto; border: 1px solid rgba(87, 109, 99, 0.13); border-radius: 7px; outline: 0; color: #52625b; background: rgba(255, 255, 255, 0.42); font: inherit; font-size: 8px; }
  .layout-toolbar :global(.lume-select) { flex: 1 1 auto; }
  .layout-toolbar button { width: 27px; height: 27px; padding: 5px; flex: 0 0 27px; display: grid; place-items: center; border: 1px solid rgba(87, 109, 99, 0.13); border-radius: 7px; color: #567165; background: rgba(255, 255, 255, 0.4); font-size: 8px; cursor: pointer; }
  .layout-toolbar button svg { width: 15px; height: 15px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .layout-toolbar button:disabled { opacity: 0.48; cursor: default; }
  .layout-spinner { width: 12px; height: 12px; border: 1.5px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: layout-spin 650ms linear infinite; }
  @keyframes layout-spin { to { transform: rotate(360deg); } }
  .layout-toolbar .layout-delete { color: #a45a58; }
  .workflow-group-row { min-width: 0; padding: 7px 8px; display: flex; align-items: center; gap: 7px; border: 1px solid rgba(82, 105, 95, 0.11); border-radius: 10px; background: rgba(255, 255, 255, 0.25); transition: border-color 160ms ease, background 160ms ease; }
  .workflow-global-mode { position: relative; margin: 5px 0 1px; }
  .workflow-group-row.enabled { border-color: rgba(55, 151, 103, 0.22); background: rgba(62, 153, 106, 0.055); }
  .workflow-group-symbol { width: 27px; height: 22px; flex: 0 0 27px; display: grid; place-items: center; overflow: visible; }
  .workflow-group-symbol svg { width: 24px; height: 20px; overflow: visible; }
  .workflow-group-symbol path { fill: none; stroke-linejoin: round; }
  .workflow-group-symbol .normal-link { stroke: #8b9d94; opacity: 1; transition: opacity 150ms ease; }
  .workflow-group-symbol .workflow-link { stroke: #4aaa79; stroke-dasharray: 48; stroke-dashoffset: 48; opacity: 0; transition: opacity 130ms ease, stroke-dashoffset 420ms cubic-bezier(.2,.8,.2,1); }
  .workflow-group-symbol circle { fill: #edf2ef; stroke: #7e968a; transform-box: fill-box; transform-origin: center; transition: transform 380ms cubic-bezier(.2,.85,.2,1), fill 180ms ease, stroke 180ms ease, filter 180ms ease; }
  .workflow-group-symbol .node-start,
  .workflow-group-symbol .node-end { transform: translateY(-7px); }
  .workflow-group-symbol .node-center { transform: translateY(7.5px); }
  .workflow-group-row.enabled .workflow-group-symbol .normal-link { opacity: 0; }
  .workflow-group-row.enabled .workflow-group-symbol .workflow-link { stroke-dashoffset: 0; opacity: 1; }
  .workflow-group-row.enabled .workflow-group-symbol circle { fill: #dff4e9; stroke: #43a572; filter: drop-shadow(0 0 3px rgba(64, 167, 111, 0.42)); transform: translateY(0); }
  .workflow-group-copy { min-width: 0; flex: 1; }
  .workflow-group-copy strong { color: #43574d; font-size: 9.5px; }
  .workflow-mode-switch { position: relative; width: 101px; height: 28px; padding: 2px; flex: 0 0 101px; display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); border-radius: 8px; background: rgba(83, 108, 96, 0.09); isolation: isolate; }
  .workflow-mode-switch::before { position: absolute; z-index: 0; top: 2px; bottom: 2px; left: 2px; width: calc((100% - 4px) / 2); border-radius: 5px; content: ""; background: rgba(255, 255, 255, 0.82); box-shadow: 0 1px 4px rgba(44, 72, 58, 0.13); transform: translateX(0); transition: transform 240ms cubic-bezier(.2,.85,.2,1), background 180ms ease, box-shadow 180ms ease; }
  .workflow-mode-switch.workflow-active::before { transform: translateX(100%); }
  .workflow-mode-switch button { position: relative; z-index: 1; min-width: 0; height: 24px; padding: 0 4px; border: 0; border-radius: 6px; color: #87948e; background: transparent; font: 720 8px Inter, sans-serif; cursor: pointer; transition: color 160ms ease, transform 160ms cubic-bezier(.2,.8,.2,1); }
  .workflow-mode-switch button:active:not(:disabled) { transform: scale(.94); }
  .workflow-mode-switch button.active { color: #397258; }
  .workflow-mode-switch button:disabled { opacity: 0.5; cursor: default; }
  .workflow-settings-trigger { width: 28px; height: 28px; padding: 5px; flex: 0 0 28px; display: grid; place-items: center; border: 0; border-radius: 7px; color: #708078; background: transparent; cursor: pointer; }
  .workflow-settings-trigger:hover,
  .workflow-settings-trigger.active { color: #377e59; background: rgba(55, 142, 98, 0.09); }
  .workflow-settings-trigger svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; }
  .workflow-settings-dismiss { position: fixed; z-index: 238; inset: 0; width: 100%; height: 100%; padding: 0; border: 0; background: transparent; cursor: default; }
  .workflow-settings-popover { position: absolute; z-index: 240; top: 84px; right: 16px; width: min(310px, calc(100% - 32px)); max-height: calc(100vh - 96px); overflow: hidden; isolation: isolate; border: 1px solid rgba(67, 105, 86, 0.18); border-radius: 12px; color: #485b51; background: #f7faf8; background-clip: padding-box; }
  .workflow-settings-scroll { box-sizing: border-box; width: 100%; max-height: calc(100vh - 96px); padding: 11px; display: grid; gap: 9px; overflow-x: hidden; overflow-y: auto; overscroll-behavior: contain; scrollbar-width: thin; }
  .workflow-settings-scroll > header { display: flex; align-items: flex-start; gap: 8px; }
  .workflow-settings-scroll > header div { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .workflow-settings-scroll > header strong { color: #33483d; font-size: 10px; }
  .workflow-settings-scroll > header small { color: #829087; font-size: 7.5px; }
  .workflow-settings-scroll > header button { width: 22px; height: 22px; padding: 0; border: 0; border-radius: 6px; color: #73827a; background: transparent; font-size: 16px; cursor: pointer; }
  .workflow-setting-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; }
  .workflow-setting-grid label { min-width: 0; display: grid; gap: 4px; color: #718078; font-size: 7.5px; font-weight: 700; }
  .workflow-setting-grid input { width: 100%; min-width: 0; height: 29px; padding: 0 7px; border: 1px solid rgba(75, 105, 90, 0.15); border-radius: 7px; outline: 0; color: #3f574a; background: rgba(73, 105, 88, 0.045); font: 750 9px Inter, sans-serif; }
  .workflow-setting-grid input:focus { border-color: rgba(50, 143, 96, 0.45); box-shadow: 0 0 0 2px rgba(50, 143, 96, 0.07); }
  .workflow-setting-toggle { min-height: 29px; display: flex; align-items: center; gap: 8px; color: #596d62; font-size: 8.5px; font-weight: 700; cursor: pointer; }
  .workflow-setting-toggle span { min-width: 0; flex: 1; }
  .workflow-setting-toggle input { position: absolute; opacity: 0; pointer-events: none; }
  .workflow-setting-toggle i { width: 27px; height: 15px; padding: 2px; flex: 0 0 auto; border-radius: 9px; background: #c4cec9; transition: background 150ms ease; }
  .workflow-setting-toggle i::before { width: 11px; height: 11px; display: block; border-radius: 50%; content: ""; background: white; box-shadow: 0 1px 3px rgba(34, 55, 44, 0.2); transition: transform 170ms cubic-bezier(.2,.8,.2,1); }
  .workflow-setting-toggle input:checked + i { background: #3e9568; }
  .workflow-setting-toggle input:checked + i::before { transform: translateX(12px); }
  .workflow-reserve-setting { display: grid; grid-template-columns: minmax(0, 1fr) 100px 30px; align-items: center; gap: 7px; color: #65766d; font-size: 8px; font-weight: 700; }
  .workflow-reserve-setting input { width: 100%; accent-color: #3c9266; }
  .workflow-reserve-setting strong { color: #397b59; font-size: 8px; text-align: right; }
  .workflow-missing-sessions { padding-top: 8px; display: grid; gap: 6px; border-top: 1px solid rgba(75, 105, 90, 0.12); }
  .workflow-missing-sessions > strong { color: #9a6c3b; font-size: 8px; text-transform: uppercase; letter-spacing: .05em; }
  .workflow-missing-sessions label { display: grid; grid-template-columns: 74px minmax(0, 1fr); align-items: center; gap: 7px; }
  .workflow-missing-sessions label > span { overflow: hidden; color: #64766c; font-size: 8px; font-weight: 700; text-overflow: ellipsis; text-transform: capitalize; white-space: nowrap; }
  .workflow-missing-sessions :global(.lume-select) { width: 100%; }
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
  .result-card-top { min-width: 0; display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
  .result-heading { min-width: 0; display: flex; align-items: center; gap: 7px; }
  .result-heading .agent-avatar { width: 25px; height: 25px; border-radius: 8px; }
  .result-heading > span:last-child { min-width: 0; display: grid; gap: 1px; }
  .result-heading strong { color: #34443d; font-size: 9px; }
  .result-heading small { overflow: hidden; color: #87928d; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .result-card > p { max-height: 78px; margin: 8px 0; overflow: hidden; display: -webkit-box; color: #52615b; font-size: 9px; line-height: 1.45; line-clamp: 4; overflow-wrap: anywhere; white-space: pre-wrap; -webkit-box-orient: vertical; -webkit-line-clamp: 4; }
  .result-artifacts { margin: 0 0 8px; display: grid; gap: 4px; }
  .result-artifacts span { overflow: hidden; color: #78867f; font-size: 8px; line-height: 1.35; text-overflow: ellipsis; white-space: nowrap; }
  .result-artifacts strong { margin-right: 5px; color: #60766c; font-size: 7px; text-transform: uppercase; }
  .result-actions { display: flex; flex: 0 0 auto; align-items: center; gap: 4px; }
  .result-action-button { position: relative; width: 25px; height: 25px; padding: 0; display: grid; place-items: center; border: 1px solid rgba(84, 109, 98, 0.12); border-radius: 7px; color: #5e756b; background: rgba(255, 255, 255, 0.36); cursor: pointer; transition: color 130ms ease, background 130ms ease, transform 130ms ease; }
  .result-action-button:hover:not(:disabled) { color: #3f745d; background: rgba(255, 255, 255, 0.72); transform: translateY(-1px); }
  .result-action-button:disabled { opacity: 0.42; cursor: default; }
  .result-action-button svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.5; }
  .result-action-button::after { position: absolute; z-index: 25; top: calc(100% + 5px); left: 50%; max-width: 120px; padding: 4px 6px; content: attr(data-label); opacity: 0; pointer-events: none; border: 1px solid rgba(74, 96, 86, 0.12); border-radius: 6px; color: #52635b; background: rgba(249, 251, 250, 0.98); box-shadow: 0 5px 15px rgba(43, 58, 51, 0.12); font-size: 7px; font-weight: 700; line-height: 1.2; text-align: center; white-space: nowrap; transform: translate(-50%, -3px); transition: opacity 110ms ease, transform 110ms ease; }
  .result-action-button:hover::after,
  .result-action-button:focus-visible::after { opacity: 1; transform: translate(-50%, 0); }
  .result-action-button:last-child::after { right: 0; left: auto; transform: translateY(-3px); }
  .result-action-button:last-child:hover::after,
  .result-action-button:last-child:focus-visible::after { transform: translateY(0); }
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
  .settings-section { border-bottom: 1px solid rgba(105, 123, 115, 0.1); }
  .settings-section > .settings-section-label {
    min-height: 39px;
    padding: 0 2px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    list-style: none;
    cursor: pointer;
    user-select: none;
  }
  .settings-section > .settings-section-label::-webkit-details-marker { display: none; }
  .settings-section > .settings-section-label::after {
    content: "+";
    color: #7d8d85;
    font-size: 15px;
    font-weight: 450;
    letter-spacing: 0;
    transition: color 130ms ease, transform 130ms ease;
  }
  .settings-section[open] > .settings-section-label::after {
    content: "−";
    color: #4f7463;
    transform: rotate(180deg);
  }
  .settings-section > .settings-section-label:hover { color: #66766f; }
  .settings-section-static > .settings-section-label { cursor: default; }
  .settings-section-static > .settings-section-label::after { content: none; }
  .settings-section-static > .settings-section-label:hover { color: #929c97; }
  .settings-section-content { padding: 0 1px 6px; }
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
  .sound-volume-row.disabled { opacity: 0.52; }
  .volume-control { width: 116px; display: flex; align-items: center; gap: 7px; }
  .volume-control input { width: 82px; height: 16px; accent-color: #527c6c; cursor: pointer; }
  .volume-control input:disabled { cursor: default; }
  .volume-control output { width: 27px; color: #718078; font-size: 8px; font-variant-numeric: tabular-nums; text-align: right; }

  .field-row :global(.lume-select) { max-width: 145px; }
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
  .shortcut-input {
    width: 112px;
    padding: 6px 7px;
    border: 1px solid rgba(92, 111, 103, 0.16);
    border-radius: 8px;
    outline: 0;
    color: #607068;
    background: rgba(80, 105, 94, 0.045);
    font-family: inherit;
    font-size: 8px;
    text-align: center;
    cursor: pointer;
  }
  .shortcut-input:focus {
    color: #3e6153;
    border-color: rgba(69, 113, 94, 0.42);
    box-shadow: 0 0 0 2px rgba(74, 122, 102, 0.08);
  }
  .profile-empty { margin: 5px 1px 2px; color: #89938f; font-size: 9px; line-height: 1.45; }
  .mobile-access-card { padding: 11px; display: grid; gap: 9px; border: 1px solid rgba(92, 111, 103, 0.11); border-radius: 13px; background: rgba(84, 111, 99, 0.035); }
  .mobile-access-header,
  .mobile-address,
  .mobile-apk,
  .mobile-pair-action,
  .mobile-pairing { display: flex; align-items: center; gap: 9px; }
  .mobile-access-header > div,
  .mobile-pair-action > span,
  .mobile-apk > span { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .mobile-access-card strong { color: #35423d; font-size: 9px; }
  .mobile-access-card span,
  .mobile-access-card small { color: #89938f; font-size: 8px; line-height: 1.4; }
  .mobile-address,
  .mobile-apk,
  .mobile-pair-action { padding-top: 8px; border-top: 1px solid rgba(92, 111, 103, 0.09); }
  .mobile-address > span { min-width: 0; flex: 1; display: flex; align-items: center; gap: 6px; }
  .mobile-address code { overflow: hidden; color: #53665d; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .mobile-apk code { overflow: hidden; color: #53665d; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .mobile-access-card button,
  .paired-devices button { min-height: 25px; padding: 0 7px; border: 1px solid rgba(82, 105, 95, 0.14); border-radius: 7px; color: #577064; background: transparent; font-size: 8px; font-weight: 680; cursor: pointer; }
  .mobile-access-card button:disabled,
  .paired-devices button:disabled { cursor: default; opacity: 0.5; }
  .mobile-pairing { flex-direction: column; align-items: center; padding: 10px; border-radius: 9px; background: rgba(255, 255, 255, 0.5); }
  .mobile-pairing img { width: 208px; max-width: 100%; height: auto; border-radius: 5px; image-rendering: pixelated; }
  .mobile-pairing > span { width: 100%; min-width: 0; display: grid; gap: 5px; text-align: center; }
  .mobile-pairing code { color: #31483e; font-size: 9px; overflow-wrap: anywhere; }
  .mobile-message { margin: 0; color: #61756b; font-size: 8px; line-height: 1.4; }
  .mobile-message.error { color: #a34f4f; }
  .paired-devices { margin-top: 9px; display: grid; gap: 8px; }
  .paired-devices-intro { padding: 1px 2px 3px; }
  .paired-devices-intro strong { color: #35423d; font-size: 9px; }
  .paired-devices-intro p { margin: 3px 0 0; color: #7d8b84; font-size: 8px; line-height: 1.45; }
  .paired-device-card { padding: 10px; border: 1px solid rgba(92, 111, 103, 0.12); border-radius: 12px; background: rgba(84, 111, 99, 0.03); }
  .paired-device-header { display: flex; align-items: center; gap: 9px; }
  .paired-device-info { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .paired-device-info strong { color: #35423d; font-size: 10px; }
  .paired-device-info small { color: #89938f; font-size: 8px; }
  .preview-badge { padding: 4px 6px; border: 1px solid rgba(82, 124, 108, 0.14); border-radius: 999px; color: #527c6c; background: rgba(82, 124, 108, 0.06); font-size: 7px; font-weight: 750; }
  .paired-devices .revoke-device { color: #8a5e5e; border-color: rgba(151, 91, 91, 0.14); }
  .device-permissions { margin-top: 8px; border-top: 1px solid rgba(92, 111, 103, 0.1); }
  .device-permission { min-height: 49px; padding: 7px 1px; display: flex; align-items: center; gap: 9px; border-bottom: 1px solid rgba(92, 111, 103, 0.08); transition: background 140ms ease; }
  .device-permission.active { background: rgba(82, 124, 108, 0.035); }
  .device-permission:last-child { border-bottom: 0; }
  .permission-copy { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .permission-copy strong { color: #42514b; font-size: 9px; }
  .permission-copy small { color: #89938f; font-size: 8px; line-height: 1.35; }
  .permission-choice { display: flex; align-items: center; gap: 7px; }
  .permission-state { max-width: 62px; color: #919b96; font-size: 7px; font-weight: 720; line-height: 1.25; text-align: right; }
  .permission-state.allowed { color: #47745f; }
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
  footer button.has-update,
  footer button.has-mobile-device { position: relative; }
  footer button.has-update::after { content: ""; position: absolute; top: 5px; right: 14px; width: 5px; height: 5px; border: 2px solid rgba(248, 250, 249, 0.95); border-radius: 50%; background: #5f8ac7; }
  footer button.has-mobile-device::after { content: ""; position: absolute; top: 5px; right: 14px; width: 5px; height: 5px; border: 2px solid rgba(248, 250, 249, 0.95); border-radius: 50%; background: #58a97d; }

  .overlay-shell:not(.dark) .lume-orb {
    border-color: rgba(73, 101, 88, 0.32);
    background: #f4f8f6;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.72), 0 5px 16px rgba(35, 62, 49, 0.1);
  }
  .overlay-shell:not(.dark) .panel {
    border-color: rgba(65, 91, 79, 0.3);
    background: #edf3f0;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.62), 0 10px 28px rgba(35, 58, 48, 0.12);
  }
  .overlay-shell:not(.dark) .panel-header,
  .overlay-shell:not(.dark) footer {
    border-color: rgba(73, 99, 87, 0.18);
    background: #f7faf8;
  }
  .overlay-shell:not(.dark) .panel-content,
  .overlay-shell:not(.dark) .session-list,
  .overlay-shell:not(.dark) .history-list,
  .overlay-shell:not(.dark) .settings,
  .overlay-shell:not(.dark) .whiteboard { background: #edf3f0; }
  .overlay-shell:not(.dark) .session-row,
  .overlay-shell:not(.dark) .history-row,
  .overlay-shell:not(.dark) .setting-row,
  .overlay-shell:not(.dark) .field-row,
  .overlay-shell:not(.dark) .terminal-picker-row,
  .overlay-shell:not(.dark) .settings-section { border-color: rgba(73, 99, 87, 0.16); }
  .overlay-shell:not(.dark) .session-row:hover,
  .overlay-shell:not(.dark) .session-row.selected { background: #e1ebe6; }
  .overlay-shell:not(.dark) .result-card,
  .overlay-shell:not(.dark) .diagnostic-card,
  .overlay-shell:not(.dark) .update-card,
  .overlay-shell:not(.dark) .mobile-access-card,
  .overlay-shell:not(.dark) .paired-device-card,
  .overlay-shell:not(.dark) .saved-note {
    border-color: rgba(70, 98, 85, 0.2);
    background: #f7faf8;
    box-shadow: 0 1px 3px rgba(42, 67, 55, 0.045);
  }
  .overlay-shell:not(.dark) .workflow-group-row {
    border-color: rgba(65, 94, 80, 0.2);
    background: #f7faf8;
  }
  .overlay-shell:not(.dark) .workflow-group-row.enabled {
    border-color: rgba(45, 139, 92, 0.34);
    background: #e7f3ed;
  }
  .overlay-shell:not(.dark) .layout-toolbar input,
  .overlay-shell:not(.dark) .layout-toolbar button,
  .overlay-shell:not(.dark) .terminal-picker-row > button,
  .overlay-shell:not(.dark) .shortcut-input,
  .overlay-shell:not(.dark) .inline-composer textarea,
  .overlay-shell:not(.dark) .permission-actions button {
    border-color: rgba(68, 94, 82, 0.22);
    background: #f8fbf9;
  }
  .overlay-shell:not(.dark) .final-response,
  .overlay-shell:not(.dark) .response-preview {
    border-color: rgba(66, 96, 82, 0.18);
    background: #f5f9f7;
  }

  @media (prefers-reduced-motion: reduce) {
    *, *::before, *::after { animation-duration: 0.01ms !important; animation-iteration-count: 1 !important; transition-duration: 0.01ms !important; }
  }

  .overlay-shell.dark { color-scheme: dark; }
  .overlay-shell.dark .lume-orb,
  .overlay-shell.dark .launcher-popover { color: #dfe8e3; border-color: rgba(190, 209, 200, 0.13); background: #1b221f; }
  .overlay-shell.dark .resume-session { color: #afc0b7; background: rgba(216, 229, 223, 0.035); }
  .overlay-shell.dark .resume-session:hover { background: rgba(101, 180, 141, 0.08); }
  .overlay-shell.dark .resume-session strong { color: #dbe7e1; }
  .overlay-shell.dark .resume-session small { color: #899a91; }
  .overlay-shell.dark .panel { color: #dfe8e3; border-color: rgba(190, 209, 200, 0.13); background: #1b221f; }
  .overlay-shell.dark .mobile-device-banner { border-color: rgba(111, 190, 151, 0.14); background: rgba(88, 167, 125, 0.07); }
  .overlay-shell.dark .mobile-device-banner-icon { color: #87c6a7; background: rgba(88, 167, 125, 0.12); }
  .overlay-shell.dark .mobile-device-banner strong { color: #b7dcc9; }
  .overlay-shell.dark .mobile-device-banner small { color: #8fa99c; }
  .overlay-shell.dark .mobile-device-banner > button { color: #a8d2bc; border-color: rgba(111, 190, 151, 0.18); background: rgba(216, 229, 223, 0.04); }
  .overlay-shell.dark .brand-lockup strong,
  .overlay-shell.dark .session-title-row strong,
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
  .overlay-shell.dark .settings-section { border-color: rgba(190, 209, 200, 0.09); }
  .overlay-shell.dark .settings-section[open] > .settings-section-label::after { color: #8eb9a5; }
  .overlay-shell.dark .session-row:hover,
  .overlay-shell.dark .session-row.selected { background: rgba(198, 218, 208, 0.045); }
  .overlay-shell.dark .session-action-button { color: #9caea5; border-color: rgba(207, 223, 215, 0.1); background: rgba(222, 233, 228, 0.035); }
  .overlay-shell.dark .session-action-button:hover:not(:disabled),
  .overlay-shell.dark .session-action-button.active { color: #9fd0b7; background: rgba(100, 180, 143, 0.09); }
  .overlay-shell.dark .session-action-button.warning { color: #d0aa67; }
  .overlay-shell.dark .session-action-button.danger { color: #d49792; }
  .overlay-shell.dark .session-action-button::after { color: #c7d5ce; border-color: rgba(205, 222, 213, 0.11); background: rgba(28, 40, 34, 0.98); box-shadow: 0 6px 18px rgba(0, 0, 0, 0.24); }
  .overlay-shell.dark .session-name-editor input,
  .overlay-shell.dark .session-name-editor button { color: #c5d0cb; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .session-name-editor button.primary { color: #f4faf7; background: #397b5c; }
  .overlay-shell.dark .project-name,
  .overlay-shell.dark .terminal-picker-copy small,
  .overlay-shell.dark .history-row span,
  .overlay-shell.dark .settings-feedback { color: #adbab4; }
  .overlay-shell.dark .settings-feedback.error { color: #d68d8d; }
  .overlay-shell.dark .update-card { border-color: rgba(190, 209, 200, 0.09); background: rgba(216, 229, 223, 0.035); }
  .overlay-shell.dark .mobile-access-card { border-color: rgba(190, 209, 200, 0.09); background: rgba(216, 229, 223, 0.035); }
  .overlay-shell.dark .mobile-access-card strong { color: #dce7e1; }
  .overlay-shell.dark .mobile-access-card span,
  .overlay-shell.dark .mobile-access-card small { color: #aebdb5; }
  .overlay-shell.dark .mobile-apk code { color: #aebdb5; }
  .overlay-shell.dark .mobile-pairing { background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .mobile-access-card button,
  .overlay-shell.dark .paired-devices button { color: #b9c8c0; border-color: rgba(207, 223, 215, 0.12); }
  .overlay-shell.dark .paired-devices-intro strong,
  .overlay-shell.dark .paired-device-info strong,
  .overlay-shell.dark .permission-copy strong { color: #dce7e1; }
  .overlay-shell.dark .paired-devices-intro p,
  .overlay-shell.dark .paired-device-info small,
  .overlay-shell.dark .permission-copy small { color: #aebdb5; }
  .overlay-shell.dark .paired-device-card { border-color: rgba(190, 209, 200, 0.09); background: rgba(216, 229, 223, 0.03); }
  .overlay-shell.dark .device-permissions,
  .overlay-shell.dark .device-permission { border-color: rgba(190, 209, 200, 0.08); }
  .overlay-shell.dark .device-permission.active { background: rgba(116, 191, 157, 0.035); }
  .overlay-shell.dark .permission-state { color: #91a098; }
  .overlay-shell.dark .permission-state.allowed { color: #91c7ae; }
  .overlay-shell.dark .preview-badge { color: #91c7ae; border-color: rgba(116, 191, 157, 0.16); background: rgba(92, 161, 130, 0.08); }
  .overlay-shell.dark .paired-devices .revoke-device { color: #d19a9a; border-color: rgba(209, 131, 131, 0.16); }
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
  .overlay-shell.dark .result-action-button { color: #b9c8c0; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .result-action-button:hover:not(:disabled) { color: #9fd0b7; background: rgba(100, 180, 143, 0.09); }
  .overlay-shell.dark .result-action-button::after { color: #c7d5ce; border-color: rgba(205, 222, 213, 0.11); background: rgba(28, 40, 34, 0.98); box-shadow: 0 6px 18px rgba(0, 0, 0, 0.24); }
  .overlay-shell.dark .empty-state strong { color: #c5d0cb; }
  .overlay-shell.dark code,
  .overlay-shell.dark .segmented { color: #bdc8c3; background: rgba(216, 229, 223, 0.06); }
  .overlay-shell.dark .permission-block > strong { color: #e2d0bd; }
  .overlay-shell.dark .question-block { border-color: rgba(83, 165, 204, 0.2); background: rgba(55, 139, 178, 0.07); }
  .overlay-shell.dark .question-block section > strong { color: #d4e2dc; }
  .overlay-shell.dark .question-block section > small { color: #8fa59b; }
  .overlay-shell.dark .question-actions button { color: #c5d7cf; border-color: rgba(178, 210, 224, 0.12); background: rgba(219, 235, 228, 0.045); }
  .overlay-shell.dark .permission-actions button,
  .overlay-shell.dark .shortcut-input,
  .overlay-shell.dark .inline-composer textarea { color: #c5d0cb; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .inline-queue-tray { color: #a7bdcd; border-color: rgba(125, 166, 199, 0.13); background: rgba(91, 143, 184, 0.065); }
  .overlay-shell.dark .inline-queue-tray:hover:not(:disabled) { border-color: rgba(128, 177, 216, 0.23); background: rgba(91, 143, 184, 0.1); }
  .overlay-shell.dark .inline-queue-tray .queue-mark { color: #87b8dc; background: rgba(105, 166, 210, 0.11); }
  .overlay-shell.dark .inline-queue-tray .queue-copy small,
  .overlay-shell.dark .inline-queue-tray .queue-shortcut { color: #829daa; }
  .overlay-shell.dark .inline-queue-tray .queue-copy strong { color: #b1c6d2; }
  .overlay-shell.dark .inline-queue-tray .queue-shortcut kbd { color: #9bb8c9; border-color: rgba(169, 197, 214, 0.14); background: rgba(220, 235, 243, 0.055); }
  .overlay-shell.dark .inline-attachments > span { border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .final-response,
  .overlay-shell.dark .response-preview { border-color: rgba(203, 221, 212, 0.08); background: rgba(210, 230, 220, 0.035); }
  .overlay-shell.dark .final-response .eyebrow,
  .overlay-shell.dark .response-preview b { color: #8ca69a; }
  .overlay-shell.dark .final-response p,
  .overlay-shell.dark .response-preview span { color: #c2d0c9; }
  .overlay-shell.dark .final-response-copy { color: #98aaa1; }
  .overlay-shell.dark .final-response-copy:hover { color: #d1ded7; background: rgba(222, 233, 228, 0.07); }
  .overlay-shell.dark .source-label { color: #9daca5; background: rgba(205, 222, 213, 0.08); }
  .overlay-shell.dark .access-badge.auto-review { border-color: rgba(123, 165, 211, 0.16); color: #9ab9d9; background: rgba(92, 137, 187, 0.12); }
  .overlay-shell.dark .access-badge.full-access { border-color: rgba(216, 157, 105, 0.17); color: #d4a77f; background: rgba(186, 122, 71, 0.12); }
  .overlay-shell.dark .terminal-picker-row,
  .overlay-shell.dark .workflow-missing-sessions { border-color: rgba(190, 209, 200, 0.09); }
  .overlay-shell.dark .terminal-picker-row > button { color: #b7c4be; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
  .overlay-shell.dark .terminal-picker-row > button:hover:not(:disabled) { background: rgba(222, 233, 228, 0.09); }
  .overlay-shell.dark .workflow-group-row { border-color: rgba(207, 223, 215, 0.09); background: rgba(222, 233, 228, 0.025); }
  .overlay-shell.dark .workflow-group-row.enabled { border-color: rgba(87, 186, 137, 0.2); background: rgba(74, 164, 116, 0.055); }
  .overlay-shell.dark .workflow-group-copy strong { color: #c0d0c8; }
  .overlay-shell.dark .workflow-mode-switch { background: rgba(220, 235, 227, 0.055); }
  .overlay-shell.dark .workflow-mode-switch::before { background: rgba(103, 183, 143, 0.12); box-shadow: none; }
  .overlay-shell.dark .workflow-mode-switch button.active { color: #91d2b1; }
  .overlay-shell.dark .workflow-settings-trigger { color: #98aaa1; }
  .overlay-shell.dark .workflow-settings-trigger:hover,
  .overlay-shell.dark .workflow-settings-trigger.active { color: #91d2b1; background: rgba(91, 177, 137, 0.08); }
  .overlay-shell.dark .workflow-settings-popover { color: #bdcbc4; border-color: rgba(202, 220, 211, 0.12); background: #18221d; box-shadow: 0 18px 44px rgba(0, 0, 0, 0.38); }
  .overlay-shell.dark .workflow-settings-scroll > header strong { color: #d9e5df; }
  .overlay-shell.dark .workflow-settings-scroll > header small,
  .overlay-shell.dark .workflow-setting-grid label,
  .overlay-shell.dark .workflow-reserve-setting { color: #91a39a; }
  .overlay-shell.dark .workflow-setting-grid input { color: #cfddd5; border-color: rgba(205, 222, 213, 0.12); background: rgba(220, 235, 227, 0.045); }
  .overlay-shell.dark .workflow-setting-toggle { color: #b4c4bc; }
  .overlay-shell.dark .workflow-setting-toggle i { background: #46534d; }
  .overlay-shell.dark .workflow-missing-sessions { border-color: rgba(205, 222, 213, 0.1); }
  .overlay-shell.dark .workflow-missing-sessions label > span { color: #9fb0a7; }
  .overlay-shell.dark .permission-actions button:hover { background: rgba(222, 233, 228, 0.09); }
  .overlay-shell.dark .segmented button.active { color: #dfe8e3; background: rgba(214, 229, 221, 0.1); }
  .overlay-shell.dark .command-palette { border-color: rgba(207, 223, 215, 0.13); background: rgba(27, 34, 31, 0.985); }
  .overlay-shell.dark .shortcut-editor { border-color: rgba(207, 223, 215, 0.13); background: rgba(27, 34, 31, 0.99); }
  .overlay-shell.dark .shortcut-editor > strong { color: #dce7e1; }
  .overlay-shell.dark .shortcut-editor > small { color: #91a198; }
  .overlay-shell.dark .shortcut-capture { color: #a9d5be; border-color: rgba(139, 195, 166, 0.22); background: rgba(116, 181, 147, 0.07); }
  .overlay-shell.dark .shortcut-editor-actions button { color: #bdcbc4; border-color: rgba(207, 223, 215, 0.12); }
  .overlay-shell.dark .command-search { border-color: rgba(207, 223, 215, 0.09); }
  .overlay-shell.dark .command-search input,
  .overlay-shell.dark .command-results strong { color: #dce7e1; }
  .overlay-shell.dark .command-results > button.active { background: rgba(213, 229, 221, 0.07); }
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
