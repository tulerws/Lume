<script lang="ts">
  import { onMount, tick } from "svelte";
  import { emit, listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import BrandIcon from "$lib/BrandIcon.svelte";
  import type { Preferences, TerminalWindowState, WorkflowConnectionDefinition, WorkflowContextPackage, WorkflowContextPolicy, WorkflowContextSelection, WorkflowGroupDefinition, WorkflowRole, WorkflowRun, WorkflowRunStatus, WorkflowStepDefinition, WorkflowStepRunStatus } from "$lib/domain";
  import { localize } from "$lib/i18n";
  import { advanceWorkflowRun, approveWorkflowHandoff, cancelWorkflowRun, loadPreferences, loadTerminalWindows, loadWorkflowBridgeContext, loadWorkflowRoleContract, loadWorkflowRun, pauseWorkflowRun, previewWorkflowContext, resumeWorkflowRun, retryWorkflowStep, savePreferences, setWorkflowBridgeExpanded, skipWorkflowStep, startWorkflowRun, type WorkflowBridgeContext } from "$lib/lume";
  import { orderTerminalsByPosition, orderWorkflowSteps } from "$lib/workflowOrder";

  const currentWindow = getCurrentWindow();
  const nativeConnectorHost = typeof navigator !== "undefined" && navigator.userAgent.toLowerCase().includes("linux");
  let preferences = $state<Preferences | null>(null);
  let context = $state<WorkflowBridgeContext | null>(null);
  let terminals = $state<TerminalWindowState[]>([]);
  let draft = $state<WorkflowGroupDefinition | null>(null);
  let connectionId = $state<string | null>(null);
  let instructionExpanded = $state(false);
  let previewControlsExpanded = $state(false);
  let previewExpanded = $state(false);
  let previewLoading = $state(false);
  let previewPackage = $state<WorkflowContextPackage | null>(null);
  let objective = $state("");
  let objectiveExpanded = $state(false);
  let objectiveCollapseTimer: ReturnType<typeof setTimeout> | undefined;
  let workflowRun = $state<WorkflowRun | null>(null);
  let workflowActionLoading = $state(false);
  let loading = $state(true);
  let saving = $state(false);
  let message = $state<string | null>(null);
  let systemDark = $state(false);
  let entering = $state(true);
  let bridgeWindowElement: HTMLElement;
  let bridgeShell: HTMLElement;
  let cableStart: HTMLCanvasElement;
  let cableEnd: HTMLCanvasElement;

  let bridgeHeightSyncRevision = 0;
  let bridgeHeightSyncQueue = Promise.resolve();

  const dark = $derived(preferences?.darkMode ?? systemDark);
  const usesNativeConnectors = $derived(context?.nativeConnectors ?? nativeConnectorHost);
  const connection = $derived(draft?.connections.find((item) => item.id === connectionId));
  const sourceStep = $derived(draft?.steps.find((step) => step.id === connection?.fromStepId));
  const targetStep = $derived(draft?.steps.find((step) => step.id === connection?.toStepId));
  const workflowIsFinished = $derived(workflowRun ? ["completed", "cancelled"].includes(workflowRun.status) : false);
  const workflowLocked = $derived(Boolean(workflowRun && !workflowIsFinished));

  function tr(english: string, portuguese: string) {
    return localize(preferences?.language ?? "en", english, portuguese);
  }

  function terminalKey(terminal: TerminalWindowState) {
    return terminal.sessionNativeId?.trim() || terminal.sessionId;
  }

  function terminalForStep(step?: WorkflowStepDefinition) {
    return step ? terminals.find((terminal) => terminalKey(terminal) === step.sessionNativeId) : undefined;
  }

  function stepName(step?: WorkflowStepDefinition) {
    return terminalForStep(step)?.sessionProject || step?.customRoleLabel || tr("Agent", "Agente");
  }

  function defaultRole(index: number): WorkflowRole {
    return (["planner", "implementer", "reviewer", "tester", "researcher"] as WorkflowRole[])[index] ?? "custom";
  }

  function defaultContextSelection(): WorkflowContextSelection {
    return { response: true, files: true, checks: true, plan: false, activity: false, diffs: false };
  }

  function effectiveContextSelection(item: WorkflowConnectionDefinition): WorkflowContextSelection {
    if (item.contextPolicy === "minimal") return { response: true, files: false, checks: false, plan: false, activity: false, diffs: false };
    if (item.contextPolicy === "detailed") return { response: true, files: true, checks: true, plan: true, activity: true, diffs: true };
    if (item.contextPolicy === "custom") return item.contextSelection;
    return defaultContextSelection();
  }

  async function initialize() {
    loading = true;
    message = null;
    try {
      const [nextContext, nextPreferences, nextTerminals] = await Promise.all([
        loadWorkflowBridgeContext(currentWindow.label),
        loadPreferences(),
        loadTerminalWindows(),
      ]);
      context = nextContext;
      preferences = nextPreferences;
      terminals = nextTerminals;
      const members = orderTerminalsByPosition(
        nextTerminals.filter((terminal) => terminal.groupId === nextContext.groupId),
      );
      const saved = nextPreferences.workflowGroups.find((group) => group.terminalGroupId === nextContext.groupId);
      const retained = saved?.steps ?? [];
      const retainedKeys = new Set(retained.map((step) => step.sessionNativeId));
      const appended = await Promise.all(members
        .filter((terminal) => !retainedKeys.has(terminalKey(terminal)))
        .map(async (terminal, index): Promise<WorkflowStepDefinition> => {
          const role = defaultRole(retained.length + index);
          return {
            id: `step-${terminalKey(terminal)}`,
            sessionNativeId: terminalKey(terminal),
            role,
            customRoleLabel: "",
            ...await loadWorkflowRoleContract(role),
            attempt: 0,
          };
        }));
      const steps = [...retained, ...appended];
      const stepIds = new Set(steps.map((step) => step.id));
      const connections = (saved?.connections ?? [])
        .filter((item) => stepIds.has(item.fromStepId) && stepIds.has(item.toStepId))
        .map((item) => ({
          ...item,
          contextPolicy: item.contextPolicy ?? "standard",
          contextSelection: item.contextSelection ?? defaultContextSelection(),
        }));
      const source = steps.find((step) => step.sessionNativeId === nextContext.sourceSessionNativeId);
      const target = steps.find((step) => step.sessionNativeId === nextContext.targetSessionNativeId);
      if (!source || !target) throw new Error("The connected agents are no longer available");
      let activeConnection = connections.find((item) =>
        (item.fromStepId === source.id && item.toStepId === target.id)
        || (item.fromStepId === target.id && item.toStepId === source.id));
      if (!activeConnection) {
        const defaultOrder = orderWorkflowSteps(steps, members, connections);
        const sourceComesFirst = defaultOrder.findIndex((step) => step.id === source.id)
          < defaultOrder.findIndex((step) => step.id === target.id);
        activeConnection = {
          id: `connection-${crypto.randomUUID()}`,
          fromStepId: sourceComesFirst ? source.id : target.id,
          toStepId: sourceComesFirst ? target.id : source.id,
          includeResponse: true,
          includeFiles: true,
          includeTests: true,
          contextPolicy: "standard",
          contextSelection: defaultContextSelection(),
          additionalInstruction: "",
          requiresApproval: true,
          advanceMode: "manual",
        };
        connections.push(activeConnection);
      }
      draft = {
        id: saved?.id ?? `workflow-${nextContext.groupId}`,
        terminalGroupId: nextContext.groupId,
        steps: orderWorkflowSteps(steps, members, connections),
        connections,
      };
      connectionId = activeConnection.id;
      instructionExpanded = Boolean(activeConnection.additionalInstruction.trim());
      workflowRun = await loadWorkflowRun(draft.id);
      if (workflowRun) {
        objective = workflowRun.objective;
        previewControlsExpanded = true;
      }
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      loading = false;
    }
    if (draft) await syncBridgeHeight();
  }

  function updateConnection(patch: Partial<WorkflowConnectionDefinition>) {
    if (!draft || !connectionId) return;
    draft = {
      ...draft,
      connections: draft.connections.map((item) => item.id === connectionId ? { ...item, ...patch } : item),
    };
    previewPackage = null;
  }

  function selectContextPolicy(policy: WorkflowContextPolicy) {
    updateConnection({ contextPolicy: policy });
  }

  function toggleContextOption(key: keyof WorkflowContextSelection) {
    if (!connection) return;
    const current = connection.contextSelection ?? defaultContextSelection();
    const next = { ...current, [key]: !current[key] };
    if (key === "diffs" && next.diffs) next.files = true;
    if (key === "files" && !next.files) next.diffs = false;
    updateConnection({
      contextPolicy: "custom",
      contextSelection: next,
      includeResponse: next.response,
      includeFiles: next.files,
      includeTests: next.checks,
    });
  }

  function reverseConnection() {
    if (!connection) return;
    updateConnection({ fromStepId: connection.toStepId, toStepId: connection.fromStepId });
  }

  async function toggleInstruction() {
    const next = !instructionExpanded;
    instructionExpanded = next;
    try {
      await syncBridgeHeight();
    } catch (error) {
      instructionExpanded = !next;
      message = String(error).replace(/^Error:\s*/, "");
      await syncBridgeHeight().catch(() => undefined);
    }
  }

  async function togglePreview() {
    if (previewExpanded) {
      previewExpanded = false;
      await syncBridgeHeight();
      return;
    }
    if (!draft || !connectionId || previewLoading || !objective.trim()) return;
    previewExpanded = true;
    previewLoading = true;
    previewPackage = null;
    message = null;
    await syncBridgeHeight();
    try {
      previewPackage = await previewWorkflowContext(draft, connectionId, objective);
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      previewLoading = false;
      await syncBridgeHeight();
    }
  }

  async function togglePreviewControls() {
    previewControlsExpanded = !previewControlsExpanded;
    if (!previewControlsExpanded) previewExpanded = false;
    await syncBridgeHeight();
  }

  function workflowStatusLabel(status: WorkflowRunStatus) {
    const labels: Record<WorkflowRunStatus, [string, string]> = {
      draft: ["Draft", "Rascunho"],
      ready: ["Ready for next step", "Pronto para a próxima etapa"],
      running: ["Running", "Executando"],
      waiting_for_approval: ["Waiting for approval", "Aguardando aprovação"],
      paused: ["Paused", "Pausado"],
      completed: ["Completed", "Concluído"],
      failed: ["Failed", "Falhou"],
      cancelled: ["Cancelled", "Cancelado"],
    };
    return tr(...labels[status]);
  }

  function workflowStepStatusLabel(status: WorkflowStepRunStatus) {
    const labels: Record<WorkflowStepRunStatus, [string, string]> = {
      pending: ["Pending", "Pendente"],
      running: ["Running", "Executando"],
      completed: ["Completed", "Concluído"],
      failed: ["Failed", "Falhou"],
      skipped: ["Skipped", "Ignorado"],
    };
    return tr(...labels[status]);
  }

  function acceptWorkflowRun(next: WorkflowRun | null, force = false): boolean {
    if (!next) return false;
    const current = workflowRun;
    const newerRun = !current
      || next.id !== current.id
        && (next.createdAt > current.createdAt
          || (next.createdAt === current.createdAt && next.updatedAt > current.updatedAt));
    const newerRevision = current?.id === next.id && next.updatedAt > current.updatedAt;
    if (!force && !newerRun && !newerRevision) return false;
    workflowRun = next;
    objective = next.objective;
    return true;
  }

  async function performWorkflowAction(action: () => Promise<WorkflowRun>) {
    if (workflowActionLoading) return;
    workflowActionLoading = true;
    message = null;
    try {
      acceptWorkflowRun(await action(), true);
      previewControlsExpanded = true;
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      workflowActionLoading = false;
      await syncBridgeHeight();
    }
  }

  async function runWorkflow() {
    if (!draft || !objective.trim()) return;
    await performWorkflowAction(async () => {
      await persist(draft!);
      return startWorkflowRun(draft!, objective);
    });
  }

  async function refreshWorkflowRun() {
    if (!draft || workflowActionLoading) return;
    try {
      const next = await loadWorkflowRun(draft.id);
      if (acceptWorkflowRun(next)) {
        await syncBridgeHeight();
      }
    } catch {
      // A transient refresh failure must not close the connection editor.
    }
  }

  function syncBridgeHeight(): Promise<void> {
    const revision = ++bridgeHeightSyncRevision;
    bridgeHeightSyncQueue = bridgeHeightSyncQueue
      .catch(() => undefined)
      .then(async () => {
        await tick();
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
        if (revision !== bridgeHeightSyncRevision || !bridgeShell || !context) return;
        const expanded = instructionExpanded || previewControlsExpanded || Boolean(workflowRun);
        const contentHeight = measureBridgeContentHeight();
        await setWorkflowBridgeExpanded(
          currentWindow.label,
          expanded,
          contentHeight,
        );
      });
    return bridgeHeightSyncQueue;
  }

  function measureBridgeContentHeight() {
    const probe = document.createElement("main");
    probe.className = bridgeWindowElement.className.replace(/\bentering\b/g, "");
    probe.setAttribute("aria-hidden", "true");
    Object.assign(probe.style, {
      position: "fixed",
      top: "0",
      left: "-10000px",
      width: `${bridgeWindowElement.getBoundingClientRect().width}px`,
      height: "auto",
      maxHeight: "none",
      visibility: "hidden",
      pointerEvents: "none",
      contain: "none",
    });
    const shell = bridgeShell.cloneNode(true) as HTMLElement;
    Object.assign(shell.style, {
      height: "auto",
      minHeight: "0",
      maxHeight: "none",
      transition: "none",
    });
    probe.appendChild(shell);
    document.body.appendChild(probe);
    const height = Math.ceil(probe.getBoundingClientRect().height);
    probe.remove();
    return height;
  }

  function handleObjectiveInput() {
    previewPackage = null;
    message = null;
    if (objective.trim().length < 48) return;
    objectiveExpanded = true;
    if (objectiveCollapseTimer) clearTimeout(objectiveCollapseTimer);
    objectiveCollapseTimer = setTimeout(() => objectiveExpanded = false, 1_800);
  }

  function collapseObjective() {
    if (objectiveCollapseTimer) clearTimeout(objectiveCollapseTimer);
    objectiveCollapseTimer = undefined;
    objectiveExpanded = false;
  }

  async function persist(nextDraft: WorkflowGroupDefinition) {
    if (!preferences) return;
    const workflowGroups = preferences.workflowGroups.some((group) => group.id === nextDraft.id)
      ? preferences.workflowGroups.map((group) => group.id === nextDraft.id ? nextDraft : group)
      : [...preferences.workflowGroups, nextDraft];
    const next = { ...preferences, workflowGroups };
    await savePreferences(next);
    preferences = next;
    await emit("lume://preferences-changed", next);
  }

  async function save() {
    if (!draft || saving) return;
    saving = true;
    message = null;
    try {
      await persist(draft);
      await currentWindow.close();
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!draft || !connectionId || saving) return;
    saving = true;
    message = null;
    try {
      const next = { ...draft, connections: draft.connections.filter((item) => item.id !== connectionId) };
      await persist(next);
      await currentWindow.close();
    } catch (error) {
      message = String(error).replace(/^Error:\s*/, "");
      saving = false;
    }
  }

  function paintCable(canvas: HTMLCanvasElement, time: number, direction: 1 | -1) {
    const ratio = window.devicePixelRatio || 1;
    const width = Math.max(1, canvas.clientWidth);
    const height = Math.max(1, canvas.clientHeight);
    const pixelWidth = Math.round(width * ratio);
    const pixelHeight = Math.round(height * ratio);
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth;
      canvas.height = pixelHeight;
    }
    const context = canvas.getContext("2d");
    if (!context) return;
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.globalCompositeOperation = "copy";
    context.fillStyle = "rgba(0,0,0,0)";
    context.fillRect(0, 0, width, height);
    context.globalCompositeOperation = "source-over";
    const phase = time * 0.0042 * direction;
    const trace = () => {
      context.beginPath();
      for (let x = 0; x <= width; x += 0.75) {
        const taper = Math.sin(Math.PI * x / width);
        const y = height / 2 + Math.sin((x / width) * Math.PI * 3 + phase) * 3.2 * taper;
        if (x === 0) context.moveTo(x, y);
        else context.lineTo(x, y);
      }
    };
    trace();
    context.strokeStyle = "rgba(66,171,116,.54)";
    context.lineWidth = 2.7;
    context.lineCap = "round";
    context.stroke();
    trace();
    context.strokeStyle = "#67cf98";
    context.lineWidth = 3;
    context.setLineDash([5, 8]);
    context.lineDashOffset = -time * 0.022 * direction;
    context.stroke();
    context.setLineDash([]);
  }

  onMount(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = media.matches;
    const sync = (event: MediaQueryListEvent) => (systemDark = event.matches);
    const escape = (event: KeyboardEvent) => event.key === "Escape" && void currentWindow.close();
    media.addEventListener("change", sync);
    window.addEventListener("keydown", escape);
    let animationFrame = 0;
    let stopReveal: (() => void) | undefined;
    let stopRunChanged: (() => void) | undefined;
    let stopSessionsChanged: (() => void) | undefined;
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;
    let bridgeResizeObserver: ResizeObserver | undefined;
    let bridgeResizeFrame = 0;
    const animateCable = (time: number) => {
      if (cableStart) paintCable(cableStart, time, 1);
      if (cableEnd) paintCable(cableEnd, time, -1);
      animationFrame = requestAnimationFrame(animateCable);
    };
    if (!nativeConnectorHost) animationFrame = requestAnimationFrame(animateCable);
    void listen("lume://workflow-bridge-reveal", () => {
      entering = false;
      requestAnimationFrame(() => (entering = true));
    }).then((unlisten) => (stopReveal = unlisten));
    void listen<WorkflowRun>("lume://workflow-run-changed", (event) => {
      if (!draft || event.payload.workflowId !== draft.id) return;
      if (!acceptWorkflowRun(event.payload)) return;
      previewControlsExpanded = true;
      void syncBridgeHeight();
    }).then((unlisten) => (stopRunChanged = unlisten));
    void listen("lume://sessions-changed", () => {
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => void refreshWorkflowRun(), 180);
    }).then((unlisten) => (stopSessionsChanged = unlisten));
    if (typeof ResizeObserver !== "undefined") {
      bridgeResizeObserver = new ResizeObserver(() => {
        if (loading || bridgeResizeFrame) return;
        bridgeResizeFrame = requestAnimationFrame(() => {
          bridgeResizeFrame = 0;
          void syncBridgeHeight().catch(() => undefined);
        });
      });
      bridgeResizeObserver.observe(bridgeShell);
    }
    void initialize();
    return () => {
      if (animationFrame) cancelAnimationFrame(animationFrame);
      if (refreshTimer) clearTimeout(refreshTimer);
      if (objectiveCollapseTimer) clearTimeout(objectiveCollapseTimer);
      if (bridgeResizeFrame) cancelAnimationFrame(bridgeResizeFrame);
      bridgeResizeObserver?.disconnect();
      stopReveal?.();
      stopRunChanged?.();
      stopSessionsChanged?.();
      media.removeEventListener("change", sync);
      window.removeEventListener("keydown", escape);
    };
  });
