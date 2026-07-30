<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import type { AgentSession, DockPreviewEvent, PermissionAction, Preferences, PromptAttachmentInput, QuestionAnswer, SessionActivity, TerminalWindowState } from "$lib/domain";
  import type { HubSession, WorkItemStatus } from "$lib/hubProtocol";
  import BrandIcon from "$lib/BrandIcon.svelte";
  import LumeLogo from "$lib/LumeLogo.svelte";
  import LumeMascot from "$lib/LumeMascot.svelte";
  import { displayText, localize, type Language } from "$lib/i18n";
  import {
    clipboardHasImage,
    clipboardMayContainImage,
    collectClipboardImages,
    createImagePreview,
    prepareClipboardImage,
  } from "$lib/imageAttachments";
  import { renderSafeMarkdown } from "$lib/markdown.js";
  import { latestResponseText, sameResponseText } from "$lib/responseDedup.js";
  import {
    mergeFileChanges,
    summarizeFileChanges,
    type FileChangeSummary,
  } from "$lib/fileChanges";
  import { sessionCapabilities } from "$lib/sessionCapabilities";
  import { resolveTerminalSession } from "$lib/sessionIdentity";
  import {
    beginLayeredTerminalResize,
    beginTerminalNativeDrag,
    answerQuestion as answerInteractiveQuestion,
    cancelTerminalWindowMove,
    closeTerminalWindow,
    decidePermission,
    finishLayeredTerminalResize,
    getSessionCollaborationMode,
    interruptPrompt,
    loadDisplayBackend,
    loadPreferences,
    loadHubSnapshot,
    loadTerminalWindowState,
    markTerminalFrontendReady,
    moveTerminalWindow,
    openSessionSource,
    readLocalImageDataUrl,
    refreshAgentRateLimits,
    renameSession,
    resizeTerminalWindow,
    setSessionCollaborationMode,
    setTerminalFileDialogActive,
    steerQueuedPrompt,
    submitPrompt,
    syncTerminalWindowPosition,
    terminalGroupFullscreenActive,
    terminateSession,
    toggleTerminalGroupFullscreen,
    undockTerminalWindow,
    type CollaborationMode,
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
    action?: "plan" | "default" | "interrupt" | "steer" | "rename" | "detach" | "fullscreen" | "zoom-in" | "zoom-out" | "close";
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
  let sending = $state(false);
  let steeringQueued = $state(false);
  let collaborationMode = $state<CollaborationMode>("default");
  let collaborationModeChanging = $state(false);
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
  let pendingResize: { x: number; y: number; width: number; height: number } | null = null;
  let resizeSyncRunning = false;
  let settling = $state(false);
  let dockMovingLabel = $state<string | null>(null);
  let dockPreview = $state<NonNullable<DockPreviewEvent["preview"]> | null>(null);
  let terminateConfirm = $state(false);
  let terminating = $state(false);
  let interrupting = $state(false);
  let renamingSession = $state(false);
  let renameDraft = $state("");
  let savingSessionName = $state(false);
  let activeTab = $state<"chat" | "changes">("chat");
  let workTrayExpanded = $state(true);
  let rateLimitRefreshRequested = false;
  let outputElement = $state<HTMLDivElement | null>(null);
  let outputFollowingTail = true;
  let language = $state<Language>("en");
  let darkMode = $state<boolean | undefined>(undefined);
  let systemDark = $state(false);
  let workClock = $state(Date.now());
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
  let textZoomOpen = $state(false);
  let headerActionsOpen = $state(false);
  const effectiveDark = $derived(darkMode ?? systemDark);
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

  function sessionDisplayName(item: AgentSession) {
    return item.sessionName?.trim() || item.project?.trim() || item.agentLabel;
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

  function agentSlashCommands(): SlashCommand[] {
    const catalog =
      session?.agent === "codex"
        ? codexSlashCommands
        : session?.agent === "claude_code"
          ? claudeSlashCommands
          : session?.agent === "gemini"
            ? geminiSlashCommands
            : [];
    return catalog.map(([name, description]) => ({ name, description, source: "agent" }));
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
  const todo = $derived(session?.workSummary.todo ?? null);
  const goal = $derived(session?.workSummary.goal ?? null);
  const completedTodoItems = $derived(
    todo?.items.filter((item) => item.status === "completed").length ?? 0,
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
    return tr(
      "This agent does not support prompts through Lume yet",
      "Este agente ainda não aceita prompts pelo Lume",
    );
  }

  const activities = $derived(session?.activities ?? []);
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
    return summarizeFileChanges(
      activity.detail ?? "",
      activityReportedFiles(activity),
      session?.workingDirectory,
    );
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
  const chatEntries = $derived.by<ChatEntry[]>(() => {
    let sequence = 0;
    const promptTimes = activities
      .filter((activity) => activity.kind === "prompt")
      .map((activity) => activity.createdAt)
      .sort((left, right) => left - right);
    const promptSegment = (createdAt: number) => {
      let segment = Number.NEGATIVE_INFINITY;
      for (const promptTime of promptTimes) {
        if (promptTime > createdAt) break;
        segment = promptTime;
      }
      return segment;
    };
    const entries: ChatEntry[] = [];
    for (const activity of activities) {
      if (activity.kind === "queued_prompt") continue;
      const files = activityChanges(activity);
      const matchingMessage = activity.kind === "message"
        ? [...entries].reverse().find((entry) =>
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
      entries.push({
        id: `activity:${activity.id}`,
        activity: { ...activity },
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
      const matchingMessage = [...entries].reverse().find((entry) =>
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
      const segment = promptSegment(session.updatedAt);
      const matchingMessage = entries.find((entry) =>
        entry.activity.kind === "message" &&
        sameResponseText(entry.activity.detail, responseKey) &&
        promptSegment(entry.activity.createdAt) === segment
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

  onMount(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let stopWindowChanges: (() => void) | undefined;
    let stopMoved: (() => void) | undefined;
    let stopResized: (() => void) | undefined;
    let stopPreferences: (() => void) | undefined;
    let stopDockPreview: (() => void) | undefined;
    let stopNativeDragEnded: (() => void) | undefined;
    const workClockInterval = setInterval(() => {
      workClock = Date.now();
    }, 30_000);
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
      if (!event.target.closest(".text-zoom-control")) textZoomOpen = false;
      if (!event.target.closest(".header-overflow")) headerActionsOpen = false;
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
      const [nextPreferences, nextDisplayBackend] = await Promise.all([
        loadPreferences(),
        loadDisplayBackend(),
      ]);
      language = nextPreferences.language;
      darkMode = nextPreferences.darkMode;
      displayBackend = nextDisplayBackend;
      fullscreen = await currentWindow.isFullscreen().catch(() => false);
      if (!fullscreen) fullscreen = await terminalGroupFullscreenActive(label).catch(() => false);
      await initializeTerminal();
      if (disposed) return;
      await tick();
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      await markTerminalFrontendReady(label);
      if (disposed) return;
      stopListening = await listen("lume://sessions-changed", async () => {
        if (session && windowState) {
          await refresh();
          if (!session && windowState.sessionSource === "cli") {
            await closeTerminal();
          }
        } else {
          await initializeTerminal();
        }
      });
      stopWindowChanges = await listen("lume://terminal-windows-changed", async () => {
        try {
          windowState = await loadTerminalWindowState(label);
          fullscreen = await terminalGroupFullscreenActive(label);
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
      colorScheme.removeEventListener("change", syncSystemTheme);
      document.removeEventListener("click", openMarkdownLink);
      document.removeEventListener("pointerdown", closeHeaderPopovers);
      window.removeEventListener("keydown", interruptOnEscape);
      if (resizeEndTimer) clearTimeout(resizeEndTimer);
      if (nativeDragEndTimer) clearTimeout(nativeDragEndTimer);
      clearInterval(workClockInterval);
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
    const snapshot = await loadHubSnapshot();
    const shouldFollow = outputFollowingTail;
    session = windowState ? resolveTerminalSession(windowState, snapshot.sessions) ?? null : null;
    if (shouldFollow && outputFollowingTail) {
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
    outputFollowingTail = outputDistanceFromTail() <= 24;
  }

  function handleOutputWheel(event: WheelEvent) {
    if (event.deltaY < 0) {
      outputFollowingTail = false;
    }
  }

  function activityMark(activity: SessionActivity) {
    return {
      prompt: "›",
      queued_prompt: "⌛",
      message: "◆",
      activity: "·",
      analysis: "···",
      plan: "≡",
      command: "$",
      file: "±",
      test: "✓",
      tool: "⌁",
      permission: "!",
      question: "?",
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
    if (fullscreen && windowState.docked) return;
    if ((event.target as HTMLElement).closest("button, input, textarea, form")) return;
    if (displayBackend === "gnome-wayland-limited") {
      message = "Window dragging requires XWayland in GNOME.";
      return;
    }
    if (displayBackend === "xwayland-fallback") {
      event.preventDefault();
      dragging = true;
      nativeDragActive = true;
      dockMovingLabel = null;
      dockPreview = null;
      void beginTerminalNativeDrag(label)
        .catch((error) => {
          nativeDragActive = false;
          dragging = false;
          message = String(error).replace(/^Error:\s*/, "");
        });
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
      if (windowState?.docked) {
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
    if (fullscreen && state.docked) return;
    event.preventDefault();
    event.stopPropagation();
    dragging = false;
    finalizeRequested = false;
    pendingMove = null;
    dockPreview = null;
    dockMovingLabel = null;
    resizing = true;
    if (state.layered || displayBackend === "xwayland-fallback") {
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
    message = null;
    try {
      collaborationMode = await setSessionCollaborationMode(session.id, nextMode);
      return true;
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
      return false;
    } finally {
      collaborationModeChanging = false;
    }
  }

  async function toggleCollaborationMode() {
    const nextMode: CollaborationMode = collaborationMode === "plan" ? "default" : "plan";
    await applyCollaborationMode(nextMode);
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
      || !canSubmit
      || !readyForPrompt
    ) return;
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

  async function chooseImages() {
    if (!canSubmit || !readyForPrompt || sending) return;
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
          filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "gif"] }],
        });
      } finally {
        if (terminalLowered) {
          await setTerminalFileDialogActive(label, false).catch(() => undefined);
        }
      }
      const paths = (Array.isArray(selected) ? selected : selected ? [selected] : [])
        .filter((path): path is string => typeof path === "string");
      for (const path of paths.slice(0, 4 - promptAttachments.length)) {
        const previewDataUrl = await imagePreview(path);
        promptAttachments = [
          ...promptAttachments,
          {
            name: path.split(/[\\/]/).pop() || "image",
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

  function removeImage(index: number) {
    promptAttachments = promptAttachments.filter((_, current) => current !== index);
  }

  async function imagePreview(path: string): Promise<string> {
    const source = await readLocalImageDataUrl(path);
    return createImagePreview(source, language);
  }

  async function pasteImages(event: ClipboardEvent) {
    if (!clipboardHasImage(event) && !clipboardMayContainImage(event)) return;
    event.preventDefault();
    message = null;
    if (!canSubmit || !readyForPrompt || sending || !capabilities?.canAttachImages) {
      message = tr(
        "Images can only be attached when this session is ready for a prompt.",
        "Imagens só podem ser anexadas quando esta sessão estiver pronta para um prompt.",
      );
      return;
    }
    try {
      const { files, paths } = await collectClipboardImages(event, language);
      const available = 4 - promptAttachments.length;
      const prepared: PromptAttachmentInput[] = [];
      for (const [index, file] of files.slice(0, available).entries()) {
        prepared.push(await prepareClipboardImage(file, index, language));
      }
      for (const path of paths.slice(0, available - prepared.length)) {
        prepared.push({
          name: path.split(/[\\/]/).pop() || "image",
          mimeType: "",
          path,
          previewDataUrl: await imagePreview(path),
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
      style:--chat-font-adjust={`${(textZoom - 1) * 9}px`}
      style:--chat-small-font-adjust={`${(textZoom - 1) * 8}px`}
      style:--chat-tiny-font-adjust={`${(textZoom - 1) * 7}px`}
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
            <small>{session.project}</small>
          </div>
          <button class="rename-button" type="button" onclick={beginSessionRename} aria-label={tr("Rename session", "Renomear sessão")} title={tr("Rename session", "Renomear sessão")}>
            <svg viewBox="0 0 20 20"><path d="m4 14-.5 2.5L6 16l9-9-2-2-9 9Z"></path><path d="m11.5 6.5 2 2"></path></svg>
          </button>
        {/if}
        <span class="source-badge">
          <BrandIcon name={sourceIcon(session)} size={10} />
          {sourceLabel(session)}
        </span>
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
          <span class="text-zoom-control">
            <button
              class:active={textZoomOpen}
              class="text-zoom-button"
              type="button"
              aria-expanded={textZoomOpen}
              aria-label={tr("Adjust terminal text size", "Ajustar tamanho dos textos do terminal")}
              title={tr("Text size", "Tamanho do texto")}
              onclick={() => (textZoomOpen = !textZoomOpen)}
            >
              <svg viewBox="0 0 20 20"><circle cx="8.5" cy="8.5" r="4.5"></circle><path d="m12 12 4 4M8.5 6.5v4M6.5 8.5h4"></path></svg>
            </button>
            {#if textZoomOpen}
              <span
                class="text-zoom-popover"
                role="group"
                aria-label={tr("Terminal text size", "Tamanho dos textos do terminal")}
                onpointerdown={(event) => event.stopPropagation()}
              >
                <button
                  disabled={textZoom <= textZoomMin}
                  type="button"
                  aria-label={tr("Decrease text size", "Diminuir textos")}
                  onclick={() => setTextZoom(textZoom - 0.1)}
                >−</button>
                <output>{Math.round(textZoom * 100)}%</output>
                <button
                  disabled={textZoom >= textZoomMax}
                  type="button"
                  aria-label={tr("Increase text size", "Aumentar textos")}
                  onclick={() => setTextZoom(textZoom + 0.1)}
                >+</button>
              </span>
            {/if}
          </span>
          <button class="fullscreen-button" type="button" onclick={toggleFullscreen} aria-label={fullscreen ? tr("Exit full screen", "Sair da tela cheia") : tr("Enter full screen", "Entrar em tela cheia")} title={fullscreen ? tr("Exit full screen", "Sair da tela cheia") : tr("Full screen", "Tela cheia")}>
            {#if fullscreen}
              <svg viewBox="0 0 20 20"><path d="M8 3v5H3M12 3v5h5M8 17v-5H3M12 17v-5h5" /></svg>
            {:else}
              <svg viewBox="0 0 20 20"><path d="M3 8V3h5M12 3h5v5M3 12v5h5M17 12v5h-5" /></svg>
            {/if}
          </button>
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
              <button type="button" role="menuitem" onclick={() => void toggleFullscreen()}>
                {#if fullscreen}
                  <svg viewBox="0 0 20 20"><path d="M8 3v5H3M12 3v5h5M8 17v-5H3M12 17v-5h5" /></svg>
                  <span>{tr("Exit full screen", "Sair da tela cheia")}</span>
                {:else}
                  <svg viewBox="0 0 20 20"><path d="M3 8V3h5M12 3h5v5M3 12v5h5M17 12v5h-5" /></svg>
                  <span>{tr("Enter full screen", "Entrar em tela cheia")}</span>
                {/if}
              </button>
              {#if windowState?.docked}
                <button type="button" role="menuitem" onclick={() => { headerActionsOpen = false; void detach(); }}>
                  <svg viewBox="0 0 20 20"><path d="M7 6 5.5 7.5a3 3 0 0 0 4.2 4.2l1.2-1.2M13 14l1.5-1.5a3 3 0 0 0-4.2-4.2L9.1 9.5" /></svg>
                  <span>{tr("Undock terminal", "Desacoplar terminal")}</span>
                </button>
              {/if}
              {#if session.source === "cli" && session.processId}
                <button class="danger" type="button" role="menuitem" onclick={() => { headerActionsOpen = false; terminateConfirm = !terminateConfirm; }}>
                  <svg viewBox="0 0 20 20"><path d="M10 3v7M5.5 5.5a6 6 0 1 0 9 0" /></svg>
                  <span>{tr("Stop agent", "Encerrar agente")}</span>
                </button>
              {/if}
              <button type="button" role="menuitem" onclick={() => { headerActionsOpen = false; void closeTerminal(); }}>
                <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8" /></svg>
                <span>{tr("Close terminal", "Fechar terminal")}</span>
              </button>
            </span>
          {/if}
        </span>
      </header>

      {#if todo || goal}
        <aside class:collapsed={!workTrayExpanded} class="work-tray" aria-label={tr("Agent work status", "Status do trabalho do agente")}>
          <button
            class="work-tray-toggle"
            type="button"
            aria-expanded={workTrayExpanded}
            aria-label={workTrayExpanded ? tr("Collapse agent work", "Recolher trabalho do agente") : tr("Expand agent work", "Expandir trabalho do agente")}
            onclick={() => (workTrayExpanded = !workTrayExpanded)}
          >
            <strong>{tr("Agent work", "Trabalho do agente")}</strong>
            <span>
              {#if todo}<small>TO DO {completedTodoItems}/{todo.items.length}</small>{/if}
              {#if goal}<small>GOAL · {goalStatusLabel(goal.status)} · {elapsedGoalTime()}</small>{/if}
            </span>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4"></path></svg>
          </button>
          <div class="work-tray-body" aria-hidden={!workTrayExpanded}>
            <div class="work-tray-grid">
              {#if todo}
                <section class="work-card todo-card">
                  <div class="work-card-heading">
                    <strong>TO DO</strong>
                    <span>{completedTodoItems}/{todo.items.length}</span>
                  </div>
                  <i class="todo-progress" style={`--todo-progress: ${(completedTodoItems / todo.items.length) * 100}%`}>
                    <em></em>
                  </i>
                  <ul>
                    {#each todo.items.slice(0, 4) as item}
                      <li class:active={item.status === "in_progress"} class:done={item.status === "completed"} title={workItemLabel(item.status)}>
                        <span aria-hidden="true"></span>
                        <small>{item.label}</small>
                      </li>
                    {/each}
                  </ul>
                  {#if todo.items.length > 4}
                    <small class="work-more">+{todo.items.length - 4} {tr("more", "a mais")}</small>
                  {/if}
                </section>
              {/if}
              {#if goal}
                <section class="work-card goal-card">
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
        <button class:active={activeTab === "chat"} type="button" onclick={() => (activeTab = "chat")}>
          {tr("Chat", "Chat")} <span>{chatEntries.length}</span>
        </button>
        <button class:active={activeTab === "changes"} type="button" onclick={() => (activeTab = "changes")}>
          {tr("Changes", "Alterações")} <span>{changedFiles.length}</span>
        </button>
      </nav>

      <div
        class="terminal-output"
        bind:this={outputElement}
        onscroll={handleOutputScroll}
        onwheel={handleOutputWheel}
      >
        <p><span>$</span> {session.agentLabel.toLowerCase()} <i>{session.project}</i></p>
        <p class="status status-{session.status}"><span>&gt;</span> {displayText(language, session.statusLabel)}</p>
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
            {#each chatEntries as entry (entry.id)}
              {@const item = entry.activity}
              {#if item.kind === "prompt" && (item.detail || item.attachments?.length)}
                <div class="chat-message user-message">
                  <header>
                    <strong>{tr("You", "Você")}</strong>
                    <time>{activityTime(item.createdAt)}</time>
                  </header>
                  {#if item.detail}<pre>{item.detail}</pre>{/if}
                  {#if item.attachments?.length}
                    <div class="message-images">
                      {#each item.attachments as attachment}
                        <img src={attachment.previewDataUrl} alt={attachment.name} />
                      {/each}
                    </div>
                  {/if}
                </div>
              {:else if item.kind === "message" && item.detail}
                <div class="chat-message agent-message">
                  <header>
                    <strong>{session.agentLabel}</strong>
                    <time>{activityTime(item.createdAt)}</time>
                  </header>
                  <div class="markdown-content">{@html renderSafeMarkdown(item.detail)}</div>
                </div>
              {:else if item.kind !== "file"}
                <details class="turn-trace">
                  <summary>
                    <span>{activityMark(item)}</span>
                    <strong class="trace-title">{displayText(language, item.title)}</strong>
                    <time>{activityTime(item.createdAt)}</time>
                  </summary>
                  {#if item.detail}<pre>{item.detail}</pre>{/if}
                </details>
              {/if}
              {#if entry.files.length}
                <div class="turn-files">
                  <strong>{tr("Files changed", "Arquivos alterados")}</strong>
                  <div>
                    {#each entry.files as file}
                      <code><span class="file-path">{file.path}</span><span class="added">+{file.added}</span><span class="removed">-{file.removed}</span></code>
                    {/each}
                  </div>
                </div>
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
        {:else}
          <section class="changes-panel">
            <strong>{tr("All changed files", "Todos os arquivos alterados")}</strong>
            {#if changedFiles.length}
              <div class="change-list">
                {#each changedFiles as file}
                  <code><span class="file-path">{file.path}</span><span class="added">+{file.added}</span><span class="removed">-{file.removed}</span></code>
                {/each}
              </div>
            {:else}
              <p class="empty-state">{tr("No file changes were reported in this session.", "Nenhuma alteração de arquivo foi informada nesta sessão.")}</p>
            {/if}
          </section>
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
        class:sending
        class:has-attachments={promptAttachments.length > 0}
        aria-busy={sending}
        style:height={`${displayedComposerHeight}px`}
        onpaste={(event) => void pasteImages(event)}
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
                ? tr("Photo attached", "Foto anexada")
                : tr(`${promptAttachments.length} photos attached`, `${promptAttachments.length} fotos anexadas`)}
            </small>
            {#each promptAttachments as attachment, index}
              <span title={attachment.name}>
                <img src={attachment.previewDataUrl} alt={attachment.name} />
                <button type="button" onclick={() => removeImage(index)} aria-label={tr("Remove image", "Remover imagem")}>×</button>
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
              <strong>{nextQueuedPrompt.detail || tr("Prompt with attached images", "Prompt com imagens anexadas")}</strong>
            </span>
            <span class="queue-shortcut"><kbd>Tab</kbd><small>{steeringQueued ? tr("Steering…", "Enviando…") : tr("Steer now", "Enviar agora")}</small></span>
          </button>
        {/if}
        <div class="composer-controls">
          {#if session.agent === "codex" || (canSubmit && capabilities?.canAttachImages)}
            <div class="composer-leading-actions">
              {#if session.agent === "codex"}
                <button
                  class:plan={collaborationMode === "plan"}
                  class="mode-button"
                  disabled={promptIsRunning || collaborationModeChanging}
                  type="button"
                  onclick={() => void toggleCollaborationMode()}
                  aria-label={collaborationMode === "plan" ? tr("Plan mode enabled. Switch to Default mode", "Modo Plan ativo. Mudar para o modo Default") : tr("Default mode enabled. Switch to Plan mode", "Modo Default ativo. Mudar para o modo Plan")}
                  title={promptIsRunning ? tr("Mode can be changed after the current prompt", "O modo pode ser alterado após o prompt atual") : collaborationMode === "plan" ? tr("Plan mode — switch to Default", "Modo Plan — mudar para Default") : tr("Default mode — switch to Plan", "Modo Default — mudar para Plan")}
                >
                  {#if collaborationModeChanging}
                    <span class="send-spinner" aria-hidden="true"></span>
                  {:else if collaborationMode === "plan"}
                    <svg aria-hidden="true" viewBox="0 0 20 20">
                      <path d="m4.8 5.8 1.3 1.3 2-2.2M10 6h5M4.8 10 6.1 11.3l2-2.2M10 10.2h5M4.8 14.2l1.3 1.3 2-2.2M10 14.4h5" />
                    </svg>
                  {:else}
                    <svg aria-hidden="true" viewBox="0 0 20 20">
                      <path d="M11 3.5 5.8 10H10l-1 6.5 5.2-7.5H10z" />
                    </svg>
                  {/if}
                </button>
              {/if}
              {#if canSubmit && capabilities?.canAttachImages}
                <button class="attach-button" disabled={!readyForPrompt || sending || promptAttachments.length >= 4} type="button" onclick={() => void chooseImages()} aria-label={tr("Attach image", "Anexar imagem")} title={tr("Attach image", "Anexar imagem")}>
                  <svg viewBox="0 0 20 20"><path d="M6.5 10.5 11 6a2.1 2.1 0 0 1 3 3l-6.2 6.2a3.4 3.4 0 1 1-4.8-4.8l6-6" /></svg>
                </button>
              {/if}
            </div>
          {/if}
          <textarea
            bind:this={promptInput}
            bind:value={prompt}
            disabled={!canSubmit || !readyForPrompt || sending}
            oninput={handlePromptInput}
            onkeydown={sendPromptOnEnter}
            rows="2"
            aria-label={tr(`Prompt for ${sessionDisplayName(session)}`, `Prompt para ${sessionDisplayName(session)}`)}
            placeholder={sending ? tr("Sending prompt…", "Enviando prompt…") : !canSubmit ? promptUnavailableText() : canSendWhileRunning ? tr("Write the next prompt and press Enter to queue…", "Escreva o próximo prompt e pressione Enter para adicionar à fila…") : readyForPrompt ? tr(`Prompt for ${sessionDisplayName(session)}…`, `Prompt para ${sessionDisplayName(session)}…`) : tr("Agent is running…", "Agente em execução…")}
          ></textarea>
          {#if sending}<span class="send-status" role="status">{tr("Sending…", "Enviando…")}</span>{/if}
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
          {:else if canSubmit}
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
      <button class="resize-handle resize-nw" type="button" tabindex="-1" aria-label={tr("Resize from top-left corner", "Redimensionar pelo canto superior esquerdo")} onpointerdown={(event) => void beginResize(event, "NorthWest")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
      <button class="resize-handle resize-ne" type="button" tabindex="-1" aria-label={tr("Resize from top-right corner", "Redimensionar pelo canto superior direito")} onpointerdown={(event) => void beginResize(event, "NorthEast")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
      <button class="resize-handle resize-sw" type="button" tabindex="-1" aria-label={tr("Resize from bottom-left corner", "Redimensionar pelo canto inferior esquerdo")} onpointerdown={(event) => void beginResize(event, "SouthWest")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
      <button class="resize-handle resize-se" type="button" tabindex="-1" aria-label={tr("Resize from bottom-right corner", "Redimensionar pelo canto inferior direito")} onpointerdown={(event) => void beginResize(event, "SouthEast")} onpointermove={moveResize} onpointerup={(event) => void endResize(event)} onpointercancel={(event) => void endResize(event)}></button>
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
  .terminal-window { width: 100%; height: 100%; }
  .terminal-card { position: relative; width: 100%; height: 100%; display: flex; flex-direction: column; overflow: hidden; container-type: inline-size; --chat-font-adjust: 0px; --chat-small-font-adjust: 0px; --chat-tiny-font-adjust: 0px; --chat-font-size: calc(9px + var(--chat-font-adjust)); --chat-small-font-size: calc(8px + var(--chat-small-font-adjust)); --chat-tiny-font-size: calc(7px + var(--chat-tiny-font-adjust)); border: 1px solid rgba(103, 126, 116, 0.2); border-radius: 17px; color: #26342e; background: #f8fbf9; box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.32); transition: border-color 150ms ease, box-shadow 180ms ease, background-color 180ms ease, transform 180ms cubic-bezier(0.22, 1, 0.36, 1); }
  @supports (width: 1cqw) {
    .terminal-card { --chat-font-size: clamp(calc(9px + var(--chat-font-adjust)), calc(7px + 0.55cqw + var(--chat-font-adjust)), calc(12px + var(--chat-font-adjust))); --chat-small-font-size: clamp(calc(8px + var(--chat-small-font-adjust)), calc(6.2px + 0.5cqw + var(--chat-small-font-adjust)), calc(10.5px + var(--chat-small-font-adjust))); --chat-tiny-font-size: clamp(calc(7px + var(--chat-tiny-font-adjust)), calc(5.8px + 0.4cqw + var(--chat-tiny-font-adjust)), calc(9px + var(--chat-tiny-font-adjust))); }
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
  .identity strong { overflow: hidden; color: #26342e; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
  .identity small { overflow: hidden; color: #829089; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
  .terminal-name-editor { min-width: 100px; display: flex; flex: 1; align-items: center; gap: 3px; }
  .terminal-name-editor input { min-width: 0; height: 27px; flex: 1; padding: 0 7px; border: 1px solid rgba(67, 116, 94, 0.25); border-radius: 7px; color: #26342e; background: rgba(255, 255, 255, 0.68); font: 650 10px Inter, sans-serif; outline: none; }
  .terminal-name-editor input:focus { border-color: rgba(48, 139, 96, 0.5); box-shadow: 0 0 0 2px rgba(48, 139, 96, 0.08); }
  .terminal-name-editor button { width: 22px; height: 22px; }
  .source-badge { padding: 3px 5px; display: inline-flex; align-items: center; gap: 3px; border-radius: 999px; color: #718079; background: rgba(80, 104, 94, 0.075); font-size: 7px; font-weight: 760; letter-spacing: 0.04em; text-transform: uppercase; }
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
  header .rename-button { width: 21px; height: 21px; }
  .header-actions { display: flex; flex: 0 0 auto; align-items: center; gap: 2px; }
  .header-overflow { position: relative; z-index: 60; display: none; flex: 0 0 auto; }
  .header-actions-menu { position: absolute; z-index: 70; top: 30px; right: 0; width: 178px; padding: 5px; display: grid; gap: 2px; border: 1px solid rgba(80, 105, 94, 0.14); border-radius: 10px; color: #53665d; background: rgba(248, 251, 249, 0.98); box-shadow: 0 10px 28px rgba(30, 55, 43, 0.17); cursor: default; }
  header .header-actions-menu > button { z-index: auto; width: 100%; min-height: 29px; height: auto; padding: 0 8px; display: flex; justify-content: flex-start; gap: 8px; border-radius: 7px; color: #53665d; font: 700 8px Inter, sans-serif; text-align: left; }
  header .header-actions-menu > button:hover { color: #287452; background: rgba(57, 145, 99, 0.08); }
  header .header-actions-menu > button.danger { color: #9d615c; }
  .header-actions-menu > button svg { width: 13px; height: 13px; flex: 0 0 auto; }
  .header-menu-zoom { min-height: 29px; padding: 0 4px 0 8px; display: grid; grid-template-columns: minmax(0, 1fr) 23px 35px 23px; align-items: center; gap: 2px; color: #53665d; font: 700 8px Inter, sans-serif; }
  .header-menu-zoom > span { min-width: 0; display: flex; align-items: center; gap: 8px; white-space: nowrap; }
  .header-menu-zoom > span svg { width: 13px; height: 13px; flex: 0 0 auto; }
  header .header-menu-zoom > button { z-index: auto; width: 23px; height: 23px; border-radius: 6px; color: #4b6c5d; background: rgba(73, 110, 93, 0.055); font: 800 12px/1 Inter, sans-serif; }
  header .header-menu-zoom > button:hover { color: #287452; background: rgba(57, 145, 99, 0.1); }
  header .header-menu-zoom > button:disabled { opacity: 0.32; cursor: default; }
  .header-menu-zoom output { color: #687970; font: 750 8px Inter, sans-serif; text-align: center; }
  @container (max-width: 390px) {
    header .rename-button,
    .header-actions { display: none; }
    .header-overflow { display: flex; }
  }
  .text-zoom-control { position: relative; z-index: 45; display: flex; flex: 0 0 auto; }
  .text-zoom-popover { position: absolute; z-index: 50; top: 30px; right: 0; min-width: 91px; height: 31px; padding: 3px; display: flex; align-items: center; gap: 2px; border: 1px solid rgba(80, 105, 94, 0.14); border-radius: 9px; color: #53665d; background: rgba(248, 251, 249, 0.98); box-shadow: 0 8px 22px rgba(30, 55, 43, 0.15); cursor: default; }
  .text-zoom-popover button { z-index: auto; width: 23px; height: 23px; border-radius: 6px; color: #4b6c5d; background: rgba(73, 110, 93, 0.055); font: 800 13px/1 Inter, sans-serif; }
  .text-zoom-popover button:hover { color: #287452; background: rgba(57, 145, 99, 0.1); }
  .text-zoom-popover button:disabled { opacity: 0.32; cursor: default; }
  .text-zoom-popover output { min-width: 35px; color: #687970; font: 750 8px Inter, sans-serif; text-align: center; }
  .dock-button { color: #4a7564; }
  .terminate-button { color: #9d615c; }
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
  .hub-tabs { min-height: 29px; padding: 4px 9px 0; display: flex; gap: 3px; border-bottom: 1px solid rgba(97, 119, 109, 0.09); }
  .hub-tabs button { min-width: 0; padding: 0 7px 4px; border: 0; border-bottom: 2px solid transparent; color: #8b9791; background: transparent; font: 700 var(--chat-small-font-size) Inter, sans-serif; cursor: pointer; }
  .hub-tabs button.active { color: #39785d; border-bottom-color: #3b9c70; }
  .hub-tabs span { min-width: 14px; height: 14px; padding: 0 4px; display: inline-grid; place-items: center; border-radius: 999px; color: #72827a; background: rgba(76, 101, 90, 0.075); font-size: var(--chat-tiny-font-size); }
  .terminal-output { min-width: 0; min-height: 0; max-width: 100%; flex: 1; padding: 10px 12px 7px; overflow-x: hidden; overflow-y: auto; color: #55635d; background: linear-gradient(180deg, rgba(61, 87, 75, 0.025), transparent); font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace; font-size: var(--chat-font-size); }
  .terminal-output p { max-width: 100%; margin: 0 0 6px; overflow-wrap: anywhere; line-height: 1.45; word-break: break-word; }
  .terminal-output p > span { color: #36a269; font-weight: 800; }
  .terminal-output i { color: #8a9690; font-style: normal; }
  .status-running, .status-running span { color: #4e7faf; }
  .status-permission_required, .status-permission_required span { color: #b06b25; }
  .status-waiting_for_input, .status-waiting_for_input span { color: #b0812d; }
  .status-completed, .status-completed span { color: #55a473; }
  .status-failed, .status-failed span { color: #ad4f4f; }
  .chat-feed { min-width: 0; max-width: 100%; margin: 9px 0 7px; display: grid; gap: 7px; overflow-x: hidden; }
  .chat-message { width: fit-content; min-width: 0; max-width: 94%; padding: 7px 8px; overflow: hidden; border: 1px solid rgba(77, 104, 91, 0.09); border-radius: 9px; background: rgba(69, 99, 84, 0.035); }
  .chat-message.user-message { margin-left: auto; border-bottom-right-radius: 3px; background: rgba(50, 145, 99, 0.075); }
  .chat-message.agent-message { margin-right: auto; border-bottom-left-radius: 3px; }
  .chat-message header { display: flex; align-items: center; gap: 6px; }
  .chat-message header strong { min-width: 0; flex: 1; color: #4f685c; font: 750 var(--chat-small-font-size) Inter, sans-serif; }
  .chat-message header time { color: #9aa59f; font-size: var(--chat-tiny-font-size); }
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
  .markdown-content :global(hr) { height: 1px; margin: 8px 0; border: 0; background: rgba(77, 104, 91, 0.12); }
  .message-images { margin-top: 7px; display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 5px; }
  .message-images img { width: 100%; max-height: 150px; display: block; border-radius: 7px; object-fit: cover; }
  .agent-typing { width: fit-content; min-width: 38px; height: 25px; padding: 0 9px; display: flex; align-items: center; gap: 4px; border: 1px solid rgba(77, 104, 91, 0.09); border-radius: 9px 9px 9px 3px; background: rgba(69, 99, 84, 0.035); }
  .agent-typing span { width: 4px; height: 4px; border-radius: 50%; background: #4e7faf; animation: agent-typing-dot 850ms ease-in-out infinite; }
  .agent-typing span:nth-child(2) { animation-delay: 130ms; }
  .agent-typing span:nth-child(3) { animation-delay: 260ms; }
  @keyframes agent-typing-dot {
    0%, 60%, 100% { opacity: 0.35; transform: translateY(1px); }
    30% { opacity: 1; transform: translateY(-3px); }
  }
  .turn-trace { min-width: 0; max-width: 100%; padding: 4px 6px; overflow: hidden; border-radius: 6px; background: rgba(72, 101, 88, 0.03); }
  .turn-trace summary { min-width: 0; max-width: 100%; display: flex; align-items: center; gap: 5px; color: #71817a; font: 700 var(--chat-tiny-font-size) Inter, sans-serif; cursor: pointer; }
  .turn-trace summary > span { width: 14px; color: #4f806a; text-align: center; }
  .turn-trace .trace-title { min-width: 0; flex: 1; overflow-wrap: anywhere; font: inherit; word-break: break-word; }
  .turn-trace summary time { margin-left: auto; flex: 0 0 auto; color: #a0aaa5; font-weight: 500; }
  .turn-trace pre { min-width: 0; max-width: calc(100% - 19px); max-height: 180px; margin: 5px 0 0 19px; overflow-x: hidden; overflow-y: auto; color: #5c6b64; font: var(--chat-small-font-size)/1.5 "SFMono-Regular", Consolas, "Liberation Mono", monospace; overflow-wrap: anywhere; white-space: pre-wrap; word-break: break-word; }
  .turn-files { padding: 7px 8px; border-left: 2px solid #4b9b73; border-radius: 0 7px 7px 0; background: rgba(55, 142, 98, 0.045); }
  .turn-files > strong { display: block; margin-bottom: 5px; color: #4f775f; font: 750 var(--chat-tiny-font-size) Inter, sans-serif; text-transform: uppercase; }
  .turn-files > div { display: grid; gap: 3px; }
  .turn-files code { display: flex; gap: 6px; color: #496258; font-size: var(--chat-small-font-size); overflow-wrap: anywhere; white-space: normal; }
  .turn-files code .file-path { min-width: 0; flex: 1; color: inherit; overflow-wrap: anywhere; word-break: break-word; }
  .turn-files code .added,
  .change-list code .added { color: #45906a; }
  .turn-files code .removed,
  .change-list code .removed { color: #b46161; }
  .privacy-note { padding-top: 6px; border-top: 1px solid rgba(81, 105, 94, 0.08); color: #9aa49f; font: var(--chat-tiny-font-size)/1.45 Inter, sans-serif; }
  .empty-state { margin: 7px 0 10px !important; color: #909c96; font: var(--chat-small-font-size)/1.5 Inter, sans-serif; }
  .changes-panel { margin-top: 9px; display: grid; gap: 7px; }
  .changes-panel > strong { color: #6a7c73; font: 760 var(--chat-small-font-size) Inter, sans-serif; letter-spacing: 0.04em; text-transform: uppercase; }
  .change-list { display: grid; gap: 4px; }
  .change-list code { padding: 5px 6px; display: flex; align-items: flex-start; gap: 6px; border-radius: 6px; color: #4f6158; background: rgba(70, 101, 86, 0.045); font-size: var(--chat-small-font-size); overflow-wrap: anywhere; white-space: normal; }
  .change-list code .file-path { min-width: 0; flex: 1; color: inherit; overflow-wrap: anywhere; word-break: break-word; }
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
  .terminate-confirm { margin: 8px 0 2px; padding: 7px 8px; display: flex; align-items: center; gap: 6px; border: 1px solid rgba(166, 77, 77, 0.14); border-radius: 8px; background: rgba(166, 77, 77, 0.04); font: 700 var(--chat-small-font-size)/1.35 Inter, sans-serif; }
  .terminate-confirm > span { min-width: 0; flex: 1; color: #7d5d58; }
  .terminate-confirm > div { display: flex; gap: 4px; }
  .terminate-confirm button { min-height: 22px; padding: 0 6px; border: 1px solid rgba(84, 101, 93, 0.14); border-radius: 6px; color: #596861; background: rgba(255, 255, 255, 0.48); font: 700 var(--chat-tiny-font-size) Inter, sans-serif; cursor: pointer; }
  .terminate-confirm button.danger { color: #a54c4c; border-color: rgba(166, 77, 77, 0.2); }
  .terminal-composer { position: relative; box-sizing: border-box; min-height: 63px; padding: 7px 8px 8px 10px; display: flex; flex: 0 0 auto; flex-direction: column; align-items: stretch; gap: 6px; border-top: 1px solid rgba(97, 119, 109, 0.11); }
  .composer-controls { min-width: 0; min-height: 0; display: flex; flex: 1; align-items: flex-end; gap: 6px; }
  .composer-leading-actions { display: flex; flex: 0 0 auto; flex-direction: column; justify-content: flex-end; gap: 4px; }
  .pending-images { width: 100%; min-height: 51px; padding: 4px 5px; display: flex; align-items: center; gap: 6px; overflow-x: auto; border-radius: 8px; background: rgba(52, 145, 99, 0.045); }
  .pending-images-label { max-width: 52px; flex: 0 0 auto; color: #829088; font: 750 var(--chat-tiny-font-size)/1.25 Inter, sans-serif; text-transform: uppercase; }
  .pending-images > span { position: relative; width: 42px; height: 42px; flex: 0 0 auto; }
  .pending-images img { width: 100%; height: 100%; display: block; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 8px; object-fit: cover; }
  .pending-images button { position: absolute; top: -4px; right: -4px; width: 15px; height: 15px; border: 1px solid rgba(82, 106, 95, 0.18); border-radius: 50%; color: #65766e; background: #eef3f0; font-size: 10px; line-height: 1; }
  .composer-controls textarea { min-width: 0; min-height: 46px; height: 100%; flex: 1; padding: 7px 8px; resize: none; border: 1px solid rgba(82, 106, 95, 0.14); border-radius: 9px; outline: none; color: #34443d; background: rgba(255, 255, 255, 0.5); font: var(--chat-font-size)/1.4 Inter, sans-serif; }
  .composer-controls textarea:focus { border-color: rgba(52, 151, 103, 0.42); box-shadow: 0 0 0 3px rgba(52, 151, 103, 0.07); }
  .composer-controls textarea:disabled { opacity: 0.58; }
  .send-status { padding-bottom: 9px; color: #70827a; font: 700 var(--chat-small-font-size) Inter, sans-serif; white-space: nowrap; }
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
  .terminal-composer .mode-button,
  .terminal-composer .attach-button { color: #5d7469; border: 1px solid rgba(82, 106, 95, 0.12); background: rgba(80, 105, 94, 0.055); }
  .terminal-composer .mode-button.plan { color: #527aa0; border-color: rgba(79, 123, 164, 0.2); background: rgba(75, 124, 169, 0.1); }
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
  .terminal-window.dark .text-zoom-button.active { color: #86cbaa; background: rgba(102, 190, 149, 0.09); }
  .terminal-window.dark .terminal-name-editor input { color: #d9e5df; border-color: rgba(195, 218, 207, 0.14); background: rgba(219, 233, 226, 0.055); }
  .terminal-window.dark .header-actions-menu { color: #b7c8bf; border-color: rgba(205, 222, 213, 0.12); background: rgba(24, 35, 30, 0.98); box-shadow: 0 10px 28px rgba(0, 0, 0, 0.3); }
  .terminal-window.dark header .header-actions-menu > button,
  .terminal-window.dark .header-menu-zoom { color: #b7c8bf; }
  .terminal-window.dark header .header-actions-menu > button:hover { color: #8bd3b0; background: rgba(96, 187, 144, 0.08); }
  .terminal-window.dark header .header-actions-menu > button.danger { color: #d48b83; }
  .terminal-window.dark header .header-menu-zoom > button { color: #b7cbc1; background: rgba(218, 234, 226, 0.055); }
  .terminal-window.dark header .header-menu-zoom > button:hover { color: #8bd3b0; background: rgba(96, 187, 144, 0.1); }
  .terminal-window.dark .header-menu-zoom output { color: #a5b6ad; }
  .terminal-window.dark .text-zoom-popover { color: #b7c8bf; border-color: rgba(205, 222, 213, 0.12); background: rgba(24, 35, 30, 0.98); box-shadow: 0 8px 24px rgba(0, 0, 0, 0.28); }
  .terminal-window.dark .text-zoom-popover button { color: #b7cbc1; background: rgba(218, 234, 226, 0.055); }
  .terminal-window.dark .text-zoom-popover button:hover { color: #8bd3b0; background: rgba(96, 187, 144, 0.1); }
  .terminal-window.dark .text-zoom-popover output { color: #a5b6ad; }
  .terminal-window.dark .rate-limit-meter small,
  .terminal-window.dark .pending-images-label { color: #8f9f97; }
  .terminal-window.dark .rate-limit-meter > i { background: rgba(181, 207, 194, 0.12); }
  .terminal-window.dark .pending-images { background: rgba(83, 174, 129, 0.055); }
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
  .terminal-window.dark .terminal-output { color: #b8c6bf; background: linear-gradient(180deg, rgba(114, 151, 134, 0.035), transparent); }
  .terminal-window.dark .chat-message { border-color: rgba(205, 222, 213, 0.08); background: rgba(218, 234, 226, 0.035); }
  .terminal-window.dark .agent-typing { border-color: rgba(205, 222, 213, 0.08); background: rgba(218, 234, 226, 0.035); }
  .terminal-window.dark .chat-message.user-message { background: rgba(76, 169, 124, 0.09); }
  .terminal-window.dark .chat-message header strong,
  .terminal-window.dark .chat-message.user-message > pre,
  .terminal-window.dark .markdown-content,
  .terminal-window.dark .turn-trace pre,
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
  .terminal-window.dark .turn-trace { background: rgba(218, 234, 226, 0.025); }
  .terminal-window.dark .turn-files { border-color: #5bad83; background: rgba(91, 177, 137, 0.055); }
  .terminal-window.dark .turn-files > strong { color: #8bc6a8; }
  .terminal-window.dark .change-list code { color: #bdcbc4; }
  .terminal-window.dark .privacy-note { border-color: rgba(205, 222, 213, 0.07); color: #78877f; }
  .terminal-window.dark textarea { color: #d0ddd6; border-color: rgba(205, 222, 213, 0.12); background: rgba(220, 234, 227, 0.045); }
  .terminal-window.dark .terminal-composer .mode-button,
  .terminal-window.dark .terminal-composer .attach-button { color: #a9bbb2; border-color: rgba(205, 222, 213, 0.1); background: rgba(218, 234, 226, 0.045); }
  .terminal-window.dark .terminal-composer .mode-button.plan { color: #9abddd; border-color: rgba(138, 183, 220, 0.18); background: rgba(91, 143, 184, 0.1); }
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
    .agent-typing span { animation: none; opacity: 0.7; }
    .send-spinner { animation: none; border-color: rgba(255, 255, 255, 0.72); }
  }
</style>
