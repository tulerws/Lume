<script lang="ts">
  import { onMount, tick } from "svelte";
  import { fade, slide } from "svelte/transition";
  import { emit, emitTo, listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import type { AgentSession, DockPreviewEvent, DockSide, PermissionAction, Preferences, PromptAttachmentInput, QuestionAnswer, SessionActivity, SessionNote, TerminalWindowState, WorkflowGroupDefinition, WorkflowRole, WorkflowRoleContract, WorkflowStepDefinition } from "$lib/domain";
  import type { HubSession, WorkItemStatus } from "$lib/hubProtocol";
  import BrandIcon from "$lib/BrandIcon.svelte";
  import ActivityTraceGroup from "$lib/ActivityTraceGroup.svelte";
  import LumeLogo from "$lib/LumeLogo.svelte";
  import LumeMascot from "$lib/LumeMascot.svelte";
  import FileTypeIcon from "$lib/FileTypeIcon.svelte";
  import WorkflowRoleIcon from "$lib/WorkflowRoleIcon.svelte";
  import ResponseAttachments from "$lib/ResponseAttachments.svelte";
  import { cleanPromptTransport, promptTextKey } from "$lib/chatAttachments";
  import { isHiddenAgentActivity, isPresentableTraceActivity, needsUserAuthorization } from "$lib/activityPresentation";
  import { displayText, localize, type Language } from "$lib/i18n";
  import {
    clipboardHasFile,
    clipboardHasImage,
    clipboardMayContainImage,
    collectClipboardFiles,
    collectClipboardImages,
    createImagePreview,
    isImageAttachmentFile,
    isImageAttachmentPath,
    prepareClipboardFile,
    prepareClipboardImage,
  } from "$lib/imageAttachments";
  import { renderSafeMarkdown } from "$lib/markdown.js";
  import { BoundedRenderCache } from "$lib/boundedRenderCache";
  import { latestResponseText, sameResponseText } from "$lib/responseDedup.js";
  import {
    displayFileChangePath,
    mergeFileChanges,
    summarizeFileChanges,
    type FileChangeSummary,
  } from "$lib/fileChanges";
  import {
    buildHandoffBody,
    buildHandoffPrompt,
    parseHandoffPrompt,
  } from "$lib/handoff";
  import { sessionCapabilities } from "$lib/sessionCapabilities";
  import { resolveTerminalSession } from "$lib/sessionIdentity";
  import {
    orderTerminalsByPosition,
    orderWorkflowSteps,
    terminalWorkflowKey,
  } from "$lib/workflowOrder";
  import {
    beginLayeredTerminalResize,
    answerQuestion as answerInteractiveQuestion,
    cancelTerminalWindowMove,
    closeTerminalWindow,
    decidePermission,
    finishLayeredTerminalResize,
    getSessionCollaborationMode,
    getClaudeSessionModelSettings,
    getSessionModelSettings,
    interruptPrompt,
    loadDisplayBackend,
    loadPreferences,
    loadSessionNotes,
    loadTerminalHubSnapshot,
    loadWorkflowRoleContract,
    loadHubSnapshot,
    loadTerminalWindows,
    loadTerminalWindowState,
    markTerminalFrontendReady,
    minimizeTerminalWindow,
    moveTerminalWindow,
    openWorkflowBridgeWindow,
    openSessionSource,
    readLocalImageDataUrl,
    prepareWorkflowBridgeWindow,
    refreshAgentRateLimits,
    renameSession,
    resizeTerminalWindow,
    savePreferences,
    saveSessionNote,
    setWorkflowConnectionHover,
    setSessionCollaborationMode,
    setClaudeSessionModelSettings,
    setSessionModelSettings,
    setTerminalFileDialogActive,
    steerQueuedPrompt,
    submitPrompt,
    syncTerminalWindowPosition,
    terminalGroupFullscreenActive,
    takeControlSession,
    terminateSession,
    deleteSessionNote,
    toggleTerminalGroupFullscreen,
    undockTerminalWindow,
    type CollaborationMode,
    type CodexThreadModelSettings,
    type DisplayBackend,
  } from "$lib/lume";

  const currentWindow = getCurrentWindow();
  const label = currentWindow.label;
  const isWindows = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);
  type ResizeDirection = "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";
  type SlashCommand = {
    name: string;
    description: string;
    source: "agent" | "lume";
    action?: "model" | "plan" | "default" | "interrupt" | "steer" | "rename" | "detach" | "fullscreen" | "zoom-in" | "zoom-out" | "close";
  };
  let windowState = $state<TerminalWindowState | null>(null);
  let session = $state<HubSession | null>(null);
  let initializationError = $state<string | null>(null);
  let initializationRun = 0;
  let prompt = $state("");
  let promptInput = $state<HTMLTextAreaElement | null>(null);
  let slashCommandIndex = $state(0);
  let slashMenuDismissed = $state(false);
  let slashCommandMenu = $state<HTMLDivElement | null>(null);
  let fullscreen = $state(false);
  let promptAttachments = $state<PromptAttachmentInput[]>([]);
  let message = $state<string | null>(null);
  type HandoffTarget = {
    terminal: TerminalWindowState;
    session: HubSession;
  };
  type HandoffDraft = {
    text: string;
    files: FileChangeSummary[];
    includeText: boolean;
    includeFiles: boolean;
    note: string;
    targetSessionId: string;
  };
  let handoffDraft = $state<HandoffDraft | null>(null);
  let handoffTargets = $state<HandoffTarget[]>([]);
  let handoffLoading = $state(false);
  let handoffSending = $state(false);
  let handoffError = $state<string | null>(null);
  let sending = $state(false);
  let steeringQueued = $state(false);
  let collaborationMode = $state<CollaborationMode>("default");
  let collaborationModeChanging = $state(false);
  let collaborationModeTarget = $state<CollaborationMode | null>(null);
  let collaborationModeNotice = $state<string | null>(null);
  let collaborationModeNoticeTimer: ReturnType<typeof setTimeout> | undefined;
  let composerToolsOpen = $state(false);
  let modelDialogOpen = $state(false);
  let modelSettings = $state<CodexThreadModelSettings | null>(null);
  let selectedModel = $state("");
  let selectedEffort = $state("");
  let claudeModel = $state("");
  let claudeEffort = $state("");
  let modelLoading = $state(false);
  let modelSaving = $state(false);
  let modelError = $state<string | null>(null);
  let questionSelections = $state<Record<string, string>>({});
  let dragging = $state(false);
  let dragMoved = false;
  let pendingMove: { x: number; y: number } | null = null;
  let lastMove: { x: number; y: number } | null = null;
  let moveSyncRunning = false;
  let finalizeRequested = false;
  let displayBackend = $state<DisplayBackend>("native");
  let nativeDragActive = false;
  let nativeDragEndTimer: ReturnType<typeof setTimeout> | undefined;
  let nativePosition: { x: number; y: number } | null = null;
  let pendingNativeSync: { x: number; y: number; finalize: boolean } | null = null;
  let nativeSyncRunning = false;
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
  let pendingResize: { x: number; y: number; width: number; height: number; fromLeft: boolean; fromTop: boolean } | null = null;
  let resizeSyncRunning = false;
  let resizeFrame: number | null = null;
  let resizeThrottleTimer: ReturnType<typeof setTimeout> | undefined;
  let lastResizeFlushAt = 0;
  let settling = $state(false);
  let dockMovingLabel = $state<string | null>(null);
  let dockPreview = $state<NonNullable<DockPreviewEvent["preview"]> | null>(null);
  let terminateConfirm = $state(false);
  let terminating = $state(false);
  let takeoverConfirm = $state(false);
  let takingControl = $state(false);
  let interrupting = $state(false);
  let renamingSession = $state(false);
  let renameDraft = $state("");
  let savingSessionName = $state(false);
  type TerminalTab = "chat" | "changes" | "plan" | "notes";
  let activeTab = $state<TerminalTab>("chat");
  let sessionNotes = $state<SessionNote[]>([]);
  let notesSessionId = $state<string | null>(null);
  let notesLoading = $state(false);
  let noteSaving = $state(false);
  let noteEditor = $state<{
    id?: string;
    title: string;
    body: string;
    kind: "plan" | "note";
    pinned: boolean;
  } | null>(null);
  let workTrayExpanded = $state(true);
  let rateLimitRefreshRequested = false;
  let outputElement = $state<HTMLDivElement | null>(null);
  let visibleChatItemLimit = $state(60);
  let outputFollowingTail = true;
  let chatFollowingTail = true;
  const tabScrollPositions: Partial<Record<TerminalTab, number>> = {};
  let language = $state<Language>("en");
  let darkMode = $state<boolean | undefined>(undefined);
  let workflowGroups = $state<WorkflowGroupDefinition[]>([]);
  let workflowTerminals = $state<TerminalWindowState[]>([]);
  let workflowDraft = $state<WorkflowGroupDefinition | null>(null);
  let workflowEditingStepId = $state<string | null>(null);
  let workflowDraftSaving = $state(false);
  let workflowDraftError = $state<string | null>(null);
  let workflowRolePickerOpen = $state(false);
  let workflowRoleConfigured = $state(false);
  let workflowContractExpanded = $state(false);
  let workflowDefaultContract = $state<WorkflowRoleContract | null>(null);
  let workflowPendingRole = $state<WorkflowRole | null>(null);
  let workflowRoleFabElement = $state<HTMLButtonElement | null>(null);
  let workflowRolePopoverStyle = $state("");
  let workflowRolePopoverAbove = $state(false);
  let workflowRolePopoverConstrained = $state(false);
  let workflowBridgeOpeningSide = $state<DockSide | null>(null);
  let hoveredWorkflowConnectionSides = $state<DockSide[]>([]);
  let workflowConnectionLeaveTimer: ReturnType<typeof setTimeout> | undefined;
  let systemDark = $state(false);
  let workClock = $state(Date.now());
  let workClockTimer: ReturnType<typeof setTimeout> | undefined;
  const markdownRenderCache = new BoundedRenderCache();
  const activityChangeCache = new Map<string, { detail: string; filesKey: string; changes: FileChangeSummary[]; cost: number }>();
  let activityChangeCacheCost = 0;
  const composerMinHeight = 63;
  const composerAttachmentMinHeight = 120;
  const composerQueueMinHeight = 104;
  const composerAttachmentQueueMinHeight = 160;
  const textZoomMin = 0.8;
  const textZoomMax = 1.8;
  let composerHeight = $state(composerMinHeight);
  let composerResizeState: {
    pointerId: number;
    startY: number;
    startHeight: number;
    target: HTMLElement;
  } | null = null;
  let textZoom = $state(1);
  let headerActionsOpen = $state(false);
  const effectiveDark = $derived(darkMode ?? systemDark);
  const workflowGroup = $derived.by(() => {
    if (!windowState?.groupId) return undefined;
    return workflowGroups.find((group) => group.terminalGroupId === windowState?.groupId);
  });
  const workflowGroupTerminals = $derived.by(() => windowState?.groupId
    ? orderTerminalsByPosition(
      workflowTerminals.filter((terminal) => terminal.groupId === windowState?.groupId),
    )
    : []);
  const workflowPhysicalOrder = $derived.by(() => {
    if (!windowState) return 0;
    return Math.max(0, workflowGroupTerminals.findIndex((terminal) => terminal.label === windowState?.label) + 1);
  });
  const orderedWorkflowSteps = $derived.by(() => workflowGroup
    ? orderWorkflowSteps(workflowGroup.steps, workflowTerminals, workflowGroup.connections)
    : []);
  const workflowStep = $derived.by((): WorkflowStepDefinition | undefined => {
    const sessionNativeId = windowState?.sessionNativeId?.trim() || windowState?.sessionId;
    if (!sessionNativeId || !windowState?.groupId) return undefined;
    return workflowGroup?.steps.find((step) => step.sessionNativeId === sessionNativeId);
  });
  const workflowStepOrder = $derived.by(() => {
    if (!workflowStep || !windowState?.groupId) return 0;
    const definedSessions = new Set(workflowGroup?.steps.map((step) => step.sessionNativeId));
    if (workflowGroupTerminals.some((terminal) => !definedSessions.has(terminalWorkflowKey(terminal)))) {
      return workflowPhysicalOrder;
    }
    return Math.max(0, orderedWorkflowSteps.findIndex((step) => step.id === workflowStep.id) + 1);
  });
  const workflowStepTotal = $derived(workflowGroupTerminals.length || orderedWorkflowSteps.length);
  const workflowEditingStep = $derived(
    workflowDraft?.steps.find((step) => step.id === workflowEditingStepId),
  );
  const workflowRoleReady = $derived(workflowStepIsReady(workflowEditingStep));
  const workflowInstructionsCustomized = $derived.by(() => {
    if (!workflowEditingStep || !workflowRoleConfigured) return false;
    if (workflowEditingStep.role === "custom") {
      return Boolean(
        workflowEditingStep.customRoleLabel.trim()
        || workflowEditingStep.instruction.trim()
        || workflowEditingStep.expectedInput.trim()
        || workflowEditingStep.producedOutput.trim()
        || workflowEditingStep.completionCondition.trim(),
      );
    }
    return workflowDefaultContract
      ? !workflowContractMatches(workflowEditingStep, workflowDefaultContract)
      : false;
  });
  const displayedComposerHeight = $derived.by(() => {
    const hasQueuedPrompt = pendingQueuedPrompts(session).length > 0;
    const modeControlHeight = session?.agent === "codex" ? 16 : 0;
    const desired = Math.max(
      composerHeight,
      (promptAttachments.length && hasQueuedPrompt
        ? composerAttachmentQueueMinHeight
        : promptAttachments.length
          ? composerAttachmentMinHeight
          : hasQueuedPrompt
            ? composerQueueMinHeight
            : composerMinHeight) + modeControlHeight,
    );
    return Math.min(desired, composerHeightLimit());
  });
  $effect(() => {
    document.documentElement.dataset.theme = effectiveDark ? "dark" : "light";
  });

  function tr(english: string, portuguese: string) {
    return localize(language, english, portuguese);
  }

  function workflowStepRoleLabel(step: WorkflowStepDefinition) {
    if (step.role === "custom") return step.customRoleLabel.trim() || tr("Custom", "Personalizado");
    return {
      planner: tr("Planner", "Planejador"),
      implementer: tr("Implementer", "Implementador"),
      reviewer: tr("Reviewer", "Revisor"),
      tester: tr("Tester", "Testador"),
      researcher: tr("Researcher", "Pesquisador"),
      custom: tr("Custom", "Personalizado"),
    }[step.role];
  }

  const workflowRoles: WorkflowRole[] = [
    "planner",
    "implementer",
    "reviewer",
    "tester",
    "researcher",
    "custom",
  ];

  function workflowRoleDescription(role: WorkflowRole) {
    return {
      planner: tr("Structures the approach and decisions", "Estrutura a abordagem e as decisões"),
      implementer: tr("Builds the approved solution", "Constrói a solução aprovada"),
      reviewer: tr("Finds risks and improvement points", "Encontra riscos e pontos de melhoria"),
      tester: tr("Validates behavior and regressions", "Valida comportamento e regressões"),
      researcher: tr("Collects evidence and alternatives", "Reúne evidências e alternativas"),
      custom: tr("Uses your own responsibility", "Usa uma responsabilidade personalizada"),
    }[role];
  }

  function workflowStepIsReady(step?: WorkflowStepDefinition) {
    if (!step) return false;
    return Boolean(
      (step.role !== "custom" || step.customRoleLabel.trim())
      && step.instruction.trim()
      && step.expectedInput.trim()
      && step.producedOutput.trim()
      && step.completionCondition.trim(),
    );
  }

  function workflowContractMatches(
    step: WorkflowStepDefinition,
    contract: WorkflowRoleContract,
  ) {
    return step.instruction.trim() === contract.instruction.trim()
      && step.expectedInput.trim() === contract.expectedInput.trim()
      && step.producedOutput.trim() === contract.producedOutput.trim()
      && step.completionCondition.trim() === contract.completionCondition.trim();
  }

  function workflowRoleOptionLabel(role: WorkflowRole) {
    return workflowStepRoleLabel({
      id: "",
      sessionNativeId: "",
      role,
      customRoleLabel: "",
      instruction: "",
      expectedInput: "",
      producedOutput: "",
      completionCondition: "",
      attempt: 0,
    });
  }

  function defaultWorkflowRole(index: number): WorkflowRole {
    return (["planner", "implementer", "reviewer", "tester"] as WorkflowRole[])[index] ?? "custom";
  }

  function positionWorkflowRolePopover() {
    if (!workflowRoleFabElement) return;
    const bounds = workflowRoleFabElement.getBoundingClientRect();
    const margin = 8;
    const gap = 9;
    const below = Math.max(0, window.innerHeight - bounds.bottom - gap - margin);
    const above = Math.max(0, bounds.top - gap - margin);
    workflowRolePopoverAbove = below < 230 && above > below;
    const available = workflowRolePopoverAbove ? above : below;
    workflowRolePopoverConstrained = available < 330;
    workflowRolePopoverStyle = workflowRolePopoverAbove
      ? `top:auto;bottom:${Math.round(window.innerHeight - bounds.top + gap)}px;max-height:${Math.round(available)}px`
      : `top:${Math.round(bounds.bottom + gap)}px;bottom:auto;max-height:${Math.round(available)}px`;
  }

  async function openWorkflowRoleEditor() {
    if (!windowState?.workflowEnabled || !windowState.groupId) return;
    workflowDraftError = null;
    workflowRolePickerOpen = false;
    workflowPendingRole = null;
    workflowDefaultContract = null;
    try {
      const allTerminals = await loadTerminalWindows();
      workflowTerminals = allTerminals;
      const terminals = orderTerminalsByPosition(
        allTerminals.filter((terminal) => terminal.groupId === windowState?.groupId),
      );
      const saved = workflowGroups.find((group) => group.terminalGroupId === windowState?.groupId);
      const retained = saved?.steps ?? [];
      const currentTerminalKey = terminalWorkflowKey(windowState);
      workflowRoleConfigured = retained.some((step) => step.sessionNativeId === currentTerminalKey);
      workflowContractExpanded = false;
      const retainedKeys = new Set(retained.map((step) => step.sessionNativeId));
      const currentTerminal = terminals.find((terminal) => terminalWorkflowKey(terminal) === currentTerminalKey);
      const appended = currentTerminal && !retainedKeys.has(currentTerminalKey)
        ? [await (async (): Promise<WorkflowStepDefinition> => {
          const sessionNativeId = currentTerminalKey;
          const role = defaultWorkflowRole(retained.length);
          const contract = await loadWorkflowRoleContract(role);
          return {
            id: `step-${sessionNativeId}`,
            sessionNativeId,
            role,
            customRoleLabel: "",
            ...contract,
            attempt: 0,
          };
        })()]
        : [];
      const draftSteps = [...retained, ...appended];
      const draftConnections = (saved?.connections ?? []).filter((connection) =>
        draftSteps.some((step) => step.id === connection.fromStepId)
        && draftSteps.some((step) => step.id === connection.toStepId));
      workflowDraft = {
        id: saved?.id ?? `workflow-${windowState.groupId}`,
        terminalGroupId: windowState.groupId,
        steps: orderWorkflowSteps(draftSteps, terminals, draftConnections),
        connections: draftConnections,
      };
      workflowEditingStepId = workflowDraft.steps
        .find((step) => step.sessionNativeId === terminalWorkflowKey(windowState!))?.id ?? null;
      const editingStep = workflowDraft.steps.find((step) => step.id === workflowEditingStepId);
      workflowDefaultContract = editingStep
        ? await loadWorkflowRoleContract(editingStep.role)
        : null;
      workflowRolePickerOpen = !workflowRoleConfigured;
      await tick();
      positionWorkflowRolePopover();
    } catch (error) {
      workflowDraftError = String(error).replace(/^Error:\s*/, "");
    }
  }

  function updateWorkflowEditingStep(patch: Partial<WorkflowStepDefinition>) {
    if (!workflowDraft || !workflowEditingStepId) return;
    workflowDraft = {
      ...workflowDraft,
      steps: workflowDraft.steps.map((step) =>
        step.id === workflowEditingStepId ? { ...step, ...patch } : step),
    };
  }

  async function selectWorkflowRole(role: WorkflowRole) {
    workflowDraftError = null;
    if (workflowRoleConfigured && workflowEditingStep?.role === role) {
      workflowRolePickerOpen = false;
      return;
    }
    if (workflowRoleConfigured && workflowInstructionsCustomized) {
      workflowPendingRole = role;
      workflowRolePickerOpen = false;
      return;
    }
    await applyWorkflowRole(role);
  }

  async function applyWorkflowRole(role: WorkflowRole) {
    try {
      const contract = await loadWorkflowRoleContract(role);
      if (!workflowDraft || !workflowEditingStepId) return;
      const saveSelectionImmediately = !workflowRoleConfigured;
      const nextDraft = {
        ...workflowDraft,
        steps: workflowDraft.steps.map((step) =>
          step.id === workflowEditingStepId
            ? { ...step, role, customRoleLabel: "", ...contract }
            : step),
      };
      workflowDraft = nextDraft;
      workflowDefaultContract = contract;
      workflowPendingRole = null;
      workflowRoleConfigured = true;
      workflowContractExpanded = role === "custom";
      workflowRolePickerOpen = false;
      const nextStep = nextDraft.steps.find((step) => step.id === workflowEditingStepId);
      if (saveSelectionImmediately && workflowStepIsReady(nextStep)) {
        await saveWorkflowRole(false, nextDraft);
      }
    } catch (error) {
      workflowDraftError = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function confirmWorkflowRoleChange() {
    if (!workflowPendingRole) return;
    const role = workflowPendingRole;
    workflowPendingRole = null;
    await applyWorkflowRole(role);
  }

  async function restoreWorkflowRoleDefaults() {
    if (!workflowEditingStep || workflowEditingStep.role === "custom") return;
    workflowDraftError = null;
    try {
      const contract = await loadWorkflowRoleContract(workflowEditingStep.role);
      workflowDefaultContract = contract;
      updateWorkflowEditingStep(contract);
    } catch (error) {
      workflowDraftError = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function openWorkflowBridgeFromConnection(event: MouseEvent, side: DockSide) {
    event.preventDefault();
    event.stopPropagation();
    if (!windowState?.groupId || workflowBridgeOpeningSide) return;
    workflowDraftError = null;
    workflowBridgeOpeningSide = side;
    let preparedLabel: string | null = null;
    const readyLabels = new Set<string>();
    let resolveReady: (() => void) | undefined;
    const ready = new Promise<void>((resolve) => (resolveReady = resolve));
    let stopReady: (() => void) | undefined;
    let readyTimeout: ReturnType<typeof setTimeout> | undefined;
    try {
      stopReady = await listen<{ label: string }>("lume://workflow-bridge-ready", ({ payload }) => {
        readyLabels.add(payload.label);
        if (payload.label === preparedLabel) resolveReady?.();
      });
      preparedLabel = await prepareWorkflowBridgeWindow(label, side);
      await emitTo(preparedLabel, "lume://workflow-bridge-prepare");
      if (!readyLabels.has(preparedLabel)) {
        await Promise.race([
          ready,
          new Promise<never>((_, reject) => {
            readyTimeout = setTimeout(() => reject(new Error(tr(
              "The connection window took too long to load.",
              "A janela de conexão demorou demais para carregar.",
            ))), 8_000);
          }),
        ]);
      }
      await openWorkflowBridgeWindow(label, side);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      if (readyTimeout) clearTimeout(readyTimeout);
      stopReady?.();
      workflowBridgeOpeningSide = null;
    }
  }

  function oppositeDockSide(side: DockSide): DockSide {
    if (side === "left") return "right";
    if (side === "right") return "left";
    if (side === "top") return "bottom";
    return "top";
  }

  function enterWorkflowConnection(side: DockSide) {
    if (workflowConnectionLeaveTimer) {
      clearTimeout(workflowConnectionLeaveTimer);
      workflowConnectionLeaveTimer = undefined;
    }
    void setWorkflowConnectionHover(label, side, true).catch(() => undefined);
  }

  function leaveWorkflowConnection(side: DockSide) {
    if (workflowConnectionLeaveTimer) clearTimeout(workflowConnectionLeaveTimer);
    workflowConnectionLeaveTimer = setTimeout(() => {
      workflowConnectionLeaveTimer = undefined;
      void setWorkflowConnectionHover(label, side, false).catch(() => undefined);
    }, 80);
  }

  async function saveWorkflowRole(closeEditor = true, draftOverride?: WorkflowGroupDefinition) {
    const draft = draftOverride ?? workflowDraft;
    if (!draft || workflowDraftSaving || !workflowRoleConfigured) return;
    const editingStep = draft.steps.find((step) => step.id === workflowEditingStepId);
    if (!workflowStepIsReady(editingStep)) {
      workflowDraftError = tr(
        "Complete all role instructions before saving.",
        "Preencha todas as instruções do papel antes de salvar.",
      );
      workflowContractExpanded = true;
      return;
    }
    workflowDraftSaving = true;
    workflowDraftError = null;
    try {
      const preferences = await loadPreferences();
      preferences.workflowGroups = preferences.workflowGroups.some((group) => group.id === draft.id)
        ? preferences.workflowGroups.map((group) => group.id === draft.id ? draft : group)
        : [...preferences.workflowGroups, draft];
      await savePreferences(preferences);
      workflowGroups = preferences.workflowGroups;
      await emit("lume://preferences-changed", preferences);
      if (closeEditor) {
        workflowDraft = null;
        workflowEditingStepId = null;
        workflowRolePickerOpen = false;
        workflowRoleConfigured = false;
        workflowContractExpanded = false;
        workflowDefaultContract = null;
        workflowPendingRole = null;
      } else {
        workflowDraft = draft;
      }
    } catch (error) {
      workflowDraftError = String(error).replace(/^Error:\s*/, "");
    } finally {
      workflowDraftSaving = false;
    }
  }

  function sessionDisplayName(item: AgentSession) {
    return item.sessionName?.trim() || item.project?.trim() || item.agentLabel;
  }

  function sessionDirectoryName(item: AgentSession) {
    const directory = item.workingDirectory?.trim().replace(/[\\/]+$/, "");
    return directory?.split(/[\\/]/).pop() || item.project?.trim() || item.agentLabel;
  }

  function pendingQueuedPrompts(item: AgentSession | null) {
    return (item?.activities ?? [])
      .filter((activity) =>
        activity.kind === "queued_prompt" && activity.status === "waiting"
      )
      .sort((left, right) => left.createdAt - right.createdAt);
  }

  function terminalStorageKey(setting: string) {
    return `lume-terminal-${setting}-v1:${label}`;
  }

  function composerHeightLimit() {
    const stateHeight = windowState?.height ?? 0;
    const viewportHeight = typeof window === "undefined" ? stateHeight : window.innerHeight;
    const height = viewportHeight || stateHeight || 400;
    return Math.max(
      composerMinHeight,
      Math.min(height - 120, Math.floor(height * 0.68)),
    );
  }

  function clampComposerHeight(value: number) {
    return Math.max(composerMinHeight, Math.min(composerHeightLimit(), Math.round(value)));
  }

  function persistTerminalSetting(setting: string, value: number) {
    try {
      localStorage.setItem(terminalStorageKey(setting), String(value));
    } catch {
      // The terminal remains usable when web storage is unavailable.
    }
  }

  function setTextZoom(value: number) {
    textZoom = Math.round(Math.max(textZoomMin, Math.min(textZoomMax, value)) * 10) / 10;
    persistTerminalSetting("text-zoom", textZoom);
  }

  function beginComposerResize(event: PointerEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    composerResizeState = {
      pointerId: event.pointerId,
      startY: event.screenY,
      startHeight: displayedComposerHeight,
      target,
    };
  }

  function moveComposerResize(event: PointerEvent) {
    if (!composerResizeState || composerResizeState.pointerId !== event.pointerId) return;
    event.preventDefault();
    event.stopPropagation();
    composerHeight = clampComposerHeight(
      composerResizeState.startHeight + composerResizeState.startY - event.screenY,
    );
  }

  function endComposerResize(event: PointerEvent) {
    if (!composerResizeState || composerResizeState.pointerId !== event.pointerId) return;
    event.preventDefault();
    event.stopPropagation();
    if (composerResizeState.target.hasPointerCapture(event.pointerId)) {
      composerResizeState.target.releasePointerCapture(event.pointerId);
    }
    composerResizeState = null;
    persistTerminalSetting("composer-height", composerHeight);
  }

  function resizeComposerWithKeyboard(event: KeyboardEvent) {
    const direction = event.key === "ArrowUp" ? 1 : event.key === "ArrowDown" ? -1 : 0;
    if (!direction) return;
    event.preventDefault();
    composerHeight = clampComposerHeight(displayedComposerHeight + direction * 20);
    persistTerminalSetting("composer-height", composerHeight);
  }

  const capabilities = $derived(
    session ? session.capabilities ?? sessionCapabilities(session) : null,
  );
  const canSubmit = $derived(Boolean(capabilities?.canPrompt));
  const canCompose = $derived(Boolean(canSubmit || capabilities?.canTakeControl));
  const promptIsRunning = $derived(session?.status === "running");
  const canSendWhileRunning = $derived(
    Boolean(
      promptIsRunning
      && capabilities?.promptDeliveries.includes("steer"),
    ),
  );
  const canInterruptRunningPrompt = $derived(
    Boolean(promptIsRunning && capabilities?.canInterrupt),
  );
  const queuedPrompts = $derived(pendingQueuedPrompts(session));
  const nextQueuedPrompt = $derived(queuedPrompts[0] ?? null);
  const readyForPrompt = $derived(
    Boolean(
      session
      && (
        ["completed", "failed", "waiting_for_input"].includes(session.status)
        || canSendWhileRunning
        || capabilities?.canTakeControl
      ),
    ),
  );

  function slashCommandQuery() {
    const value = prompt.trimStart();
    if (!value.startsWith("/") || /\s/.test(value)) return null;
    return value.slice(1).toLowerCase();
  }

  const codexSlashCommands: Array<[string, string]> = [
    ["model", "Choose the model and reasoning effort"],
    ["fast", "Toggle the faster service tier"],
    ["personality", "Choose how Codex communicates"],
    ["permissions", "Change approval and sandbox permissions"],
    ["plan", "Switch to Plan mode"],
    ["goal", "View or manage the current task goal"],
    ["status", "Show session configuration and token usage"],
    ["usage", "Show account usage and rate limits"],
    ["diff", "Show working tree changes"],
    ["review", "Review the current working tree"],
    ["mention", "Attach a file or folder to the prompt"],
    ["compact", "Summarize the chat to free context"],
    ["new", "Start a new chat"],
    ["rename", "Rename the current chat"],
    ["resume", "Resume a saved chat"],
    ["fork", "Fork the current chat"],
    ["side", "Start an ephemeral side chat"],
    ["agent", "Switch between agent threads"],
    ["ps", "Show background terminals"],
    ["stop", "Stop background terminals"],
    ["approve", "Retry a recent auto-review denial"],
    ["experimental", "Configure experimental features"],
    ["memories", "Configure memory use and generation"],
    ["skills", "Browse and use skills"],
    ["import", "Import setup and chats from Claude Code"],
    ["ide", "Include current IDE context"],
    ["apps", "Browse connected apps"],
    ["plugins", "Browse and manage plugins"],
    ["hooks", "View and manage lifecycle hooks"],
    ["mcp", "List configured MCP tools"],
    ["init", "Generate an AGENTS.md file"],
    ["copy", "Copy the latest completed response"],
    ["raw", "Toggle raw scrollback"],
    ["clear", "Clear the terminal and start a new chat"],
    ["archive", "Archive this session and exit"],
    ["delete", "Permanently delete this session"],
    ["statusline", "Configure status-line items"],
    ["title", "Configure terminal title items"],
    ["theme", "Choose the syntax theme"],
    ["pets", "Choose or hide the terminal pet"],
    ["keymap", "Configure TUI keyboard shortcuts"],
    ["vim", "Toggle Vim mode"],
    ["app", "Continue this session in the desktop app"],
    ["feedback", "Send feedback to Codex"],
    ["logout", "Sign out of Codex"],
    ["quit", "Exit Codex"],
  ];

  const claudeSlashCommands: Array<[string, string]> = [
    ["model", "Choose the Claude model"],
    ["permissions", "View or update tool permissions"],
    ["plan", "Enter plan mode"],
    ["btw", "Ask a side question without interrupting the task"],
    ["compact", "Compact the conversation context"],
    ["context", "Inspect context usage"],
    ["cost", "Show token usage and cost"],
    ["diff", "Review changed files"],
    ["doctor", "Check the Claude Code installation"],
    ["hooks", "Manage lifecycle hooks"],
    ["ide", "Manage the IDE integration"],
    ["mcp", "Manage MCP servers"],
    ["memory", "Edit project memory"],
    ["review", "Review current changes"],
    ["resume", "Resume another conversation"],
    ["rename", "Rename the current conversation"],
    ["status", "Show session status"],
    ["vim", "Toggle Vim editing mode"],
    ["clear", "Clear conversation history"],
    ["help", "Show Claude Code help"],
    ["exit", "Exit Claude Code"],
  ];

  const geminiSlashCommands: Array<[string, string]> = [
    ["model", "Choose the Gemini model"],
    ["memory", "Manage saved context"],
    ["chat", "Manage conversation history"],
    ["compress", "Compress the conversation context"],
    ["directory", "Manage workspace directories"],
    ["extensions", "Manage Gemini CLI extensions"],
    ["mcp", "Manage MCP servers"],
    ["settings", "Open Gemini CLI settings"],
    ["stats", "Show session usage statistics"],
    ["tools", "List available tools"],
    ["help", "Show Gemini CLI help"],
    ["clear", "Clear the screen and conversation"],
    ["quit", "Exit Gemini CLI"],
  ];

  const antigravitySlashCommands: Array<[string, string]> = [
    ["model", "Choose the Antigravity model"],
    ["effort", "Choose the reasoning effort"],
    ["resume", "Resume or switch conversations"],
    ["tasks", "Inspect background tasks"],
    ["permissions", "Review agent permissions"],
    ["usage", "Inspect current usage"],
    ["diff", "Review changed files"],
    ["btw", "Add context without interrupting the task"],
    ["hooks", "Manage lifecycle hooks"],
    ["settings", "Open Antigravity settings"],
    ["help", "Show Antigravity help"],
    ["quit", "Exit Antigravity CLI"],
  ];

  function agentSlashCommands(): SlashCommand[] {
    const catalog =
      session?.agent === "codex"
        ? codexSlashCommands
        : session?.agent === "claude_code"
          ? claudeSlashCommands
          : session?.agent === "antigravity"
            ? antigravitySlashCommands
          : session?.agent === "gemini"
            ? geminiSlashCommands
            : [];
    return catalog.map(([name, description]) => ({
      name,
      description,
      source: ["codex", "claude_code"].includes(session?.agent ?? "") && name === "model" ? "lume" : "agent",
      action: ["codex", "claude_code"].includes(session?.agent ?? "") && name === "model" ? "model" : undefined,
    }));
  }

  function availableSlashCommands(): SlashCommand[] {
    const commands = agentSlashCommands();
    const lumeCommands: SlashCommand[] = [];
    if (session?.agent === "codex" && !promptIsRunning) {
      lumeCommands.push({ name: "lume-default", description: "Switch Codex to Default mode", source: "lume", action: "default" });
    }
    if (canInterruptRunningPrompt) {
      lumeCommands.push({ name: "lume-interrupt", description: "Interrupt the current prompt", source: "lume", action: "interrupt" });
    }
    if (nextQueuedPrompt && canSendWhileRunning) {
      lumeCommands.push({ name: "lume-steer", description: "Steer the next queued prompt now", source: "lume", action: "steer" });
    }
    lumeCommands.push(
      { name: "lume-rename", description: "Rename this session", source: "lume", action: "rename" },
      { name: "lume-zoom-in", description: "Increase chat text size", source: "lume", action: "zoom-in" },
      { name: "lume-zoom-out", description: "Decrease chat text size", source: "lume", action: "zoom-out" },
    );
    if (windowState?.docked) {
      lumeCommands.push({ name: "lume-detach", description: "Undock this terminal", source: "lume", action: "detach" });
    }
    lumeCommands.push({ name: "lume-fullscreen", description: fullscreen ? "Exit full screen" : "Enter full screen", source: "lume", action: "fullscreen" });
    lumeCommands.push({ name: "lume-close", description: "Close this terminal", source: "lume", action: "close" });
    return [...commands, ...lumeCommands];
  }

  function filteredSlashCommands() {
    const query = slashCommandQuery();
    if (query === null || slashMenuDismissed) return [];
    return availableSlashCommands().filter((command) =>
      !query
      || command.name.includes(query)
      || command.description.toLowerCase().includes(query)
    );
  }

  async function selectSlashCommand(command: SlashCommand) {
    prompt = `/${command.name}`;
    slashCommandIndex = 0;
    slashMenuDismissed = true;
    await tick();
    promptInput?.focus();
    promptInput?.setSelectionRange(prompt.length, prompt.length);
  }

  function handlePromptInput(event: Event) {
    prompt = (event.currentTarget as HTMLTextAreaElement).value;
    slashCommandIndex = 0;
    slashMenuDismissed = false;
  }

  async function revealSelectedSlashCommand() {
    await tick();
    slashCommandMenu
      ?.querySelector<HTMLElement>(`[data-slash-index="${slashCommandIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }

  const activeRateLimit = $derived.by(() => {
    const limits = (session?.rateLimits ?? [])
      .filter((limit) => Number.isFinite(Number(limit.usedPercent)));
    return [...limits].sort(
      (left, right) => Number(right.usedPercent) - Number(left.usedPercent),
    )[0] ?? null;
  });
  const rateLimitRemaining = $derived(
    activeRateLimit
      ? Math.max(0, Math.min(100, Math.round(100 - Number(activeRateLimit.usedPercent))))
      : 0,
  );
  const plan = $derived(session?.workSummary.plan ?? null);
  const goalSource = $derived(session?.workSummary.goal ?? null);
  const activePlan = $derived(
    plan
      && (
        plan.items.some((item) => item.status !== "completed")
        || workClock - plan.updatedAt < 12_000
      )
      ? plan
      : null,
  );
  const goal = $derived(
    goalSource
      && (
        goalSource.status !== "complete"
        || workClock - goalSource.updatedAt < 12_000
      )
      ? goalSource
      : null,
  );
  $effect(() => {
    const currentPlan = plan;
    const currentGoal = goalSource;
    const activeGoal = currentGoal?.status === "active";
    const now = Date.now();
    const expiryDelays = [
      currentPlan && currentPlan.items.every((item) => item.status === "completed")
        ? currentPlan.updatedAt + 12_000 - now
        : Number.POSITIVE_INFINITY,
      currentGoal && currentGoal.status !== "active"
        ? currentGoal.updatedAt + 12_000 - now
        : Number.POSITIVE_INFINITY,
    ];
    const labelOffset = Array.from(label).reduce(
      (hash, character) => (hash * 31 + character.charCodeAt(0)) % 8_000,
      0,
    );
    const firstDelay = Math.min(
      activeGoal ? 60_000 + labelOffset : Number.POSITIVE_INFINITY,
      ...expiryDelays,
    );
    if (!Number.isFinite(firstDelay)) return;

    let cancelled = false;
    const tickWorkClock = () => {
      if (cancelled) return;
      workClock = Date.now();
      if (activeGoal) {
        workClockTimer = setTimeout(tickWorkClock, 60_000 + labelOffset);
      }
    };
    workClockTimer = setTimeout(tickWorkClock, Math.max(16, firstDelay));
    return () => {
      cancelled = true;
      if (workClockTimer) clearTimeout(workClockTimer);
      workClockTimer = undefined;
    };
  });
  const completedPlanItems = $derived(
    plan?.items.filter((item) => item.status === "completed").length ?? 0,
  );

  function rateLimitWindow(minutes?: number) {
    if (!minutes) return tr("Rate limit", "Limite");
    if (minutes % 1_440 === 0) return `${minutes / 1_440}d`;
    if (minutes % 60 === 0) return `${minutes / 60}h`;
    return `${minutes}m`;
  }

  function rateLimitTitle() {
    if (!activeRateLimit) return "";
    const reset = activeRateLimit.resetsAt
      ? new Intl.DateTimeFormat(language, {
          dateStyle: "short",
          timeStyle: "short",
        }).format(new Date(activeRateLimit.resetsAt))
      : null;
    return [
      `${activeRateLimit.label}: ${rateLimitRemaining}% ${tr("remaining", "restante")}`,
      `${rateLimitWindow(activeRateLimit.windowMinutes)} ${tr("window", "janela")}`,
      reset ? `${tr("resets", "reinicia")} ${reset}` : null,
    ].filter(Boolean).join(" · ");
  }

  function workItemLabel(status: WorkItemStatus) {
    return {
      pending: tr("Pending", "Pendente"),
      in_progress: tr("In progress", "Em andamento"),
      completed: tr("Done", "Concluído"),
    }[status];
  }

  function goalStatusLabel(status: "active" | "complete" | "blocked") {
    return {
      active: tr("Active", "Ativa"),
      complete: tr("Complete", "Concluída"),
      blocked: tr("Blocked", "Bloqueada"),
    }[status];
  }

  function elapsedGoalTime() {
    if (!goal) return "";
    const end = goal.status === "active" ? workClock : goal.updatedAt;
    const elapsedSeconds = Math.max(0, Math.floor((end - goal.startedAt) / 1_000));
    const days = Math.floor(elapsedSeconds / 86_400);
    const hours = Math.floor((elapsedSeconds % 86_400) / 3_600);
    const minutes = Math.floor((elapsedSeconds % 3_600) / 60);
    if (days) return `${days}d ${hours}h`;
    if (hours) return `${hours}h ${minutes}m`;
    return `${minutes}m`;
  }

  function promptUnavailableText() {
    if (capabilities?.promptUnavailableReason === "session_not_connected") {
      return tr(
        "Waiting for this session to connect to Lume",
        "Aguardando esta sessão se conectar ao Lume",
      );
    }
    if (capabilities?.promptUnavailableReason === "working_directory_missing") {
      return tr(
        "The project folder is unavailable for resuming this session",
        "A pasta do projeto está indisponível para retomar esta sessão",
      );
    }
    if (capabilities?.promptUnavailableReason === "agent_busy") {
      return tr(
        "Wait for the web agent to finish before sending another prompt",
        "Aguarde o agente web terminar antes de enviar outro prompt",
      );
    }
    if (capabilities?.promptUnavailableReason === "external_session") {
      return tr(
        "Write a prompt to transfer this session to Lume",
        "Escreva um prompt para transferir esta sessão para o Lume",
      );
    }
    return tr(
      "This agent does not support prompts through Lume yet",
      "Este agente ainda não aceita prompts pelo Lume",
    );
  }

  const activities = $derived(session?.activities ?? []);
  function isInternalGoalActivity(activity: SessionActivity): boolean {
    return /^functions\s*[·:]\s*(?:create_goal|get_goal|update_goal)$/i.test(activity.title.trim());
  }
  const chatActivities = $derived(activities.filter((activity) => !isInternalGoalActivity(activity)));
  function activityReportedFiles(activity: SessionActivity): string[] {
    const files = [...activity.files];
    const title = activity.title.trim();
    const titleLooksLikePath =
      activity.kind === "file" &&
      !/^(?:arquivos alterados|alterações da tarefa|\d+\s+arquivos alterados)$/i.test(title) &&
      (title.includes("/") || title.includes("\\") || /\.[a-z0-9]{1,8}$/i.test(title));
    if (titleLooksLikePath && !files.includes(title)) files.push(title);
    return files;
  }

  function activityChanges(activity: SessionActivity): FileChangeSummary[] {
    const detail = activity.detail ?? "";
    const reportedFiles = activityReportedFiles(activity);
    const filesKey = reportedFiles.join("\u0000");
    const cached = activityChangeCache.get(activity.id);
    if (cached?.detail === detail && cached.filesKey === filesKey) {
      return cached.changes.map((change) => ({ ...change }));
    }
    if (cached) activityChangeCacheCost -= cached.cost;
    const changes = summarizeFileChanges(detail, reportedFiles, session?.workingDirectory);
    const nextCacheEntry = {
      detail,
      filesKey,
      changes: changes.map((change) => ({ ...change })),
      cost: (detail.length + filesKey.length) * 2,
    };
    activityChangeCache.set(activity.id, nextCacheEntry);
    activityChangeCacheCost += nextCacheEntry.cost;
    while (activityChangeCache.size > 160 || activityChangeCacheCost > 4 * 1024 * 1024) {
      const oldest = activityChangeCache.keys().next().value;
      if (!oldest) break;
      const removed = activityChangeCache.get(oldest);
      activityChangeCache.delete(oldest);
      if (removed) activityChangeCacheCost -= removed.cost;
    }
    return changes;
  }

  function renderCachedMarkdown(key: string, value: string): string {
    return markdownRenderCache.render(key, value, renderSafeMarkdown);
  }
  const changedFiles = $derived.by(() => {
    const files: FileChangeSummary[] = [];
    for (const activity of activities) mergeFileChanges(files, activityChanges(activity));
    for (const result of session?.results ?? []) {
      mergeFileChanges(
        files,
        summarizeFileChanges(result.response, result.files, session?.workingDirectory),
      );
    }
    return files;
  });
  type ChatEntry = {
    id: string;
    activity: SessionActivity;
    files: FileChangeSummary[];
    sequence: number;
  };
  function chatTextKey(value?: string): string {
    return (value ?? "")
      .replace(/\r\n?/g, "\n")
      .replace(/[ \t]+$/gm, "")
      .trim();
  }
  function fileChangesKey(files: FileChangeSummary[]): string {
    return files
      .map((file) => `${file.path}\u0000${file.added}\u0000${file.removed}`)
      .sort()
      .join("\u0001");
  }
  function mergeChatAttachments(target: SessionActivity, source: SessionActivity) {
    const attachments = [...(target.attachments ?? [])];
    for (const attachment of source.attachments ?? []) {
      const duplicate = attachments.some((existing) =>
        existing.path && attachment.path
          ? existing.path.replace(/\\/g, "/").toLowerCase() === attachment.path.replace(/\\/g, "/").toLowerCase()
          : existing.name === attachment.name
      );
      if (!duplicate) attachments.push(attachment);
    }
    if (attachments.length) target.attachments = attachments;
  }
  const chatEntries = $derived.by<ChatEntry[]>(() => {
    let sequence = 0;
    const uniquePrompts: SessionActivity[] = [];
    let messageSinceLastPrompt = false;
    for (const activity of [...chatActivities].sort((left, right) => left.createdAt - right.createdAt)) {
      if (activity.kind === "message") messageSinceLastPrompt = true;
      if (activity.kind !== "prompt") continue;
      const existing = uniquePrompts.at(-1);
      const duplicate = existing
        && !messageSinceLastPrompt
        && !(existing.id.startsWith("local:") && activity.id.startsWith("local:"))
        && promptTextKey(existing.detail) === promptTextKey(activity.detail)
        && Math.abs(existing.createdAt - activity.createdAt) < 60_000;
      if (!duplicate) uniquePrompts.push(activity);
      messageSinceLastPrompt = false;
    }
    const promptTimes = uniquePrompts.map((activity) => activity.createdAt);
    const promptSegment = (createdAt: number) => {
      let low = 0;
      let high = promptTimes.length - 1;
      let segment = Number.NEGATIVE_INFINITY;
      while (low <= high) {
        const middle = (low + high) >> 1;
        const promptTime = promptTimes[middle];
        if (promptTime <= createdAt) {
          segment = promptTime;
          low = middle + 1;
        } else {
          high = middle - 1;
        }
      }
      return segment;
    };
    const entries: ChatEntry[] = [];
    for (const activity of chatActivities) {
      if (
        activity.kind === "queued_prompt"
        || activity.kind === "plan"
        || activity.kind === "plan_document"
      ) continue;
      if (activity.kind === "prompt") {
        let duplicateIndex = -1;
        for (let index = entries.length - 1; index >= 0; index -= 1) {
          const existing = entries[index].activity;
          if (
            existing.kind === "prompt"
            && !(existing.id.startsWith("local:") && activity.id.startsWith("local:"))
            && promptTextKey(existing.detail) === promptTextKey(activity.detail)
            && Math.abs(existing.createdAt - activity.createdAt) < 60_000
          ) {
            duplicateIndex = index;
            break;
          }
        }
        const duplicatePrompt = duplicateIndex >= 0
          && !entries.slice(duplicateIndex + 1).some((entry) =>
            entry.activity.kind === "prompt" || entry.activity.kind === "message"
          )
          ? entries[duplicateIndex]
          : undefined;
        if (duplicatePrompt) {
          mergeChatAttachments(duplicatePrompt.activity, activity);
          continue;
        }
      }
      const files = activityChanges(activity);
      const matchingMessage = activity.kind === "message"
        ? entries.findLast((entry) =>
            entry.activity.kind === "message" &&
            sameResponseText(entry.activity.detail, activity.detail) &&
            promptSegment(entry.activity.createdAt) === promptSegment(activity.createdAt)
          )
        : undefined;
      if (matchingMessage) {
        const previousCreatedAt = matchingMessage.activity.createdAt;
        matchingMessage.activity.detail = latestResponseText(
          matchingMessage.activity.detail,
          activity.detail,
          previousCreatedAt,
          activity.createdAt,
        );
        if (activity.createdAt >= previousCreatedAt) {
          matchingMessage.activity = {
            ...matchingMessage.activity,
            ...activity,
            detail: matchingMessage.activity.detail,
          };
        }
        mergeFileChanges(matchingMessage.files, files);
        continue;
      }
      if (activity.kind === "file" && files.length) {
        const signature = fileChangesKey(files);
        const duplicateFileEntry = entries.findLast((entry) =>
          entry.activity.kind === "file" &&
          promptSegment(entry.activity.createdAt) === promptSegment(activity.createdAt) &&
          fileChangesKey(entry.files) === signature
        );
        if (duplicateFileEntry) {
          mergeFileChanges(duplicateFileEntry.files, files);
          continue;
        }
      }
      entries.push({
        id: `activity:${activity.id}`,
        activity: {
          ...activity,
          detail: activity.kind === "prompt"
            ? cleanPromptTransport(activity.detail) || undefined
            : activity.detail,
        },
        files,
        sequence: sequence++,
      });
    }
    for (const result of session?.results ?? []) {
      const resultFiles = summarizeFileChanges(
        result.response,
        result.files,
        session?.workingDirectory,
      );
      const responseKey = chatTextKey(result.response);
      const matchingMessage = entries.findLast((entry) =>
        entry.activity.kind === "message" &&
        sameResponseText(entry.activity.detail, responseKey) &&
        promptSegment(entry.activity.createdAt) === promptSegment(result.createdAt)
      );
      if (matchingMessage) {
        matchingMessage.activity.detail = latestResponseText(
          matchingMessage.activity.detail,
          result.response,
          matchingMessage.activity.createdAt,
          result.createdAt,
        );
        if (result.createdAt >= matchingMessage.activity.createdAt) {
          matchingMessage.activity.createdAt = result.createdAt;
          matchingMessage.activity.status = "completed";
        }
        mergeFileChanges(matchingMessage.files, resultFiles);
        matchingMessage.activity.files = Array.from(new Set([
          ...matchingMessage.activity.files,
          ...result.files,
        ]));
      } else if (result.response || resultFiles.length) {
        entries.push({
          id: `result:${result.id}`,
          activity: {
            id: `response:${result.id}`,
            kind: result.response ? "message" : "file",
            title: result.response ? "Resposta do agente" : "Arquivos alterados",
            detail: result.response || undefined,
            status: "completed",
            createdAt: result.createdAt,
            files: result.files,
          },
          files: resultFiles,
          sequence: sequence++,
        });
      }
    }
    if (session?.lastResponse) {
      const responseKey = chatTextKey(session.lastResponse);
      const matchingMessage = entries.find((entry) =>
        entry.activity.kind === "message" &&
        sameResponseText(entry.activity.detail, responseKey)
      );
      if (matchingMessage) {
        matchingMessage.activity.detail = latestResponseText(
          matchingMessage.activity.detail,
          session.lastResponse,
          matchingMessage.activity.createdAt,
          session.updatedAt,
        );
        if (session.updatedAt >= matchingMessage.activity.createdAt) {
          matchingMessage.activity.createdAt = session.updatedAt;
          matchingMessage.activity.status = "completed";
        }
      } else {
        entries.push({
          id: `last-response:${session.id}:${session.updatedAt}`,
          activity: {
            id: `response:${session.id}:${session.updatedAt}`,
            kind: "message",
            title: "Resposta do agente",
            detail: session.lastResponse,
            status: "completed",
            createdAt: session.updatedAt,
            files: [],
          },
          files: [],
          sequence: sequence++,
        });
      }
    }
    return entries.sort((left, right) => {
      return left.activity.createdAt - right.activity.createdAt ||
        left.sequence - right.sequence;
    });
  });
  type ChatFeedItem =
    | { kind: "entry"; id: string; entry: ChatEntry }
    | { kind: "trace"; id: string; entries: ChatEntry[]; files: FileChangeSummary[] };
  const chatFeedItems = $derived.by<ChatFeedItem[]>(() => {
    const feed: ChatFeedItem[] = [];
    let trace: Extract<ChatFeedItem, { kind: "trace" }> | null = null;
    for (const entry of chatEntries) {
      if (isHiddenAgentActivity(entry.activity)) continue;
      if (isPresentableTraceActivity(entry.activity)) {
        const previous = trace?.entries[trace.entries.length - 1];
        if (!trace || (previous && entry.activity.createdAt - previous.activity.createdAt > 180_000)) {
          trace = {
            kind: "trace",
            id: `trace:${entry.id}`,
            entries: [],
            files: [],
          };
          feed.push(trace);
        }
        trace.entries.push(entry);
        mergeFileChanges(trace.files, entry.files);
        continue;
      }
      trace = null;
      feed.push({ kind: "entry", id: entry.id, entry });
    }
    return feed;
  });
  const unloadedActivityCount = $derived(Math.max(
    0,
    (session?.activityTotal ?? activities.length) - activities.length,
  ));
  const locallyHiddenChatItemCount = $derived(Math.max(0, chatFeedItems.length - visibleChatItemLimit));
  const hiddenChatItemCount = $derived(unloadedActivityCount + locallyHiddenChatItemCount);
  const visibleChatFeedItems = $derived(
    locallyHiddenChatItemCount > 0 ? chatFeedItems.slice(-visibleChatItemLimit) : chatFeedItems,
  );
  const activeTraceId = $derived.by(() => {
    if (!session || !["running", "permission_required"].includes(session.status)) return null;
    const latest = chatFeedItems.at(-1);
    return latest?.kind === "trace" ? latest.id : null;
  });
  const authorizationMessageId = $derived.by<string | null>(() => {
    if (!session) return null;
    const visibleEntries = chatEntries.filter((entry) => !isHiddenAgentActivity(entry.activity));
    const latestAgentMessage = visibleEntries.findLast((entry) => entry.activity.kind === "message");
    if (!latestAgentMessage || !needsUserAuthorization(latestAgentMessage.activity.detail)) return null;
    if (session.pendingPermission || session.status === "permission_required") return latestAgentMessage.id;
    return visibleEntries.at(-1)?.id === latestAgentMessage.id ? latestAgentMessage.id : null;
  });

  async function revealEarlierChatItems() {
    const previousHeight = outputElement?.scrollHeight ?? 0;
    const previousTop = outputElement?.scrollTop ?? 0;
    visibleChatItemLimit += 60;
    outputFollowingTail = false;
    chatFollowingTail = false;
    await refresh();
    await tick();
    if (outputElement) {
      outputElement.scrollTop = previousTop + outputElement.scrollHeight - previousHeight;
    }
  }

  function traceHandoffEntry(item: Extract<ChatFeedItem, { kind: "trace" }>): ChatEntry {
    const source = item.entries[item.entries.length - 1];
    return {
      ...source,
      activity: { ...source.activity, detail: undefined },
      files: item.files,
    };
  }

  onMount(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let stopWindowChanges: (() => void) | undefined;
    let stopMoved: (() => void) | undefined;
    let stopResized: (() => void) | undefined;
    let stopPreferences: (() => void) | undefined;
    let stopDockPreview: (() => void) | undefined;
    let stopNativeDragEnded: (() => void) | undefined;
    let stopWorkflowConnectionHover: (() => void) | undefined;
    let sessionRefreshTimer: ReturnType<typeof setTimeout> | undefined;
    let sessionRefreshRunning = false;
    let sessionRefreshQueued = false;
    let sessionRefreshDeferred = false;
    let terminalVisible = document.visibilityState !== "hidden";
    let handleDocumentVisibility: (() => void) | undefined;
    const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    const syncSystemTheme = (event: MediaQueryListEvent | MediaQueryList) => {
      systemDark = event.matches;
    };
    const openMarkdownLink = (event: MouseEvent) => {
      if (!(event.target instanceof Element)) return;
      const anchor = event.target.closest<HTMLAnchorElement>(".markdown-content a");
      if (!anchor) return;
      event.preventDefault();
      void openUrl(anchor.href).catch((error) => {
        message = String(error).replace(/^Error:\s*/, "");
      });
    };
    const closeHeaderPopovers = (event: PointerEvent) => {
      if (!(event.target instanceof Element)) return;
      if (!event.target.closest(".header-overflow")) headerActionsOpen = false;
      if (!event.target.closest(".composer-tools")) composerToolsOpen = false;
      if (!event.target.closest(".workflow-role-control")) {
        workflowDraft = null;
        workflowRolePickerOpen = false;
      }
    };
    const interruptOnEscape = (event: KeyboardEvent) => {
      if (
        event.key !== "Escape"
        || event.repeat
        || event.isComposing
        || !canInterruptRunningPrompt
        || interrupting
      ) return;
      event.preventDefault();
      event.stopPropagation();
      void interruptAgentPrompt();
    };
    try {
      const savedComposerHeight = Number(
        localStorage.getItem(terminalStorageKey("composer-height")),
      );
      const savedTextZoom = Number(localStorage.getItem(terminalStorageKey("text-zoom")));
      if (Number.isFinite(savedComposerHeight) && savedComposerHeight > 0) {
        composerHeight = clampComposerHeight(savedComposerHeight);
      }
      if (Number.isFinite(savedTextZoom) && savedTextZoom > 0) {
        textZoom = Math.round(
          Math.max(textZoomMin, Math.min(textZoomMax, savedTextZoom)) * 10,
        ) / 10;
      }
    } catch {
      // Use the defaults when web storage is unavailable.
    }
    syncSystemTheme(colorScheme);
    colorScheme.addEventListener("change", syncSystemTheme);
    document.addEventListener("click", openMarkdownLink);
    document.addEventListener("pointerdown", closeHeaderPopovers);
    window.addEventListener("keydown", interruptOnEscape);
    void (async () => {
      const [nextPreferences, nextDisplayBackend, nextWorkflowTerminals, nextTerminalVisible] = await Promise.all([
        loadPreferences(),
        loadDisplayBackend(),
        loadTerminalWindows().catch(() => []),
        currentWindow.isVisible().catch(() => true),
      ]);
      language = nextPreferences.language;
      darkMode = nextPreferences.darkMode;
      workflowGroups = nextPreferences.workflowGroups;
      workflowTerminals = nextWorkflowTerminals;
      displayBackend = nextDisplayBackend;
      terminalVisible = nextTerminalVisible;
      fullscreen = await currentWindow.isFullscreen().catch(() => false);
      if (!fullscreen) fullscreen = await terminalGroupFullscreenActive(label).catch(() => false);
      await initializeTerminal();
      if (disposed) return;
      await tick();
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      await markTerminalFrontendReady(label);
      if (disposed) return;
      const flushSessionRefresh = async () => {
        if (sessionRefreshRunning || disposed) return;
        sessionRefreshRunning = true;
        sessionRefreshQueued = false;
        try {
          if (session && windowState) {
            await refresh();
            if (!session && windowState.sessionSource === "cli") {
              await initializeTerminal();
            }
          } else {
            await initializeTerminal();
          }
        } finally {
          sessionRefreshRunning = false;
          if (sessionRefreshQueued && !disposed) queueSessionRefresh();
        }
      };
      const queueSessionRefresh = () => {
        if (!terminalVisible || document.visibilityState === "hidden") {
          sessionRefreshDeferred = true;
          return;
        }
        sessionRefreshQueued = true;
        if (sessionRefreshRunning || sessionRefreshTimer) return;
        const refreshDelay = resizing || dragging
          ? 750
          : session && ["running", "permission_required"].includes(session.status)
            ? 120
            : 650;
        sessionRefreshTimer = setTimeout(() => {
          sessionRefreshTimer = undefined;
          void flushSessionRefresh();
        }, refreshDelay);
      };
      const resumeDeferredRefresh = () => {
        if (!terminalVisible || document.visibilityState === "hidden" || !sessionRefreshDeferred) return;
        sessionRefreshDeferred = false;
        queueSessionRefresh();
      };
      handleDocumentVisibility = () => {
        terminalVisible = document.visibilityState !== "hidden";
        resumeDeferredRefresh();
      };
      document.addEventListener("visibilitychange", handleDocumentVisibility);
      stopListening = await listen<{
        sessionId?: string;
        nativeSessionId?: string;
      }>("lume://sessions-changed", ({ payload }) => {
        const targeted = Boolean(payload?.sessionId || payload?.nativeSessionId);
        const affectsTerminal = !targeted
          || payload.sessionId === session?.id
          || payload.sessionId === windowState?.sessionId
          || payload.nativeSessionId === session?.nativeSessionId
          || payload.nativeSessionId === windowState?.sessionNativeId;
        if (affectsTerminal) queueSessionRefresh();
      });
      stopWindowChanges = await listen("lume://terminal-windows-changed", async () => {
        try {
          const [nextWindowState, nextWorkflowTerminals, nextTerminalVisible] = await Promise.all([
            loadTerminalWindowState(label),
            loadTerminalWindows(),
            currentWindow.isVisible().catch(() => terminalVisible),
          ]);
          windowState = nextWindowState;
          workflowTerminals = nextWorkflowTerminals;
          terminalVisible = nextTerminalVisible;
          fullscreen = await terminalGroupFullscreenActive(label);
          resumeDeferredRefresh();
        } catch {
          // The window may be closing.
        }
      });
      stopPreferences = await listen<Preferences>("lume://preferences-changed", ({ payload }) => {
        language = payload.language;
        darkMode = payload.darkMode;
        workflowGroups = payload.workflowGroups;
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
      stopWorkflowConnectionHover = await listen<{
        connectionLabel: string;
        sourceLabel: string;
        targetLabel: string;
        side: DockSide;
        visible: boolean;
      }>("lume://workflow-connection-hover", ({ payload }) => {
        const side = payload.sourceLabel === label
          ? payload.side
          : payload.targetLabel === label
            ? oppositeDockSide(payload.side)
            : null;
        if (!side) return;
        hoveredWorkflowConnectionSides = payload.visible
          ? Array.from(new Set([...hoveredWorkflowConnectionSides, side]))
          : hoveredWorkflowConnectionSides.filter((entry) => entry !== side);
      });
      stopMoved = await currentWindow.onMoved(({ payload }) => {
        if ((!isWindows && displayBackend !== "native-gnome") || !nativeDragActive) return;
        nativePosition = { x: payload.x, y: payload.y };
        queueNativePositionSync(payload.x, payload.y, false);
        if (nativeDragEndTimer) clearTimeout(nativeDragEndTimer);
        nativeDragEndTimer = setTimeout(() => {
          nativeDragEndTimer = undefined;
          finishNativeWindowDrag();
        }, isWindows ? 180 : 450);
      });
      stopResized = await currentWindow.onResized(() => {
        if (workflowDraft) positionWorkflowRolePopover();
        if (settling) return;
        if (resizeDragState) return;
        composerHeight = clampComposerHeight(composerHeight);
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
      stopMoved?.();
      stopResized?.();
      stopPreferences?.();
      stopDockPreview?.();
      stopNativeDragEnded?.();
      stopWorkflowConnectionHover?.();
      colorScheme.removeEventListener("change", syncSystemTheme);
      document.removeEventListener("click", openMarkdownLink);
      document.removeEventListener("pointerdown", closeHeaderPopovers);
      if (handleDocumentVisibility) document.removeEventListener("visibilitychange", handleDocumentVisibility);
      window.removeEventListener("keydown", interruptOnEscape);
      if (resizeEndTimer) clearTimeout(resizeEndTimer);
      if (resizeFrame !== null) cancelAnimationFrame(resizeFrame);
      if (resizeThrottleTimer) clearTimeout(resizeThrottleTimer);
      if (nativeDragEndTimer) clearTimeout(nativeDragEndTimer);
      if (collaborationModeNoticeTimer) clearTimeout(collaborationModeNoticeTimer);
      if (workflowConnectionLeaveTimer) clearTimeout(workflowConnectionLeaveTimer);
      if (workClockTimer) clearTimeout(workClockTimer);
      if (sessionRefreshTimer) clearTimeout(sessionRefreshTimer);
    };
  });

  async function initializeTerminal() {
    const run = ++initializationRun;
    initializationError = null;
    let lastError = tr(
      "The session is no longer available.",
      "A sessão não está mais disponível.",
    );
    for (let attempt = 0; attempt < 8; attempt += 1) {
      try {
        windowState = await loadTerminalWindowState(label);
        await refresh();
        if (session) {
          if (session.agent === "codex") {
            try {
              collaborationMode = await getSessionCollaborationMode(session.id);
            } catch {
              collaborationMode = "default";
            }
            if (!rateLimitRefreshRequested) {
              rateLimitRefreshRequested = true;
              void refreshAgentRateLimits(session.agent)
                .then(() => refresh())
                .catch(() => undefined);
            }
          }
          return;
        }
      } catch (error) {
        lastError = String(error).replace(/^Error:\s*/, "");
      }
      await new Promise((resolve) => setTimeout(resolve, 120 + attempt * 80));
      if (run !== initializationRun) return;
    }
    if (run !== initializationRun) return;
    if (windowState?.sessionSource === "cli") {
      await closeTerminal();
      return;
    }
    initializationError = lastError;
  }

  async function refresh() {
    const snapshot = await loadTerminalHubSnapshot(label, visibleChatItemLimit);
    const shouldFollow = activeTab === "chat" && outputFollowingTail;
    const nextSession = windowState ? resolveTerminalSession(windowState, snapshot.sessions) ?? null : null;
    const notesSourceChanged = session?.id !== nextSession?.id;
    const previousActivity = session?.activities.at(-1);
    const nextActivity = nextSession?.activities.at(-1);
    const previousRateLimits = session?.rateLimits ?? [];
    const nextRateLimits = nextSession?.rateLimits ?? [];
    const rateLimitsChanged = previousRateLimits.length !== nextRateLimits.length
      || previousRateLimits.some((limit, index) => {
        const next = nextRateLimits[index];
        return !next
          || limit.id !== next.id
          || limit.usedPercent !== next.usedPercent
          || limit.resetsAt !== next.resetsAt
          || limit.windowMinutes !== next.windowMinutes;
      });
    const sessionChanged = !session || !nextSession
      ? session !== nextSession
      : session.id !== nextSession.id
        || session.updatedAt !== nextSession.updatedAt
        || session.status !== nextSession.status
        || session.activities.length !== nextSession.activities.length
        || session.results.length !== nextSession.results.length
        || session.pendingPermission?.id !== nextSession.pendingPermission?.id
        || previousActivity?.id !== nextActivity?.id
        || previousActivity?.status !== nextActivity?.status
        || previousActivity?.detail?.length !== nextActivity?.detail?.length
        || rateLimitsChanged;
    if (sessionChanged) session = nextSession;
    if (notesSourceChanged) {
      sessionNotes = [];
      notesSessionId = null;
      noteEditor = null;
      if (activeTab === "notes" && session) void refreshSessionNotes();
    }
    if (sessionChanged && shouldFollow && outputFollowingTail) {
      await tick();
      if (outputFollowingTail) {
        outputElement?.scrollTo({ top: outputElement.scrollHeight });
      }
    }
  }

  function outputDistanceFromTail() {
    if (!outputElement) return 0;
    return outputElement.scrollHeight - outputElement.scrollTop - outputElement.clientHeight;
  }

  function handleOutputScroll() {
    if (activeTab !== "chat") return;
    outputFollowingTail = outputDistanceFromTail() <= 24;
    chatFollowingTail = outputFollowingTail;
  }

  function handleOutputWheel(event: WheelEvent) {
    if (activeTab === "chat" && event.deltaY < 0) {
      outputFollowingTail = false;
      chatFollowingTail = false;
    }
  }

  async function selectTab(nextTab: TerminalTab) {
    if (nextTab === activeTab) return;
    if (outputElement) {
      tabScrollPositions[activeTab] = outputElement.scrollTop;
    }
    if (activeTab === "chat") {
      chatFollowingTail = outputFollowingTail;
    }
    activeTab = nextTab;
    if (nextTab === "notes") await refreshSessionNotes();
    await tick();
    if (!outputElement) return;
    if (nextTab === "chat" && chatFollowingTail) {
      outputElement.scrollTop = outputElement.scrollHeight;
    } else {
      outputElement.scrollTop = tabScrollPositions[nextTab] ?? 0;
    }
    if (nextTab === "chat") {
      outputFollowingTail = outputDistanceFromTail() <= 24;
      chatFollowingTail = outputFollowingTail;
    }
  }

  async function refreshSessionNotes(force = false) {
    if (!session || notesLoading) return;
    if (!force && notesSessionId === session.id) return;
    notesLoading = true;
    try {
      sessionNotes = await loadSessionNotes(session.id);
      notesSessionId = session.id;
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      notesLoading = false;
    }
  }

  function currentPlanBody() {
    if (!plan) return "";
    if (plan.content) return plan.content;
    return [
      plan.explanation?.trim(),
      plan.items.map((item) => `- [${item.status === "completed" ? "x" : " "}] ${item.label}`).join("\n"),
    ].filter(Boolean).join("\n\n");
  }

  function createNoteFromCurrentPlan() {
    if (!plan) return;
    noteEditor = {
      title: tr("Saved plan", "Planejamento salvo"),
      body: currentPlanBody(),
      kind: "plan",
      pinned: false,
    };
  }

  function createBlankNote() {
    noteEditor = { title: "", body: "", kind: "note", pinned: false };
  }

  function editSessionNote(note: SessionNote) {
    noteEditor = {
      id: note.id,
      title: note.title,
      body: note.body,
      kind: note.kind,
      pinned: note.pinned,
    };
  }

  async function persistSessionNote() {
    if (!session || !noteEditor?.body.trim() || noteSaving) return;
    noteSaving = true;
    try {
      await saveSessionNote(session.id, noteEditor);
      noteEditor = null;
      notesSessionId = null;
      await refreshSessionNotes(true);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      noteSaving = false;
    }
  }

  async function toggleSessionNotePin(note: SessionNote) {
    if (!session) return;
    try {
      await saveSessionNote(session.id, { ...note, pinned: !note.pinned });
      notesSessionId = null;
      await refreshSessionNotes(true);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function removeSessionNote(note: SessionNote) {
    try {
      await deleteSessionNote(note.id);
      sessionNotes = sessionNotes.filter((candidate) => candidate.id !== note.id);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function useSessionNote(note: SessionNote) {
    prompt = `${tr("Continue from this saved context:", "Continue a partir deste contexto salvo:")}\n\n${note.body}`;
    await selectTab("chat");
    await tick();
    promptInput?.focus();
  }

  function activityTime(createdAt: number) {
    return new Intl.DateTimeFormat(language, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }).format(new Date(createdAt));
  }

  function noteDate(updatedAt: number) {
    return new Intl.DateTimeFormat(language, {
      dateStyle: "short",
      timeStyle: "short",
    }).format(new Date(updatedAt));
  }

  function handoffTargetCanReceive(target: HandoffTarget): boolean {
    if (!target.session.capabilities.canPrompt) return false;
    return target.session.status !== "running"
      || target.session.capabilities.promptDeliveries.includes("queue");
  }

  async function openHandoff(entry: ChatEntry) {
    if (!session || !windowState?.groupId) {
      message = tr(
        "Dock this terminal to another one before sharing context.",
        "Acople este terminal a outro antes de compartilhar contexto.",
      );
      return;
    }
    handoffLoading = true;
    handoffError = null;
    try {
      const [terminals, snapshot] = await Promise.all([
        loadTerminalWindows(),
        loadHubSnapshot(),
      ]);
      handoffTargets = terminals.flatMap((terminal) => {
        if (
          terminal.label === label
          || terminal.groupId !== windowState?.groupId
        ) return [];
        const targetSession = resolveTerminalSession(terminal, snapshot.sessions);
        return targetSession ? [{ terminal, session: targetSession }] : [];
      });
      if (handoffTargets.length === 0) {
        message = tr(
          "No other active terminal is connected to this docked group.",
          "Nenhum outro terminal ativo está conectado a este grupo.",
        );
        return;
      }
      handoffDraft = {
        text: entry.activity.detail?.trim() ?? "",
        files: entry.files,
        includeText: Boolean(entry.activity.detail?.trim()),
        includeFiles: entry.files.length > 0,
        note: "",
        targetSessionId:
          handoffTargets.find(handoffTargetCanReceive)?.session.id
          ?? handoffTargets[0].session.id,
      };
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      handoffLoading = false;
    }
  }

  function handoffPreview(draft: HandoffDraft): string {
    return buildHandoffBody(draft);
  }

  async function sendHandoff() {
    if (!session || !handoffDraft || handoffSending) return;
    const target = handoffTargets.find(
      (candidate) => candidate.session.id === handoffDraft?.targetSessionId,
    );
    const body = handoffPreview(handoffDraft);
    if (!target || !handoffTargetCanReceive(target) || !body) return;
    handoffSending = true;
    handoffError = null;
    try {
      const sourceName = sessionDisplayName(session);
      const prompt = buildHandoffPrompt(session.agentLabel, sourceName, body);
      const delivery = target.session.status === "running" ? "queue" : "new_turn";
      await submitPrompt(target.session.id, prompt, [], delivery);
      handoffDraft = null;
      message = tr(
        `Context sent to ${sessionDisplayName(target.session)}.`,
        `Contexto enviado para ${sessionDisplayName(target.session)}.`,
      );
    } catch (error) {
      handoffError = String(error).replace(/^Error:\s*/, "");
    } finally {
      handoffSending = false;
    }
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
    if (fullscreen) return;
    if ((event.target as HTMLElement).closest("button, input, textarea, form")) return;
    if (windowState.workflowBridgeOpen) {
      message = tr(
        "Close the workflow connection before moving these terminals.",
        "Feche a conexão do workflow antes de mover estes terminais.",
      );
      return;
    }
    if (displayBackend === "gnome-wayland-limited") {
      message = "Window dragging requires XWayland in GNOME.";
      return;
    }
    if (isWindows || displayBackend === "native-gnome") {
      event.preventDefault();
      dragging = true;
      nativeDragActive = true;
      nativePosition = null;
      pendingNativeSync = null;
      if (nativeDragEndTimer) {
        clearTimeout(nativeDragEndTimer);
        nativeDragEndTimer = undefined;
      }
      dockMovingLabel = null;
      dockPreview = null;
      void currentWindow
        .startDragging()
        .catch((error) => {
          nativeDragActive = false;
          dragging = false;
          message = String(error).replace(/^Error:\s*/, "");
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
    if (nativeDragActive) {
      if (isWindows || displayBackend === "native-gnome") finishNativeWindowDrag();
      return;
    }
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

  function queueNativePositionSync(x: number, y: number, finalize: boolean) {
    pendingNativeSync = { x, y, finalize };
    if (nativeSyncRunning) return;
    void flushNativePositionSync();
  }

  async function flushNativePositionSync() {
    nativeSyncRunning = true;
    try {
      while (pendingNativeSync) {
        const next = pendingNativeSync;
        pendingNativeSync = null;
        windowState = await syncTerminalWindowPosition(
          label,
          next.x,
          next.y,
          next.finalize,
        );
      }
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      nativeSyncRunning = false;
      if (pendingNativeSync) void flushNativePositionSync();
    }
  }

  function finishNativeWindowDrag() {
    if (!nativeDragActive) return;
    nativeDragActive = false;
    dragging = false;
    if (nativeDragEndTimer) {
      clearTimeout(nativeDragEndTimer);
      nativeDragEndTimer = undefined;
    }
    if (nativePosition) {
      queueNativePositionSync(nativePosition.x, nativePosition.y, true);
    } else {
      dockMovingLabel = null;
      dockPreview = null;
    }
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
    if (windowState?.docked && await terminalGroupFullscreenActive(label)) {
      await toggleTerminalGroupFullscreen(label);
      fullscreen = false;
    }
    windowState = await undockTerminalWindow(label);
  }

  async function toggleFullscreen() {
    try {
      if (
        windowState?.docked ||
        windowState?.layered ||
        displayBackend === "xwayland-fallback"
      ) {
        const groupFullscreen = await toggleTerminalGroupFullscreen(label);
        if (groupFullscreen !== null) {
          fullscreen = groupFullscreen;
          headerActionsOpen = false;
          return;
        }
      }
      const next = !await currentWindow.isFullscreen();
      await currentWindow.setFullscreen(next);
      fullscreen = next;
      headerActionsOpen = false;
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  async function beginResize(event: PointerEvent, direction: ResizeDirection) {
    const state = windowState;
    if (event.button !== 0 || !state) return;
    if (fullscreen) return;
    if (state.workflowBridgeOpen) {
      message = tr(
        "Close the workflow connection before resizing these terminals.",
        "Feche a conexão do workflow antes de redimensionar estes terminais.",
      );
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    dragging = false;
    finalizeRequested = false;
    pendingMove = null;
    dockPreview = null;
    dockMovingLabel = null;
    resizing = true;
    if (
      state.layered
      || displayBackend === "xwayland-fallback"
      || (state.docked && displayBackend !== "native-gnome")
    ) {
      const target = event.currentTarget as HTMLElement;
      target.setPointerCapture(event.pointerId);
      resizeDragState = {
        pointerId: event.pointerId,
        direction,
        startX: event.screenX,
        startY: event.screenY,
        originX: state.x,
        originY: state.y,
        originWidth: state.width,
        originHeight: state.height,
        scale: state.scale,
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
          if (pendingResize) scheduleResizeFlush();
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
    const width = Math.max(300, Math.round(desiredWidth));
    const height = Math.max(240, Math.round(desiredHeight));
    pendingResize = {
      x: east
        ? resizeDragState.originX
        : resizeDragState.originX + Math.round((resizeDragState.originWidth - width) * scale),
      y: south
        ? resizeDragState.originY
        : resizeDragState.originY + Math.round((resizeDragState.originHeight - height) * scale),
      width,
      height,
      fromLeft: !east,
      fromTop: !south,
    };
    scheduleResizeFlush();
  }

  function scheduleResizeFlush() {
    if (
      resizeFrame !== null
      || resizeThrottleTimer
      || resizeSyncRunning
      || resizePreparing
      || !pendingResize
    ) return;
    const remaining = 32 - (performance.now() - lastResizeFlushAt);
    if (remaining > 1) {
      resizeThrottleTimer = setTimeout(() => {
        resizeThrottleTimer = undefined;
        scheduleResizeFlush();
      }, remaining);
      return;
    }
    resizeFrame = requestAnimationFrame(() => {
      resizeFrame = null;
      void flushResize();
    });
  }

  async function flushResize(final = false) {
    if (resizeSyncRunning || resizePreparing) return;
    const next = pendingResize;
    if (!next) return;
    pendingResize = null;
    resizeSyncRunning = true;
    lastResizeFlushAt = performance.now();
    try {
      await resizeTerminalWindow(
        label,
        next.x,
        next.y,
        next.width,
        next.height,
        next.fromLeft,
        next.fromTop,
      );
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
      pendingResize = null;
    } finally {
      resizeSyncRunning = false;
      if (!final && resizeDragState && pendingResize) scheduleResizeFlush();
    }
  }

  async function endResize(event: PointerEvent) {
    if (!resizeDragState || resizeDragState.pointerId !== event.pointerId) return;
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    resizeDragState = null;
    if (resizePreparing) await resizePreparing;
    if (resizeFrame !== null) {
      cancelAnimationFrame(resizeFrame);
      resizeFrame = null;
    }
    if (resizeThrottleTimer) {
      clearTimeout(resizeThrottleTimer);
      resizeThrottleTimer = undefined;
    }
    while (resizeSyncRunning) {
      await new Promise((resolve) => setTimeout(resolve, 4));
    }
    if (resizeFrame !== null) {
      cancelAnimationFrame(resizeFrame);
      resizeFrame = null;
    }
    if (pendingResize) await flushResize(true);
    try {
      windowState = await finishLayeredTerminalResize(label);
      composerHeight = clampComposerHeight(composerHeight);
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

  async function minimizeTerminal() {
    message = null;
    try {
      await minimizeTerminalWindow(label);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  function beginSessionRename() {
    if (!session) return;
    renameDraft = sessionDisplayName(session);
    renamingSession = true;
    message = null;
  }

  async function saveSessionRename() {
    if (!session || savingSessionName) return;
    const requested = renameDraft.trim();
    if (!requested) {
      message = tr("Enter a name for this session.", "Digite um nome para esta sessão.");
      return;
    }
    savingSessionName = true;
    message = null;
    try {
      const sessionName = await renameSession(session.id, requested);
      session = { ...session, sessionName };
      renamingSession = false;
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      savingSessionName = false;
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

  async function interruptAgentPrompt() {
    if (!session || !capabilities?.canInterrupt || interrupting) return;
    interrupting = true;
    message = null;
    try {
      await interruptPrompt(session.id);
      message = tr("Prompt interrupted.", "Prompt interrompido.");
      await refresh();
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      interrupting = false;
    }
  }

  async function applyCollaborationMode(nextMode: CollaborationMode) {
    if (
      !session
      || session.agent !== "codex"
      || promptIsRunning
      || collaborationModeChanging
    ) return false;
    collaborationModeChanging = true;
    collaborationModeTarget = nextMode;
    collaborationModeNotice = null;
    if (collaborationModeNoticeTimer) clearTimeout(collaborationModeNoticeTimer);
    message = null;
    try {
      collaborationMode = await setSessionCollaborationMode(session.id, nextMode);
      collaborationModeNotice = collaborationMode === "plan"
        ? tr("Plan mode enabled", "Modo Plan ativado")
        : tr("Default mode enabled", "Modo Default ativado");
      collaborationModeNoticeTimer = setTimeout(() => {
        collaborationModeNotice = null;
        collaborationModeNoticeTimer = undefined;
      }, 2_400);
      return true;
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
      return false;
    } finally {
      collaborationModeChanging = false;
      collaborationModeTarget = null;
    }
  }

  async function toggleCollaborationMode() {
    const nextMode: CollaborationMode = collaborationMode === "plan" ? "default" : "plan";
    await applyCollaborationMode(nextMode);
  }

  function currentModelOption() {
    return modelSettings?.models.find((option) => option.model === selectedModel) ?? null;
  }

  function effortValues() {
    if (session?.agent === "claude_code") {
      return ["", "low", "medium", "high", "xhigh", "max"];
    }
    return currentModelOption()?.supportedReasoningEfforts.map((effort) => effort.value) ?? [];
  }

  function currentEffort() {
    return session?.agent === "claude_code" ? claudeEffort : selectedEffort;
  }

  function currentEffortIndex() {
    return Math.max(0, effortValues().indexOf(currentEffort()));
  }

  function chooseEffortIndex(event: Event) {
    const values = effortValues();
    const index = Number((event.currentTarget as HTMLInputElement).value);
    const effort = values[Math.max(0, Math.min(values.length - 1, index))] ?? "";
    if (session?.agent === "claude_code") claudeEffort = effort;
    else selectedEffort = effort;
  }

  function effortLabel(value = currentEffort()) {
    return value || tr("Default", "Padrão");
  }

  function chooseModel(model: string) {
    if (!modelSettings || modelSaving) return;
    selectedModel = model;
    const option = modelSettings.models.find((candidate) => candidate.model === model);
    if (!option) return;
    if (!option.supportedReasoningEfforts.some((effort) => effort.value === selectedEffort)) {
      selectedEffort = option.defaultReasoningEffort;
    }
  }

  async function openModelDialog() {
    if (!session || !["codex", "claude_code"].includes(session.agent)) return false;
    composerToolsOpen = false;
    if (session.controlOrigin !== "lume") {
      message = tr(
        "Take control of this external CLI before changing its model.",
        "Assuma o controle desta CLI externa antes de mudar o modelo.",
      );
      return true;
    }
    if (promptIsRunning) {
      message = tr(
        "The model can be changed after the current task finishes.",
        "O modelo pode ser alterado depois que a tarefa atual terminar.",
      );
      return true;
    }
    modelDialogOpen = true;
    modelLoading = true;
    modelError = null;
    modelSettings = null;
    try {
      if (session.agent === "codex") {
        modelSettings = await getSessionModelSettings(session.id);
        selectedModel = modelSettings.model;
        const option = currentModelOption();
        selectedEffort = modelSettings.reasoningEffort
          ?? option?.defaultReasoningEffort
          ?? option?.supportedReasoningEfforts[0]?.value
          ?? "";
      } else {
        const settings = await getClaudeSessionModelSettings(session.id);
        claudeModel = settings.model ?? "";
        claudeEffort = settings.reasoningEffort ?? "";
      }
    } catch (error) {
      modelError = String(error).replace(/^Error:\s*/, "");
    } finally {
      modelLoading = false;
    }
    return true;
  }

  async function saveModelSettings() {
    if (!session || modelSaving) return;
    if (session.agent === "codex" && (!selectedModel || !selectedEffort)) return;
    modelSaving = true;
    modelError = null;
    try {
      if (session.agent === "codex") {
        modelSettings = await setSessionModelSettings(
          session.id,
          selectedModel,
          selectedEffort,
        );
      } else if (session.agent === "claude_code") {
        const settings = await setClaudeSessionModelSettings(
          session.id,
          claudeModel.trim() || undefined,
          claudeEffort || undefined,
        );
        claudeModel = settings.model ?? "";
        claudeEffort = settings.reasoningEffort ?? "";
      } else {
        return;
      }
      modelDialogOpen = false;
      message = tr(
        "Model settings will apply to the next prompt.",
        "As configurações de modelo serão aplicadas ao próximo prompt.",
      );
    } catch (error) {
      modelError = String(error).replace(/^Error:\s*/, "");
    } finally {
      modelSaving = false;
    }
  }

  async function runSlashCommand(value: string) {
    const command = value.trim().toLowerCase();
    const selected = availableSlashCommands().find((item) => `/${item.name}` === command);
    if (!selected) {
      return false;
    }
    const action = selected.action
      ?? (session?.agent === "codex" && selected.name === "plan" ? "plan" : undefined)
      ?? (selected.name === "rename" ? "rename" : undefined);
    if (!action) return false;
    let handled = true;
    switch (action) {
      case "model":
        await openModelDialog();
        break;
      case "plan":
        await applyCollaborationMode("plan");
        break;
      case "default":
        await applyCollaborationMode("default");
        break;
      case "interrupt":
        await interruptAgentPrompt();
        break;
      case "steer":
        await steerNextQueuedPrompt();
        break;
      case "rename":
        beginSessionRename();
        break;
      case "detach":
        await detach();
        break;
      case "fullscreen":
        await toggleFullscreen();
        break;
      case "zoom-in":
        setTextZoom(textZoom + 0.1);
        break;
      case "zoom-out":
        setTextZoom(textZoom - 0.1);
        break;
      case "close":
        await closeTerminal();
        break;
      default:
        handled = false;
    }
    if (handled) {
      prompt = "";
      slashMenuDismissed = false;
    }
    return handled;
  }

  async function sendPrompt() {
    if (
      !session
      || (!prompt.trim() && promptAttachments.length === 0)
      || sending
      || !canCompose
      || !readyForPrompt
    ) return;
    if (!canSubmit && capabilities?.canTakeControl) {
      takeoverConfirm = true;
      return;
    }
    if (promptAttachments.length === 0 && await runSlashCommand(prompt)) return;
    sending = true;
    message = null;
    try {
      const delivery = session.status === "running" ? "queue" : "new_turn";
      await submitPrompt(session.id, prompt.trim(), promptAttachments, delivery);
      prompt = "";
      promptAttachments = [];
      session = {
        ...session,
        status: "running",
        statusLabel:
          delivery === "queue"
            ? tr("Prompt queued", "Prompt na fila")
            : "Prompt sent by Lume",
        lastResponse: delivery === "new_turn" ? undefined : session.lastResponse,
      };
      await refresh();
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      sending = false;
    }
  }

  async function confirmTakeover() {
    if (!session || !capabilities?.canTakeControl || takingControl) return;
    takingControl = true;
    message = null;
    try {
      await takeControlSession(session.id);
      takeoverConfirm = false;
      await refresh();
      await tick();
      await sendPrompt();
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
      await refresh().catch(() => undefined);
    } finally {
      takingControl = false;
    }
  }

  async function steerNextQueuedPrompt() {
    if (!session || !nextQueuedPrompt || !canSendWhileRunning || steeringQueued) return;
    steeringQueued = true;
    message = null;
    try {
      await steerQueuedPrompt(session.id, nextQueuedPrompt.id);
      message = tr("Queued prompt steered into the current task.", "Prompt da fila enviado para a tarefa atual.");
      await refresh();
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
      await refresh().catch(() => undefined);
    } finally {
      steeringQueued = false;
    }
  }

  async function chooseAttachments() {
    if (!canCompose || !readyForPrompt || sending) return;
    composerToolsOpen = false;
    message = null;
    try {
      let terminalLowered = false;
      let selected: string | string[] | null = null;
      try {
        try {
          await setTerminalFileDialogActive(label, true);
          terminalLowered = true;
        } catch {
          // The native picker can still open on window managers without topmost control.
        }
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
        selected = await openDialog({
          multiple: true,
          directory: false,
        });
      } finally {
        if (terminalLowered) {
          await setTerminalFileDialogActive(label, false).catch(() => undefined);
        }
      }
      const paths = (Array.isArray(selected) ? selected : selected ? [selected] : [])
        .filter((path): path is string => typeof path === "string");
      for (const path of paths.slice(0, 4 - promptAttachments.length)) {
        const previewDataUrl = isImageAttachmentPath(path) ? await imagePreview(path) : "";
        promptAttachments = [
          ...promptAttachments,
          {
            name: path.split(/[\\/]/).pop() || "file",
            mimeType: "",
            path,
            previewDataUrl,
          },
        ];
      }
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  function removeAttachment(index: number) {
    promptAttachments = promptAttachments.filter((_, current) => current !== index);
  }

  async function imagePreview(path: string): Promise<string> {
    const source = await readLocalImageDataUrl(path);
    return createImagePreview(source, language);
  }

  async function pasteAttachments(event: ClipboardEvent) {
    if (!clipboardHasFile(event) && !clipboardHasImage(event) && !clipboardMayContainImage(event)) return;
    event.preventDefault();
    message = null;
    if (!canSubmit || !readyForPrompt || sending || !capabilities?.canAttachImages) {
      message = tr(
        "Files can only be attached when this session is ready for a prompt.",
        "Arquivos só podem ser anexados quando esta sessão estiver pronta para um prompt.",
      );
      return;
    }
    try {
      let { files, paths } = collectClipboardFiles(event);
      if (!files.length && !paths.length) {
        ({ files, paths } = await collectClipboardImages(event, language));
      }
      const available = 4 - promptAttachments.length;
      const prepared: PromptAttachmentInput[] = [];
      for (const [index, file] of files.slice(0, available).entries()) {
        prepared.push(
          isImageAttachmentFile(file)
            ? await prepareClipboardImage(file, index, language)
            : await prepareClipboardFile(file, index, language),
        );
      }
      for (const path of paths.slice(0, available - prepared.length)) {
        prepared.push({
          name: path.split(/[\\/]/).pop() || "file",
          mimeType: "",
          path,
          previewDataUrl: isImageAttachmentPath(path) ? await imagePreview(path) : "",
        });
      }
      promptAttachments = [...promptAttachments, ...prepared];
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    }
  }

  function sendPromptOnEnter(event: KeyboardEvent) {
    const slashCommands = filteredSlashCommands();
    if (slashCommands.length) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const direction = event.key === "ArrowDown" ? 1 : -1;
        slashCommandIndex =
          (slashCommandIndex + direction + slashCommands.length) % slashCommands.length;
        void revealSelectedSlashCommand();
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        slashMenuDismissed = true;
        return;
      }
      if (
        (event.key === "Enter" && !event.shiftKey)
        || (event.key === "Tab" && !event.shiftKey)
      ) {
        event.preventDefault();
        void selectSlashCommand(
          slashCommands[Math.min(slashCommandIndex, slashCommands.length - 1)],
        );
        return;
      }
    }
    if (
      event.key === "Tab"
      && !event.shiftKey
      && !event.isComposing
      && nextQueuedPrompt
      && canSendWhileRunning
    ) {
      event.preventDefault();
      void steerNextQueuedPrompt();
      return;
    }
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
    event.preventDefault();
    void sendPrompt();
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

  async function selectQuestionOption(questionId: string, value: string) {
    if (!session?.pendingQuestion) return;
    questionSelections = { ...questionSelections, [questionId]: value };
    if (session.pendingQuestion.questions.length === 1) {
      await submitQuestionAnswers([{ questionId, answers: [value] }]);
    }
  }

  async function submitSelectedQuestionAnswers() {
    if (!session?.pendingQuestion) return;
    const answers = session.pendingQuestion.questions
      .map((question) => ({
        questionId: question.id,
        answers: questionSelections[question.id]
          ? [questionSelections[question.id]]
          : [],
      }))
      .filter((answer) => answer.answers.length > 0);
    if (answers.length !== session.pendingQuestion.questions.length) {
      message = tr("Choose one option for each question.", "Escolha uma opção para cada pergunta.");
      return;
    }
    await submitQuestionAnswers(answers);
  }

  async function submitQuestionAnswers(answers: QuestionAnswer[]) {
    if (!session?.pendingQuestion || sending) return;
    sending = true;
    message = null;
    try {
      await answerInteractiveQuestion(session.id, session.pendingQuestion.id, answers);
      questionSelections = {};
      await refresh();
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      sending = false;
    }
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

<main class:dark={effectiveDark} class="terminal-window" onpointerdown={() => void currentWindow.setFocus().catch(() => undefined)}>
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
      class:workflow-mode={windowState?.workflowEnabled}
      class:workflow-preview={windowState?.workflowEnabled && Boolean(dockPreview)}
      class:normal-preview={!windowState?.workflowEnabled && Boolean(dockPreview)}
      class:bridge-locked={windowState?.workflowBridgeOpen}
      class:header-menu-open={headerActionsOpen}
      class:dock-ready={Boolean(dockPreview) && (dockPreview?.proximity ?? 0) >= 0.78}
      class:joined-left={windowState?.connectedSides.includes("left") || windowState?.bridgeSides?.includes("left")}
      class:joined-right={windowState?.connectedSides.includes("right") || windowState?.bridgeSides?.includes("right")}
      class:joined-top={windowState?.connectedSides.includes("top") || windowState?.bridgeSides?.includes("top")}
      class:joined-bottom={windowState?.connectedSides.includes("bottom") || windowState?.bridgeSides?.includes("bottom")}
      class="terminal-card"
      style:--chat-font-adjust={`${(textZoom - 1) * 9}px`}
      style:--chat-small-font-adjust={`${(textZoom - 1) * 8}px`}
      style:--chat-tiny-font-adjust={`${(textZoom - 1) * 7}px`}
      style:--dock-pull={`${(dockPreview?.proximity ?? 0) * 14}px`}
      style:--dock-squeeze={`${(dockPreview?.proximity ?? 0) * 0.018}`}
      style:--dock-proximity={`${dockPreview?.proximity ?? 0}`}
      style:--dock-tension={`${1 - Math.pow(1 - (dockPreview?.proximity ?? 0), 2.4)}`}
      style:--dock-glow={`${0.12 + (dockPreview?.proximity ?? 0) * 0.48}`}
    >
      {#if dockPreview}
        <div class="workflow-merge-preview" aria-hidden="true"></div>
      {/if}
      {#if windowState?.workflowEnabled}
        {#each windowState.connectedSides as side}
          <span class="workflow-connection workflow-{side}" aria-hidden="true"></span>
          <span
            class:visible={hoveredWorkflowConnectionSides.includes(side)}
            class="workflow-connection-control workflow-control-{side}"
            role="group"
            aria-label={tr("Workflow connection", "Conexão do workflow")}
            onpointerenter={() => enterWorkflowConnection(side)}
            onpointerleave={() => leaveWorkflowConnection(side)}
          >
            <button
              class:loading={workflowBridgeOpeningSide === side}
              class="workflow-connection-editor workflow-editor-{side}"
              type="button"
              aria-busy={workflowBridgeOpeningSide === side}
              aria-disabled={workflowBridgeOpeningSide !== null}
              aria-label={tr("Edit workflow connection", "Editar conexão do workflow")}
              title={tr("Edit workflow", "Editar workflow")}
              onpointerdown={(event) => event.stopPropagation()}
              onfocus={() => enterWorkflowConnection(side)}
              onblur={() => leaveWorkflowConnection(side)}
              onclick={(event) => void openWorkflowBridgeFromConnection(event, side)}
            >
              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M7.2 12.8 5.5 14.5a3 3 0 0 1-4.2-4.2L4 7.6a3 3 0 0 1 4.2 0" /><path d="m12.8 7.2 1.7-1.7a3 3 0 0 1 4.2 4.2L16 12.4a3 3 0 0 1-4.2 0" /><path d="m7 13 6-6" /></svg>
            </button>
          </span>
        {/each}
        {#each windowState.bridgeSides ?? [] as side}
          <span class="workflow-connection workflow-{side} workflow-bridge-link" aria-hidden="true"></span>
        {/each}
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
        {#if renamingSession}
          <form class="terminal-name-editor" onsubmit={(event) => { event.preventDefault(); void saveSessionRename(); }}>
            <input maxlength="80" bind:value={renameDraft} aria-label={tr("Session name", "Nome da sessão")} />
            <button disabled={savingSessionName} type="submit" aria-label={tr("Save session name", "Salvar nome da sessão")}>
              <svg viewBox="0 0 20 20"><path d="m5 10 3 3 7-7"></path></svg>
            </button>
            <button disabled={savingSessionName} type="button" aria-label={tr("Cancel rename", "Cancelar renomeação")} onclick={() => (renamingSession = false)}>
              <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8"></path></svg>
            </button>
          </form>
        {:else}
          <div class="identity">
            <strong>{sessionDisplayName(session)}</strong>
            <small title={session.workingDirectory}>{sessionDirectoryName(session)}</small>
          </div>
        {/if}
        <span class="source-badge" title={sourceLabel(session)}>
          <BrandIcon name={sourceIcon(session)} size={10} />
          <span class="badge-label">{sourceLabel(session)}</span>
        </span>
        {#if session.permissionProfile.approvalsReviewer === "auto_review" && session.permissionProfile.mode !== "full_access"}
          <span class="access-badge auto-review" title={tr("Automatic approval", "Aprovação automática")}>
            <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M6.8.8 2.9 6.3h2.5L4.9 11l4.2-5.7H6.5Z" /></svg>
            <span class="badge-label">{tr("Auto", "Auto")}</span>
          </span>
        {/if}
        {#if session.permissionProfile.mode === "full_access"}
          <span class="access-badge full-access" title={tr("Full access", "Acesso total")}>
            <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 5V3.7a3 3 0 0 1 5.6-1.5M2.2 5.2h7.6v5.5H2.2Z" /></svg>
            <span class="badge-label">{tr("Full access", "Acesso total")}</span>
          </span>
        {/if}
        {#if session.controlOrigin === "external"}
          <span class="access-badge external-session" title={tr("External session", "Sessão externa")}>
            <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M4.2 2.5H2.3v7.2h7.2V7.8M6.3 2.3h3.4v3.4M9.7 2.3 5.4 6.6" /></svg>
            <span class="badge-label">{tr("External", "Externa")}</span>
          </span>
        {/if}
        {#if activeRateLimit}
          <div
            class:warning={rateLimitRemaining <= 50 && rateLimitRemaining > 20}
            class:danger={rateLimitRemaining <= 20}
            class="rate-limit-meter"
            role="img"
            aria-label={rateLimitTitle()}
            title={rateLimitTitle()}
          >
            <span><b>{rateLimitRemaining}%</b><small>{rateLimitWindow(activeRateLimit.windowMinutes)}</small></span>
            <i><em style={`width: ${rateLimitRemaining}%`}></em></i>
          </div>
        {/if}
        <span class="header-actions">
          {#if windowState?.docked}
            <button class="dock-button" type="button" onclick={detach} aria-label={tr("Undock terminal", "Desacoplar terminal")} title={tr("Undock", "Desacoplar")}>
              <svg viewBox="0 0 20 20"><path d="M7 6 5.5 7.5a3 3 0 0 0 4.2 4.2l1.2-1.2M13 14l1.5-1.5a3 3 0 0 0-4.2-4.2L9.1 9.5" /></svg>
            </button>
          {/if}
          <button type="button" onclick={minimizeTerminal} aria-label={tr("Minimize terminal", "Minimizar terminal")} title={tr("Minimize", "Minimizar")}>
            <svg viewBox="0 0 20 20"><path d="M5 14h10" /></svg>
          </button>
          <button class="fullscreen-button" type="button" onclick={toggleFullscreen} aria-label={fullscreen ? tr("Exit full screen", "Sair da tela cheia") : tr("Enter full screen", "Entrar em tela cheia")} title={fullscreen ? tr("Exit full screen", "Sair da tela cheia") : tr("Full screen", "Tela cheia")}>
            {#if fullscreen}
              <svg viewBox="0 0 20 20"><path d="M8 3v5H3M12 3v5h5M8 17v-5H3M12 17v-5h5" /></svg>
            {:else}
              <svg viewBox="0 0 20 20"><path d="M3 8V3h5M12 3h5v5M3 12v5h5M17 12v5h-5" /></svg>
            {/if}
          </button>
          <button class="close-button" type="button" onclick={closeTerminal} aria-label={tr("Close terminal", "Fechar terminal")}>
            <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8" /></svg>
          </button>
        </span>
        <span class="header-overflow">
          <button
            class:active={headerActionsOpen}
            class="header-overflow-trigger"
            type="button"
            aria-expanded={headerActionsOpen}
            aria-haspopup="menu"
            aria-label={tr("Terminal actions", "Ações do terminal")}
            title={tr("Terminal actions", "Ações do terminal")}
            onclick={() => (headerActionsOpen = !headerActionsOpen)}
          >
            <svg viewBox="0 0 20 20"><circle cx="5" cy="10" r="1"></circle><circle cx="10" cy="10" r="1"></circle><circle cx="15" cy="10" r="1"></circle></svg>
          </button>
          {#if headerActionsOpen}
            <span class="header-actions-menu" role="menu" tabindex="-1" onpointerdown={(event) => event.stopPropagation()}>
              {#if !renamingSession}
                <button type="button" role="menuitem" onclick={() => { headerActionsOpen = false; beginSessionRename(); }}>
                  <svg viewBox="0 0 20 20"><path d="m4 14-.5 2.5L6 16l9-9-2-2-9 9Z"></path><path d="m11.5 6.5 2 2"></path></svg>
                  <span>{tr("Rename session", "Renomear sessão")}</span>
                </button>
              {/if}
              <span class="header-menu-zoom" role="group" aria-label={tr("Terminal text size", "Tamanho dos textos do terminal")}>
                <span>
                  <svg viewBox="0 0 20 20"><circle cx="8.5" cy="8.5" r="4.5"></circle><path d="m12 12 4 4M8.5 6.5v4M6.5 8.5h4"></path></svg>
                  {tr("Text size", "Tamanho do texto")}
                </span>
                <button disabled={textZoom <= textZoomMin} type="button" aria-label={tr("Decrease text size", "Diminuir textos")} onclick={() => setTextZoom(textZoom - 0.1)}>−</button>
                <output>{Math.round(textZoom * 100)}%</output>
                <button disabled={textZoom >= textZoomMax} type="button" aria-label={tr("Increase text size", "Aumentar textos")} onclick={() => setTextZoom(textZoom + 0.1)}>+</button>
              </span>
              {#if windowState?.docked}
                <button class="compact-only" type="button" role="menuitem" onclick={() => { headerActionsOpen = false; void detach(); }}>
                  <svg viewBox="0 0 20 20"><path d="M7 6 5.5 7.5a3 3 0 0 0 4.2 4.2l1.2-1.2M13 14l1.5-1.5a3 3 0 0 0-4.2-4.2L9.1 9.5" /></svg>
                  <span>{tr("Undock terminal", "Desacoplar terminal")}</span>
                </button>
              {/if}
              <button class="compact-only" type="button" role="menuitem" onclick={() => { headerActionsOpen = false; void minimizeTerminal(); }}>
                <svg viewBox="0 0 20 20"><path d="M5 14h10" /></svg>
                <span>{tr("Minimize terminal", "Minimizar terminal")}</span>
              </button>
              <button class="compact-only" type="button" role="menuitem" onclick={() => { headerActionsOpen = false; void toggleFullscreen(); }}>
                {#if fullscreen}
                  <svg viewBox="0 0 20 20"><path d="M8 3v5H3M12 3v5h5M8 17v-5H3M12 17v-5h5" /></svg>
                  <span>{tr("Exit full screen", "Sair da tela cheia")}</span>
                {:else}
                  <svg viewBox="0 0 20 20"><path d="M3 8V3h5M12 3h5v5M3 12v5h5M17 12v5h-5" /></svg>
                  <span>{tr("Enter full screen", "Entrar em tela cheia")}</span>
                {/if}
              </button>
              <button class="compact-only" type="button" role="menuitem" onclick={() => { headerActionsOpen = false; void closeTerminal(); }}>
                <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8" /></svg>
                <span>{tr("Close terminal", "Fechar terminal")}</span>
              </button>
              {#if session.source === "cli" && session.processId}
                <button class="danger terminal-stop-menu" type="button" role="menuitem" onclick={() => { headerActionsOpen = false; terminateConfirm = true; }}>
                  <svg viewBox="0 0 20 20"><path d="M10 3v7M5.5 5.5a6 6 0 1 0 9 0" /></svg>
                  <span>{tr("Stop agent", "Encerrar agente")}</span>
                </button>
              {/if}
            </span>
          {/if}
        </span>
      </header>

      {#if activePlan || goal}
        <aside class:collapsed={!workTrayExpanded} class="work-tray" aria-label={tr("Agent work status", "Status do trabalho do agente")} transition:slide={{ duration: 160 }}>
          <button
            class="work-tray-toggle"
            type="button"
            aria-expanded={workTrayExpanded}
            aria-label={workTrayExpanded ? tr("Collapse agent work", "Recolher trabalho do agente") : tr("Expand agent work", "Expandir trabalho do agente")}
            onclick={() => (workTrayExpanded = !workTrayExpanded)}
          >
            <strong>{tr("Agent work", "Trabalho do agente")}</strong>
            <span>
              {#if activePlan}<small>PLAN {completedPlanItems}/{activePlan.items.length}</small>{/if}
              {#if goal}<small>GOAL · {goalStatusLabel(goal.status)} · {elapsedGoalTime()}</small>{/if}
            </span>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4"></path></svg>
          </button>
          <div class="work-tray-body" aria-hidden={!workTrayExpanded}>
            <div class="work-tray-grid">
              {#if activePlan}
                <section class="work-card todo-card" transition:fade={{ duration: 140 }}>
                  <div class="work-card-heading">
                    <strong>{tr("PLAN", "PLANO")}</strong>
                    <span>{completedPlanItems}/{activePlan.items.length}</span>
                  </div>
                  <i class="todo-progress" style={`--todo-progress: ${(completedPlanItems / activePlan.items.length) * 100}%`}>
                    <em></em>
                  </i>
                  <ul>
                    {#each activePlan.items.slice(0, 4) as item}
                      <li class:active={item.status === "in_progress"} class:done={item.status === "completed"} title={workItemLabel(item.status)}>
                        <span aria-hidden="true"></span>
                        <small>{item.label}</small>
                      </li>
                    {/each}
                  </ul>
                  {#if activePlan.items.length > 4}
                    <small class="work-more">+{activePlan.items.length - 4} {tr("more", "a mais")}</small>
                  {/if}
                </section>
              {/if}
              {#if goal}
                <section class="work-card goal-card" transition:fade={{ duration: 140 }}>
                  <div class="work-card-heading">
                    <strong>GOAL</strong>
                    <span class:complete={goal.status === "complete"} class:blocked={goal.status === "blocked"}>
                      {goalStatusLabel(goal.status)}
                    </span>
                  </div>
                  <p title={goal.objective}>{goal.objective}</p>
                  <small class="goal-time">
                    <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="7"></circle><path d="M10 6v4l3 2"></path></svg>
                    {elapsedGoalTime()}
                  </small>
                </section>
              {/if}
            </div>
          </div>
        </aside>
      {/if}

      <nav class="hub-tabs" aria-label={tr("Session details", "Detalhes da sessão")}>
        <button class:active={activeTab === "chat"} type="button" onclick={() => void selectTab("chat")}>
          {tr("Chat", "Chat")} <span>{chatEntries.length}</span>
        </button>
        <button class:active={activeTab === "changes"} type="button" onclick={() => void selectTab("changes")}>
          {tr("Changes", "Alterações")} <span>{changedFiles.length}</span>
        </button>
        {#if plan}
          <button class:active={activeTab === "plan"} type="button" onclick={() => void selectTab("plan")}>
            {tr("Plan", "Plano")} <span>{plan.items.length || 1}</span>
          </button>
        {/if}
        <button class:active={activeTab === "notes"} type="button" onclick={() => void selectTab("notes")}>
          {tr("Notes", "Notas")} {#if sessionNotes.length}<span>{sessionNotes.length}</span>{/if}
        </button>
      </nav>

      {#if activeTab === "chat" && windowState?.workflowEnabled && windowState.docked}
        <span class="workflow-role-control">
          <button
            bind:this={workflowRoleFabElement}
            class:unconfigured={!workflowStep}
            class:open={Boolean(workflowDraft)}
            class="workflow-role-fab"
            type="button"
            title={workflowStep?.instruction || tr("Configure this terminal role", "Configurar o papel deste terminal")}
            aria-label={tr("Configure this terminal role", "Configurar o papel deste terminal")}
            aria-expanded={Boolean(workflowDraft)}
            onclick={() => workflowDraft ? (workflowDraft = null) : void openWorkflowRoleEditor()}
          >
            {#if workflowStep}
              <i class="role-symbol role-{workflowStep.role}" aria-hidden="true"><WorkflowRoleIcon role={workflowStep.role} /></i>
            {:else}
              <i class="role-symbol role-custom" aria-hidden="true"><WorkflowRoleIcon role="custom" /></i>
            {/if}
            {#if workflowStepOrder}<span class="workflow-step-badge" aria-hidden="true">{workflowStepOrder}</span>{/if}
            <span class="workflow-role-tooltip">
              <strong>{workflowStep ? workflowStepRoleLabel(workflowStep) : tr("Set role", "Definir papel")}</strong>
              <small>{workflowStepOrder
                ? tr(`Step ${workflowStepOrder} of ${workflowStepTotal}`, `Etapa ${workflowStepOrder} de ${workflowStepTotal}`)
                : tr("Not configured", "Não configurado")}</small>
            </span>
          </button>

          {#if workflowDraft && workflowEditingStep}
            {@const draftIndex = workflowDraft.steps.findIndex((step) => step.id === workflowEditingStep?.id)}
            <div
              class:above={workflowRolePopoverAbove}
              class:constrained={workflowRolePopoverConstrained}
              class="workflow-role-popover"
              style={workflowRolePopoverStyle}
              role="dialog"
              tabindex="-1"
              aria-label={tr("Configure workflow role", "Configurar papel do workflow")}
              onpointerdown={(event) => event.stopPropagation()}
            >
              <div class="workflow-popover-bridge" aria-hidden="true"></div>
              <div class="workflow-popover-heading">
                <span>
                  <small>{tr(`Step ${workflowStepOrder || workflowPhysicalOrder || draftIndex + 1} of ${workflowStepTotal || workflowDraft.steps.length}`, `Etapa ${workflowStepOrder || workflowPhysicalOrder || draftIndex + 1} de ${workflowStepTotal || workflowDraft.steps.length}`)}</small>
                  <strong>{sessionDisplayName(session)}</strong>
                </span>
              </div>

              <div class:picker-open={workflowRolePickerOpen} class="workflow-role-fields">
                <div class="workflow-role-picker">
                  <span class="workflow-field-label">{tr("Agent role", "Papel do agente")}</span>
                  <button class:open={workflowRolePickerOpen} class="workflow-role-trigger" type="button" aria-haspopup="listbox" aria-expanded={workflowRolePickerOpen} onclick={() => { workflowPendingRole = null; workflowRolePickerOpen = !workflowRolePickerOpen; }}>
                    <i class="role-symbol role-{workflowRoleConfigured ? workflowEditingStep.role : 'custom'}" aria-hidden="true"><WorkflowRoleIcon role={workflowRoleConfigured ? workflowEditingStep.role : "custom"} /></i>
                    <span>
                      <strong>{workflowRoleConfigured ? workflowStepRoleLabel(workflowEditingStep) : tr("Choose a role", "Escolha um papel")}</strong>
                      <small>{workflowRoleConfigured ? workflowRoleDescription(workflowEditingStep.role) : tr("Select this agent's responsibility", "Selecione a responsabilidade deste agente")}</small>
                    </span>
                    <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4"></path></svg>
                  </button>
                  {#if workflowRolePickerOpen}
                    <div class="workflow-role-menu" role="listbox" aria-label={tr("Agent role", "Papel do agente")}>
                      {#each workflowRoles as role}
                        <button class:active={workflowRoleConfigured && workflowEditingStep.role === role} disabled={workflowDraftSaving} type="button" role="option" aria-selected={workflowRoleConfigured && workflowEditingStep.role === role} onclick={() => void selectWorkflowRole(role)}>
                          <i class="role-symbol role-{role}" aria-hidden="true"><WorkflowRoleIcon {role} /></i>
                          <span><strong>{workflowRoleOptionLabel(role)}</strong><small>{workflowRoleDescription(role)}</small></span>
                          {#if workflowRoleConfigured && workflowEditingStep.role === role}<svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5 10 3 3 7-7"></path></svg>{/if}
                        </button>
                      {/each}
                    </div>
                  {/if}
                </div>
                {#if workflowPendingRole}
                  <div class="workflow-role-replace-warning" role="alert">
                    <i class="role-symbol role-{workflowPendingRole}" aria-hidden="true"><WorkflowRoleIcon role={workflowPendingRole} /></i>
                    <span>
                      <strong>{tr("Replace customized instructions?", "Substituir instruções personalizadas?")}</strong>
                      <small>{tr(`Changing to ${workflowRoleOptionLabel(workflowPendingRole)} restores that role's defaults.`, `Mudar para ${workflowRoleOptionLabel(workflowPendingRole)} restaura os padrões desse papel.`)}</small>
                    </span>
                    <div>
                      <button type="button" onclick={() => (workflowPendingRole = null)}>{tr("Keep", "Manter")}</button>
                      <button class="confirm" disabled={workflowDraftSaving} type="button" onclick={() => void confirmWorkflowRoleChange()}>{tr("Replace", "Substituir")}</button>
                    </div>
                  </div>
                {/if}
                {#if workflowRoleConfigured}
                  <button class:open={workflowContractExpanded} class="workflow-contract-toggle" type="button" aria-expanded={workflowContractExpanded} onclick={() => (workflowContractExpanded = !workflowContractExpanded)}>
                    <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 3.5h7l3 3v10H5z"></path><path d="M12 3.5v3h3M8 10h4M8 13h4"></path></svg>
                    <span><strong>{workflowContractExpanded ? tr("Hide role instructions", "Ocultar instruções do papel") : tr("Show role instructions", "Exibir instruções do papel")}</strong></span>
                    <svg class="contract-chevron" viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4"></path></svg>
                  </button>
                  {#if workflowContractExpanded}
                    <div class="workflow-contract-body">
                      <div class="workflow-instruction-meta">
                        <span class:customized={workflowInstructionsCustomized || workflowEditingStep.role === "custom"} class:ready={workflowRoleReady}>
                          {workflowEditingStep.role === "custom" ? tr("Custom role", "Papel personalizado") : workflowInstructionsCustomized ? tr("Customized", "Personalizado") : tr("Lume default", "Padrão do Lume")}
                        </span>
                        {#if workflowEditingStep.role !== "custom" && workflowInstructionsCustomized}
                          <button type="button" onclick={() => void restoreWorkflowRoleDefaults()}>{tr("Restore default", "Restaurar padrão")}</button>
                        {/if}
                      </div>
                      {#if workflowEditingStep.role === "custom"}
                        <label><span>{tr("Custom role name", "Nome do papel personalizado")}</span><input maxlength="80" value={workflowEditingStep.customRoleLabel} oninput={(event) => updateWorkflowEditingStep({ customRoleLabel: event.currentTarget.value })} /></label>
                      {/if}
                      <label class="workflow-instruction-field"><span>{tr("Instruction", "Instrução")}</span><textarea maxlength="4000" rows="2" value={workflowEditingStep.instruction} placeholder={tr("What this agent must do", "O que este agente deve fazer")} oninput={(event) => updateWorkflowEditingStep({ instruction: event.currentTarget.value })}></textarea></label>
                      <div class="workflow-contract-pair">
                        <label><span>{tr("Expected input", "Entrada esperada")}</span><textarea maxlength="4000" rows="2" value={workflowEditingStep.expectedInput} oninput={(event) => updateWorkflowEditingStep({ expectedInput: event.currentTarget.value })}></textarea></label>
                        <label><span>{tr("Produced output", "Saída produzida")}</span><textarea maxlength="4000" rows="2" value={workflowEditingStep.producedOutput} oninput={(event) => updateWorkflowEditingStep({ producedOutput: event.currentTarget.value })}></textarea></label>
                      </div>
                      <label><span>{tr("Completion condition", "Condição de conclusão")}</span><textarea maxlength="4000" rows="2" value={workflowEditingStep.completionCondition} placeholder={tr("How Lume knows this step is complete", "Como o Lume sabe que esta etapa terminou")} oninput={(event) => updateWorkflowEditingStep({ completionCondition: event.currentTarget.value })}></textarea></label>
                      {#if !workflowRoleReady}
                        <p class="workflow-readiness-note">{tr("Complete every field so this step is ready for the workflow.", "Preencha todos os campos para deixar esta etapa pronta para o workflow.")}</p>
                      {/if}
                    </div>
                  {/if}
                {/if}
              </div>
              {#if workflowDraftError}<p class="handoff-error">{workflowDraftError}</p>{/if}
              {#if workflowRoleConfigured}
                <div class="workflow-popover-actions">
                  <button type="button" onclick={() => (workflowDraft = null)}>{tr("Cancel", "Cancelar")}</button>
                  <button class="primary" disabled={workflowDraftSaving || !workflowRoleReady} type="button" onclick={() => void saveWorkflowRole()}>{workflowDraftSaving ? tr("Saving…", "Salvando…") : tr("Save instructions", "Salvar instruções")}</button>
                </div>
              {/if}
            </div>
          {/if}
        </span>
      {/if}

      <div
        class:with-workflow-role={activeTab === "chat" && windowState?.workflowEnabled && windowState.docked}
        class="terminal-output"
        bind:this={outputElement}
        onscroll={handleOutputScroll}
        onwheel={handleOutputWheel}
      >
        {#if activeTab === "chat"}
          <p><span>$</span> {session.agentLabel.toLowerCase()} <i>{session.project}</i></p>
          <p class="status status-{session.status}"><span>&gt;</span> {displayText(language, session.statusLabel)}</p>
        {/if}
        {#if session.pendingQuestion}
          <section class="agent-question" aria-label={tr("Agent question", "Pergunta do agente")}>
            {#each session.pendingQuestion.questions as question}
              <div class="agent-question-item">
                <small>{displayText(language, question.header)}</small>
                <strong>{displayText(language, question.question)}</strong>
                {#if question.options.length}
                  <div class="question-options">
                    {#each question.options as option, index}
                      <button
                        class:selected={questionSelections[question.id] === option.label}
                        disabled={sending}
                        type="button"
                        onclick={() => void selectQuestionOption(question.id, option.label)}
                      >
                        <b>{index + 1}</b>
                        <span>
                          {displayText(language, option.label)}
                          {#if option.description}<small>{displayText(language, option.description)}</small>{/if}
                        </span>
                      </button>
                    {/each}
                  </div>
                {/if}
                <em>{tr("Click an option or type its number below.", "Clique em uma opção ou digite o número abaixo.")}</em>
              </div>
            {/each}
            {#if session.pendingQuestion.questions.length > 1}
              <button class="question-submit" disabled={sending} type="button" onclick={() => void submitSelectedQuestionAnswers()}>
                {tr("Answer", "Responder")}
              </button>
            {/if}
          </section>
        {/if}

        {#if activeTab === "chat"}
          <div class="chat-feed">
            {#if hiddenChatItemCount > 0}
              <button class="load-earlier-chat" type="button" onclick={() => void revealEarlierChatItems()}>
                <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 12 4-4 4 4" /></svg>
                <span>{tr(`Load ${Math.min(60, hiddenChatItemCount)} earlier items`, `Carregar ${Math.min(60, hiddenChatItemCount)} itens anteriores`)}</span>
                <small>{hiddenChatItemCount}</small>
              </button>
            {/if}
            {#each visibleChatFeedItems as feedItem (feedItem.id)}
              {#if feedItem.kind === "trace"}
                <ActivityTraceGroup
                  activities={feedItem.entries.map((entry) => entry.activity)}
                  active={feedItem.id === activeTraceId}
                  {language}
                />
                {#if feedItem.files.length}
                  <div class="turn-files">
                    <header>
                      <span class="turn-files-mark"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 3.5h6l4 4v9H5zM11 3.5v4h4M8 11h4M8 14h3" /></svg></span>
                      <strong>{tr("Files changed", "Arquivos alterados")}</strong>
                      <small>{feedItem.files.length}</small>
                      <button
                        class="handoff-button"
                        disabled={handoffLoading}
                        type="button"
                        title={tr("Send files to a docked agent", "Enviar arquivos para um agente acoplado")}
                        aria-label={tr("Send these changed files to a docked agent", "Enviar estes arquivos alterados para um agente acoplado")}
                        onclick={() => void openHandoff(traceHandoffEntry(feedItem))}
                      >
                        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 6h8M9 3l3 3-3 3M16 14H8M11 11l-3 3 3 3" /></svg>
                      </button>
                    </header>
                    <div>
                      {#each feedItem.files as file}
                        <code title={file.path}><FileTypeIcon path={file.path} /><span class="file-path">{displayFileChangePath(file.path)}</span><span class="added">+{file.added}</span><span class="removed">-{file.removed}</span></code>
                      {/each}
                    </div>
                  </div>
                {/if}
              {:else}
                {@const entry = feedItem.entry}
                {@const item = entry.activity}
                {#if item.kind === "prompt" && (item.detail || item.attachments?.length)}
                  {@const receivedHandoff = parseHandoffPrompt(item.detail)}
                  <div class="chat-message user-message">
                    <header>
                      <strong>
                        {receivedHandoff
                          ? tr(`Context from ${receivedHandoff.source}`, `Contexto de ${receivedHandoff.source}`)
                          : tr("You", "Você")}
                      </strong>
                      <time>{activityTime(item.createdAt)}</time>
                    </header>
                    {#if receivedHandoff}
                      <div class="markdown-content">{@html renderCachedMarkdown(`handoff:${entry.id}`, receivedHandoff.body)}</div>
                    {:else if item.detail}
                      <pre>{item.detail}</pre>
                    {/if}
                    {#if item.attachments?.length}
                      <div class="message-images">
                        {#each item.attachments as attachment}
                          {#if attachment.previewDataUrl}
                            <img src={attachment.previewDataUrl} alt={attachment.name} />
                          {:else}
                            <span class="message-file" title={attachment.name}>
                              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 2.8h6l4 4V17H5zM11 3v4h4M7.5 11h5M7.5 14h4" /></svg>
                              <small>{attachment.name}</small>
                            </span>
                          {/if}
                        {/each}
                      </div>
                    {/if}
                  </div>
                {:else if item.kind === "message" && (item.detail || item.attachments?.length)}
                  {@const authorizationNeeded = authorizationMessageId === entry.id}
                  <div class:intervention-required={authorizationNeeded} class="chat-message agent-message">
                    <header>
                      <strong>{session.agentLabel}</strong>
                      <button
                        class="handoff-button"
                        disabled={handoffLoading}
                        type="button"
                        title={tr("Send to a docked agent", "Enviar para um agente acoplado")}
                        aria-label={tr("Send this response to a docked agent", "Enviar esta resposta para um agente acoplado")}
                        onclick={() => void openHandoff(entry)}
                      >
                        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 6h8M9 3l3 3-3 3M16 14H8M11 11l-3 3 3 3" /></svg>
                      </button>
                      <time>{activityTime(item.createdAt)}</time>
                    </header>
                    {#if authorizationNeeded}
                      <aside class="intervention-indicator" aria-label={tr("Your intervention is required", "Sua intervenção é necessária")}>
                        <span aria-hidden="true">
                          <svg viewBox="0 0 20 20"><path class="indicator-shape" d="M10 2.8 18 16.5H2z"></path><path class="indicator-mark" d="M10 7v4.5M10 14.2v.2"></path></svg>
                        </span>
                        <div>
                          <strong>{tr("Your authorization is needed", "Sua autorização é necessária")}</strong>
                        </div>
                      </aside>
                    {/if}
                    {#if item.detail}<div class="markdown-content">{@html renderCachedMarkdown(`message:${entry.id}`, item.detail)}</div>{/if}
                    <ResponseAttachments
                      text={item.detail}
                      attachments={item.attachments ?? []}
                      workingDirectory={session.workingDirectory}
                      {language}
                      onError={(error) => (message = error)}
                    />
                  </div>
                {:else if item.kind === "analysis" && item.detail}
                  <section class:running={item.status === "running"} class="reasoning-update">
                    <header>
                      <span><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 2.5 11.4 7 16 8.5 11.4 10 10 14.5 8.6 10 4 8.5 8.6 7ZM15.5 13l.7 2.1 2.1.7-2.1.7-.7 2.1-.7-2.1-2.1-.7 2.1-.7Z" /></svg></span>
                      <strong>{displayText(language, item.title)}</strong>
                      <time>{activityTime(item.createdAt)}</time>
                    </header>
                    <div class="markdown-content">{@html renderCachedMarkdown(`analysis:${entry.id}`, item.detail)}</div>
                  </section>
                {/if}
                {#if entry.files.length}
                  <div class="turn-files">
                    <header>
                      <span class="turn-files-mark"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 3.5h6l4 4v9H5zM11 3.5v4h4M8 11h4M8 14h3" /></svg></span>
                      <strong>{tr("Files changed", "Arquivos alterados")}</strong>
                      <small>{entry.files.length}</small>
                      <button
                        class="handoff-button"
                        disabled={handoffLoading}
                        type="button"
                        title={tr("Send files to a docked agent", "Enviar arquivos para um agente acoplado")}
                        aria-label={tr("Send these changed files to a docked agent", "Enviar estes arquivos alterados para um agente acoplado")}
                        onclick={() => void openHandoff(entry)}
                      >
                        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 6h8M9 3l3 3-3 3M16 14H8M11 11l-3 3 3 3" /></svg>
                      </button>
                    </header>
                    <div>
                      {#each entry.files as file}
                        <code title={file.path}><FileTypeIcon path={file.path} /><span class="file-path">{displayFileChangePath(file.path)}</span><span class="added">+{file.added}</span><span class="removed">-{file.removed}</span></code>
                      {/each}
                    </div>
                  </div>
                {/if}
              {/if}
            {:else}
              <p class="empty-state">{tr("Messages and agent activity will appear here in real time.", "As mensagens e a atividade do agente aparecerão aqui em tempo real.")}</p>
            {/each}
            {#if session.status === "running"}
              <div class="agent-typing" aria-label={tr(`${sessionDisplayName(session)} is working`, `${sessionDisplayName(session)} está trabalhando`)}>
                <span></span><span></span><span></span>
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
            {/if}
          </div>
        {:else if activeTab === "changes"}
          <section class="changes-panel">
            <strong>{tr("All changed files", "Todos os arquivos alterados")}</strong>
            {#if changedFiles.length}
              <div class="change-list">
                {#each changedFiles as file}
                  <code title={file.path}><FileTypeIcon path={file.path} /><span class="file-path">{displayFileChangePath(file.path)}</span><span class="added">+{file.added}</span><span class="removed">-{file.removed}</span></code>
                {/each}
              </div>
            {:else}
              <p class="empty-state">{tr("No file changes were reported in this session.", "Nenhuma alteração de arquivo foi informada nesta sessão.")}</p>
            {/if}
          </section>
        {:else if activeTab === "plan" && plan}
          <section class="plan-panel">
            <header>
              <div>
                <small>{tr("Agent planning", "Planejamento do agente")}</small>
                <strong>{tr("Current plan", "Plano atual")}</strong>
              </div>
              <div class="plan-header-actions">
                {#if plan.items.length}
                  <span>{completedPlanItems}/{plan.items.length} {tr("completed", "concluídos")}</span>
                {/if}
                <button type="button" onclick={createNoteFromCurrentPlan}>
                  <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 3.5h9l3 3V16.5H4zM7 3.5v5h6v-5M7 13h6" /></svg>
                  {tr("Save to Notes", "Salvar em Notas")}
                </button>
              </div>
            </header>
            {#if plan.content}
              <div class="plan-document markdown-content">{@html renderCachedMarkdown(`plan:${plan.updatedAt}`, plan.content)}</div>
            {:else}
              {#if plan.explanation}
                <p class="plan-explanation">{plan.explanation}</p>
              {/if}
              {#if plan.items.length}
                <i class="plan-progress" style={`--plan-progress: ${(completedPlanItems / plan.items.length) * 100}%`}>
                  <em></em>
                </i>
                <ol class="plan-items">
                  {#each plan.items as item, index}
                    <li class:active={item.status === "in_progress"} class:done={item.status === "completed"}>
                      <span>{item.status === "completed" ? "✓" : index + 1}</span>
                      <div>
                        <strong>{item.label}</strong>
                        <small>{workItemLabel(item.status)}</small>
                      </div>
                    </li>
                  {/each}
                </ol>
              {/if}
            {/if}
          </section>
        {:else if activeTab === "notes"}
          <section class="notes-panel">
            <header>
              <div>
                <small>{tr("Session notebook", "Caderno da sessão")}</small>
                <strong>{tr("Notes and previous plans", "Notas e planos anteriores")}</strong>
              </div>
              <button type="button" title={tr("New note", "Nova nota")} aria-label={tr("New note", "Nova nota")} onclick={createBlankNote}>
                <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 3h7l3 3v11H5Z" /><path d="M12 3v3h3M10 8v6M7 11h6" /></svg>
              </button>
            </header>

            {#if noteEditor}
              <form class="note-editor" onsubmit={(event) => { event.preventDefault(); void persistSessionNote(); }}>
                <div class="note-editor-heading">
                  <input bind:value={noteEditor.title} maxlength="120" placeholder={tr("Note title", "Título da nota")} />
                  <label title={tr("Keep this note at the top", "Manter esta nota no topo")}>
                    <input type="checkbox" bind:checked={noteEditor.pinned} />
                    <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m7 3 6 2-1.5 4 3 3-4.5.5L8 17l-1-4-4-2.5 4-2z" /></svg>
                  </label>
                </div>
                <textarea bind:value={noteEditor.body} rows="8" maxlength="131072" placeholder={tr("Write a note or paste a longer plan…", "Escreva uma nota ou cole um planejamento maior…")}></textarea>
                <footer>
                  <button type="button" title={tr("Cancel", "Cancelar")} aria-label={tr("Cancel", "Cancelar")} onclick={() => (noteEditor = null)}><svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 6 8 8M14 6l-8 8" /></svg></button>
                  <button class="primary" disabled={noteSaving || !noteEditor.body.trim()} type="submit" title={noteSaving ? tr("Saving…", "Salvando…") : tr("Save", "Salvar")} aria-label={noteSaving ? tr("Saving…", "Salvando…") : tr("Save", "Salvar")}><svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5 10 3 3 7-7" /></svg></button>
                </footer>
              </form>
            {/if}

            {#if notesLoading}
              <p class="empty-state">{tr("Loading notes…", "Carregando notas…")}</p>
            {:else if sessionNotes.length}
              <div class="session-note-list">
                {#each sessionNotes as note (note.id)}
                  <article class:pinned={note.pinned} class="session-note">
                    <header>
                      <div>
                        <small>{note.kind === "plan" ? tr("PLAN", "PLANO") : tr("NOTE", "NOTA")} · {noteDate(note.updatedAt)}</small>
                        <strong>{note.title}</strong>
                      </div>
                      <span class="session-note-actions">
                        <button class:active={note.pinned} type="button" title={tr("Pin note", "Fixar nota")} aria-label={tr("Pin note", "Fixar nota")} onclick={() => void toggleSessionNotePin(note)}><svg viewBox="0 0 20 20"><path d="m7 3 6 2-1.5 4 3 3-4.5.5L8 17l-1-4-4-2.5 4-2z" /></svg></button>
                        <button type="button" title={tr("Edit note", "Editar nota")} aria-label={tr("Edit note", "Editar nota")} onclick={() => editSessionNote(note)}><svg viewBox="0 0 20 20"><path d="m4 14.5-.5 2 2-.5L15 6.5 12.5 4zM11.5 5l2.5 2.5" /></svg></button>
                        <button type="button" title={tr("Delete note", "Excluir nota")} aria-label={tr("Delete note", "Excluir nota")} onclick={() => void removeSessionNote(note)}><svg viewBox="0 0 20 20"><path d="M5 6h10M8 6V4h4v2M7 8v7M10 8v7M13 8v7M6 6l.6 11h6.8L14 6" /></svg></button>
                      </span>
                    </header>
                    <div class="session-note-body markdown-content">{@html renderCachedMarkdown(`note:${note.id}`, note.body)}</div>
                    <button class="use-note-button" type="button" title={tr("Use as context", "Usar como contexto")} aria-label={tr("Use as context", "Usar como contexto")} onclick={() => void useSessionNote(note)}><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 4h12v10H9l-4 3v-3H4Z" /><path d="M8 9h5M11 6l3 3-3 3" /></svg></button>
                  </article>
                {/each}
              </div>
            {:else if !noteEditor}
              <p class="empty-state">{tr("Save the current plan or create a note to keep long-term context here.", "Salve o plano atual ou crie uma nota para manter aqui o contexto de longo prazo.")}</p>
            {/if}
          </section>
        {/if}

        {#if handoffDraft}
          {@const selectedHandoffTarget = handoffTargets.find((target) => target.session.id === handoffDraft?.targetSessionId)}
          <div class="handoff-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) handoffDraft = null; }}>
            <div class="handoff-dialog" role="dialog" aria-modal="true" aria-labelledby="handoff-title">
              <header>
                <div>
                  <small>{tr("Context handoff", "Transferência de contexto")}</small>
                  <strong id="handoff-title">{tr("Send to a docked agent", "Enviar para um agente acoplado")}</strong>
                </div>
                <button type="button" aria-label={tr("Close", "Fechar")} onclick={() => (handoffDraft = null)}>×</button>
              </header>

              <label class="handoff-target">
                <span>{tr("Destination", "Destino")}</span>
                <select bind:value={handoffDraft.targetSessionId}>
                  {#each handoffTargets as target (target.terminal.label)}
                    <option value={target.session.id} disabled={!handoffTargetCanReceive(target)}>
                      {sessionDisplayName(target.session)} · {target.session.agentLabel}{handoffTargetCanReceive(target) ? "" : ` · ${tr("Unavailable", "Indisponível")}`}
                    </option>
                  {/each}
                </select>
              </label>

              <div class="handoff-options">
                <label class:disabled={!handoffDraft.text}>
                  <input type="checkbox" bind:checked={handoffDraft.includeText} disabled={!handoffDraft.text} />
                  <span><strong>{tr("Response", "Resposta")}</strong><small>{tr("Share the selected agent message", "Compartilhar a mensagem selecionada")}</small></span>
                </label>
                <label class:disabled={!handoffDraft.files.length}>
                  <input type="checkbox" bind:checked={handoffDraft.includeFiles} disabled={!handoffDraft.files.length} />
                  <span><strong>{tr("Changed files", "Arquivos alterados")}</strong><small>{handoffDraft.files.length} {tr("reported files", "arquivos informados")}</small></span>
                </label>
              </div>

              <label class="handoff-note">
                <span>{tr("Instruction for the next agent", "Instrução para o próximo agente")}</span>
                <textarea bind:value={handoffDraft.note} rows="2" placeholder={tr("Optional: explain what the next agent should do…", "Opcional: explique o que o próximo agente deve fazer…")}></textarea>
              </label>

              <div class="handoff-preview">
                <strong>{tr("Preview", "Pré-visualização")}</strong>
                <pre>{handoffPreview(handoffDraft) || tr("Select content or write an instruction.", "Selecione um conteúdo ou escreva uma instrução.")}</pre>
              </div>

              {#if handoffError}<p class="handoff-error">{handoffError}</p>{/if}
              {#if selectedHandoffTarget && !handoffTargetCanReceive(selectedHandoffTarget)}
                <p class="handoff-error">{tr("This agent cannot receive context while its current task is running.", "Este agente não pode receber contexto enquanto a tarefa atual estiver em execução.")}</p>
              {/if}

              <footer>
                <button type="button" onclick={() => (handoffDraft = null)}>{tr("Cancel", "Cancelar")}</button>
                <button
                  class="primary"
                  disabled={handoffSending || !selectedHandoffTarget || !handoffTargetCanReceive(selectedHandoffTarget) || !handoffPreview(handoffDraft)}
                  type="button"
                  onclick={() => void sendHandoff()}
                >
                  {handoffSending ? tr("Sending…", "Enviando…") : tr("Send context", "Enviar contexto")}
                </button>
              </footer>
            </div>
          </div>
        {/if}
      </div>

      {#if terminateConfirm}
        <div class="terminate-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget && !terminating) terminateConfirm = false; }}>
          <div class="terminate-dialog" role="alertdialog" aria-modal="true" aria-labelledby="terminate-title" tabindex="-1" onpointerdown={(event) => event.stopPropagation()}>
            <div>
              <strong id="terminate-title">{tr("Stop this agent?", "Encerrar este agente?")}</strong>
              <p>{tr("This will close the original connection and stop the agent.", "Ao encerrar, a conexão original será fechada e o agente será interrompido.")}</p>
            </div>
            <footer>
              <button disabled={terminating} type="button" onclick={() => (terminateConfirm = false)}>{tr("Cancel", "Cancelar")}</button>
              <button class="danger" disabled={terminating} type="button" onclick={() => void terminateAgent()}>{terminating ? tr("Stopping…", "Encerrando…") : tr("Stop agent", "Encerrar agente")}</button>
            </footer>
          </div>
        </div>
      {/if}

      {#if takeoverConfirm}
        <div class="terminate-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget && !takingControl) takeoverConfirm = false; }}>
          <div class="terminate-dialog takeover-dialog" role="alertdialog" aria-modal="true" aria-labelledby="takeover-title" tabindex="-1" onpointerdown={(event) => event.stopPropagation()}>
            <div>
              <strong id="takeover-title">{tr("Continue this session in Lume?", "Continuar esta sessão no Lume?")}</strong>
              <p>{session.status === "running"
                ? tr("The external CLI is running a task. Taking control will stop that task, close only the agent process, and resume the same thread in Lume.", "A CLI externa está executando uma tarefa. Assumir o controle interromperá essa tarefa, fechará somente o processo do agente e retomará a mesma thread no Lume.")
                : tr("Lume will close only the external agent process and resume the same thread here. The terminal and chat history will be preserved.", "O Lume fechará somente o processo externo do agente e retomará a mesma thread aqui. O terminal e o histórico do chat serão preservados.")}</p>
            </div>
            <footer>
              <button disabled={takingControl} type="button" onclick={() => (takeoverConfirm = false)}>{tr("Cancel", "Cancelar")}</button>
              <button class="takeover" disabled={takingControl} type="button" onclick={() => void confirmTakeover()}>{takingControl ? tr("Transferring…", "Transferindo…") : tr("Take control", "Assumir controle")}</button>
            </footer>
          </div>
        </div>
      {/if}

      {#if modelDialogOpen}
        <div class="terminate-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget && !modelSaving) modelDialogOpen = false; }}>
          <div class="terminate-dialog model-settings-dialog" role="dialog" aria-modal="true" aria-labelledby="model-settings-title" tabindex="-1" onpointerdown={(event) => event.stopPropagation()}>
            <header>
              <span class="model-settings-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24"><path d="M7 7.5 12 4l5 3.5v9L12 20l-5-3.5zM12 4v16m-5-3.5 5-3.5 5 3.5M7 7.5l5 3.5 5-3.5" /></svg>
              </span>
              <div>
                <strong id="model-settings-title">{tr("Model and reasoning", "Modelo e raciocínio")}</strong>
                <p>{tr("Next prompt", "Próximo prompt")}</p>
              </div>
            </header>

            {#if modelLoading}
              <div class="model-settings-loading"><span></span>{tr("Loading available models…", "Carregando modelos disponíveis…")}</div>
            {:else if session.agent === "claude_code"}
              <section class="model-settings-section claude-model-settings">
                <label>
                  <span class="model-settings-label">{tr("Model", "Modelo")}</span>
                  <input bind:value={claudeModel} maxlength="128" placeholder={tr("Session default or model alias", "Padrão da sessão ou alias do modelo")} />
                </label>
                <small>{tr("Use an alias such as sonnet or opus, or the full model name. Leave blank to keep the session default.", "Use um alias como sonnet ou opus, ou o nome completo. Deixe vazio para manter o padrão da sessão.")}</small>
              </section>
              <section class="model-settings-section">
                <span class="model-settings-label">{tr("Reasoning effort", "Nível de raciocínio")}<b>{effortLabel()}</b></span>
                <div class="effort-slider">
                  <input aria-label={tr("Reasoning effort", "Nível de raciocínio")} type="range" min="0" max={Math.max(0, effortValues().length - 1)} step="1" value={currentEffortIndex()} oninput={chooseEffortIndex} />
                  <div class="effort-scale" aria-hidden="true">
                    {#each effortValues() as effort (effort || "default")}
                      <span class:active={currentEffort() === effort}>{effortLabel(effort)}</span>
                    {/each}
                  </div>
                </div>
              </section>
            {:else if modelSettings}
              <section class="model-settings-section">
                <span class="model-settings-label">{tr("Model", "Modelo")}</span>
                <div class="model-options">
                  {#each modelSettings.models as option (option.model)}
                    <button class:active={selectedModel === option.model} type="button" onclick={() => chooseModel(option.model)} title={option.description}>
                      <span><strong>{option.displayName}</strong>{#if option.isDefault}<small>{tr("Default", "Padrão")}</small>{/if}</span>
                    </button>
                  {/each}
                </div>
              </section>

              {#if currentModelOption()}
                <section class="model-settings-section">
                  <span class="model-settings-label">{tr("Reasoning effort", "Nível de raciocínio")}<b>{effortLabel()}</b></span>
                  <div class="effort-slider">
                    <input aria-label={tr("Reasoning effort", "Nível de raciocínio")} type="range" min="0" max={Math.max(0, effortValues().length - 1)} step="1" value={currentEffortIndex()} oninput={chooseEffortIndex} />
                    <div class="effort-scale" aria-hidden="true">
                      {#each effortValues() as effort (effort)}
                        <span class:active={currentEffort() === effort}>{effort}</span>
                      {/each}
                    </div>
                  </div>
                </section>
              {/if}
            {/if}

            {#if modelError}<p class="model-settings-error">{modelError}</p>{/if}
            <footer>
              <button disabled={modelSaving} type="button" onclick={() => (modelDialogOpen = false)}>{tr("Cancel", "Cancelar")}</button>
              <button class="takeover" disabled={modelLoading || modelSaving || (session.agent === "codex" && (!selectedModel || !selectedEffort))} type="button" onclick={() => void saveModelSettings()}>
                {modelSaving ? tr("Saving…", "Salvando…") : tr("Save", "Salvar")}
              </button>
            </footer>
          </div>
        </div>
      {/if}

      <form
        class="terminal-composer"
        class:sending
        class:has-attachments={promptAttachments.length > 0}
        aria-busy={sending}
        style:height={`${displayedComposerHeight}px`}
        onpaste={(event) => void pasteAttachments(event)}
        onsubmit={(event) => {
          event.preventDefault();
          void sendPrompt();
        }}
      >
        <button
          class="composer-resize-handle"
          type="button"
          aria-label={tr("Drag up to enlarge the prompt field", "Arraste para cima para aumentar o campo de prompt")}
          title={tr("Drag up to enlarge the prompt field", "Arraste para cima para aumentar o campo de prompt")}
          onkeydown={resizeComposerWithKeyboard}
          onpointerdown={beginComposerResize}
          onpointermove={moveComposerResize}
          onpointerup={endComposerResize}
          onpointercancel={endComposerResize}
        ><span></span></button>
        {#if filteredSlashCommands().length}
          <div bind:this={slashCommandMenu} class="slash-command-menu" aria-label={tr("Slash commands", "Comandos com barra")}>
            <div class="slash-command-heading">
              <strong>{tr("Commands", "Comandos")}</strong>
              <small><kbd>↑↓</kbd> {tr("navigate", "navegar")} · <kbd>Enter</kbd> {tr("select", "selecionar")}</small>
            </div>
            {#each filteredSlashCommands() as command, index (command.name)}
              <button
                class:active={slashCommandIndex === index}
                data-slash-index={index}
                type="button"
                onmouseenter={() => (slashCommandIndex = index)}
                onclick={() => void selectSlashCommand(command)}
              >
                <code>/{command.name}</code>
                <span>{command.description}<small>{command.source === "agent" ? session.agentLabel : "Lume"}</small></span>
              </button>
            {/each}
          </div>
        {/if}
        {#if promptAttachments.length}
          <div class="pending-images">
            <small class="pending-images-label">
              {promptAttachments.length === 1
                ? tr("File attached", "Arquivo anexado")
                : tr(`${promptAttachments.length} files attached`, `${promptAttachments.length} arquivos anexados`)}
            </small>
            {#each promptAttachments as attachment, index}
              <span class:file-attachment={!attachment.previewDataUrl} title={attachment.name}>
                {#if attachment.previewDataUrl}
                  <img src={attachment.previewDataUrl} alt={attachment.name} />
                {:else}
                  <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 2.8h6l4 4V17H5zM11 3v4h4M7.5 11h5M7.5 14h4" /></svg>
                  <small>{attachment.name}</small>
                {/if}
                <button type="button" onclick={() => removeAttachment(index)} aria-label={tr("Remove file", "Remover arquivo")}>×</button>
              </span>
            {/each}
          </div>
        {/if}
        {#if nextQueuedPrompt}
          <button
            class="queued-prompt-tray"
            disabled={steeringQueued || !canSendWhileRunning}
            type="button"
            onclick={() => void steerNextQueuedPrompt()}
            aria-label={tr("Steer the next queued prompt now", "Enviar agora o próximo prompt da fila")}
          >
            <span class="queue-mark" aria-hidden="true">↳</span>
            <span class="queue-copy">
              <small>{queuedPrompts.length > 1 ? tr(`${queuedPrompts.length} queued prompts`, `${queuedPrompts.length} prompts na fila`) : tr("Queued next", "Próximo na fila")}</small>
              <strong>{nextQueuedPrompt.detail || tr("Prompt with attached files", "Prompt com arquivos anexados")}</strong>
            </span>
            <span class="queue-shortcut"><kbd>Tab</kbd><small>{steeringQueued ? tr("Steering…", "Enviando…") : tr("Steer now", "Enviar agora")}</small></span>
          </button>
        {/if}
        <div class="composer-controls">
          {#if ["codex", "claude_code"].includes(session.agent) || (canCompose && capabilities?.canAttachImages)}
            <div class="composer-leading-actions composer-tools">
              <button
                class:active={composerToolsOpen}
                class="composer-tools-trigger"
                type="button"
                aria-expanded={composerToolsOpen}
                aria-haspopup="menu"
                aria-label={tr("Prompt tools", "Ferramentas do prompt")}
                title={tr("Prompt tools", "Ferramentas do prompt")}
                onclick={() => (composerToolsOpen = !composerToolsOpen)}
              >
                <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 6h7M14 6h2M4 14h2M9 14h7M11 4v4M6 12v4" /></svg>
              </button>
              {#if composerToolsOpen}
                <div class="composer-tools-menu" role="menu" tabindex="-1" onpointerdown={(event) => event.stopPropagation()}>
                  {#if canCompose && capabilities?.canAttachImages}
                    <button disabled={!readyForPrompt || sending || promptAttachments.length >= 4} type="button" role="menuitem" onclick={() => void chooseAttachments()}>
                      <span class="tool-icon"><svg viewBox="0 0 20 20"><path d="M6.5 10.5 11 6a2.1 2.1 0 0 1 3 3l-6.2 6.2a3.4 3.4 0 1 1-4.8-4.8l6-6" /></svg></span>
                      <span><strong>{tr("Attach file", "Anexar arquivo")}</strong><small>{promptAttachments.length}/4</small></span>
                    </button>
                  {/if}
                  {#if session.agent === "codex" && session.controlOrigin === "lume"}
                    <button
                      class:active={collaborationMode === "plan"}
                      disabled={promptIsRunning || collaborationModeChanging}
                      type="button"
                      role="menuitem"
                      onclick={() => { composerToolsOpen = false; void toggleCollaborationMode(); }}
                    >
                      <span class="tool-icon">
                        {#if collaborationModeChanging}
                          <span class="mode-spinner" aria-hidden="true"></span>
                        {:else if collaborationMode === "plan"}
                          <svg viewBox="0 0 20 20"><path d="m4.8 5.8 1.3 1.3 2-2.2M10 6h5M4.8 10 6.1 11.3l2-2.2M10 10.2h5M4.8 14.2l1.3 1.3 2-2.2M10 14.4h5" /></svg>
                        {:else}
                          <svg viewBox="0 0 20 20"><path d="M11 3.5 5.8 10H10l-1 6.5 5.2-7.5H10z" /></svg>
                        {/if}
                      </span>
                      <span><strong>{tr("Agent mode", "Modo do agente")}</strong><small>{collaborationMode === "plan" ? "Plan" : "Default"}</small></span>
                    </button>
                  {/if}
                  {#if ["codex", "claude_code"].includes(session.agent)}
                    <button type="button" role="menuitem" onclick={() => void openModelDialog()}>
                      <span class="tool-icon"><svg viewBox="0 0 20 20"><path d="M5 5.5 10 3l5 2.5v9L10 17l-5-2.5zM5 5.5l5 2.5 5-2.5M10 8v9" /></svg></span>
                      <span><strong>{tr("Model and effort", "Modelo e effort")}</strong><small>{tr("Configure", "Configurar")}</small></span>
                      <svg class="tool-chevron" viewBox="0 0 20 20"><path d="m8 5 5 5-5 5" /></svg>
                    </button>
                  {/if}
                </div>
              {/if}
              {#if collaborationModeChanging && collaborationModeTarget}
                <span class="mode-feedback" aria-live="polite">
                  {collaborationModeTarget === "plan"
                    ? tr("Switching to Plan…", "Mudando para Plan…")
                    : tr("Switching to Default…", "Mudando para Default…")}
                </span>
              {:else if collaborationModeNotice}
                <span class="mode-feedback success" aria-live="polite">{collaborationModeNotice}</span>
              {/if}
            </div>
          {/if}
          <textarea
            bind:this={promptInput}
            bind:value={prompt}
            disabled={!canCompose || !readyForPrompt || sending}
            oninput={handlePromptInput}
            onkeydown={sendPromptOnEnter}
            rows="2"
            aria-label={tr(`Prompt for ${sessionDisplayName(session)}`, `Prompt para ${sessionDisplayName(session)}`)}
            placeholder={sending ? tr("Sending prompt…", "Enviando prompt…") : !canSubmit ? promptUnavailableText() : canSendWhileRunning ? tr("Write the next prompt and press Enter to queue…", "Escreva o próximo prompt e pressione Enter para adicionar à fila…") : readyForPrompt ? tr(`Prompt for ${sessionDisplayName(session)}…`, `Prompt para ${sessionDisplayName(session)}…`) : tr("Agent is running…", "Agente em execução…")}
          ></textarea>
          {#if promptIsRunning}
            <button
              class="interrupt-submit"
              disabled={!canInterruptRunningPrompt || interrupting}
              type="button"
              onclick={() => void interruptAgentPrompt()}
              aria-keyshortcuts="Escape"
              aria-label={interrupting ? tr("Interrupting prompt", "Interrompendo prompt") : canInterruptRunningPrompt ? tr("Interrupt current prompt", "Interromper prompt atual") : tr("Prompt interruption unavailable", "Interrupção do prompt indisponível")}
              title={canInterruptRunningPrompt ? tr("Interrupt prompt (Esc)", "Interromper prompt (Esc)") : tr("This source cannot be interrupted safely", "Esta origem não pode ser interrompida com segurança")}
            >
              {#if interrupting}
                <span class="send-spinner" aria-hidden="true"></span>
              {:else}
                <svg viewBox="0 0 20 20"><rect x="6" y="6" width="8" height="8" rx="1"></rect></svg>
              {/if}
            </button>
          {:else if canCompose}
            <button disabled={(!prompt.trim() && promptAttachments.length === 0) || !readyForPrompt || sending} type="submit" aria-label={sending ? tr("Sending prompt", "Enviando prompt") : tr("Send prompt", "Enviar prompt")}>
              {#if sending}
                <span class="send-spinner" aria-hidden="true"></span>
              {:else}
                <svg viewBox="0 0 20 20"><path d="m4 10 12-6-4 12-2-4zM10 12l2-2" /></svg>
              {/if}
            </button>
          {:else}
            <button type="button" onclick={openOrigin} aria-label={tr("Open source", "Abrir origem")}>
              <svg viewBox="0 0 20 20"><path d="M7 5h8v8M14.5 5.5 6 14" /></svg>
            </button>
          {/if}
        </div>
      </form>
      {#if message}<p class="message">{message}</p>{/if}
      {#if !windowState?.workflowBridgeOpen}
        <button class="resize-handle resize-nw" type="button" tabindex="-1" aria-label={tr("Resize from top-left corner", "Redimensionar pelo canto superior esquerdo")} onpointerdown={(event) => void beginResize(event, "NorthWest")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
        <button class="resize-handle resize-ne" type="button" tabindex="-1" aria-label={tr("Resize from top-right corner", "Redimensionar pelo canto superior direito")} onpointerdown={(event) => void beginResize(event, "NorthEast")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
        <button class="resize-handle resize-sw" type="button" tabindex="-1" aria-label={tr("Resize from bottom-left corner", "Redimensionar pelo canto inferior esquerdo")} onpointerdown={(event) => void beginResize(event, "SouthWest")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
        <button class="resize-handle resize-se" type="button" tabindex="-1" aria-label={tr("Resize from bottom-right corner", "Redimensionar pelo canto inferior direito")} onpointerdown={(event) => void beginResize(event, "SouthEast")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
      {/if}
    </section>
  {:else if initializationError}
    <section class="terminal-card loading">
      <LumeLogo size={34} />
      <span>{initializationError}</span>
      <div class="loading-actions">
        <button type="button" onclick={() => void initializeTerminal()}>{tr("Try again", "Tentar novamente")}</button>
        <button type="button" onclick={() => void closeTerminal()}>{tr("Close", "Fechar")}</button>
      </div>
    </section>
  {:else}
    <section class="terminal-card loading">
      <button class="loading-close" type="button" onclick={() => void closeTerminal()} aria-label={tr("Close terminal", "Fechar terminal")}>
        <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8" /></svg>
      </button>
      <LumeLogo size={34} />
      <span>{tr("Connecting to session…", "Conectando à sessão…")}</span>
    </section>
  {/if}
</main>

<style>
  .terminal-window { width: 100%; height: 100%; container-type: inline-size; --terminal-scroll-thumb: #b9c6bf; --terminal-scroll-thumb-hover: #8fa79b; scrollbar-width: thin; scrollbar-color: var(--terminal-scroll-thumb) transparent; }
  .terminal-window :global(*) { scrollbar-width: thin; scrollbar-color: var(--terminal-scroll-thumb) transparent; }
  .terminal-window :global(*::-webkit-scrollbar) { width: 5px; height: 5px; background: transparent; }
  .terminal-window :global(*::-webkit-scrollbar-button) { width: 0; height: 0; display: none; }
  .terminal-window :global(*::-webkit-scrollbar-track),
  .terminal-window :global(*::-webkit-scrollbar-corner) { background: transparent; }
  .terminal-window :global(*::-webkit-scrollbar-thumb) { border-radius: 999px; background: var(--terminal-scroll-thumb); }
  .terminal-window :global(*::-webkit-scrollbar-thumb:hover) { background: var(--terminal-scroll-thumb-hover); }
  .terminal-card { position: relative; width: 100%; height: 100%; display: flex; flex-direction: column; overflow: hidden; container-type: inline-size; --chat-font-adjust: 0px; --chat-small-font-adjust: 0px; --chat-tiny-font-adjust: 0px; --chat-font-size: calc(9px + var(--chat-font-adjust)); --chat-small-font-size: calc(8px + var(--chat-small-font-adjust)); --chat-tiny-font-size: calc(7px + var(--chat-tiny-font-adjust)); border: 1px solid rgba(103, 126, 116, 0.2); border-radius: 17px; color: #26342e; background: #f8fbf9; box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.32); transition: border-color 150ms ease, box-shadow 180ms ease, background-color 180ms ease, transform 180ms cubic-bezier(0.22, 1, 0.36, 1); }
  @container (min-width: 520px) {
    .terminal-card { --chat-font-size: calc(10px + var(--chat-font-adjust)); --chat-small-font-size: calc(9px + var(--chat-small-font-adjust)); --chat-tiny-font-size: calc(8px + var(--chat-tiny-font-adjust)); }
  }
  @container (min-width: 700px) {
    .terminal-card { --chat-font-size: calc(11px + var(--chat-font-adjust)); --chat-small-font-size: calc(10px + var(--chat-small-font-adjust)); --chat-tiny-font-size: calc(8.5px + var(--chat-tiny-font-adjust)); }
  }
  @container (min-width: 880px) {
    .terminal-card { --chat-font-size: calc(12px + var(--chat-font-adjust)); --chat-small-font-size: calc(10.5px + var(--chat-small-font-adjust)); --chat-tiny-font-size: calc(9px + var(--chat-tiny-font-adjust)); }
  }
  .terminal-card > header { min-height: 48px; padding: 7px 8px 7px 9px; display: flex; align-items: center; gap: 7px; border-bottom: 1px solid rgba(97, 119, 109, 0.11); cursor: grab; touch-action: none; }
  .terminal-card.dragging > header { cursor: grabbing; }
  .terminal-card.bridge-locked > header { cursor: default; }
  .terminal-card.resizing { user-select: none; }
  .terminal-card.header-menu-open .workflow-role-control { opacity: 0; pointer-events: none; }
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
  .terminal-card.workflow-preview { transition: border-color 90ms ease, transform 110ms cubic-bezier(0.22, 0.8, 0.3, 1); }
  .terminal-card.workflow-preview.dock-target,
  .terminal-card.workflow-preview.dock-moving { border-color: rgba(103, 126, 116, 0.2); box-shadow: none; transform: none; }
  .terminal-card.workflow-preview.dock-target.dock-left { transform: translateX(calc(var(--dock-tension) * -1.1px)) scaleX(calc(1 + var(--dock-tension) * 0.012)) scaleY(calc(1 - var(--dock-tension) * 0.0035)); transform-origin: right center; }
  .terminal-card.workflow-preview.dock-moving.dock-left { transform: translateX(calc(var(--dock-tension) * 1.1px)) scaleX(calc(1 + var(--dock-tension) * 0.012)) scaleY(calc(1 - var(--dock-tension) * 0.0035)); transform-origin: left center; }
  .terminal-card.workflow-preview.dock-target.dock-right { transform: translateX(calc(var(--dock-tension) * 1.1px)) scaleX(calc(1 + var(--dock-tension) * 0.012)) scaleY(calc(1 - var(--dock-tension) * 0.0035)); transform-origin: left center; }
  .terminal-card.workflow-preview.dock-moving.dock-right { transform: translateX(calc(var(--dock-tension) * -1.1px)) scaleX(calc(1 + var(--dock-tension) * 0.012)) scaleY(calc(1 - var(--dock-tension) * 0.0035)); transform-origin: right center; }
  .terminal-card.workflow-preview.dock-target.dock-top { transform: translateY(calc(var(--dock-tension) * -1.1px)) scaleY(calc(1 + var(--dock-tension) * 0.012)) scaleX(calc(1 - var(--dock-tension) * 0.0035)); transform-origin: center bottom; }
  .terminal-card.workflow-preview.dock-moving.dock-top { transform: translateY(calc(var(--dock-tension) * 1.1px)) scaleY(calc(1 + var(--dock-tension) * 0.012)) scaleX(calc(1 - var(--dock-tension) * 0.0035)); transform-origin: center top; }
  .terminal-card.workflow-preview.dock-target.dock-bottom { transform: translateY(calc(var(--dock-tension) * 1.1px)) scaleY(calc(1 + var(--dock-tension) * 0.012)) scaleX(calc(1 - var(--dock-tension) * 0.0035)); transform-origin: center top; }
  .terminal-card.workflow-preview.dock-moving.dock-bottom { transform: translateY(calc(var(--dock-tension) * -1.1px)) scaleY(calc(1 + var(--dock-tension) * 0.012)) scaleX(calc(1 - var(--dock-tension) * 0.0035)); transform-origin: center bottom; }
  .terminal-card.workflow-mode.settling { animation: workflow-dock-settle 360ms cubic-bezier(0.2, 0.85, 0.3, 1) 1; }
  @keyframes workflow-dock-settle {
    0% { transform: scale(0.996); }
    42% { transform: scale(1.003); }
    72% { transform: scale(0.999); }
    100% { transform: scale(1); }
  }
  .terminal-card.normal-preview.dock-target,
  .terminal-card.normal-preview.dock-moving { border-color: rgba(103, 126, 116, 0.2); box-shadow: none; transform: none; }
  .terminal-card.normal-preview .workflow-merge-preview { opacity: calc(0.1 + var(--dock-proximity) * 0.5); }
  .terminal-card.normal-preview .workflow-merge-preview::before { background: radial-gradient(ellipse at center, rgba(116, 151, 134, calc(0.1 + var(--dock-proximity) * 0.24)) 0 28%, rgba(82, 126, 105, calc(0.06 + var(--dock-proximity) * 0.15)) 56%, transparent 82%); filter: saturate(0.45); }
  .terminal-card.normal-preview .workflow-merge-preview::after { background: rgba(105, 148, 127, calc(0.17 + var(--dock-proximity) * 0.34)); }
  .terminal-card.normal-preview.dock-ready .workflow-merge-preview::before { filter: saturate(0.5) brightness(1.04); }
  .terminal-card.normal-preview.dock-ready .workflow-merge-preview::after { background: rgba(124, 166, 145, 0.72); transform: scale(1.05); }
  .workflow-merge-preview { position: absolute; z-index: 31; pointer-events: none; opacity: calc(0.14 + var(--dock-proximity) * 0.64); transition: width 55ms linear, height 55ms linear, opacity 55ms linear; }
  .workflow-merge-preview::before,
  .workflow-merge-preview::after { position: absolute; content: ""; pointer-events: none; }
  .workflow-merge-preview::before { background: radial-gradient(ellipse at center, rgba(91, 164, 126, calc(0.12 + var(--dock-proximity) * 0.3)) 0 28%, rgba(53, 137, 94, calc(0.08 + var(--dock-proximity) * 0.2)) 56%, transparent 82%); filter: saturate(0.86); transition: background 55ms linear, filter 55ms linear; }
  .workflow-merge-preview::after { border-radius: 999px; background: rgba(91, 165, 126, calc(0.2 + var(--dock-proximity) * 0.42)); transition: background 55ms linear, transform 55ms linear; }
  .dock-target.dock-left .workflow-merge-preview,
  .dock-moving.dock-right .workflow-merge-preview { left: 0; }
  .dock-target.dock-right .workflow-merge-preview,
  .dock-moving.dock-left .workflow-merge-preview { right: 0; }
  .dock-target.dock-left .workflow-merge-preview,
  .dock-target.dock-right .workflow-merge-preview,
  .dock-moving.dock-left .workflow-merge-preview,
  .dock-moving.dock-right .workflow-merge-preview { top: calc(23% - var(--dock-pull) * 0.25); bottom: calc(23% - var(--dock-pull) * 0.25); width: calc(11px + var(--dock-pull) * 1.02); }
  .dock-target.dock-left .workflow-merge-preview::before,
  .dock-target.dock-right .workflow-merge-preview::before,
  .dock-moving.dock-left .workflow-merge-preview::before,
  .dock-moving.dock-right .workflow-merge-preview::before { inset: 5% -2px; border-radius: 22% 92% 92% 22% / 48% 52% 52% 48%; }
  .dock-target.dock-right .workflow-merge-preview::before,
  .dock-moving.dock-left .workflow-merge-preview::before { border-radius: 92% 22% 22% 92% / 52% 48% 48% 52%; }
  .dock-target.dock-left .workflow-merge-preview::after,
  .dock-moving.dock-right .workflow-merge-preview::after { top: 29%; bottom: 29%; left: 0; width: 1px; }
  .dock-target.dock-right .workflow-merge-preview::after,
  .dock-moving.dock-left .workflow-merge-preview::after { top: 29%; right: 0; bottom: 29%; width: 1px; }
  .dock-target.dock-top .workflow-merge-preview,
  .dock-moving.dock-bottom .workflow-merge-preview { top: 0; }
  .dock-target.dock-bottom .workflow-merge-preview,
  .dock-moving.dock-top .workflow-merge-preview { bottom: 0; }
  .dock-target.dock-top .workflow-merge-preview,
  .dock-target.dock-bottom .workflow-merge-preview,
  .dock-moving.dock-top .workflow-merge-preview,
  .dock-moving.dock-bottom .workflow-merge-preview { right: calc(23% - var(--dock-pull) * 0.25); left: calc(23% - var(--dock-pull) * 0.25); height: calc(11px + var(--dock-pull) * 1.02); }
  .dock-target.dock-top .workflow-merge-preview::before,
  .dock-target.dock-bottom .workflow-merge-preview::before,
  .dock-moving.dock-top .workflow-merge-preview::before,
  .dock-moving.dock-bottom .workflow-merge-preview::before { inset: -2px 5%; border-radius: 48% 48% 52% 52% / 22% 22% 92% 92%; }
  .dock-target.dock-top .workflow-merge-preview::before,
  .dock-moving.dock-bottom .workflow-merge-preview::before { border-radius: 48% 48% 52% 52% / 22% 22% 92% 92%; }
  .dock-target.dock-bottom .workflow-merge-preview::before,
  .dock-moving.dock-top .workflow-merge-preview::before { border-radius: 52% 52% 48% 48% / 92% 92% 22% 22%; }
  .dock-target.dock-top .workflow-merge-preview::after,
  .dock-moving.dock-bottom .workflow-merge-preview::after { top: 0; right: 29%; left: 29%; height: 1px; }
  .dock-target.dock-bottom .workflow-merge-preview::after,
  .dock-moving.dock-top .workflow-merge-preview::after { right: 29%; bottom: 0; left: 29%; height: 1px; }
  .dock-ready .workflow-merge-preview { opacity: 0.92; }
  .dock-ready .workflow-merge-preview::before { filter: saturate(0.92) brightness(1.08); }
  .dock-ready .workflow-merge-preview::after { background: rgba(112, 186, 145, 0.84); transform: scale(1.08); }
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
  .identity strong { overflow: hidden; color: #26342e; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
  .identity small { overflow: hidden; color: #829089; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .terminal-name-editor { min-width: 100px; display: flex; flex: 1; align-items: center; gap: 3px; }
  .terminal-name-editor input { min-width: 0; height: 27px; flex: 1; padding: 0 7px; border: 1px solid rgba(67, 116, 94, 0.25); border-radius: 7px; color: #26342e; background: rgba(255, 255, 255, 0.68); font: 650 10px Inter, sans-serif; outline: none; }
  .terminal-name-editor input:focus { border-color: rgba(48, 139, 96, 0.5); box-shadow: 0 0 0 2px rgba(48, 139, 96, 0.08); }
  .terminal-name-editor button { width: 22px; height: 22px; }
  .source-badge { padding: 3px 5px; display: inline-flex; align-items: center; gap: 3px; border-radius: 999px; color: #718079; background: rgba(80, 104, 94, 0.075); font-size: 7px; font-weight: 760; letter-spacing: 0.04em; text-transform: uppercase; }
  .access-badge { padding: 3px 5px; display: inline-flex; flex: 0 0 auto; align-items: center; gap: 3px; border-radius: 999px; font-size: 7px; font-weight: 780; line-height: 1; white-space: nowrap; }
  .access-badge svg { width: 9px; height: 9px; flex: 0 0 auto; fill: none; stroke: currentColor; stroke-width: 1.35; }
  .access-badge.auto-review { color: #315f86; background: #cbdff0; }
  .access-badge.auto-review svg { fill: currentColor; stroke: none; }
  .access-badge.full-access { color: #764c2e; background: #e8ceb1; }
  .access-badge.external-session { color: #405d50; background: #c7d6ca; }
  .workflow-role-control { position: relative; z-index: 82; height: 0; display: flex; flex: 0 0 0; justify-content: center; overflow: visible; transition: opacity 100ms ease; }
  .workflow-role-fab { position: relative; top: 7px; width: 34px; height: 34px; padding: 0; display: grid; place-items: center; border: 1px solid rgba(54, 148, 101, 0.24); border-radius: 50%; color: #47755f; background: radial-gradient(circle at 34% 24%, rgba(255,255,255,.96) 0 13%, rgba(255,255,255,.34) 31%, transparent 52%), linear-gradient(145deg, #fbfdfc 12%, #e5efe9 88%); box-shadow: inset 0 1px 0 rgba(255,255,255,.95), inset 0 -3px 5px rgba(43,91,67,.1), 0 5px 10px rgba(24,58,41,.18), 0 1px 2px rgba(24,58,41,.14), 0 0 0 3px rgba(59,151,104,.045); cursor: pointer; transition: border-color 150ms ease, box-shadow 170ms ease, transform 170ms cubic-bezier(.2,.8,.2,1); }
  .workflow-role-fab:hover { border-color: rgba(54, 148, 101, 0.38); box-shadow: inset 0 1px 0 rgba(255,255,255,.98), inset 0 -3px 5px rgba(43,91,67,.11), 0 7px 14px rgba(24,58,41,.2), 0 2px 3px rgba(24,58,41,.14), 0 0 0 4px rgba(59,151,104,.07); transform: translateY(-2px); }
  .workflow-role-fab:active { box-shadow: inset 0 2px 5px rgba(38,80,59,.14), 0 2px 5px rgba(24,58,41,.16), 0 0 0 3px rgba(59,151,104,.055); transform: translateY(1px) scale(.98); }
  .workflow-role-fab.open { border-color: rgba(54, 148, 101, 0.48); box-shadow: inset 0 1px 0 rgba(255,255,255,.95), inset 0 -3px 5px rgba(43,91,67,.1), 0 7px 16px rgba(24,58,41,.19), 0 0 0 4px rgba(59,151,104,.09); }
  .workflow-role-fab.unconfigured { border-style: dashed; color: #77877f; }
  .workflow-role-fab > .role-symbol { width: 30px; height: 30px; border: 0; border-radius: 50%; background: transparent; transform: scale(.9); }
  .workflow-step-badge { position: absolute; top: -3px; right: -4px; min-width: 14px; height: 14px; padding: 0 3px; display: grid; place-items: center; border: 1px solid rgba(255,255,255,.86); border-radius: 999px; color: #fff; background: #398b63; box-shadow: 0 2px 5px rgba(29,74,51,.24); font: 800 7px/1 Inter, sans-serif; }
  .workflow-role-tooltip { position: absolute; top: 39px; left: 50%; min-width: max-content; padding: 5px 8px; display: grid; gap: 1px; border: 1px solid rgba(70, 112, 91, 0.14); border-radius: 8px; color: #50665a; background: rgba(248, 251, 249, 0.98); box-shadow: 0 8px 20px rgba(24, 51, 38, 0.14); opacity: 0; pointer-events: none; transform: translate(-50%, -4px) scale(.96); transition: opacity 130ms ease, transform 150ms cubic-bezier(.2,.8,.2,1); }
  .workflow-role-tooltip strong { font: 780 8px/1.25 Inter, sans-serif; }
  .workflow-role-tooltip small { color: #829089; font: 690 7px/1.25 Inter, sans-serif; }
  .workflow-role-fab:hover:not(.open) .workflow-role-tooltip,
  .workflow-role-fab:focus-visible:not(.open) .workflow-role-tooltip { opacity: 1; transform: translate(-50%, 0) scale(1); }
  .workflow-role-popover { position: fixed; z-index: 85; top: 118px; left: 50%; width: min(420px, calc(100vw - 16px)); max-height: calc(100dvh - 126px); padding: 10px; display: flex; flex-direction: column; gap: 8px; overflow: visible; border: 1px solid rgba(70, 112, 91, 0.17); border-radius: 13px; color: #34463d; background: #f8fbf9; box-shadow: 0 18px 42px rgba(20, 39, 29, 0.22); cursor: default; transform: translateX(-50%); }
  .workflow-popover-bridge { position: absolute; top: -8px; left: calc(50% - 10px); width: 20px; height: 9px; overflow: hidden; }
  .workflow-popover-bridge::before { position: absolute; right: 4px; bottom: -6px; width: 12px; height: 12px; content: ""; border: 1px solid rgba(70, 112, 91, 0.17); background: #f8fbf9; transform: rotate(45deg); }
  .workflow-role-popover.above .workflow-popover-bridge { top: auto; bottom: -8px; }
  .workflow-role-popover.above .workflow-popover-bridge::before { top: -6px; bottom: auto; }
  .workflow-role-popover.constrained { overflow: hidden; }
  .workflow-role-popover.constrained .workflow-popover-bridge { display: none; }
  .workflow-role-popover.constrained .workflow-popover-heading { display: none; }
  .workflow-popover-heading { min-height: 32px; display: flex; align-items: center; gap: 8px; }
  .workflow-popover-heading > span { min-width: 0; flex: 1; display: grid; gap: 1px; }
  .workflow-popover-heading small { color: #71867b; font: 750 7px Inter, sans-serif; letter-spacing: 0.06em; text-transform: uppercase; }
  .workflow-popover-heading strong { overflow: hidden; color: #31483c; font: 800 11px Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .workflow-popover-actions { display: flex; justify-content: flex-end; gap: 5px; }
  .workflow-popover-actions button { width: auto; min-width: 62px; height: 27px; padding: 0 8px; border: 1px solid rgba(81, 112, 97, 0.14); border-radius: 8px; color: #60736a; background: transparent; font: 750 8px Inter, sans-serif; cursor: pointer; }
  .workflow-popover-actions button.primary { color: white; border-color: #32895f; background: #32895f; }
  .workflow-popover-actions button:disabled { opacity: 0.45; cursor: default; }
  .rate-limit-meter { width: clamp(58px, 21cqw, 96px); min-width: 58px; display: grid; flex: 0 1 auto; gap: 3px; color: #438161; pointer-events: none; }
  .rate-limit-meter > span { display: flex; align-items: baseline; justify-content: space-between; gap: 4px; font: 750 7px Inter, sans-serif; white-space: nowrap; }
  .rate-limit-meter b { color: currentColor; font-size: 8px; }
  .rate-limit-meter small { color: #87938d; font-size: 6px; font-weight: 700; text-transform: uppercase; }
  .rate-limit-meter > i { height: 3px; overflow: hidden; border-radius: 999px; background: rgba(65, 130, 95, 0.12); }
  .rate-limit-meter > i > em { height: 100%; display: block; border-radius: inherit; background: currentColor; transition: width 280ms ease, background-color 180ms ease; }
  .rate-limit-meter.warning { color: #b4812f; }
  .rate-limit-meter.danger { color: #b65656; }
  header button { position: relative; z-index: 25; width: 25px; height: 25px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 7px; color: #73817b; background: transparent; cursor: pointer; }
  header button:hover { color: #43574e; background: rgba(72, 99, 87, 0.07); }
  header button.active { color: #347b5b; background: rgba(52, 139, 94, 0.09); }
  .header-actions { display: flex; flex: 0 0 auto; align-items: center; gap: 2px; }
  .header-overflow { position: relative; z-index: 60; display: flex; flex: 0 0 auto; }
  .header-actions-menu { position: absolute; z-index: 70; top: 30px; right: 0; width: 190px; padding: 5px; display: grid; gap: 2px; border: 1px solid rgba(80, 105, 94, 0.14); border-radius: 10px; color: #53665d; background: rgba(248, 251, 249, 0.98); box-shadow: 0 10px 28px rgba(30, 55, 43, 0.17); cursor: default; }
  header .header-actions-menu > button { z-index: auto; width: 100%; min-height: 29px; height: auto; padding: 0 8px; display: flex; justify-content: flex-start; gap: 8px; border-radius: 7px; color: #53665d; font: 700 8px Inter, sans-serif; text-align: left; }
  header .header-actions-menu > button:hover { color: #287452; background: rgba(57, 145, 99, 0.08); }
  header .header-actions-menu > button.danger { color: #9d615c; }
  header .header-actions-menu > button.terminal-stop-menu { margin-top: 3px; box-shadow: 0 -1px rgba(80, 105, 94, 0.11); }
  header .header-actions-menu > button.compact-only { display: none; }
  .header-actions-menu > button svg { width: 13px; height: 13px; flex: 0 0 auto; }
  .header-menu-zoom { box-sizing: border-box; width: 100%; min-height: 29px; padding: 0 4px 0 8px; display: grid; grid-template-columns: minmax(0, 1fr) 23px 34px 23px; align-items: center; gap: 2px; color: #53665d; font: 700 8px Inter, sans-serif; }
  .header-menu-zoom > span { min-width: 0; display: flex; align-items: center; gap: 8px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .header-menu-zoom > span svg { width: 13px; height: 13px; flex: 0 0 auto; }
  header .header-menu-zoom > button { z-index: auto; width: 23px; height: 23px; border-radius: 6px; color: #4b6c5d; background: rgba(73, 110, 93, 0.055); font: 800 12px/1 Inter, sans-serif; }
  header .header-menu-zoom > button:hover { color: #287452; background: rgba(57, 145, 99, 0.1); }
  header .header-menu-zoom > button:disabled { opacity: 0.32; cursor: default; }
  .header-menu-zoom output { color: #687970; font: 750 8px Inter, sans-serif; text-align: center; }
  @container (max-width: 480px) {
    .header-actions { display: none; }
    .header-overflow { display: flex; }
    header .header-actions-menu > button.compact-only { display: flex; }
    .source-badge,
    .access-badge { width: 20px; height: 20px; padding: 0; justify-content: center; gap: 0; }
    .source-badge > .badge-label,
    .access-badge > .badge-label { display: none; }
  }
  .dock-button { color: #4a7564; }
  .workflow-connection { position: absolute; z-index: 25; overflow: hidden; background: linear-gradient(180deg, rgba(62, 162, 111, 0.14), rgba(82, 185, 130, 0.72) 45%, rgba(62, 162, 111, 0.14)); pointer-events: none; opacity: 0.62; animation: workflow-connect-confirm 620ms cubic-bezier(0.22, 1, 0.36, 1) 1; }
  .workflow-bridge-link { opacity: .92; box-shadow: 0 0 10px rgba(68, 183, 123, .42); animation: workflow-bridge-current 2.2s ease-in-out infinite; }
  .workflow-left, .workflow-right { top: 18px; bottom: 18px; width: 3px; }
  .workflow-left { left: 0; border-radius: 0 999px 999px 0; }
  .workflow-right { right: 0; border-radius: 999px 0 0 999px; }
  .workflow-top, .workflow-bottom { right: 18px; left: 18px; height: 3px; background: linear-gradient(90deg, rgba(62, 162, 111, 0.18), #52b982 45%, rgba(62, 162, 111, 0.18)); }
  .workflow-top { top: 0; border-radius: 0 0 999px 999px; }
  .workflow-bottom { bottom: 0; border-radius: 999px 999px 0 0; }
  .workflow-connection-control { position: absolute; z-index: 60; display: grid; place-items: center; }
  .workflow-control-right { top: 50%; right: -21px; width: 42px; height: 88px; transform: translateY(-50%); }
  .workflow-control-left { top: 50%; left: -21px; width: 42px; height: 88px; transform: translateY(-50%); }
  .workflow-control-bottom { bottom: -21px; left: 50%; width: 88px; height: 42px; transform: translateX(-50%); }
  .workflow-control-top { top: -21px; left: 50%; width: 88px; height: 42px; transform: translateX(-50%); }
  .workflow-connection-editor { position: relative; width: 27px; height: 27px; padding: 6px; display: grid; place-items: center; border: 1px solid rgba(70, 161, 111, .3); border-radius: 50%; color: #36865d; background: radial-gradient(circle at 35% 28%, rgba(255,255,255,.96), rgba(240,249,244,.96) 58%, rgba(213,237,224,.98)); box-shadow: 0 4px 13px rgba(30,70,49,.2), 0 0 0 3px rgba(74,174,121,.055); cursor: pointer; opacity: 0; pointer-events: auto; transform: scale(.82); transform-origin: center; transition: opacity 130ms ease, transform 160ms cubic-bezier(.2,.8,.2,1), filter 150ms ease, box-shadow 150ms ease; }
  .workflow-connection-editor svg { width: 14px; height: 14px; }
  .workflow-connection-editor.loading { opacity: 1; cursor: wait; }
  .workflow-connection-control.visible .workflow-connection-editor,
  .workflow-connection-editor:focus-visible { opacity: 1; transform: scale(1); outline: none; filter: brightness(1.05); box-shadow: 0 5px 16px rgba(34,112,70,.26), 0 0 0 3px rgba(74,174,121,.1); }
  .workflow-control-right .workflow-connection-editor.loading { transform: translateX(-14px) scale(1); }
  .workflow-control-left .workflow-connection-editor.loading { transform: translateX(14px) scale(1); }
  .workflow-control-bottom .workflow-connection-editor.loading { transform: translateY(-14px) scale(1); }
  .workflow-control-top .workflow-connection-editor.loading { transform: translateY(14px) scale(1); }
  @keyframes workflow-connect-confirm {
    0% { opacity: 0.18; filter: brightness(0.85); }
    48% { opacity: 1; filter: brightness(1.55); }
    100% { opacity: 0.62; filter: brightness(1); }
  }
  @keyframes workflow-bridge-current {
    0%, 100% { opacity: .7; }
    50% { opacity: 1; }
  }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.7; }
  .work-tray { overflow: hidden; border-bottom: 1px solid rgba(97, 119, 109, 0.09); background: rgba(63, 91, 78, 0.022); }
  .work-tray-toggle { width: 100%; min-height: 27px; padding: 4px 9px; display: flex; align-items: center; gap: 7px; border: 0; color: #62756c; background: transparent; cursor: pointer; }
  .work-tray-toggle:hover { background: rgba(65, 100, 83, 0.035); }
  .work-tray-toggle > strong { flex: 0 0 auto; font: 800 var(--chat-tiny-font-size) Inter, sans-serif; letter-spacing: 0.1em; text-transform: uppercase; }
  .work-tray-toggle > span { min-width: 0; display: flex; flex: 1; justify-content: flex-end; gap: 5px; overflow: hidden; }
  .work-tray-toggle small { min-width: 0; overflow: hidden; color: #83928b; font: 680 var(--chat-tiny-font-size) Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .work-tray-toggle svg { width: 11px; height: 11px; flex: 0 0 auto; transition: transform 180ms cubic-bezier(0.22, 1, 0.36, 1); }
  .work-tray.collapsed .work-tray-toggle svg { transform: rotate(-90deg); }
  .work-tray-body { max-height: 150px; overflow: hidden; opacity: 1; transition: max-height 200ms cubic-bezier(0.22, 1, 0.36, 1), opacity 140ms ease; }
  .work-tray.collapsed .work-tray-body { max-height: 0; opacity: 0; pointer-events: none; }
  .work-tray-grid { max-height: 145px; padding: 0 8px 6px; display: grid; grid-template-columns: repeat(auto-fit, minmax(145px, 1fr)); gap: 5px; overflow-x: hidden; overflow-y: auto; }
  .work-card { min-width: 0; padding: 6px 7px; display: grid; align-content: start; gap: 4px; overflow: hidden; border: 1px solid rgba(78, 106, 93, 0.09); border-radius: 8px; background: rgba(255, 255, 255, 0.34); }
  .work-card-heading { min-width: 0; display: flex; align-items: center; justify-content: space-between; gap: 5px; }
  .work-card-heading strong { color: #587064; font: 800 var(--chat-tiny-font-size) Inter, sans-serif; letter-spacing: 0.12em; }
  .work-card-heading span { padding: 2px 4px; border-radius: 999px; color: #4e8068; background: rgba(57, 145, 99, 0.08); font: 750 var(--chat-tiny-font-size) Inter, sans-serif; white-space: nowrap; }
  .work-card-heading span.complete { color: #3c8460; background: rgba(50, 153, 99, 0.1); }
  .work-card-heading span.blocked { color: #a15e58; background: rgba(170, 78, 78, 0.08); }
  .todo-progress { height: 2px; overflow: hidden; border-radius: 999px; background: rgba(69, 112, 91, 0.1); }
  .todo-progress em { width: var(--todo-progress); height: 100%; display: block; border-radius: inherit; background: #4d9e76; transition: width 240ms ease; }
  .todo-card ul { min-width: 0; margin: 0; padding: 0; display: grid; gap: 2px; list-style: none; }
  .todo-card li { min-width: 0; display: flex; align-items: center; gap: 5px; color: #88948e; }
  .todo-card li > span { width: 5px; height: 5px; flex: 0 0 auto; border: 1px solid #98a49e; border-radius: 50%; }
  .todo-card li.active { color: #4b755f; }
  .todo-card li.active > span { border-color: #4d98c3; background: #4d98c3; box-shadow: 0 0 0 2px rgba(77, 152, 195, 0.1); }
  .todo-card li.done { color: #91a099; }
  .todo-card li.done > span { border-color: #55a77b; background: #55a77b; }
  .todo-card li small { min-width: 0; overflow: hidden; font: 650 var(--chat-small-font-size)/1.35 Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .work-more { color: #96a19c; font: 650 var(--chat-tiny-font-size) Inter, sans-serif; }
  .goal-card p { min-width: 0; margin: 0; overflow: hidden; color: #4b5f55; font: 680 var(--chat-small-font-size)/1.35 Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .goal-time { display: flex; align-items: center; gap: 3px; color: #86948d; font: 650 var(--chat-tiny-font-size) Inter, sans-serif; }
  .goal-time svg { width: 9px; height: 9px; }
  .hub-tabs { min-height: 29px; padding: 4px 9px 0; display: flex; gap: 3px; overflow-x: auto; border-bottom: 1px solid rgba(97, 119, 109, 0.09); scrollbar-width: none; }
  .hub-tabs::-webkit-scrollbar { display: none; }
  .hub-tabs button { min-width: 0; padding: 0 7px 4px; border: 0; border-bottom: 2px solid transparent; color: #8b9791; background: transparent; font: 700 var(--chat-small-font-size) Inter, sans-serif; cursor: pointer; }
  .hub-tabs button.active { color: #39785d; border-bottom-color: #3b9c70; }
  .hub-tabs span { min-width: 14px; height: 14px; padding: 0 4px; display: inline-grid; place-items: center; border-radius: 999px; color: #72827a; background: rgba(76, 101, 90, 0.075); font-size: var(--chat-tiny-font-size); }
  .terminal-output { min-width: 0; min-height: 0; max-width: 100%; flex: 1; padding: 10px 12px 7px; overflow-x: hidden; overflow-y: auto; color: #55635d; background: linear-gradient(180deg, rgba(61, 87, 75, 0.025), transparent); font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace; font-size: var(--chat-font-size); }
  .terminal-output.with-workflow-role { padding-top: 49px; }
  .terminal-output p { max-width: 100%; margin: 0 0 6px; overflow-wrap: anywhere; line-height: 1.45; word-break: break-word; }
  .terminal-output p > span { color: #36a269; font-weight: 800; }
  .terminal-output i { color: #8a9690; font-style: normal; }
  .load-earlier-chat { width: min(240px, 88%); min-height: 29px; margin: 1px auto 8px; padding: 0 8px; display: flex; align-items: center; justify-content: center; gap: 6px; border: 1px solid rgba(77, 104, 91, 0.1); border-radius: 8px; color: #6d7e76; background: rgba(69, 99, 84, 0.025); font: 700 var(--chat-tiny-font-size) Inter, sans-serif; cursor: pointer; }
  .load-earlier-chat:hover { color: #37785a; border-color: rgba(52, 139, 94, 0.18); background: rgba(52, 139, 94, 0.045); }
  .load-earlier-chat svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.5; }
  .load-earlier-chat small { padding: 2px 5px; border-radius: 999px; color: #7e8d86; background: rgba(77, 104, 91, 0.06); }
  .status-running, .status-running span { color: #4e7faf; }
  .status-permission_required, .status-permission_required span { color: #b06b25; }
  .status-waiting_for_input, .status-waiting_for_input span { color: #b0812d; }
  .status-completed, .status-completed span { color: #55a473; }
  .status-failed, .status-failed span { color: #ad4f4f; }
  .chat-feed { min-width: 0; max-width: 100%; margin: 9px 0 7px; display: grid; gap: 7px; overflow-x: hidden; }
  .chat-feed > * { min-width: 0; max-width: 100%; }
  .chat-message { box-sizing: border-box; width: fit-content; min-width: 0; max-width: 94%; padding: 7px 8px; overflow: clip; overflow-clip-margin: 1px; border: 1px solid rgba(77, 104, 91, 0.09); border-radius: 9px; background: rgba(69, 99, 84, 0.035); }
  .chat-message.user-message { margin-left: auto; border-bottom-right-radius: 3px; background: rgba(50, 145, 99, 0.075); }
  .chat-message.agent-message { margin-right: auto; border-bottom-left-radius: 3px; }
  .chat-message.agent-message.intervention-required { width: min(94%, 460px); border-color: rgba(190, 132, 42, 0.18); background: rgba(196, 139, 47, 0.025); }
  .chat-message header { display: flex; align-items: center; gap: 6px; }
  .chat-message header strong { min-width: 0; flex: 1; color: #4f685c; font: 750 var(--chat-small-font-size) Inter, sans-serif; }
  .chat-message header time { flex: 0 0 auto; color: #9aa59f; font-size: var(--chat-tiny-font-size); }
  .intervention-indicator { margin: 6px 0 2px; padding: 3px 1px; display: flex; align-items: center; gap: 7px; color: #806128; }
  .intervention-indicator > span { width: 22px; height: 22px; display: grid; flex: 0 0 auto; place-items: center; color: #a87120; }
  .intervention-indicator svg { width: 18px; height: 18px; overflow: visible; }
  .intervention-indicator .indicator-shape { fill: currentColor; stroke: none; }
  .intervention-indicator .indicator-mark { fill: none; stroke: white; stroke-linecap: round; stroke-width: 1.5; }
  .intervention-indicator > div { min-width: 0; display: grid; gap: 1px; }
  .intervention-indicator strong { color: #78591f; font: 800 var(--chat-small-font-size)/1.3 Inter, sans-serif; }
  .handoff-button { width: 20px; height: 20px; padding: 3px; display: grid; place-items: center; border: 0; border-radius: 5px; color: #668076; background: transparent; cursor: pointer; transition: color 130ms ease, background 130ms ease; }
  .handoff-button:hover:not(:disabled) { color: #2f8b63; background: rgba(47, 139, 99, 0.09); }
  .handoff-button:disabled { opacity: 0.4; cursor: default; }
  .handoff-button svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .chat-message.user-message > pre { min-width: 0; max-width: 100%; margin: 5px 0 0; overflow-x: hidden; color: #4b5c54; font: var(--chat-font-size)/1.5 "SFMono-Regular", Consolas, "Liberation Mono", monospace; overflow-wrap: anywhere; white-space: pre-wrap; word-break: break-word; }
  .markdown-content { min-width: 0; max-width: 100%; margin-top: 5px; overflow: hidden; color: #4b5c54; font: var(--chat-font-size)/1.55 Inter, sans-serif; overflow-wrap: anywhere; word-break: break-word; }
  .markdown-content :global(> :first-child) { margin-top: 0; }
  .markdown-content :global(> :last-child) { margin-bottom: 0; }
  .markdown-content :global(p) { max-width: 100%; margin: 0 0 6px; }
  .markdown-content :global(strong) { color: #40564b; font-weight: 800; }
  .markdown-content :global(em) { font-style: italic; }
  .markdown-content :global(del) { color: #87948e; }
  .markdown-content :global(a) { color: #2f8560; font-weight: 650; text-decoration: underline; text-decoration-color: rgba(47, 133, 96, 0.38); text-underline-offset: 2px; }
  .markdown-content :global(a:hover) { color: #216b4b; text-decoration-color: currentColor; }
  .markdown-content :global(ul),
  .markdown-content :global(ol) { margin: 5px 0 7px; padding-left: 18px; }
  .markdown-content :global(li) { margin: 2px 0; padding-left: 1px; }
  .markdown-content :global(h1),
  .markdown-content :global(h2),
  .markdown-content :global(h3),
  .markdown-content :global(h4) { margin: 8px 0 4px; color: #40564b; font-family: Inter, sans-serif; line-height: 1.3; }
  .markdown-content :global(h1) { font-size: calc(var(--chat-font-size) + 3px); }
  .markdown-content :global(h2) { font-size: calc(var(--chat-font-size) + 2px); }
  .markdown-content :global(h3),
  .markdown-content :global(h4) { font-size: calc(var(--chat-font-size) + 1px); }
  .markdown-content :global(blockquote) { margin: 6px 0; padding: 4px 8px; border-left: 2px solid #58a37d; color: #687970; background: rgba(62, 143, 101, 0.05); }
  .markdown-content :global(code) { max-width: 100%; padding: 1px 4px; border-radius: 4px; color: #3f6553; background: rgba(53, 116, 84, 0.08); font: 0.92em/1.45 "SFMono-Regular", Consolas, "Liberation Mono", monospace; overflow-wrap: anywhere; white-space: break-spaces; }
  .markdown-content :global(pre) { max-width: 100%; max-height: 220px; margin: 6px 0; padding: 7px 8px; overflow: auto; border: 1px solid rgba(77, 104, 91, 0.09); border-radius: 6px; background: rgba(42, 63, 53, 0.055); }
  .markdown-content :global(pre code) { padding: 0; color: #465a50; background: transparent; font-size: var(--chat-small-font-size); white-space: pre-wrap; word-break: break-word; }
  .markdown-content :global(.markdown-table-wrap) { box-sizing: border-box; width: 100%; max-width: 100%; margin: 7px 0; overflow-x: auto; border: 1px solid rgba(74, 107, 91, 0.12); border-radius: 8px; background: rgba(58, 91, 75, 0.025); }
  .markdown-content :global(table) { width: 100%; min-width: 310px; border-collapse: collapse; }
  .markdown-content :global(th),
  .markdown-content :global(td) { min-width: 88px; padding: 6px 8px; overflow-wrap: anywhere; border-right: 1px solid rgba(74, 107, 91, 0.09); border-bottom: 1px solid rgba(74, 107, 91, 0.09); word-break: normal; }
  .markdown-content :global(th:last-child),
  .markdown-content :global(td:last-child) { border-right: 0; }
  .markdown-content :global(tbody tr:last-child td) { border-bottom: 0; }
  .markdown-content :global(th) { color: #385146; background: rgba(62, 126, 94, 0.075); font-weight: 800; }
  .markdown-content :global(tbody tr:nth-child(even)) { background: rgba(63, 101, 82, 0.028); }
  .markdown-content :global(.align-center) { text-align: center; }
  .markdown-content :global(.align-right) { text-align: right; }
  .markdown-content :global(img),
  .markdown-content :global(video) { max-width: 100%; height: auto; }
  .markdown-content :global(hr) { height: 1px; margin: 8px 0; border: 0; background: rgba(77, 104, 91, 0.12); }
  .message-images { margin-top: 7px; display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 5px; }
  .message-images img { width: 100%; max-height: 150px; display: block; border-radius: 7px; object-fit: cover; }
  .message-file { min-width: 0; min-height: 38px; padding: 7px 8px; display: flex; align-items: center; gap: 7px; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 7px; color: #52665d; background: rgba(52, 145, 99, 0.045); }
  .message-file svg { width: 18px; height: 18px; flex: 0 0 auto; fill: none; stroke: currentColor; stroke-width: 1.35; stroke-linecap: round; stroke-linejoin: round; }
  .message-file small { min-width: 0; overflow: hidden; color: inherit; font: 650 var(--chat-tiny-font-size)/1.2 Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .agent-typing { width: fit-content; min-width: 38px; height: 25px; padding: 0 9px; display: flex; align-items: center; gap: 4px; border: 1px solid rgba(77, 104, 91, 0.09); border-radius: 9px 9px 9px 3px; background: rgba(69, 99, 84, 0.035); }
  .agent-typing span { width: 4px; height: 4px; border-radius: 50%; background: #4e7faf; animation: agent-typing-dot 850ms ease-in-out infinite; }
  .agent-typing span,
  .reasoning-update.running > header > span,
  .workflow-bridge-link { will-change: transform, opacity; }
  .agent-typing span:nth-child(2) { animation-delay: 130ms; }
  .agent-typing span:nth-child(3) { animation-delay: 260ms; }
  @keyframes agent-typing-dot {
    0%, 60%, 100% { opacity: 0.35; transform: translateY(1px); }
    30% { opacity: 1; transform: translateY(-3px); }
  }
  .reasoning-update { min-width: 0; max-width: 94%; padding: 6px 8px 7px; border-left: 2px solid #6d91ae; border-radius: 0 8px 8px 0; background: linear-gradient(90deg, rgba(91, 133, 164, .07), rgba(91, 133, 164, .018)); }
  .reasoning-update > header { display: flex; align-items: center; gap: 6px; color: #698092; }
  .reasoning-update > header > span { width: 21px; height: 21px; display: grid; place-items: center; flex: 0 0 auto; border-radius: 6px; background: rgba(91, 133, 164, .09); }
  .reasoning-update > header svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.35; stroke-linecap: round; stroke-linejoin: round; }
  .reasoning-update > header strong { min-width: 0; flex: 1; color: #5c7181; font: 740 var(--chat-small-font-size) Inter, sans-serif; }
  .reasoning-update > header time { color: #98a4aa; font: 500 var(--chat-tiny-font-size) Inter, sans-serif; }
  .reasoning-update .markdown-content { margin: 5px 0 0 27px; color: #586961; }
  .reasoning-update.running > header > span { animation: reasoning-pulse 1.25s ease-in-out infinite; }
  @keyframes reasoning-pulse { 50% { opacity: .55; transform: scale(.92); } }
  @container (max-width: 390px) {
    .terminal-output { padding-right: 8px; padding-left: 8px; }
    .chat-message,
    .reasoning-update,
    .turn-files { box-sizing: border-box; width: 100%; max-width: 100%; }
    .reasoning-update .markdown-content { margin-left: 0; }
    .message-images { grid-template-columns: minmax(0, 1fr); }
  }
  .turn-files { padding: 6px; overflow: hidden; border: 1px solid rgba(70, 139, 105, .13); border-radius: 10px; background: linear-gradient(145deg, rgba(66, 143, 104, .055), rgba(66, 143, 104, .018)); }
  .turn-files > header { min-height: 25px; margin-bottom: 5px; padding: 0 2px; display: flex; align-items: center; gap: 6px; }
  .turn-files-mark { width: 22px; height: 22px; display: grid; place-items: center; flex: 0 0 auto; border-radius: 6px; color: #448466; background: rgba(55, 142, 98, .085); }
  .turn-files-mark svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-width: 1.45; stroke-linecap: round; stroke-linejoin: round; }
  .turn-files > header strong { min-width: 0; flex: 1; color: #4f775f; font: 760 var(--chat-small-font-size) Inter, sans-serif; }
  .turn-files > header small { min-width: 18px; height: 18px; padding: 0 4px; display: grid; place-items: center; border-radius: 9px; color: #688076; background: rgba(76, 105, 91, .07); font: 700 var(--chat-tiny-font-size) Inter, sans-serif; }
  .turn-files > div { display: grid; gap: 4px; }
  .turn-files code { min-height: 28px; padding: 5px 3px; display: grid; grid-template-columns: 14px minmax(0, 1fr) auto auto; align-items: center; gap: 5px; overflow: hidden; border-bottom: 1px solid rgba(75, 112, 94, .075); color: #496258; background: transparent; font-size: var(--chat-small-font-size); white-space: normal; }
  .turn-files code:last-child { border-bottom: 0; }
  .turn-files code .file-path { min-width: 0; flex: 1; color: inherit; overflow-wrap: anywhere; word-break: break-word; }
  .turn-files code .added,
  .change-list code .added { color: #45906a; }
  .turn-files code .added { padding: 2px 4px; border-radius: 5px; background: rgba(69, 144, 106, .08); }
  .turn-files code .removed,
  .change-list code .removed { color: #b46161; }
  .turn-files code .removed { padding: 2px 4px; border-radius: 5px; background: rgba(180, 97, 97, .07); }
  .privacy-note { padding-top: 6px; border-top: 1px solid rgba(81, 105, 94, 0.08); color: #9aa49f; font: var(--chat-tiny-font-size)/1.45 Inter, sans-serif; }
  .empty-state { margin: 7px 0 10px !important; color: #909c96; font: var(--chat-small-font-size)/1.5 Inter, sans-serif; }
  .changes-panel { margin-top: 9px; display: grid; gap: 7px; }
  .changes-panel > strong { color: #6a7c73; font: 760 var(--chat-small-font-size) Inter, sans-serif; letter-spacing: 0.04em; text-transform: uppercase; }
  .change-list { display: grid; gap: 4px; }
  .change-list code { padding: 5px 6px; display: flex; align-items: flex-start; gap: 6px; border-radius: 6px; color: #4f6158; background: rgba(70, 101, 86, 0.045); font-size: var(--chat-small-font-size); overflow-wrap: anywhere; white-space: normal; }
  .change-list code .file-path { min-width: 0; flex: 1; color: inherit; overflow-wrap: anywhere; word-break: break-word; }
  .plan-panel { margin: 9px 0 10px; display: grid; gap: 9px; }
  .plan-panel > header { display: flex; align-items: flex-end; justify-content: space-between; gap: 8px; }
  .plan-panel > header > div { min-width: 0; display: grid; gap: 2px; }
  .plan-panel > header small { color: #8b9791; font: 700 var(--chat-tiny-font-size) Inter, sans-serif; letter-spacing: 0.08em; text-transform: uppercase; }
  .plan-panel > header strong { color: #4d6358; font: 780 var(--chat-font-size) Inter, sans-serif; }
  .plan-header-actions { display: flex; flex: 0 0 auto; align-items: center; gap: 6px; }
  .plan-header-actions > span { color: #4c8b6c; font: 700 var(--chat-small-font-size) Inter, sans-serif; white-space: nowrap; }
  .plan-header-actions button,
  .notes-panel > header > button { min-height: 29px; padding: 0 9px; display: inline-flex; flex: 0 0 auto; align-items: center; gap: 5px; border: 1px solid rgba(62, 137, 98, 0.14); border-radius: 7px; color: #39795a; background: rgba(61, 145, 100, 0.06); font: 700 var(--chat-small-font-size)/1 Inter, sans-serif; white-space: nowrap; cursor: pointer; }
  .plan-header-actions button:hover,
  .notes-panel > header > button:hover { background: rgba(61, 145, 100, 0.11); }
  .plan-header-actions svg,
  .notes-panel > header > button svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .plan-explanation { margin: 0 !important; padding: 7px 8px; border-left: 2px solid #4d98c3; border-radius: 0 7px 7px 0; color: #5d6f67; background: rgba(77, 152, 195, 0.055); font: var(--chat-small-font-size)/1.5 Inter, sans-serif; white-space: pre-wrap; }
  .plan-document { min-width: 0; padding: 9px 10px; overflow-x: hidden; border: 1px solid rgba(78, 106, 93, 0.09); border-radius: 9px; background: rgba(70, 101, 86, 0.03); overflow-wrap: anywhere; word-break: break-word; }
  .plan-progress { height: 3px; overflow: hidden; border-radius: 999px; background: rgba(69, 112, 91, 0.1); }
  .plan-progress em { width: var(--plan-progress); height: 100%; display: block; border-radius: inherit; background: #4d9e76; transition: width 240ms ease; }
  .plan-items { margin: 0; padding: 0; display: grid; gap: 5px; list-style: none; }
  .plan-items li { min-width: 0; padding: 7px 8px; display: flex; align-items: flex-start; gap: 8px; border: 1px solid rgba(78, 106, 93, 0.08); border-radius: 8px; background: rgba(70, 101, 86, 0.03); }
  .plan-items li > span { width: 19px; height: 19px; flex: 0 0 auto; display: grid; place-items: center; border: 1px solid rgba(91, 112, 102, 0.18); border-radius: 6px; color: #7c8b84; font: 750 var(--chat-tiny-font-size) Inter, sans-serif; }
  .plan-items li > div { min-width: 0; display: grid; gap: 2px; }
  .plan-items strong { color: #576a61; font: 680 var(--chat-small-font-size)/1.4 Inter, sans-serif; overflow-wrap: anywhere; }
  .plan-items small { color: #929d98; font: 650 var(--chat-tiny-font-size) Inter, sans-serif; }
  .plan-items li.active { border-color: rgba(77, 152, 195, 0.17); background: rgba(77, 152, 195, 0.055); }
  .plan-items li.active > span { border-color: #4d98c3; color: #4d98c3; box-shadow: 0 0 0 2px rgba(77, 152, 195, 0.08); }
  .plan-items li.done > span { border-color: #55a77b; color: white; background: #55a77b; }
  .notes-panel { margin: 9px 0 10px; display: grid; gap: 9px; }
  .notes-panel > header { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .notes-panel > header > div { min-width: 0; display: grid; gap: 2px; }
  .notes-panel > header small { color: #8b9791; font: 700 var(--chat-tiny-font-size) Inter, sans-serif; letter-spacing: .08em; text-transform: uppercase; }
  .notes-panel > header strong { color: #4d6358; font: 780 var(--chat-font-size) Inter, sans-serif; }
  .notes-panel > header > button { width: 29px; height: 29px; padding: 0; justify-content: center; }
  .note-editor { padding: 8px; display: grid; gap: 7px; border: 1px solid rgba(64, 126, 96, .13); border-radius: 9px; background: rgba(68, 116, 91, .035); }
  .note-editor-heading { display: grid; grid-template-columns: minmax(0, 1fr) 26px; gap: 6px; }
  .note-editor-heading > input { min-width: 0; padding: 6px 7px; border: 1px solid rgba(79, 108, 94, .13); border-radius: 7px; outline: 0; color: #405249; background: rgba(255, 255, 255, .48); font: 700 var(--chat-small-font-size) Inter, sans-serif; }
  .note-editor-heading label { width: 26px; height: 26px; display: grid; place-items: center; border: 1px solid rgba(79, 108, 94, .12); border-radius: 7px; color: #849188; cursor: pointer; }
  .note-editor-heading label:has(input:checked) { color: #408661; border-color: rgba(55, 143, 96, .24); background: rgba(55, 143, 96, .08); }
  .note-editor-heading input[type="checkbox"] { position: absolute; opacity: 0; pointer-events: none; }
  .note-editor-heading svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .note-editor textarea { min-height: 105px; padding: 7px 8px; resize: vertical; border: 1px solid rgba(79, 108, 94, .13); border-radius: 7px; outline: 0; color: #465a50; background: rgba(255, 255, 255, .48); font: var(--chat-small-font-size)/1.5 "SFMono-Regular", Consolas, monospace; }
  .note-editor input:focus,
  .note-editor textarea:focus { border-color: rgba(52, 151, 103, .4); box-shadow: 0 0 0 2px rgba(52, 151, 103, .06); }
  .note-editor footer { display: flex; justify-content: flex-end; gap: 5px; }
  .note-editor footer button { width: 28px; min-height: 28px; padding: 0; display: grid; place-items: center; border: 0; border-radius: 7px; color: #718078; background: rgba(80, 107, 94, .06); cursor: pointer; }
  .note-editor footer button svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.55; stroke-linecap: round; stroke-linejoin: round; }
  .note-editor footer button.primary { color: white; background: #3f9369; }
  .note-editor footer button:disabled { opacity: .45; cursor: default; }
  .session-note-list { display: grid; gap: 7px; }
  .session-note { min-width: 0; padding: 8px 9px; display: grid; gap: 6px; border: 1px solid rgba(76, 106, 91, .09); border-radius: 9px; background: rgba(70, 101, 86, .025); }
  .session-note.pinned { border-color: rgba(63, 145, 99, .18); background: rgba(63, 145, 99, .045); }
  .session-note > header { display: flex; align-items: flex-start; justify-content: space-between; gap: 7px; }
  .session-note > header > div { min-width: 0; display: grid; gap: 2px; }
  .session-note > header small { color: #8a9790; font: 650 var(--chat-tiny-font-size) Inter, sans-serif; }
  .session-note > header strong { overflow: hidden; color: #4b6055; font: 760 var(--chat-small-font-size) Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .session-note-actions { display: flex; flex: 0 0 auto; gap: 3px; }
  .session-note-actions button { width: 27px; height: 27px; padding: 5px; display: grid; place-items: center; border: 0; border-radius: 7px; color: #87938d; background: transparent; cursor: pointer; }
  .session-note-actions button:hover,
  .session-note-actions button.active { color: #3f8862; background: rgba(61, 145, 100, .08); }
  .session-note-actions svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .session-note-body { max-height: 180px; margin: 0; overflow: auto; }
  .use-note-button { width: 28px; height: 28px; padding: 0; display: grid; place-items: center; border: 0; border-radius: 7px; color: #3c805d; background: rgba(61, 145, 100, .07); cursor: pointer; }
  .use-note-button svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.45; stroke-linecap: round; stroke-linejoin: round; }
  .permission { margin: 7px 0 2px; padding-left: 9px; display: grid; gap: 6px; border-left: 2px solid #c87d32; }
  .permission strong { color: #5a4633; font: 700 var(--chat-font-size)/1.35 Inter, sans-serif; }
  .permission code { padding: 5px 6px; overflow: hidden; border-radius: 6px; color: #5f6b66; background: rgba(74, 99, 88, 0.055); font-size: var(--chat-small-font-size); text-overflow: ellipsis; white-space: nowrap; }
  .permission > div { display: flex; gap: 4px; }
  .permission button { min-height: 23px; padding: 0 7px; border: 1px solid rgba(82, 101, 93, 0.15); border-radius: 6px; color: #4b5d55; background: rgba(255, 255, 255, 0.58); font: 700 var(--chat-small-font-size) Inter, sans-serif; cursor: pointer; }
  .permission button.danger { color: #a64d4d; }
  .agent-question { margin: 8px 0 3px; padding: 9px; display: grid; gap: 9px; border: 1px solid rgba(48, 133, 176, 0.2); border-radius: 9px; background: rgba(48, 133, 176, 0.055); }
  .agent-question-item { min-width: 0; display: grid; gap: 5px; }
  .agent-question-item > small { color: #367b9c; font: 800 var(--chat-small-font-size)/1.2 Inter, sans-serif; letter-spacing: 0.04em; text-transform: uppercase; }
  .agent-question-item > strong { color: #344b52; font: 700 var(--chat-font-size)/1.4 Inter, sans-serif; overflow-wrap: anywhere; }
  .agent-question-item > em { color: #698078; font: 500 var(--chat-small-font-size)/1.3 Inter, sans-serif; }
  .question-options { display: grid; gap: 4px; }
  .question-options button { min-width: 0; padding: 6px 7px; display: flex; align-items: flex-start; gap: 7px; border: 1px solid rgba(65, 112, 133, 0.14); border-radius: 7px; color: #425a61; background: rgba(255, 255, 255, 0.55); text-align: left; cursor: pointer; }
  .question-options button:hover,
  .question-options button.selected { border-color: rgba(44, 137, 178, 0.42); background: rgba(48, 145, 187, 0.1); }
  .question-options button b { color: #2f83aa; font-size: var(--chat-small-font-size); }
  .question-options button span { min-width: 0; display: grid; gap: 2px; font: 700 var(--chat-small-font-size)/1.3 Inter, sans-serif; }
  .question-options button small { color: #71827b; font: 500 var(--chat-small-font-size)/1.3 Inter, sans-serif; overflow-wrap: anywhere; }
  .agent-question .question-submit { justify-self: end; min-height: 25px; padding: 0 10px; border: 0; border-radius: 7px; color: white; background: #318e62; font: 750 var(--chat-small-font-size) Inter, sans-serif; cursor: pointer; }
  .hint { color: #89948f; font-size: var(--chat-small-font-size); }
  .hint.docked { color: #4f7566; }
  .terminate-backdrop { position: absolute; z-index: 80; inset: 49px 0 64px; padding: 12px; display: grid; place-items: center; background: rgba(25, 34, 30, 0.22); backdrop-filter: blur(2px); }
  .terminate-dialog { box-sizing: border-box; width: min(280px, 100%); padding: 11px; display: grid; gap: 10px; border: 1px solid rgba(166, 77, 77, 0.17); border-radius: 12px; color: #394a42; background: #f8fbf9; box-shadow: 0 12px 34px rgba(26, 38, 32, 0.2); }
  .terminate-dialog > div { min-width: 0; display: grid; gap: 4px; }
  .terminate-dialog strong { color: #704a47; font: 800 var(--chat-font-size)/1.3 Inter, sans-serif; }
  .terminate-dialog p { margin: 0; color: #6c7973; font: 550 var(--chat-small-font-size)/1.45 Inter, sans-serif; overflow-wrap: anywhere; }
  .terminate-dialog footer { display: flex; justify-content: flex-end; gap: 5px; }
  .terminate-dialog button { min-height: 26px; padding: 0 8px; border: 1px solid rgba(84, 101, 93, 0.14); border-radius: 7px; color: #596861; background: transparent; font: 750 var(--chat-small-font-size) Inter, sans-serif; cursor: pointer; }
  .terminate-dialog button.danger { color: white; border-color: #a85656; background: #a85656; }
  .terminate-dialog button.takeover { color: white; border-color: #3d8063; background: #3d8063; }
  .terminate-dialog button:disabled { opacity: 0.48; cursor: default; }
  .model-settings-dialog { width: min(370px, 100%); max-height: min(500px, 86%); grid-template-rows: auto minmax(0, 1fr) auto auto; gap: 11px; border-color: rgba(74, 112, 94, 0.18); }
  .model-settings-dialog > header { min-width: 0; display: flex; align-items: center; gap: 9px; }
  .model-settings-dialog > header > div { min-width: 0; display: grid; gap: 2px; }
  .model-settings-dialog > header strong { color: #3b5549; }
  .model-settings-icon { width: 31px; height: 31px; flex: 0 0 auto; display: grid; place-items: center; border-radius: 9px; color: #397a5c; background: rgba(57, 122, 92, 0.09); }
  .model-settings-icon svg { width: 18px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; }
  .model-settings-section { min-height: 0; display: grid; gap: 6px; }
  .model-settings-section:first-of-type { overflow: hidden; }
  .claude-model-settings label { display: grid; gap: 6px; }
  .claude-model-settings input { box-sizing: border-box; width: 100%; min-height: 35px; padding: 0 9px; border: 1px solid rgba(72, 103, 88, 0.14); border-radius: 8px; outline: none; color: #40564b; background: rgba(76, 113, 95, 0.035); font: 650 var(--chat-small-font-size) Inter, sans-serif; }
  .claude-model-settings input:focus { border-color: rgba(48, 139, 95, 0.52); box-shadow: 0 0 0 2px rgba(57, 143, 99, 0.08); }
  .claude-model-settings > small { color: #7b8982; font: 550 var(--chat-tiny-font-size)/1.4 Inter, sans-serif; }
  .model-settings-label { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: #718078; font: 800 var(--chat-tiny-font-size) Inter, sans-serif; letter-spacing: 0.06em; text-transform: uppercase; }
  .model-settings-label b { padding: 2px 6px; border-radius: 999px; color: #3d8063; background: rgba(57, 143, 99, 0.09); font: 780 var(--chat-tiny-font-size) Inter, sans-serif; letter-spacing: 0; text-transform: capitalize; }
  .model-options { min-height: 0; max-height: 170px; padding-right: 2px; display: grid; grid-template-columns: repeat(auto-fit, minmax(135px, 1fr)); gap: 5px; overflow-y: auto; scrollbar-width: thin; }
  .terminate-dialog .model-options > button { min-width: 0; min-height: 38px; height: auto; padding: 6px 8px; display: grid; align-content: center; gap: 2px; border-color: rgba(72, 103, 88, 0.11); background: rgba(76, 113, 95, 0.035); text-align: left; }
  .terminate-dialog .model-options > button:hover { border-color: rgba(53, 126, 90, 0.24); background: rgba(59, 133, 96, 0.07); }
  .terminate-dialog .model-options > button.active { border-color: rgba(48, 139, 95, 0.48); background: rgba(57, 143, 99, 0.1); box-shadow: inset 2px 0 #3a9267; }
  .model-options button > span { min-width: 0; display: flex; align-items: center; gap: 5px; }
  .model-options button strong { min-width: 0; overflow: hidden; color: #40564b; font-size: var(--chat-small-font-size); text-overflow: ellipsis; white-space: nowrap; }
  .model-options button small { padding: 2px 4px; border-radius: 4px; color: #4c8067; background: rgba(63, 137, 101, 0.09); font: 750 var(--chat-tiny-font-size) Inter, sans-serif; }
  .effort-slider { display: grid; gap: 5px; }
  .effort-slider input { width: 100%; height: 16px; margin: 0; accent-color: #3d8b64; cursor: pointer; }
  .effort-scale { display: flex; align-items: center; justify-content: space-between; gap: 3px; }
  .effort-scale span { min-width: 0; color: #93a098; font: 650 calc(var(--chat-tiny-font-size) - 1px) Inter, sans-serif; text-transform: capitalize; transition: color 140ms ease, transform 140ms ease; }
  .effort-scale span.active { color: #397d5d; font-weight: 820; transform: translateY(-1px); }
  .model-settings-loading { min-height: 110px; place-content: center; color: #718078; font: 650 var(--chat-small-font-size) Inter, sans-serif; }
  .model-settings-loading span { width: 16px; height: 16px; margin: 0 auto 4px; border: 2px solid rgba(61, 128, 99, 0.18); border-top-color: #3d8063; border-radius: 50%; animation: model-spin 0.8s linear infinite; }
  @keyframes model-spin { to { transform: rotate(360deg); } }
  .model-settings-error { padding: 6px 8px; border-radius: 7px; color: #9b4f4f !important; background: rgba(170, 79, 79, 0.08); }
  .terminal-composer { position: relative; box-sizing: border-box; min-height: 63px; padding: 7px 8px 8px 10px; display: flex; flex: 0 0 auto; flex-direction: column; align-items: stretch; gap: 6px; border-top: 1px solid rgba(97, 119, 109, 0.11); }
  .composer-controls { min-width: 0; min-height: 0; display: flex; flex: 1; align-items: flex-end; gap: 6px; }
  .composer-leading-actions { position: relative; display: flex; flex: 0 0 auto; flex-direction: column; justify-content: flex-end; gap: 4px; }
  .terminal-composer .composer-tools-trigger { color: #5d7469; border: 1px solid rgba(82, 106, 95, 0.12); background: rgba(80, 105, 94, 0.055); transition: color 140ms ease, border-color 140ms ease, background 140ms ease, transform 140ms ease; }
  .terminal-composer .composer-tools-trigger:hover,
  .terminal-composer .composer-tools-trigger.active { color: #347c59; border-color: rgba(52, 139, 94, 0.25); background: rgba(52, 139, 94, 0.09); }
  .terminal-composer .composer-tools-trigger.active { transform: rotate(4deg); }
  .composer-tools-trigger svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 1.55; stroke-linecap: round; stroke-linejoin: round; }
  .composer-tools-menu { position: absolute; z-index: 52; bottom: calc(100% + 7px); left: 0; width: 212px; padding: 5px; display: grid; gap: 2px; border: 1px solid rgba(73, 106, 90, 0.15); border-radius: 11px; color: #52665c; background: rgba(248, 251, 249, 0.99); box-shadow: 0 14px 34px rgba(24, 45, 34, 0.19); animation: composer-tools-in 140ms cubic-bezier(.2,.8,.2,1); }
  @keyframes composer-tools-in { from { opacity: 0; transform: translateY(5px) scale(.97); } }
  .terminal-composer .composer-tools-menu > button { width: 100%; min-height: 38px; height: auto; padding: 5px 6px; display: grid; grid-template-columns: 27px minmax(0, 1fr) auto; align-items: center; gap: 7px; place-items: initial; border: 0; border-radius: 8px; color: inherit; background: transparent; text-align: left; }
  .terminal-composer .composer-tools-menu > button:hover:not(:disabled),
  .terminal-composer .composer-tools-menu > button.active { color: #347b59; background: rgba(52, 139, 94, 0.075); }
  .composer-tools-menu .tool-icon { width: 27px; height: 27px; display: grid; place-items: center; border-radius: 7px; background: rgba(75, 112, 94, 0.07); }
  .composer-tools-menu .tool-icon svg,
  .composer-tools-menu .tool-chevron { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.45; stroke-linecap: round; stroke-linejoin: round; }
  .composer-tools-menu .tool-icon .mode-spinner { width: 11px; height: 11px; }
  .composer-tools-menu button > span:nth-child(2) { min-width: 0; display: flex; align-items: center; justify-content: space-between; gap: 6px; }
  .composer-tools-menu strong { overflow: hidden; color: #465b50; font: 740 var(--chat-small-font-size) Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .composer-tools-menu small { flex: 0 0 auto; color: #8a9891; font: 680 var(--chat-tiny-font-size) Inter, sans-serif; }
  .composer-tools-menu .tool-chevron { width: 13px; color: #94a199; }
  .pending-images { width: 100%; min-height: 51px; padding: 4px 5px; display: flex; align-items: center; gap: 6px; overflow-x: auto; border-radius: 8px; background: rgba(52, 145, 99, 0.045); }
  .pending-images-label { max-width: 52px; flex: 0 0 auto; color: #829088; font: 750 var(--chat-tiny-font-size)/1.25 Inter, sans-serif; text-transform: uppercase; }
  .pending-images > span { position: relative; width: 42px; height: 42px; flex: 0 0 auto; }
  .pending-images img { width: 100%; height: 100%; display: block; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 8px; object-fit: cover; }
  .pending-images > span.file-attachment { width: 116px; padding: 5px 7px; display: flex; align-items: center; gap: 5px; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 8px; color: #52665d; background: rgba(255, 255, 255, 0.5); }
  .pending-images .file-attachment > svg { width: 18px; height: 18px; flex: 0 0 auto; fill: none; stroke: currentColor; stroke-width: 1.35; stroke-linecap: round; stroke-linejoin: round; }
  .pending-images .file-attachment > small { min-width: 0; overflow: hidden; font: 650 var(--chat-tiny-font-size)/1.2 Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .pending-images button { position: absolute; top: -4px; right: -4px; width: 15px; height: 15px; border: 1px solid rgba(82, 106, 95, 0.18); border-radius: 50%; color: #65766e; background: #eef3f0; font-size: 10px; line-height: 1; }
  .composer-controls textarea { min-width: 0; min-height: 46px; height: 100%; flex: 1; padding: 7px 8px; resize: none; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 9px; outline: none; color: #34443d; background: rgba(255, 255, 255, 0.5); font: var(--chat-font-size)/1.4 Inter, sans-serif; }
  .composer-controls textarea:focus { border-color: rgba(52, 151, 103, 0.42); box-shadow: 0 0 0 3px rgba(52, 151, 103, 0.07); }
  .composer-controls textarea:disabled { opacity: 0.58; }
  .terminal-composer button { width: 29px; height: 29px; display: grid; flex: 0 0 auto; place-items: center; border: 0; border-radius: 8px; color: white; background: #318e62; cursor: pointer; }
  .slash-command-menu { position: absolute; z-index: 45; right: 8px; bottom: calc(100% + 5px); left: 10px; max-height: min(230px, 48vh); padding: 5px; display: grid; gap: 2px; overflow-x: hidden; overflow-y: auto; border: 1px solid rgba(80, 105, 94, 0.15); border-radius: 11px; color: #53665d; background: #f8fbf9; box-shadow: 0 12px 32px rgba(28, 52, 41, 0.2); }
  .slash-command-heading { min-height: 24px; padding: 2px 7px 4px; display: flex; align-items: center; justify-content: space-between; gap: 8px; border-bottom: 1px solid rgba(80, 105, 94, 0.08); }
  .slash-command-heading strong { color: #667970; font: 800 var(--chat-tiny-font-size) Inter, sans-serif; letter-spacing: 0.08em; text-transform: uppercase; }
  .slash-command-heading small { color: #8b9892; font: 650 var(--chat-tiny-font-size) Inter, sans-serif; white-space: nowrap; }
  .slash-command-heading kbd { padding: 1px 3px; border: 1px solid rgba(80, 105, 94, 0.13); border-radius: 4px; color: #718079; background: rgba(80, 105, 94, 0.045); font: inherit; }
  .terminal-composer .slash-command-menu > button { width: 100%; min-height: 35px; height: auto; padding: 5px 7px; display: grid; grid-template-columns: minmax(64px, auto) minmax(0, 1fr); align-items: center; gap: 8px; place-items: initial; border-radius: 7px; color: #63746c; background: transparent; text-align: left; }
  .terminal-composer .slash-command-menu > button:hover,
  .terminal-composer .slash-command-menu > button.active { color: #2e7657; background: rgba(54, 143, 97, 0.08); }
  .slash-command-menu code { color: #397d5d; font: 750 var(--chat-small-font-size) "SFMono-Regular", Consolas, "Liberation Mono", monospace; white-space: nowrap; }
  .slash-command-menu button > span { min-width: 0; display: grid; gap: 2px; overflow: hidden; font: 620 var(--chat-small-font-size) Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .slash-command-menu button > span small { overflow: hidden; color: #8b9892; font: 650 var(--chat-tiny-font-size) Inter, sans-serif; text-overflow: ellipsis; text-transform: uppercase; }
  .terminal-composer .interrupt-submit { background: #bd5c52; }
  .terminal-composer .interrupt-submit:hover:not(:disabled) { background: #aa4d44; }
  .terminal-composer .queued-prompt-tray { width: 100%; height: 35px; padding: 0 7px; display: flex; align-items: center; gap: 7px; border: 1px solid rgba(80, 119, 160, 0.13); border-radius: 9px; color: #4f6d83; background: rgba(74, 119, 157, 0.055); text-align: left; }
  .queued-prompt-tray:hover:not(:disabled) { border-color: rgba(67, 119, 164, 0.24); background: rgba(74, 119, 157, 0.09); }
  .queue-mark { width: 17px; height: 17px; display: grid; flex: 0 0 auto; place-items: center; border-radius: 5px; color: #477fa9; background: rgba(66, 127, 174, 0.1); font: 800 11px Inter, sans-serif; }
  .queue-copy { min-width: 0; flex: 1; display: grid; gap: 1px; }
  .queue-copy small { color: #7790a1; font: 760 var(--chat-tiny-font-size) Inter, sans-serif; letter-spacing: 0.035em; text-transform: uppercase; }
  .queue-copy strong { overflow: hidden; color: #4c6576; font: 620 var(--chat-small-font-size) Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .queue-shortcut { display: flex; flex: 0 0 auto; align-items: center; gap: 4px; color: #748b9a; }
  .queue-shortcut kbd { min-width: 23px; padding: 2px 4px; border: 1px solid rgba(75, 106, 127, 0.17); border-bottom-width: 2px; border-radius: 5px; color: #547286; background: rgba(255, 255, 255, 0.48); font: 750 var(--chat-tiny-font-size) Inter, sans-serif; text-align: center; }
  .queue-shortcut small { font: 650 var(--chat-tiny-font-size) Inter, sans-serif; white-space: nowrap; }
  .mode-spinner { width: 12px; height: 12px; border: 2px solid color-mix(in srgb, currentColor 24%, transparent); border-top-color: currentColor; border-radius: 50%; animation: send-spin 650ms linear infinite; }
  .mode-feedback { position: absolute; bottom: calc(100% + 6px); left: 0; z-index: 20; padding: 4px 6px; border: 1px solid rgba(74, 106, 91, 0.12); border-radius: 6px; color: #687b72; background: rgba(249, 252, 250, 0.97); box-shadow: 0 5px 14px rgba(35, 54, 45, 0.1); font: 650 var(--chat-tiny-font-size) Inter, sans-serif; white-space: nowrap; pointer-events: none; }
  .mode-feedback.success { color: #377a59; }
  .terminal-composer button:disabled { opacity: 0.35; cursor: default; }
  .terminal-composer.sending button:disabled { opacity: 0.82; }
  .terminal-composer .composer-resize-handle { position: absolute; z-index: 18; top: -6px; left: 50%; width: 54px; height: 12px; padding: 0; display: grid; place-items: center; border: 0; border-radius: 999px; color: #7c8d85; background: transparent; cursor: ns-resize; touch-action: none; transform: translateX(-50%); }
  .terminal-composer .composer-resize-handle:hover,
  .terminal-composer .composer-resize-handle:focus-visible { opacity: 1; background: rgba(69, 111, 91, 0.07); outline: none; }
  .composer-resize-handle span { width: 25px; height: 2px; display: block; border-radius: 999px; background: currentColor; opacity: 0.55; transition: width 120ms ease, opacity 120ms ease; }
  .composer-resize-handle:hover span,
  .composer-resize-handle:focus-visible span { width: 31px; opacity: 0.9; }
  .send-spinner { width: 12px; height: 12px; border: 2px solid rgba(255, 255, 255, 0.38); border-top-color: white; border-radius: 50%; animation: send-spin 650ms linear infinite; }
  @keyframes send-spin { to { transform: rotate(360deg); } }
  .message { margin: -4px 11px 6px; color: #ad4f4f; font-size: var(--chat-small-font-size); }
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
  .loading { align-items: center; justify-content: center; gap: 9px; padding: 18px; color: #78857f; font-size: 9px; text-align: center; }
  .loading .loading-close { position: absolute; top: 8px; right: 8px; width: 27px; height: 27px; padding: 0; display: grid; place-items: center; border: 0; border-radius: 8px; color: #718079; background: transparent; cursor: pointer; }
  .loading .loading-close:hover { color: #3f5149; background: rgba(72, 99, 87, 0.07); }
  .loading-actions { display: flex; gap: 6px; }
  .loading-actions button { min-height: 27px; padding: 0 9px; border: 1px solid rgba(82, 105, 95, 0.16); border-radius: 8px; color: #4d6f61; background: rgba(255, 255, 255, 0.55); font-size: 8px; font-weight: 720; cursor: pointer; }

  .terminal-window:not(.dark) .terminal-card {
    border-color: rgba(48, 86, 64, 0.36);
    background: #dce6d8;
    box-shadow: inset 0 0 0 1px rgba(244, 239, 218, 0.5), 0 8px 24px rgba(31, 56, 44, 0.13);
  }
  .terminal-window:not(.dark) .terminal-card > header,
  .terminal-window:not(.dark) .terminal-composer {
    border-color: rgba(68, 95, 82, 0.18);
    background: #e8e5d2;
  }
  .terminal-window:not(.dark) .terminal-output {
    color: #43564d;
    background: linear-gradient(180deg, #d2e0d2 0, #dce6d8 84px);
  }
  .terminal-window:not(.dark) .hub-tabs,
  .terminal-window:not(.dark) .work-tray { border-color: rgba(68, 95, 82, 0.16); }
  .terminal-window:not(.dark) .work-tray { background: #cfdccd; }
  .terminal-window:not(.dark) .work-card,
  .terminal-window:not(.dark) .plan-items li,
  .terminal-window:not(.dark) .chat-message {
    border-color: rgba(66, 94, 81, 0.18);
    background: #ebe8d6;
    box-shadow: 0 1px 3px rgba(39, 66, 53, 0.04);
  }
  .terminal-window:not(.dark) .chat-message.user-message {
    border-color: rgba(46, 132, 88, 0.2);
    background: #c7dfce;
  }
  .terminal-window:not(.dark) .reasoning-update {
    border-color: #668aa6;
    background: linear-gradient(90deg, rgba(91, 133, 164, 0.12), rgba(91, 133, 164, 0.035));
  }
  .terminal-window:not(.dark) .turn-files {
    border-color: rgba(55, 132, 91, 0.22);
    background: #cce2d1;
  }
  .terminal-window:not(.dark) .turn-files code { border-color: rgba(65, 103, 84, 0.13); }
  .terminal-window:not(.dark) .agent-typing,
  .terminal-window:not(.dark) .load-earlier-chat {
    border-color: rgba(66, 94, 81, 0.18);
    background: #e9e6d4;
  }
  .terminal-window:not(.dark) .slash-command-menu,
  .terminal-window:not(.dark) .header-actions-menu,
  .terminal-window:not(.dark) .workflow-role-menu,
  .terminal-window:not(.dark) .workflow-role-popover,
  .terminal-window:not(.dark) .handoff-dialog {
    border-color: rgba(64, 91, 78, 0.24);
    background: #e9e6d4;
    box-shadow: 0 14px 34px rgba(27, 52, 40, 0.18);
  }
  .terminal-window:not(.dark) .composer-controls textarea,
  .terminal-window:not(.dark) .pending-images > span.file-attachment,
  .terminal-window:not(.dark) .terminal-name-editor input { background: #eee9d8; }
  .terminal-window:not(.dark) .composer-tools-menu,
  .terminal-window:not(.dark) .terminate-dialog { background: #e9e6d4; }
  .terminal-window:not(.dark) .hub-tabs button.active { color: #246b47; border-bottom-color: #32915e; }

  .terminal-window.dark { --terminal-scroll-thumb: #50665b; --terminal-scroll-thumb-hover: #6f8c7d; color-scheme: dark; }
  .handoff-backdrop { position: absolute; z-index: 60; inset: 0; padding: 14px; display: grid; place-items: center; background: rgba(19, 29, 24, 0.38); backdrop-filter: blur(3px); }
  .handoff-dialog { width: min(430px, 100%); max-height: 100%; padding: 12px; display: grid; gap: 10px; overflow-y: auto; border: 1px solid rgba(81, 112, 97, 0.18); border-radius: 13px; color: #34463d; background: #f8fbf9; box-shadow: 0 16px 44px rgba(20, 35, 28, 0.2); }
  .handoff-dialog > header { min-height: auto; padding: 0; display: flex; align-items: flex-start; border: 0; }
  .handoff-dialog > header div { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .handoff-dialog > header small { color: #6f8178; font: 700 8px Inter, sans-serif; text-transform: uppercase; letter-spacing: 0.07em; }
  .handoff-dialog > header strong { color: #30453a; font: 800 12px Inter, sans-serif; }
  .handoff-dialog > header button { width: 25px; height: 25px; padding: 0; border: 0; border-radius: 7px; color: #71827a; background: transparent; font-size: 17px; cursor: pointer; }
  .handoff-target,
  .handoff-note { display: grid; gap: 4px; color: #60736a; font: 700 8px Inter, sans-serif; }
  .handoff-target select,
  .handoff-note textarea { width: 100%; min-width: 0; border: 1px solid rgba(81, 112, 97, 0.16); border-radius: 8px; outline: 0; color: #364a40; background: rgba(82, 121, 101, 0.045); font: 9px/1.4 Inter, sans-serif; }
  .handoff-target select { height: 30px; padding: 0 8px; }
  .handoff-note textarea { min-height: 48px; padding: 7px 8px; resize: vertical; }
  .handoff-target select:focus,
  .handoff-note textarea:focus { border-color: rgba(47, 139, 99, 0.48); box-shadow: 0 0 0 2px rgba(47, 139, 99, 0.08); }
  .handoff-options { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; }
  .handoff-options > label { min-width: 0; padding: 7px; display: flex; align-items: flex-start; gap: 6px; border: 1px solid rgba(81, 112, 97, 0.12); border-radius: 8px; background: rgba(82, 121, 101, 0.035); cursor: pointer; }
  .handoff-options > label.disabled { opacity: 0.48; cursor: default; }
  .handoff-options input { margin: 2px 0 0; accent-color: #359268; }
  .handoff-options span { min-width: 0; display: grid; gap: 2px; }
  .handoff-options strong { color: #42594e; font: 750 9px Inter, sans-serif; }
  .handoff-options small { color: #7b8a83; font: 7px/1.35 Inter, sans-serif; }
  .handoff-preview { min-height: 64px; padding: 7px 8px; display: grid; gap: 5px; border-left: 2px solid #52a37d; border-radius: 0 8px 8px 0; background: rgba(55, 142, 98, 0.045); }
  .handoff-preview > strong { color: #507463; font: 750 8px Inter, sans-serif; text-transform: uppercase; }
  .handoff-preview pre { max-height: 120px; margin: 0; overflow: auto; color: #53665c; font: 8px/1.5 "SFMono-Regular", Consolas, monospace; overflow-wrap: anywhere; white-space: pre-wrap; word-break: break-word; }
  .handoff-error { margin: 0; color: #a55451; font: 8px/1.4 Inter, sans-serif; }
  .handoff-dialog > footer { display: flex; justify-content: flex-end; gap: 6px; }
  .handoff-dialog > footer button { min-height: 28px; padding: 0 10px; border: 1px solid rgba(81, 112, 97, 0.16); border-radius: 8px; color: #60736a; background: transparent; font: 750 8px Inter, sans-serif; cursor: pointer; }
  .handoff-dialog > footer button.primary { border-color: #32895f; color: white; background: #32895f; }
  .handoff-dialog > footer button:disabled { opacity: 0.45; cursor: default; }
  .workflow-role-fields { min-height: 0; display: grid; flex: 1; align-content: start; gap: 7px; overflow-x: hidden; overflow-y: auto; }
  .workflow-role-fields.picker-open { overflow: visible; }
  .workflow-role-popover.constrained .workflow-role-fields.picker-open { overflow-x: hidden; overflow-y: auto; }
  .workflow-role-fields label { min-width: 0; display: grid; gap: 3px; color: #708079; font: 700 8px Inter, sans-serif; }
  .workflow-role-fields input,
  .workflow-role-fields textarea { width: 100%; min-width: 0; padding: 6px 7px; border: 1px solid rgba(76, 105, 91, 0.13); border-radius: 8px; outline: none; color: #3d5047; background: rgba(255, 255, 255, 0.58); font: 9px/1.4 Inter, sans-serif; }
  .workflow-role-fields textarea { resize: vertical; }
  .workflow-role-fields input:focus,
  .workflow-role-fields textarea:focus { border-color: rgba(53, 142, 97, 0.38); }
  .workflow-role-picker { position: relative; display: grid; gap: 4px; }
  .workflow-field-label { color: #708079; font: 700 8px Inter, sans-serif; }
  .workflow-role-trigger { width: 100%; min-height: 47px; padding: 6px 8px; display: flex; align-items: center; gap: 8px; border: 1px solid rgba(65, 135, 101, 0.14); border-radius: 10px; color: #3f554a; background: linear-gradient(135deg, rgba(255, 255, 255, 0.72), rgba(68, 143, 105, 0.035)); cursor: pointer; text-align: left; transition: border-color 130ms ease, background 130ms ease; }
  .workflow-role-trigger.open { border-color: rgba(47, 142, 94, 0.34); background: rgba(64, 151, 106, 0.06); }
  .workflow-role-trigger > span { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .workflow-role-trigger strong { color: #395044; font: 800 9px Inter, sans-serif; }
  .workflow-role-trigger small { overflow: hidden; color: #7b8d84; font: 7px Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .workflow-role-trigger > svg { width: 13px; height: 13px; transition: transform 140ms ease; }
  .workflow-role-trigger.open > svg { transform: rotate(180deg); }
  .workflow-role-menu { position: absolute; z-index: 95; top: calc(100% + 5px); right: 0; left: 0; max-height: clamp(56px, calc(100vh - 154px), 218px); padding: 5px; display: grid; gap: 2px; overflow-y: auto; border: 1px solid rgba(73, 108, 90, 0.15); border-radius: 11px; background: #f8fbf9; box-shadow: 0 14px 30px rgba(25, 48, 37, 0.2); }
  .workflow-role-popover.constrained .workflow-role-menu { position: relative; top: auto; right: auto; left: auto; max-height: none; margin-top: 5px; box-shadow: none; }
  .workflow-role-menu > button { width: 100%; min-height: 40px; padding: 5px 6px; display: flex; align-items: center; gap: 7px; border: 0; border-radius: 8px; color: #53665d; background: transparent; cursor: pointer; text-align: left; }
  .workflow-role-menu > button:hover,
  .workflow-role-menu > button.active { background: rgba(54, 145, 99, 0.065); }
  .workflow-role-menu > button:hover .role-planner :global(.workflow-role-icon) { animation: role-icon-plan 1.15s ease-in-out infinite; }
  .workflow-role-menu > button:hover .role-implementer :global(.workflow-role-icon) { animation: role-icon-build .85s ease-in-out infinite; }
  .workflow-role-menu > button:hover .role-reviewer :global(.workflow-role-icon) { animation: role-icon-review 1.05s ease-in-out infinite; }
  .workflow-role-menu > button:hover .role-tester :global(.workflow-role-icon) { animation: role-icon-test .95s ease-in-out infinite; }
  .workflow-role-menu > button:hover .role-researcher :global(.workflow-role-icon) { animation: role-icon-research 1.8s linear infinite; }
  .workflow-role-menu > button:hover .role-custom :global(.workflow-role-icon) { animation: role-icon-custom .9s ease-in-out infinite; }
  .workflow-role-menu > button > span { min-width: 0; flex: 1; display: grid; gap: 1px; }
  .workflow-role-menu strong { color: #40544a; font: 780 8px Inter, sans-serif; }
  .workflow-role-menu small { color: #829088; font: 7px Inter, sans-serif; }
  .workflow-role-menu > button > svg { width: 13px; height: 13px; color: #36855e; }
  .workflow-role-replace-warning { padding: 7px; display: grid; grid-template-columns: 27px minmax(0, 1fr); align-items: center; gap: 6px 7px; border: 1px solid rgba(178, 126, 50, 0.2); border-radius: 10px; background: rgba(185, 132, 51, 0.065); }
  .workflow-role-replace-warning > .role-symbol { width: 27px; height: 27px; }
  .workflow-role-replace-warning > span { min-width: 0; display: grid; gap: 2px; }
  .workflow-role-replace-warning strong { color: #755d39; font: 800 8px Inter, sans-serif; }
  .workflow-role-replace-warning small { color: #8c7a5d; font: 7px/1.35 Inter, sans-serif; }
  .workflow-role-replace-warning > div { grid-column: 1 / -1; display: flex; justify-content: flex-end; gap: 5px; }
  .workflow-role-replace-warning button,
  .workflow-instruction-meta button { min-height: 24px; padding: 0 7px; border: 1px solid rgba(89, 112, 101, 0.15); border-radius: 7px; color: #62736b; background: rgba(255, 255, 255, 0.45); font: 750 7px Inter, sans-serif; cursor: pointer; }
  .workflow-role-replace-warning button.confirm { color: #fff; border-color: #9b7134; background: #9b7134; }
  .workflow-contract-toggle { width: 100%; min-height: 42px; padding: 6px 8px; display: grid; grid-template-columns: 24px minmax(0, 1fr) 14px; align-items: center; gap: 7px; border: 1px solid rgba(65, 135, 101, 0.13); border-radius: 10px; color: #4b6256; background: rgba(67, 139, 103, 0.035); cursor: pointer; text-align: left; }
  .workflow-contract-toggle > svg:first-child { width: 15px; height: 15px; justify-self: center; color: #438763; }
  .workflow-contract-toggle > span { min-width: 0; display: grid; gap: 1px; }
  .workflow-contract-toggle strong { color: #40564b; font: 800 8px Inter, sans-serif; }
  .workflow-contract-toggle .contract-chevron { width: 13px; height: 13px; transition: transform 150ms ease; }
  .workflow-contract-toggle.open .contract-chevron { transform: rotate(180deg); }
  .workflow-contract-body { min-height: 0; display: grid; gap: 7px; }
  .workflow-instruction-meta { display: flex; align-items: center; justify-content: space-between; gap: 7px; }
  .workflow-instruction-meta > span { padding: 3px 6px; border-radius: 999px; color: #6b7b73; background: rgba(86, 110, 99, 0.07); font: 780 7px Inter, sans-serif; }
  .workflow-instruction-meta > span.ready { color: #347a59; background: rgba(54, 145, 99, 0.09); }
  .workflow-instruction-meta > span.customized { color: #8a6836; background: rgba(170, 119, 43, 0.09); }
  .workflow-readiness-note { margin: 0; color: #a06f45; font: 7px/1.4 Inter, sans-serif; }
  .role-symbol { position: relative; width: 29px; height: 29px; display: grid; flex: 0 0 auto; place-items: center; overflow: hidden; border: 1px solid rgba(73, 121, 98, 0.12); border-radius: 9px; background: rgba(70, 108, 89, 0.055); }
  .role-planner { color: #4e78a0; background: rgba(70, 123, 171, 0.08); }
  .role-implementer { color: #438c66; background: rgba(54, 145, 94, 0.08); }
  .role-reviewer { color: #8a6c46; background: rgba(155, 111, 56, 0.08); }
  .role-tester { color: #745e9c; background: rgba(111, 82, 158, 0.08); }
  .role-researcher { color: #4e7f88; background: rgba(63, 128, 138, 0.08); }
  .role-custom { color: #6d7d75; background: rgba(85, 109, 98, 0.07); }
  @keyframes role-icon-plan { 0%, 100% { transform: translate(0, 0); } 48% { transform: translate(1px, -1px); } }
  @keyframes role-icon-build { 0%, 100% { transform: scale(1); } 45% { transform: scale(.9); } 70% { transform: scale(1.04); } }
  @keyframes role-icon-review { 0%, 100% { transform: translate(0, 0) rotate(0); } 50% { transform: translate(1px, 1px) rotate(-4deg); } }
  @keyframes role-icon-test { 0%, 100% { transform: translateY(0); } 42% { transform: translateY(-1.5px); } 65% { transform: translateY(.5px); } }
  @keyframes role-icon-research { to { transform: rotate(360deg); } }
  @keyframes role-icon-custom { 0%, 100% { transform: translateX(0); } 40% { transform: translateX(-1px); } 70% { transform: translateX(1px); } }
  .workflow-instruction-field textarea { min-height: 54px; }
  .workflow-contract-pair { display: grid; grid-template-columns: 1fr 1fr; gap: 7px; }

  .terminal-window.dark .terminal-card { color: #dbe7e1; border-color: rgba(190, 209, 200, 0.13); background: #141d19; }
  .terminal-window.dark .terminal-card.dock-moving,
  .terminal-window.dark .terminal-card.dock-target,
  .terminal-window.dark .terminal-card.settling { border-color: rgba(91, 186, 143, 0.5); box-shadow: inset 0 0 0 2px rgba(91, 186, 143, 0.1), inset 0 -10px 24px rgba(8, 21, 15, 0.18); }
  .terminal-window.dark .dock-silhouette { border-color: rgba(96, 193, 149, 0.5); background: rgba(72, 157, 116, 0.06); box-shadow: inset 0 0 0 1px rgba(154, 220, 188, 0.08); }
  .terminal-window.dark .dock-silhouette::before { border-color: rgba(99, 197, 152, 0.52); background: linear-gradient(135deg, rgba(79, 174, 128, 0.22), rgba(69, 149, 111, 0.08)); }
  .terminal-window.dark .dock-silhouette span { color: #a8d9c2; background: rgba(27, 51, 40, 0.92); }
  .terminal-window.dark .terminal-card.workflow-preview.dock-target,
  .terminal-window.dark .terminal-card.workflow-preview.dock-moving { border-color: rgba(190, 209, 200, 0.13); box-shadow: none; }
  .terminal-window.dark .workflow-merge-preview::before { background: radial-gradient(ellipse at center, rgba(78, 156, 115, calc(0.11 + var(--dock-proximity) * 0.29)) 0 28%, rgba(39, 119, 79, calc(0.07 + var(--dock-proximity) * 0.18)) 56%, transparent 82%); }
  .terminal-window.dark .workflow-merge-preview::after { background: rgba(68, 157, 109, calc(0.22 + var(--dock-proximity) * 0.5)); }
  .terminal-window.dark .terminal-card.normal-preview .workflow-merge-preview::before { background: radial-gradient(ellipse at center, rgba(103, 142, 123, calc(0.09 + var(--dock-proximity) * 0.22)) 0 28%, rgba(68, 111, 90, calc(0.05 + var(--dock-proximity) * 0.14)) 56%, transparent 82%); filter: saturate(0.4); }
  .terminal-window.dark .terminal-card.normal-preview .workflow-merge-preview::after { background: rgba(91, 137, 114, calc(0.15 + var(--dock-proximity) * 0.32)); }
  .terminal-window.dark .terminal-card > header,
  .terminal-window.dark .terminal-composer { border-color: rgba(190, 209, 200, 0.09); }
  .terminal-window.dark .identity strong { color: #e2ebe6; }
  .terminal-window.dark .identity small,
  .terminal-window.dark .hint { color: #93a19a; }
  .terminal-window.dark .agent-icon,
  .terminal-window.dark .source-badge { background: rgba(205, 222, 213, 0.07); }
  .terminal-window.dark .source-badge { color: #a7b5ae; }
  .terminal-window.dark .access-badge.auto-review { color: #b4d3ee; background: #29445d; }
  .terminal-window.dark .access-badge.full-access { color: #e4b88f; background: #543b29; }
  .terminal-window.dark .access-badge.external-session { color: #c3d1c9; background: #35473e; }
  .terminal-window.dark .terminal-name-editor input { color: #d9e5df; border-color: rgba(195, 218, 207, 0.14); background: rgba(219, 233, 226, 0.055); }
  .terminal-window.dark .header-actions-menu { color: #b7c8bf; border-color: rgba(205, 222, 213, 0.12); background: rgba(24, 35, 30, 0.98); box-shadow: 0 10px 28px rgba(0, 0, 0, 0.3); }
  .terminal-window.dark header .header-actions-menu > button,
  .terminal-window.dark .header-menu-zoom { color: #b7c8bf; }
  .terminal-window.dark header .header-actions-menu > button:hover { color: #8bd3b0; background: rgba(96, 187, 144, 0.08); }
  .terminal-window.dark header .header-actions-menu > button.danger { color: #d48b83; }
  .terminal-window.dark header .header-menu-zoom > button { color: #b7cbc1; background: rgba(218, 234, 226, 0.055); }
  .terminal-window.dark header .header-menu-zoom > button:hover { color: #8bd3b0; background: rgba(96, 187, 144, 0.1); }
  .terminal-window.dark .header-menu-zoom output { color: #a5b6ad; }
  .terminal-window.dark .terminate-backdrop { background: rgba(4, 9, 7, 0.5); }
  .terminal-window.dark .terminate-dialog { color: #d5e1db; border-color: rgba(211, 128, 121, 0.2); background: #18221d; box-shadow: 0 14px 38px rgba(0, 0, 0, 0.36); }
  .terminal-window.dark .terminate-dialog strong { color: #e0a39d; }
  .terminal-window.dark .terminate-dialog p { color: #a7b7af; }
  .terminal-window.dark .terminate-dialog button:not(.danger) { color: #afbeb7; border-color: rgba(205, 222, 213, 0.13); }
  .terminal-window.dark .model-settings-dialog { border-color: rgba(133, 188, 161, 0.16); }
  .terminal-window.dark .model-settings-dialog > header strong,
  .terminal-window.dark .model-options button strong { color: #d0e1d8; }
  .terminal-window.dark .model-settings-icon { color: #8ed0b0; background: rgba(99, 181, 141, 0.1); }
  .terminal-window.dark .model-settings-label,
  .terminal-window.dark .model-settings-loading { color: #91a299; }
  .terminal-window.dark .model-settings-label b { color: #8fd0af; background: rgba(91, 177, 136, 0.1); }
  .terminal-window.dark .effort-scale span { color: #71847a; }
  .terminal-window.dark .effort-scale span.active { color: #8fd0af; }
  .terminal-window.dark .claude-model-settings input { color: #d0e1d8; border-color: rgba(205, 222, 213, 0.1); background: rgba(213, 233, 223, 0.035); }
  .terminal-window.dark .claude-model-settings > small { color: #91a299; }
  .terminal-window.dark .terminate-dialog .model-options > button { border-color: rgba(205, 222, 213, 0.08); background: rgba(213, 233, 223, 0.025); }
  .terminal-window.dark .terminate-dialog .model-options > button:hover { border-color: rgba(117, 194, 155, 0.2); background: rgba(94, 176, 135, 0.07); }
  .terminal-window.dark .terminate-dialog .model-options > button.active { border-color: rgba(112, 203, 157, 0.42); background: rgba(84, 171, 127, 0.11); box-shadow: inset 2px 0 #64b98f; }
  .terminal-window.dark .rate-limit-meter small,
  .terminal-window.dark .pending-images-label { color: #8f9f97; }
  .terminal-window.dark .rate-limit-meter > i { background: rgba(181, 207, 194, 0.12); }
  .terminal-window.dark .pending-images { background: rgba(83, 174, 129, 0.055); }
  .terminal-window.dark .message-file,
  .terminal-window.dark .pending-images > span.file-attachment { color: #b2c2ba; border-color: rgba(205, 222, 213, 0.12); background: rgba(255, 255, 255, 0.035); }
  .terminal-window.dark .slash-command-menu { color: #b7c8bf; border-color: rgba(205, 222, 213, 0.12); background: #18231e; box-shadow: 0 12px 32px rgba(0, 0, 0, 0.34); }
  .terminal-window.dark .slash-command-heading { border-color: rgba(205, 222, 213, 0.08); }
  .terminal-window.dark .slash-command-heading strong,
  .terminal-window.dark .slash-command-heading kbd { color: #9cafa5; }
  .terminal-window.dark .slash-command-heading small { color: #81938a; }
  .terminal-window.dark .slash-command-heading kbd { border-color: rgba(205, 222, 213, 0.11); background: rgba(218, 234, 226, 0.045); }
  .terminal-window.dark .terminal-composer .slash-command-menu > button { color: #a8b9b0; }
  .terminal-window.dark .terminal-composer .slash-command-menu > button:hover,
  .terminal-window.dark .terminal-composer .slash-command-menu > button.active { color: #98d3b7; background: rgba(91, 174, 132, 0.1); }
  .terminal-window.dark .slash-command-menu code { color: #8dceb0; }
  .terminal-window.dark .terminal-composer .queued-prompt-tray { color: #a7bdcd; border-color: rgba(125, 166, 199, 0.13); background: rgba(91, 143, 184, 0.065); }
  .terminal-window.dark .terminal-composer .queued-prompt-tray:hover:not(:disabled) { border-color: rgba(128, 177, 216, 0.23); background: rgba(91, 143, 184, 0.1); }
  .terminal-window.dark .queue-mark { color: #87b8dc; background: rgba(105, 166, 210, 0.11); }
  .terminal-window.dark .queue-copy small,
  .terminal-window.dark .queue-shortcut { color: #829daa; }
  .terminal-window.dark .queue-copy strong { color: #b1c6d2; }
  .terminal-window.dark .queue-shortcut kbd { color: #9bb8c9; border-color: rgba(169, 197, 214, 0.14); background: rgba(220, 235, 243, 0.055); }
  .terminal-window.dark .work-tray { border-color: rgba(190, 209, 200, 0.07); background: rgba(202, 222, 212, 0.018); }
  .terminal-window.dark .work-tray-toggle { color: #99aea4; }
  .terminal-window.dark .work-tray-toggle:hover { background: rgba(205, 225, 215, 0.025); }
  .terminal-window.dark .work-tray-toggle small { color: #7f9188; }
  .terminal-window.dark .work-card { border-color: rgba(205, 222, 213, 0.07); background: rgba(218, 234, 226, 0.025); }
  .terminal-window.dark .work-card-heading strong { color: #93aa9f; }
  .terminal-window.dark .todo-card li { color: #77867f; }
  .terminal-window.dark .todo-card li.active,
  .terminal-window.dark .goal-card p { color: #b4c7bd; }
  .terminal-window.dark .todo-card li.done { color: #84958c; }
  .terminal-window.dark .hub-tabs { border-color: rgba(190, 209, 200, 0.07); }
  .terminal-window.dark .hub-tabs button { color: #84938c; }
  .terminal-window.dark .hub-tabs button.active { color: #83c6a6; border-bottom-color: #59ad84; }
  .terminal-window.dark .hub-tabs span { background: rgba(205, 222, 213, 0.07); }
  .terminal-window.dark .plan-panel > header strong,
  .terminal-window.dark .plan-items strong,
  .terminal-window.dark .notes-panel > header strong,
  .terminal-window.dark .session-note > header strong { color: #b8cbc1; }
  .terminal-window.dark .plan-explanation { color: #a9bbb2; background: rgba(77, 152, 195, 0.07); }
  .terminal-window.dark .plan-items li { border-color: rgba(205, 222, 213, 0.07); background: rgba(218, 234, 226, 0.025); }
  .terminal-window.dark .plan-items li.active { border-color: rgba(100, 170, 207, 0.2); background: rgba(77, 152, 195, 0.07); }
  .terminal-window.dark .note-editor,
  .terminal-window.dark .session-note { border-color: rgba(205, 222, 213, .08); background: rgba(218, 234, 226, .025); }
  .terminal-window.dark .session-note.pinned { border-color: rgba(91, 177, 137, .17); background: rgba(91, 177, 137, .045); }
  .terminal-window.dark .note-editor-heading > input,
  .terminal-window.dark .note-editor textarea { color: #c7d6ce; border-color: rgba(205, 222, 213, .1); background: rgba(218, 234, 226, .035); }
  .terminal-window.dark .note-editor-heading label { border-color: rgba(205, 222, 213, .09); }
  .terminal-window.dark .session-note-body { color: #aebfb6; }
  .terminal-window.dark .terminal-output { color: #b8c6bf; background: linear-gradient(180deg, rgba(114, 151, 134, 0.035), transparent); }
  .terminal-window.dark .load-earlier-chat { color: #9aaba2; border-color: rgba(205, 222, 213, 0.08); background: rgba(218, 234, 226, 0.02); }
  .terminal-window.dark .load-earlier-chat:hover { color: #88c8a8; border-color: rgba(91, 177, 137, 0.17); background: rgba(91, 177, 137, 0.045); }
  .terminal-window.dark .load-earlier-chat small { color: #8fa198; background: rgba(205, 222, 213, 0.055); }
  .terminal-window.dark .chat-message { border-color: rgba(205, 222, 213, 0.08); background: rgba(218, 234, 226, 0.035); }
  .terminal-window.dark .chat-message.agent-message.intervention-required { border-color: rgba(215, 166, 78, 0.17); background: rgba(202, 149, 55, 0.025); }
  .terminal-window.dark .intervention-indicator { color: #dfbd79; }
  .terminal-window.dark .intervention-indicator > span { color: #e1ad51; }
  .terminal-window.dark .intervention-indicator strong { color: #e3c17c; }
  .terminal-window.dark .handoff-button { color: #91a79c; }
  .terminal-window.dark .workflow-connection { background: linear-gradient(180deg, rgba(70, 182, 127, 0.15), #59c58b 45%, rgba(70, 182, 127, 0.15)); box-shadow: 0 0 9px rgba(75, 201, 137, 0.38); }
  .terminal-window.dark .workflow-connection-editor { color: #7ed4a6; border-color: rgba(101, 210, 153, .22); background: rgba(25, 43, 34, .97); box-shadow: 0 8px 24px rgba(0, 0, 0, .32); }
  .terminal-window.dark .workflow-role-fab { color: #91cbae; border-color: rgba(86, 184, 134, 0.22); background: radial-gradient(circle at 34% 24%, rgba(129,208,168,.16) 0 12%, rgba(83,153,116,.06) 31%, transparent 52%), linear-gradient(145deg, #24342c 8%, #15201a 90%); box-shadow: inset 0 1px 0 rgba(190,232,209,.12), inset 0 -3px 5px rgba(0,0,0,.24), 0 6px 13px rgba(0,0,0,.36), 0 1px 2px rgba(0,0,0,.5), 0 0 0 3px rgba(74,164,116,.055); }
  .terminal-window.dark .workflow-role-fab:hover,
  .terminal-window.dark .workflow-role-fab.open { border-color: rgba(100, 205, 151, 0.4); box-shadow: inset 0 1px 0 rgba(190,232,209,.13), inset 0 -3px 5px rgba(0,0,0,.25), 0 8px 17px rgba(0,0,0,.4), 0 0 0 4px rgba(74,174,122,.08); }
  .terminal-window.dark .workflow-step-badge { border-color: rgba(24,35,29,.9); background: #4ba978; box-shadow: 0 2px 6px rgba(0,0,0,.34); }
  .terminal-window.dark .workflow-role-tooltip { color: #c4d5cc; border-color: rgba(200, 219, 210, 0.13); background: rgba(24, 34, 29, 0.98); box-shadow: 0 9px 24px rgba(0, 0, 0, 0.34); }
  .terminal-window.dark .workflow-role-tooltip small { color: #8fa198; }
  .terminal-window.dark .workflow-role-popover { color: #d4e1da; border-color: rgba(200, 219, 210, 0.13); background: #18221d; box-shadow: 0 18px 48px rgba(0, 0, 0, 0.36); }
  .terminal-window.dark .workflow-popover-bridge::before { border-color: rgba(200, 219, 210, 0.13); background: #18221d; }
  .terminal-window.dark .workflow-popover-heading strong { color: #dce9e2; }
  .terminal-window.dark .workflow-popover-heading small { color: #93a69c; }
  .terminal-window.dark .workflow-popover-heading button,
  .terminal-window.dark .workflow-popover-actions button { color: #aebfb6; border-color: rgba(205, 222, 213, 0.13); background: rgba(220, 235, 227, 0.035); }
  .terminal-window.dark .workflow-popover-actions button.primary { color: white; border-color: #347c5a; background: #347c5a; }
  .terminal-window.dark .workflow-top,
  .terminal-window.dark .workflow-bottom { background: linear-gradient(90deg, rgba(70, 182, 127, 0.15), #59c58b 45%, rgba(70, 182, 127, 0.15)); }
  .terminal-window.dark .agent-typing { border-color: rgba(205, 222, 213, 0.08); background: rgba(218, 234, 226, 0.035); }
  .terminal-window.dark .chat-message.user-message { background: rgba(76, 169, 124, 0.09); }
  .terminal-window.dark .chat-message header strong,
  .terminal-window.dark .chat-message.user-message > pre,
  .terminal-window.dark .markdown-content,
  .terminal-window.dark .turn-files code { color: #bdcbc4; }
  .terminal-window.dark .markdown-content :global(strong),
  .terminal-window.dark .markdown-content :global(h1),
  .terminal-window.dark .markdown-content :global(h2),
  .terminal-window.dark .markdown-content :global(h3),
  .terminal-window.dark .markdown-content :global(h4) { color: #d1ddd7; }
  .terminal-window.dark .markdown-content :global(a) { color: #76c49d; text-decoration-color: rgba(118, 196, 157, 0.42); }
  .terminal-window.dark .markdown-content :global(blockquote) { color: #a5b6ad; background: rgba(91, 177, 137, 0.055); }
  .terminal-window.dark .markdown-content :global(code) { color: #b9d9c9; background: rgba(157, 205, 181, 0.075); }
  .terminal-window.dark .markdown-content :global(pre) { border-color: rgba(205, 222, 213, 0.08); background: rgba(7, 16, 12, 0.2); }
  .terminal-window.dark .markdown-content :global(pre code) { color: #bdcbc4; background: transparent; }
  .terminal-window.dark .markdown-content :global(.markdown-table-wrap) { border-color: rgba(205, 222, 213, 0.1); background: rgba(7, 16, 12, 0.12); }
  .terminal-window.dark .markdown-content :global(th) { color: #d2e0d9; background: rgba(98, 178, 138, 0.08); }
  .terminal-window.dark .markdown-content :global(th),
  .terminal-window.dark .markdown-content :global(td) { border-color: rgba(205, 222, 213, 0.075); }
  .terminal-window.dark .reasoning-update { border-color: #6c9abc; background: linear-gradient(90deg, rgba(90, 139, 174, .085), rgba(90, 139, 174, .018)); }
  .terminal-window.dark .reasoning-update > header { color: #92b3ca; }
  .terminal-window.dark .reasoning-update > header > span { background: rgba(105, 157, 194, .1); }
  .terminal-window.dark .reasoning-update > header strong { color: #a9c4d6; }
  .terminal-window.dark .reasoning-update .markdown-content { color: #bccbc4; }
  .terminal-window.dark .turn-files > header strong { color: #8bc6a8; }
  .terminal-window.dark .turn-files { border-color: rgba(104, 180, 143, .14); background: linear-gradient(145deg, rgba(91, 177, 137, .065), rgba(91, 177, 137, .018)); }
  .terminal-window.dark .turn-files-mark { color: #8bc6a8; background: rgba(91, 177, 137, .09); }
  .terminal-window.dark .turn-files > header small { color: #91a59a; background: rgba(205, 222, 213, .06); }
  .terminal-window.dark .turn-files code { border-color: rgba(205, 222, 213, .065); background: transparent; }
  .terminal-window.dark .handoff-backdrop { background: rgba(4, 9, 7, 0.55); }
  .terminal-window.dark .handoff-dialog { color: #d4e1da; border-color: rgba(200, 219, 210, 0.13); background: #18221d; box-shadow: 0 18px 48px rgba(0, 0, 0, 0.36); }
  .terminal-window.dark .workflow-role-fields label { color: #98aaa1; }
  .terminal-window.dark .workflow-role-fields input,
  .terminal-window.dark .workflow-role-fields textarea { color: #d7e3dc; border-color: rgba(205, 222, 213, 0.11); background: rgba(222, 235, 228, 0.04); }
  .terminal-window.dark .workflow-role-trigger { color: #d5e2db; border-color: rgba(94, 177, 136, 0.12); background: linear-gradient(135deg, rgba(222, 235, 228, 0.055), rgba(64, 151, 106, 0.035)); }
  .terminal-window.dark .workflow-role-trigger.open { border-color: rgba(91, 190, 140, 0.28); background: rgba(70, 160, 113, 0.07); }
  .terminal-window.dark .workflow-role-trigger strong,
  .terminal-window.dark .workflow-role-menu strong { color: #d6e3dc; }
  .terminal-window.dark .workflow-role-trigger small,
  .terminal-window.dark .workflow-role-menu small { color: #91a39a; }
  .terminal-window.dark .workflow-role-menu { border-color: rgba(205, 222, 213, 0.12); background: #18221d; box-shadow: 0 16px 34px rgba(0, 0, 0, 0.34); }
  .terminal-window.dark .workflow-role-menu > button { color: #becdc5; }
  .terminal-window.dark .workflow-role-menu > button:hover,
  .terminal-window.dark .workflow-role-menu > button.active { background: rgba(76, 171, 122, 0.08); }
  .terminal-window.dark .workflow-role-replace-warning { border-color: rgba(211, 162, 84, 0.18); background: rgba(190, 132, 48, 0.075); }
  .terminal-window.dark .workflow-role-replace-warning strong { color: #d6b987; }
  .terminal-window.dark .workflow-role-replace-warning small { color: #a99779; }
  .terminal-window.dark .workflow-role-replace-warning button,
  .terminal-window.dark .workflow-instruction-meta button { color: #b7c7bf; border-color: rgba(205, 222, 213, 0.12); background: rgba(222, 235, 228, 0.035); }
  .terminal-window.dark .workflow-role-replace-warning button.confirm { color: #fff; border-color: #8c6734; background: #8c6734; }
  .terminal-window.dark .workflow-contract-toggle { color: #b8c9c0; border-color: rgba(205, 222, 213, 0.11); background: rgba(222, 235, 228, 0.035); }
  .terminal-window.dark .workflow-contract-toggle strong { color: #d6e3dc; }
  .terminal-window.dark .workflow-instruction-meta > span { color: #a2b0a9; background: rgba(218, 234, 226, 0.055); }
  .terminal-window.dark .workflow-instruction-meta > span.ready { color: #8bc9aa; background: rgba(76, 171, 122, 0.09); }
  .terminal-window.dark .workflow-instruction-meta > span.customized { color: #d1b27e; background: rgba(190, 132, 48, 0.09); }
  .terminal-window.dark .workflow-readiness-note { color: #c69c72; }
  .terminal-window.dark .role-symbol { border-color: rgba(205, 222, 213, 0.1); }
  .terminal-window.dark .handoff-dialog > header strong,
  .terminal-window.dark .handoff-options strong { color: #d9e6df; }
  .terminal-window.dark .handoff-dialog > header small,
  .terminal-window.dark .handoff-target,
  .terminal-window.dark .handoff-note,
  .terminal-window.dark .handoff-options small { color: #94a69d; }
  .terminal-window.dark .handoff-target select,
  .terminal-window.dark .handoff-note textarea,
  .terminal-window.dark .handoff-options > label { color: #d1ded7; border-color: rgba(205, 222, 213, 0.11); background: rgba(218, 234, 226, 0.035); }
  .terminal-window.dark .handoff-preview { border-color: #5bad83; background: rgba(91, 177, 137, 0.055); }
  .terminal-window.dark .handoff-preview > strong { color: #8bc6a8; }
  .terminal-window.dark .handoff-preview pre { color: #b8c8c0; }
  .terminal-window.dark .handoff-dialog > footer button { color: #aebfb6; border-color: rgba(205, 222, 213, 0.13); }
  .terminal-window.dark .change-list code { color: #bdcbc4; }
  .terminal-window.dark .privacy-note { border-color: rgba(205, 222, 213, 0.07); color: #78877f; }
  .terminal-window.dark textarea { color: #d0ddd6; border-color: rgba(205, 222, 213, 0.12); background: rgba(220, 234, 227, 0.045); }
  .terminal-window.dark .terminal-composer .composer-tools-trigger { color: #a9bbb2; border-color: rgba(205, 222, 213, 0.1); background: rgba(218, 234, 226, 0.045); }
  .terminal-window.dark .terminal-composer .composer-tools-trigger:hover,
  .terminal-window.dark .terminal-composer .composer-tools-trigger.active { color: #8ed0ae; border-color: rgba(119, 202, 160, 0.22); background: rgba(88, 176, 132, 0.1); }
  .terminal-window.dark .composer-tools-menu { color: #afc0b7; border-color: rgba(205, 222, 213, 0.12); background: rgba(24, 34, 29, 0.99); box-shadow: 0 14px 34px rgba(0, 0, 0, 0.34); }
  .terminal-window.dark .terminal-composer .composer-tools-menu > button:hover:not(:disabled),
  .terminal-window.dark .terminal-composer .composer-tools-menu > button.active { color: #8fd0af; background: rgba(91, 177, 136, 0.09); }
  .terminal-window.dark .composer-tools-menu .tool-icon { background: rgba(205, 225, 215, 0.055); }
  .terminal-window.dark .composer-tools-menu strong { color: #cfddd6; }
  .terminal-window.dark .composer-tools-menu small,
  .terminal-window.dark .composer-tools-menu .tool-chevron { color: #85978e; }
  .terminal-window.dark .mode-feedback { color: #a6b8af; border-color: rgba(205, 222, 213, 0.1); background: rgba(29, 41, 35, 0.98); box-shadow: 0 5px 14px rgba(0, 0, 0, 0.22); }
  .terminal-window.dark .mode-feedback.success { color: #84c9a7; }
  .terminal-window.dark .terminal-composer .composer-resize-handle { color: #8fa49a; }
  .terminal-window.dark .terminal-composer .composer-resize-handle:hover,
  .terminal-window.dark .terminal-composer .composer-resize-handle:focus-visible { background: rgba(205, 225, 215, 0.045); }
  .terminal-window.dark .pending-images button { color: #c6d5ce; border-color: rgba(205, 222, 213, 0.14); background: #26332d; }
  .terminal-window.dark .permission strong { color: #dfc6ac; }
  .terminal-window.dark .permission code,
  .terminal-window.dark .permission button { color: #bdcbc4; background: rgba(218, 232, 225, 0.055); }
  .terminal-window.dark .agent-question { border-color: rgba(83, 165, 204, 0.2); background: rgba(55, 139, 178, 0.07); }
  .terminal-window.dark .agent-question-item > strong { color: #d4e2dc; }
  .terminal-window.dark .agent-question-item > em,
  .terminal-window.dark .question-options button small { color: #8fa59b; }
  .terminal-window.dark .question-options button { color: #c5d7cf; border-color: rgba(178, 210, 224, 0.12); background: rgba(219, 235, 228, 0.045); }
  @media (prefers-reduced-motion: reduce) {
    .terminal-card { transition-duration: 0.01ms; }
    .dock-silhouette { animation: none; }
    .workflow-merge-preview::before,
    .workflow-merge-preview::after { animation: none; }
    .agent-typing span { animation: none; opacity: 0.7; }
    .send-spinner,
    .mode-spinner { animation: none; }
    .workflow-connection { animation: none; opacity: 0.82; }
  }
</style>