</script>

<main bind:this={bridgeWindowElement} class:dark class:entering class:run-locked={workflowLocked} class:vertical={context?.side === "bottom"} class:native-connectors={usesNativeConnectors} class="bridge-window" onpointerdown={() => void currentWindow.setFocus().catch(() => undefined)}>
  <canvas bind:this={cableStart} class="energy-cable cable-start" aria-hidden="true"></canvas>
  <canvas bind:this={cableEnd} class="energy-cable cable-end" aria-hidden="true"></canvas>
  <section bind:this={bridgeShell} class="bridge-shell">
    {#if loading}
      <div class="loading"><i></i><span>{tr("Opening connection…", "Abrindo conexão…")}</span></div>
    {:else if connection && draft}
      <header class="route-heading">
        <span class="agent">
          {#if terminalForStep(sourceStep)}<BrandIcon name={terminalForStep(sourceStep)!.sessionAgent} size={17} />{/if}
          <strong>{stepName(sourceStep)}</strong>
        </span>
        <button class="reverse" type="button" title={tr("Reverse direction", "Inverter direção")} aria-label={tr("Reverse direction", "Inverter direção")} onclick={reverseConnection}>
          <svg viewBox="0 0 20 20"><path d="M4 7h11m-3-3 3 3-3 3M16 13H5m3 3-3-3 3-3" /></svg>
        </button>
        <span class="agent target">
          {#if terminalForStep(targetStep)}<BrandIcon name={terminalForStep(targetStep)!.sessionAgent} size={17} />{/if}
          <strong>{stepName(targetStep)}</strong>
        </span>
        <button class="close" type="button" aria-label={tr("Close", "Fechar")} onclick={() => void currentWindow.close()}>
          <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8" /></svg>
        </button>
      </header>

      <section class="context-policy" aria-label={tr("Context policy", "Política de contexto")}>
        <strong>{tr("Context", "Contexto")}</strong>
        <div>
          {#each ["minimal", "standard", "detailed", "custom"] as policy}
            <button class:active={connection.contextPolicy === policy} type="button" onclick={() => selectContextPolicy(policy as WorkflowContextPolicy)}>{policy}</button>
          {/each}
        </div>
      </section>

      {@const contextSelection = effectiveContextSelection(connection)}
      <section class="share-options six" aria-label={tr("Shared context", "Contexto compartilhado")}>
            <button class:active={contextSelection.response} type="button" aria-label={tr("Include the agent's final response", "Incluir a resposta final do agente")} data-tooltip={tr("Final response", "Resposta final")} onclick={() => toggleContextOption("response")}>
              <svg viewBox="0 0 20 20"><path d="M4 4h12v9H9l-4 3v-3H4Z" /></svg><span>{tr("Response", "Resposta")}</span><i>✓</i>
            </button>
            <button class:active={contextSelection.files} type="button" aria-label={tr("Include changed files", "Incluir arquivos alterados")} data-tooltip={tr("Changed files", "Arquivos alterados")} onclick={() => toggleContextOption("files")}>
              <svg viewBox="0 0 20 20"><path d="M3.5 5.5h5l1.5 2h6.5v8h-13Z" /></svg><span>{tr("Files", "Arquivos")}</span><i>✓</i>
            </button>
            <button class:active={contextSelection.checks} type="button" aria-label={tr("Include checks and test results", "Incluir validações e resultados dos testes")} data-tooltip={tr("Tests and checks", "Testes e validações")} onclick={() => toggleContextOption("checks")}>
              <svg viewBox="0 0 20 20"><path d="m4 10 3.5 3.5L16 5" /></svg><span>{tr("Checks", "Validações")}</span><i>✓</i>
            </button>
            <button class:active={contextSelection.plan} type="button" aria-label={tr("Include the relevant plan", "Incluir o plano relevante")} data-tooltip={tr("Relevant plan", "Plano relevante")} onclick={() => toggleContextOption("plan")}>
              <svg viewBox="0 0 20 20"><path d="M5 5h10M5 10h10M5 15h7" /></svg><span>{tr("Plan", "Plano")}</span><i>✓</i>
            </button>
            <button class:active={contextSelection.activity} type="button" aria-label={tr("Include relevant activity", "Incluir atividade relevante")} data-tooltip={tr("Relevant activity", "Atividade relevante")} onclick={() => toggleContextOption("activity")}>
              <svg viewBox="0 0 20 20"><path d="M4 13h3l2-7 3 9 2-5h2" /></svg><span>{tr("Activity", "Atividade")}</span><i>✓</i>
            </button>
            <button class:active={contextSelection.diffs} type="button" aria-label={tr("Include sanitized diff excerpts", "Incluir trechos sanitizados do diff")} data-tooltip={tr("Sanitized diffs", "Diffs sanitizados")} onclick={() => toggleContextOption("diffs")}>
              <svg viewBox="0 0 20 20"><path d="M7 4 3 10l4 6M13 4l4 6-4 6M11 3 9 17" /></svg><span>{tr("Diffs", "Diffs")}</span><i>✓</i>
            </button>
      </section>

      <div class="behavior-row">
        <span class="transition-toggle" class:manual-active={connection.advanceMode === "manual"}>
          <button class:active={connection.advanceMode === "manual"} aria-label={tr("Manual", "Manual")} type="button" onclick={() => updateConnection({ advanceMode: "manual" })}>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 4v12M9 6v8M13 7.5v5M17 9v2" /></svg>
            <span>{tr("Manual", "Manual")}</span>
          </button>
          <button class:active={connection.advanceMode === "automatic"} aria-label={tr("Automatic", "Automático")} title={tr("Continue automatically when the step is ready", "Continuar automaticamente quando a etapa estiver pronta")} type="button" onclick={() => updateConnection({ advanceMode: "automatic" })}>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m11.5 2.5-6 8H10l-1.5 7 6-9H10Z" /></svg>
            <span>{tr("Auto", "Auto")}</span>
          </button>
        </span>
        <button class:active={connection.requiresApproval} class="approval" type="button" role="switch" aria-checked={connection.requiresApproval} onclick={() => updateConnection({ requiresApproval: !connection.requiresApproval })}>
          <svg viewBox="0 0 20 20"><path d="M10 3 16 5v4c0 4-2.5 6.5-6 8-3.5-1.5-6-4-6-8V5Z" /></svg><span>{tr("Approval", "Aprovação")}</span><i></i>
        </button>
      </div>

      <button class:open={instructionExpanded} class="instruction-toggle" type="button" onclick={() => void toggleInstruction()}>
        <svg viewBox="0 0 20 20"><path d="M4 15.5h3l8-8-3-3-8 8Zm6.5-9.5 3 3" /></svg>
        <span>{connection.additionalInstruction.trim() ? tr("Edit instruction", "Editar instrução") : tr("Add instruction", "Adicionar instrução")}</span>
        <svg viewBox="0 0 20 20"><path d="m6 8 4 4 4-4" /></svg>
      </button>
      {#if instructionExpanded}
        <textarea rows="3" maxlength="4000" placeholder={tr("What should the next agent do?", "O que o próximo agente deve fazer?")} value={connection.additionalInstruction} oninput={(event) => updateConnection({ additionalInstruction: event.currentTarget.value })}></textarea>
      {/if}

      <button class:open={previewControlsExpanded} class="instruction-toggle preview-toggle" type="button" onclick={() => void togglePreviewControls()}>
        <svg viewBox="0 0 20 20"><path d="M2.5 10s2.8-5 7.5-5 7.5 5 7.5 5-2.8 5-7.5 5-7.5-5-7.5-5Z" /><circle cx="10" cy="10" r="2" /></svg>
        <span>{tr("Preview", "Prévia")}</span>
        <svg viewBox="0 0 20 20"><path d="m6 8 4 4 4-4" /></svg>
      </button>
      {#if previewControlsExpanded}
        <div class="preview-actions">
          <textarea class:expanded={objectiveExpanded} disabled={workflowLocked} rows="1" maxlength="4000" placeholder={tr("Objective for this run", "Objetivo desta execução")} bind:value={objective} onfocus={() => objective.trim().length >= 48 && (objectiveExpanded = true)} onblur={collapseObjective} oninput={handleObjectiveInput}></textarea>
          <button class:open={previewExpanded} disabled={previewLoading || !objective.trim()} type="button" aria-label={tr("Preview context", "Visualizar contexto")} title={tr("Preview context", "Visualizar contexto")} onclick={() => void togglePreview()}>
            <svg viewBox="0 0 20 20"><path d="M2.5 10s2.8-5 7.5-5 7.5 5 7.5 5-2.8 5-7.5 5-7.5-5-7.5-5Z" /><circle cx="10" cy="10" r="2" /></svg>
          </button>
        </div>
        {#if !workflowRun}
          <section class="workflow-runtime" data-status="draft">
            <header><span class="run-state"><i></i><strong>{tr("Draft", "Rascunho")}</strong></span></header>
            <button class="workflow-start" disabled={workflowActionLoading || !objective.trim()} type="button" onclick={() => void runWorkflow()}>
              <svg viewBox="0 0 20 20"><path d="m7 4 8 6-8 6Z" /></svg>
              <span>{workflowActionLoading ? tr("Starting…", "Iniciando…") : tr("Run workflow", "Executar workflow")}</span>
            </button>
          </section>
        {/if}
      {/if}

      {#if workflowRun}
        <section class="workflow-runtime" data-status={workflowRun.status} aria-live="polite">
          <header>
            <span class="run-state"><i></i><strong>{workflowRun.recovering
              ? tr("Reconnecting", "Reconectando")
              : workflowStatusLabel(workflowRun.status)}</strong></span>
            {#if workflowRun.currentStepId}
              <small>{stepName(draft.steps.find((step) => step.id === workflowRun?.currentStepId))}</small>
            {/if}
          </header>
          <div class="run-progress" aria-label={tr("Workflow progress", "Progresso do workflow")}>
            {#each workflowRun.steps as runStep}
              {@const definition = draft.steps.find((step) => step.id === runStep.stepId)}
              <i class:active={runStep.stepId === workflowRun.currentStepId} data-status={runStep.status} title={`${stepName(definition)} · ${workflowStepStatusLabel(runStep.status)}`}></i>
            {/each}
          </div>
          <div class="run-actions">
            {#if workflowRun.recovering}
              <span class="recovery-note">{tr("Waiting for the original agent", "Aguardando o agente original")}</span>
            {:else if workflowRun.status === "running"}
              <button disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => pauseWorkflowRun(draft!.id))}>{tr("Pause", "Pausar")}</button>
            {:else if workflowRun.status === "waiting_for_approval"}
              <button class="primary" disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => approveWorkflowHandoff(draft!.id))}>{tr("Approve handoff", "Aprovar handoff")}</button>
              <button disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => pauseWorkflowRun(draft!.id))}>{tr("Pause", "Pausar")}</button>
            {:else if workflowRun.status === "ready"}
              <button class="primary" disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => advanceWorkflowRun(draft!.id))}>{tr("Run next", "Executar próxima")}</button>
              <button disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => pauseWorkflowRun(draft!.id))}>{tr("Pause", "Pausar")}</button>
            {:else if workflowRun.status === "paused"}
              <button class="primary" disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => resumeWorkflowRun(draft!.id))}>{tr("Resume", "Retomar")}</button>
            {:else if workflowRun.status === "failed"}
              <button class="primary" disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => retryWorkflowStep(draft!.id))}>{tr("Retry", "Tentar novamente")}</button>
              <button disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => skipWorkflowStep(draft!.id))}>{tr("Skip", "Ignorar")}</button>
            {:else if workflowIsFinished}
              <button class="primary" disabled={workflowActionLoading || !objective.trim()} type="button" onclick={() => void runWorkflow()}>{tr("Run again", "Executar novamente")}</button>
            {/if}
            {#if !workflowIsFinished}
              <button class="stop" disabled={workflowActionLoading} type="button" onclick={() => void performWorkflowAction(() => cancelWorkflowRun(draft!.id))}>{tr("Stop", "Parar")}</button>
            {/if}
          </div>
          {#if workflowRun.error}<p>{workflowRun.error}</p>{/if}
        </section>
      {/if}
      {#if previewExpanded}
        <section class="context-preview" aria-live="polite">
          <header>
            <strong>{tr("Exact context", "Contexto exato")}</strong>
            {#if previewPackage}<span>~{previewPackage.estimatedTokens} tokens</span>{/if}
            <button type="button" aria-label={tr("Close preview", "Fechar prévia")} onclick={() => void togglePreview()}>
              <svg viewBox="0 0 20 20"><path d="m6 6 8 8M14 6l-8 8" /></svg>
            </button>
          </header>
          {#if previewLoading}
            <span class="preview-loading"><i></i>{tr("Building safe context…", "Montando contexto seguro…")}</span>
          {:else if previewPackage}
            {#if previewPackage.redactions.length}
              <div class="redaction-summary">
                {#each previewPackage.redactions as redaction}<span>{redaction.count} · {redaction.summary}</span>{/each}
              </div>
            {/if}
            <pre>{previewPackage.markdown}</pre>
          {:else}
            <span class="preview-empty">{objective.trim() ? tr("Context changed. Build the preview again.", "O contexto mudou. Gere a prévia novamente.") : tr("Add an objective to generate the preview.", "Adicione um objetivo para gerar a prévia.")}</span>
          {/if}
        </section>
      {/if}

      {#if message}<p class="error">{message}</p>{/if}
      <footer>
        <button class="remove" disabled={saving} type="button" onclick={() => void remove()}>{tr("Remove", "Remover")}</button>
        <span class="connection-help">
          <button class="help-trigger" type="button" aria-label={tr("Explain connection options", "Explicar opções da conexão")}>?</button>
          <span class="help-tooltip" role="tooltip">
            <strong>{tr("How this connection works", "Como esta conexão funciona")}</strong>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 5h12M4 10h12M4 15h12"></path></svg><span><b>{tr("Context policy", "Política de contexto")}</b>{tr("Minimal, standard, detailed, or your custom selection.", "Mínimo, padrão, detalhado ou sua seleção personalizada.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 4h12v9H9l-4 3v-3H4z"></path></svg><span><b>{tr("Response", "Resposta")}</b>{tr("Sends the agent's final answer.", "Envia a resposta final do agente.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3.5 5.5h5l1.5 2h6.5v8h-13z"></path></svg><span><b>{tr("Files", "Arquivos")}</b>{tr("Includes paths and change totals.", "Inclui caminhos e totais alterados.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="m4 10 3.5 3.5L16 5"></path></svg><span><b>{tr("Checks", "Validações")}</b>{tr("Shares tests and validation results.", "Compartilha testes e resultados de validação.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 5h10M5 10h10M5 15h7"></path></svg><span><b>{tr("Plan and activity", "Plano e atividade")}</b>{tr("Adds the relevant plan and recent actions.", "Adiciona o plano relevante e as ações recentes.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M7 4 3 10l4 6M13 4l4 6-4 6"></path></svg><span><b>{tr("Diffs", "Diffs")}</b>{tr("Adds sanitized excerpts from changed files.", "Adiciona trechos sanitizados dos arquivos alterados.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 7h10m-3-3 3 3-3 3M16 13H6m3-3-3 3 3 3"></path></svg><span><b>{tr("Manual / Auto", "Manual / Auto")}</b>{tr("Manual waits for you. Auto continues when ready, pausing whenever approval is required.", "Manual espera por você. Auto continua quando estiver pronto e pausa sempre que uma aprovação for exigida.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 3.5 15 5v4c0 3.2-1.9 5.6-5 7-3.1-1.4-5-3.8-5-7V5z"></path></svg><span><b>{tr("Approval", "Aprovação")}</b>{tr("Requires confirmation before continuing.", "Exige confirmação antes de continuar.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5 14 1-4 7-7 3 3-7 7zM12 4l3 3"></path></svg><span><b>{tr("Instruction", "Instrução")}</b>{tr("Adds guidance for the next agent.", "Adiciona orientação para o próximo agente.")}</span></span>
            <span class="help-item"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M2.5 10s2.8-5 7.5-5 7.5 5 7.5 5-2.8 5-7.5 5-7.5-5-7.5-5Z"></path><circle cx="10" cy="10" r="2"></circle></svg><span><b>{tr("Preview", "Prévia")}</b>{tr("Requires an objective and shows the exact safe context.", "Exige um objetivo e mostra o contexto seguro exato.")}</span></span>
          </span>
        </span>
        <button class="save" disabled={saving} type="button" onclick={() => void save()}>{saving ? tr("Saving…", "Salvando…") : tr("Save", "Salvar")}</button>
      </footer>
    {:else}
      <div class="loading error-state"><span>{message || tr("Connection unavailable", "Conexão indisponível")}</span><button type="button" onclick={() => void currentWindow.close()}>{tr("Close", "Fechar")}</button></div>
    {/if}
  </section>
</main>

<style>
  :global(html), :global(body) { width: 100%; height: 100%; margin: 0; overflow: hidden; background: transparent !important; }
  * { box-sizing: border-box; }
  button, textarea { font: inherit; -webkit-tap-highlight-color: transparent; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.65; stroke-linecap: round; stroke-linejoin: round; }
  .bridge-window { position: relative; width: 100vw; height: max-content; min-height: 0; padding: 4px 22px; contain: paint; isolation: isolate; color: #31443a; background: transparent; font-family: Inter,system-ui,sans-serif; }
  .bridge-window.entering { animation: bridge-fade-in 180ms cubic-bezier(.2,.8,.2,1) both; }
  .energy-cable { position: absolute; z-index: 3; top: 50%; width: 22px; height: 18px; overflow: hidden; contain: paint; isolation: isolate; background: transparent; transform: translateY(-50%); pointer-events: none; }
  .cable-start { left: 0; }.cable-end { right: 0; }
  .native-connectors { padding: 0; background: transparent; }
  .native-connectors .energy-cable { display: none; }
  .native-connectors .bridge-shell { height: max-content; }
  .bridge-shell { position: relative; z-index: 1; width: 100%; height: max-content; padding: 11px; display: flex; flex-direction: column; gap: 9px; overflow: visible; border: 1px solid rgba(54,103,79,.34); border-radius: 15px; background: #edf3f0; box-shadow: inset 0 0 0 1px rgba(255,255,255,.66), 0 8px 24px rgba(30,58,44,.12); }
  .bridge-shell::before, .bridge-shell::after { position: absolute; top: 50%; width: 5px; height: 44%; content: ""; background: linear-gradient(180deg,transparent,rgba(59,154,105,.48),transparent); transform: translateY(-50%); }
  .bridge-shell::before { left: -1px; }.bridge-shell::after { right: -1px; }
  .vertical .bridge-shell::before, .vertical .bridge-shell::after { top: auto; width: 44%; height: 5px; background: linear-gradient(90deg,transparent,rgba(59,154,105,.48),transparent); transform: translateX(-50%); }
  .vertical .bridge-shell::before { top: -1px; left: 50%; }.vertical .bridge-shell::after { right: auto; bottom: -1px; left: 50%; }
  .dark { color: #dce8e1; }.dark.native-connectors { background: transparent; }.dark .bridge-shell { border-color: rgba(97,194,145,.24); background: #16211b; box-shadow: inset 0 0 0 1px rgba(220,238,228,.035); }
  .route-heading { min-height: 35px; display: grid; grid-template-columns: minmax(0,1fr) 25px minmax(0,1fr) 23px; align-items: center; gap: 4px; }
  .agent { min-width: 0; display: flex; align-items: center; gap: 5px; }.agent strong { overflow: hidden; color: currentColor; font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }.agent.target { justify-content: flex-end; text-align: right; }
  .route-heading button { width: 23px; height: 23px; padding: 0; display: grid; place-items: center; border: 0; border-radius: 7px; color: #71837a; background: transparent; cursor: pointer; }.route-heading button:hover { color: #2f835b; background: rgba(55,145,98,.08); }.route-heading .reverse:hover { transform: rotate(180deg); transition: transform 200ms ease; }
  .run-locked .route-heading .reverse, .run-locked .context-policy button, .run-locked .share-options button, .run-locked .behavior-row button, .run-locked .instruction-toggle:not(.preview-toggle), .run-locked textarea, .run-locked footer .remove, .run-locked footer .save { opacity: .42; pointer-events: none; }
  .context-policy { display: grid; gap: 6px; }.context-policy > strong { color: #466052; font-size: 10px; }.context-policy > div { height: 34px; padding: 3px; display: grid; grid-template-columns: repeat(4,minmax(0,1fr)); gap: 3px; border: 1px solid rgba(62,93,78,.2); border-radius: 9px; background: #e3ebe7; }.context-policy button { min-width: 0; padding: 0 4px; overflow: hidden; border: 0; border-radius: 6px; color: #65776e; background: transparent; font-size: 8px; font-weight: 760; text-overflow: ellipsis; text-transform: capitalize; cursor: pointer; }.context-policy button.active { color: #287a52; background: #f9fbfa; box-shadow: 0 2px 6px rgba(24,53,37,.1); }
  .share-options { display: grid; grid-template-columns: repeat(3,minmax(0,1fr)); gap: 4px; }
  .share-options button { position: relative; min-width: 0; height: 46px; padding: 4px 2px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px; border: 1px solid rgba(72,104,89,.13); border-radius: 9px; color: #718078; background: rgba(76,108,92,.035); cursor: pointer; }.share-options button > svg { width: 16px; height: 16px; }.share-options button.active { color: #287a52; border-color: rgba(53,145,96,.28); background: rgba(54,151,101,.08); }.share-options span { width: 100%; overflow: hidden; font-size: 7.5px; font-weight: 720; text-align: center; text-overflow: ellipsis; white-space: nowrap; }.share-options i { position: absolute; top: 3px; right: 3px; width: 10px; height: 10px; display: grid; place-items: center; border: 1px solid rgba(72,104,89,.16); border-radius: 50%; color: transparent; font: 800 6px Inter,sans-serif; font-style: normal; }.share-options button.active i { color: #fff; border-color: #388f64; background: #388f64; }
  .share-options.six { grid-template-columns: repeat(3,minmax(0,1fr)); gap: 4px; }
  .behavior-row { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; }.transition-toggle { height: 36px; padding: 3px; display: flex; gap: 2px; overflow: hidden; border: 1px solid rgba(72,104,89,.13); border-radius: 8px; background: rgba(76,108,92,.035); }.transition-toggle button { width: 28px; min-width: 28px; padding: 0; display: flex; align-items: center; justify-content: center; gap: 2px; overflow: hidden; border: 0; border-radius: 6px; color: #74837b; background: transparent; font-size: 8px; font-weight: 740; cursor: pointer; transition: width 180ms cubic-bezier(.2,.8,.2,1), color 140ms ease, background 160ms ease, box-shadow 160ms ease; }.transition-toggle button svg { width: 13px; height: 13px; flex: 0 0 auto; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }.transition-toggle button span { max-width: 0; overflow: hidden; opacity: 0; transform: translateX(-3px); white-space: nowrap; transition: max-width 180ms cubic-bezier(.2,.8,.2,1), opacity 120ms ease, transform 180ms ease; }.transition-toggle button.active { width: calc(100% - 30px); color: #287a52; background: rgba(255,255,255,.72); box-shadow: 0 2px 6px rgba(24,53,37,.1); }.transition-toggle button.active span { max-width: 48px; opacity: 1; transform: translateX(0); }
  .transition-toggle.manual-active button:first-child { width: calc(100% - 25px); }.transition-toggle.manual-active button:last-child { width: 23px; min-width: 23px; }
  .approval { height: 36px; padding: 0 7px; display: flex; align-items: center; gap: 5px; border: 1px solid rgba(72,104,89,.13); border-radius: 8px; color: #718078; background: rgba(76,108,92,.035); cursor: pointer; }.approval span { min-width: 0; flex: 1; overflow: hidden; font-size: 9px; font-weight: 740; text-overflow: ellipsis; white-space: nowrap; }.approval > i { width: 21px; height: 12px; padding: 2px; flex: 0 0 auto; border-radius: 7px; background: #c2ccc7; }.approval > i::before { width: 8px; height: 8px; display: block; border-radius: 50%; content: ""; background: #fff; transition: transform 140ms ease; }.approval.active { color: #287a52; border-color: rgba(53,145,96,.28); background: rgba(54,151,101,.08); }.approval.active > i { background: #388f64; }.approval.active > i::before { transform: translateX(9px); }
  .instruction-toggle { height: 32px; padding: 0 6px; display: flex; align-items: center; gap: 6px; border: 0; border-radius: 7px; color: #718078; background: transparent; font-size: 9px; font-weight: 730; cursor: pointer; }.instruction-toggle:hover { color: #287a52; background: rgba(54,151,101,.07); }.instruction-toggle span { min-width: 0; flex: 1; text-align: left; }.instruction-toggle svg:last-child { transition: transform 140ms ease; }.instruction-toggle.open svg:last-child { transform: rotate(180deg); }
  textarea { width: 100%; min-width: 0; min-height: 54px; padding: 7px; resize: none; border: 1px solid rgba(72,104,89,.14); border-radius: 8px; outline: none; color: inherit; background: rgba(76,108,92,.035); font: 9px/1.4 Inter,sans-serif; }.dark textarea { border-color: rgba(205,222,213,.11); background: rgba(222,235,228,.035); }
  .preview-actions { position: relative; display: grid; grid-template-columns: minmax(0,1fr) 34px; gap: 6px; }.preview-actions textarea { grid-column: 1; width: 100%; min-width: 0; min-height: 34px; height: 34px; padding: 9px; resize: none; overflow: hidden; border: 1px solid rgba(72,104,89,.14); border-radius: 8px; outline: 0; color: inherit; background: rgba(76,108,92,.035); font: 9px/14px Inter,sans-serif; transition: height 180ms cubic-bezier(.2,.8,.2,1), box-shadow 160ms ease, background 160ms ease; }.preview-actions textarea:focus { border-color: rgba(48,139,96,.48); box-shadow: 0 0 0 2px rgba(48,139,96,.07); }.preview-actions textarea.expanded { position: absolute; z-index: 60; right: 40px; bottom: 0; left: 0; width: auto; height: 100px; overflow: auto; background: #f8fbf9; box-shadow: 0 12px 28px rgba(24,53,37,.22); }.preview-actions button { position: relative; z-index: 61; grid-column: 2; width: 34px; height: 34px; padding: 0; display: grid; place-items: center; border: 1px solid rgba(53,145,96,.2); border-radius: 8px; color: #337d59; background: rgba(54,151,101,.07); cursor: pointer; }.preview-actions button svg { width: 16px; height: 16px; }.preview-actions button.open { color: #fff; border-color: #398d64; background: #398d64; }.preview-actions button:disabled { opacity: .5; }
  .workflow-start { width: 100%; height: 32px; display: flex; align-items: center; justify-content: center; gap: 6px; border: 1px solid #317e59; border-radius: 8px; color: #fff; background: #317e59; font-size: 9px; font-weight: 780; cursor: pointer; }.workflow-start:disabled { opacity: .45; cursor: default; }
  .workflow-runtime { padding: 8px; display: grid; gap: 7px; border: 1px solid rgba(53,145,96,.2); border-radius: 10px; background: rgba(54,151,101,.055); }.workflow-runtime > header { display: flex; align-items: center; gap: 6px; }.workflow-runtime > header small { min-width: 0; margin-left: auto; overflow: hidden; color: #73847b; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }.run-state { min-width: 0; display: flex; align-items: center; gap: 5px; }.run-state > i { width: 7px; height: 7px; flex: 0 0 auto; border-radius: 50%; background: #3e9a6a; box-shadow: 0 0 0 3px rgba(62,154,106,.1); }.run-state strong { color: #38674e; font-size: 9px; }.workflow-runtime[data-status="draft"] .run-state > i { background: #87958e; box-shadow: 0 0 0 3px rgba(98,116,106,.1); }.workflow-runtime[data-status="running"] .run-state > i { background: #4b9ed3; box-shadow: 0 0 0 3px rgba(75,158,211,.12); animation: run-pulse 1.2s ease-in-out infinite; }.workflow-runtime[data-status="waiting_for_approval"] .run-state > i, .workflow-runtime[data-status="paused"] .run-state > i { background: #d0a541; box-shadow: 0 0 0 3px rgba(208,165,65,.12); }.workflow-runtime[data-status="failed"] .run-state > i, .workflow-runtime[data-status="cancelled"] .run-state > i { background: #bd625d; box-shadow: 0 0 0 3px rgba(189,98,93,.12); }
  .run-progress { display: flex; align-items: center; gap: 4px; }.run-progress::before, .run-progress::after { height: 1px; flex: 1; content: ""; background: rgba(72,104,89,.11); }.run-progress i { width: 7px; height: 7px; flex: 0 0 auto; border: 1px solid rgba(72,104,89,.22); border-radius: 50%; background: transparent; }.run-progress i[data-status="completed"] { border-color: #3d9668; background: #3d9668; }.run-progress i[data-status="running"], .run-progress i.active { border-color: #4b9ed3; box-shadow: 0 0 0 2px rgba(75,158,211,.12); }.run-progress i[data-status="failed"] { border-color: #bd625d; background: #bd625d; }.run-progress i[data-status="skipped"] { border-color: #ba9b50; background: #ba9b50; }
  .run-actions { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 4px; }.recovery-note { min-width: 0; margin-right: auto; color: #71847a; font-size: 8px; }.run-actions button { min-width: 0; height: 27px; padding: 0 8px; border: 1px solid rgba(72,104,89,.14); border-radius: 7px; color: #66776e; background: transparent; font-size: 8px; font-weight: 760; cursor: pointer; }.run-actions button.primary { color: #fff; border-color: #317e59; background: #317e59; }.run-actions button.stop { color: #a35855; border-color: rgba(163,88,85,.18); }.run-actions button:disabled { opacity: .45; cursor: default; }.workflow-runtime > p { margin: 0; color: #aa5752; font-size: 8px; line-height: 1.35; }
  .context-preview { position: absolute; z-index: 40; inset: 47px 10px 47px; padding: 11px; display: flex; flex-direction: column; gap: 8px; overflow: hidden; border: 1px solid rgba(55,145,99,.22); border-radius: 11px; background: #f6faf8; box-shadow: 0 14px 34px rgba(24,53,37,.2); }.context-preview > header { min-height: 28px; padding: 0; display: grid; grid-template-columns: minmax(0,1fr) auto 25px; align-items: center; gap: 6px; border: 0; }.context-preview > header strong { color: #426552; font-size: 10px; }.context-preview > header span { color: #75877d; font-size: 8px; }.context-preview > header button { width: 25px; height: 25px; padding: 0; display: grid; place-items: center; border: 0; border-radius: 7px; color: #71837a; background: transparent; cursor: pointer; }.context-preview > header button:hover { color: #2f835b; background: rgba(55,145,98,.08); }.context-preview pre { min-height: 0; margin: 0; padding: 9px; flex: 1; overflow: auto; border-radius: 8px; color: #52665b; background: rgba(255,255,255,.62); font: 9px/1.55 "SFMono-Regular",Consolas,monospace; overflow-wrap: anywhere; white-space: pre-wrap; word-break: break-word; }.redaction-summary { display: flex; flex-wrap: wrap; gap: 4px; }.redaction-summary span { padding: 3px 5px; border-radius: 5px; color: #856f35; background: rgba(207,168,68,.1); font-size: 7.5px; }.preview-loading, .preview-empty { min-height: 0; display: flex; flex: 1; align-items: center; justify-content: center; gap: 7px; color: #73847b; font-size: 9px; text-align: center; }.preview-loading i { width: 14px; height: 14px; border: 1.5px solid rgba(72,104,89,.14); border-top-color: #398d64; border-radius: 50%; animation: spin .7s linear infinite; }
  footer { position: relative; display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; gap: 4px; } footer button, .error-state button { min-width: 54px; height: 27px; padding: 0 7px; border: 1px solid rgba(72,104,89,.14); border-radius: 7px; color: #66776e; background: transparent; font-size: 8px; font-weight: 750; cursor: pointer; } footer .remove { justify-self: start; color: #a35855; border-color: transparent; } footer .save { justify-self: end; color: #fff; border-color: #317e59; background: #317e59; } footer button:disabled { opacity: .45; }
  .connection-help { position: relative; display: grid; place-items: center; }.connection-help .help-trigger { width: 23px; min-width: 23px; height: 23px; padding: 0; border-radius: 50%; color: #5d7569; background: rgba(73,112,92,.055); font-size: 9px; }.connection-help:hover .help-trigger, .connection-help:focus-within .help-trigger { color: #287a52; border-color: rgba(53,145,96,.28); background: rgba(54,151,101,.09); }.help-tooltip { position: fixed; z-index: 50; top: 7px; right: 7px; left: 7px; padding: 9px; display: grid; gap: 6px; border: 1px solid rgba(65,105,85,.18); border-radius: 11px; color: #60736a; background: #f8fbf9; box-shadow: 0 14px 32px rgba(20,43,31,.2); opacity: 0; pointer-events: none; transform: translateY(4px) scale(.97); transform-origin: center bottom; transition: opacity 130ms ease, transform 150ms cubic-bezier(.2,.8,.2,1); }.connection-help:hover .help-tooltip, .connection-help:focus-within .help-tooltip { opacity: 1; transform: translateY(0) scale(1); }.help-tooltip > strong { color: #354d41; font-size: 9px; }.help-item { display: grid; grid-template-columns: 17px minmax(0,1fr); align-items: start; gap: 5px; font-size: 7px; line-height: 1.35; }.help-item > svg { width: 13px; height: 13px; margin-top: 1px; justify-self: center; color: #438763; }.help-item > span { display: grid; gap: 1px; }.help-tooltip b { color: #3f6954; font-size: 7px; }
  .error { margin: 0; color: #aa5752; font-size: 8px; line-height: 1.35; }.loading { height: 100%; display: grid; place-content: center; justify-items: center; gap: 8px; color: #718078; font-size: 9px; }.loading > i { width: 20px; height: 20px; border: 2px solid rgba(72,104,89,.14); border-top-color: #398d64; border-radius: 50%; animation: spin .7s linear infinite; }.error-state button { margin-top: 5px; }
  .dark .share-options button, .dark .transition-toggle, .dark .approval, .dark .context-policy > div { color: #9cafa5; border-color: rgba(205,222,213,.1); background: rgba(222,235,228,.035); }.dark .share-options button.active, .dark .approval.active { color: #8fd1af; border-color: rgba(101,201,150,.22); background: rgba(74,164,116,.09); }.dark .transition-toggle button, .dark .context-policy button { color: #91a39a; }.dark .transition-toggle button.active, .dark .context-policy button.active { color: #9bd8b8; background: rgba(220,235,227,.065); }.dark .context-policy strong { color: #c9d9d0; }.dark .context-preview { border-color: rgba(101,201,150,.17); background: #16211b; }.dark .context-preview > header strong { color: #91cdae; }.dark .context-preview > header span, .dark .preview-loading, .dark .preview-empty { color: #97a99f; }.dark .context-preview pre { color: #bdcdc4; background: rgba(2,8,5,.2); }
  .dark .preview-actions textarea { border-color: rgba(205,222,213,.11); background: rgba(222,235,228,.035); }.dark .preview-actions textarea.expanded { background: #1b2821; box-shadow: 0 14px 30px rgba(0,0,0,.38); }
  .dark .workflow-runtime { border-color: rgba(101,201,150,.15); background: rgba(74,164,116,.055); }.dark .run-state strong { color: #a9d7bd; }.dark .workflow-runtime > header small { color: #93a59b; }.dark .run-actions button { color: #a5b6ad; border-color: rgba(205,222,213,.11); }.dark .run-actions button.primary { color: #fff; border-color: #398d64; background: #398d64; }.dark .run-actions button.stop { color: #d58e88; border-color: rgba(213,142,136,.16); }
  .vertical.bridge-window { padding: 22px 4px; }.vertical.native-connectors { padding: 0; }.vertical .energy-cable { top: auto; left: 50%; transform: translateX(-50%) rotate(90deg); }.vertical .cable-start { top: 2px; }.vertical .cable-end { right: auto; bottom: 2px; }.vertical .bridge-shell { height: max-content; padding-right: 12px; padding-left: 12px; gap: 6px; }.vertical.native-connectors .bridge-shell { height: max-content; gap: 9px; }.vertical .route-heading { grid-template-columns: minmax(0,1fr) 28px minmax(0,1fr) 25px; }.vertical .share-options button { height: 46px; }.vertical .behavior-row { grid-template-columns: 1fr 1fr; }.vertical .instruction-toggle { height: 32px; }
  .dark .help-tooltip { color: #9eb0a7; border-color: rgba(205,222,213,.12); background: #1b2821; box-shadow: 0 16px 34px rgba(0,0,0,.38); }.dark .help-tooltip > strong { color: #d8e5de; }.dark .help-tooltip b { color: #91cdae; }
  @keyframes bridge-fade-in { from { opacity: 0; } to { opacity: 1; } }
  @keyframes spin { to { transform: rotate(360deg); } }
  @keyframes run-pulse { 50% { opacity: .45; transform: scale(.78); } }
</style>
