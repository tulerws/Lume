<script lang="ts">
  import { onMount } from "svelte";
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import { fade, fly, slide } from "svelte/transition";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    availableMonitors,
    getCurrentWindow,
    LogicalSize,
    PhysicalPosition,
    primaryMonitor,
  } from "@tauri-apps/api/window";
  import type {
    AgentKind,
    AgentSession,
    CompanionStatus,
    HistoryEntry,
    IntegrationStatus,
    PermissionAction,
    Preferences,
    SessionStatus,
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
    openSessionSource,
    loadVscodeStatus,
    revealBrowserCompanion,
    launchAgentSession,
    savePreferences,
  } from "$lib/lume";

  type View = "sessions" | "history" | "settings";
  type ShellStatus = SessionStatus | "idle";
  type MonitorOption = { id: string; label: string };

  const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  let expanded = $state(!isTauri);
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
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let pollTimer: ReturnType<typeof setInterval> | undefined;

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
        pollTimer = setInterval(() => void refreshSessions(false), 5_000);
      }
    })();

    return () => {
      disposed = true;
      stopListening?.();
      if (pollTimer) clearInterval(pollTimer);
    };
  });

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

  async function positionWindow() {
    if (!isTauri) return;
    try {
      const currentWindow = getCurrentWindow();
      const target = expanded
        ? { width: 392, height: 560 }
        : { width: 78, height: 46 };
      await currentWindow.setSize(new LogicalSize(target.width, target.height));

      const found = await availableMonitors();
      const configured = preferences.monitorId
        ? found.find((monitor, index) =>
            (monitor.name ?? `monitor-${index}`) === preferences.monitorId,
          )
        : undefined;
      const monitor = configured ?? (await primaryMonitor());
      if (!monitor) return;

      const windowSize = await currentWindow.outerSize();
      const x = monitor.position.x + Math.round((monitor.size.width - windowSize.width) / 2);
      const y = monitor.position.y + 12;
      await currentWindow.setPosition(new PhysicalPosition(x, y));
    } catch {
      // Alguns compositores Wayland ignoram posicionamento solicitado pelo cliente.
    }
  }

  async function toggleExpanded() {
    expanded = !expanded;
    if (!expanded) {
      selectedId = null;
      view = "sessions";
      launcherOpen = false;
    }
    await positionWindow();
  }

  function openSession(session: AgentSession) {
    selectedId = selectedId === session.id ? null : session.id;
    permissionError = null;
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

  function agentInitial(agent: AgentKind) {
    return { codex: "C", claude: "A", gemini: "G", unknown: "?" }[agent];
  }

  function sourceLabel(source: AgentSession["source"]) {
    return { cli: "CLI", vscode: "VS Code", web: "Web", desktop: "Desktop" }[source];
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

<main class:expanded class="overlay-shell" aria-label="Lume, monitor de agentes">
  {#if !expanded}
    <button
      class="lume-orb status-{shellStatus}"
      type="button"
      onclick={toggleExpanded}
      aria-label="Abrir Lume, {activeCount} agentes ativos"
      transition:fade={{ duration: 120 }}
    >
      <span class="lume-mark" aria-hidden="true">
        {#if shellStatus === "completed"}
          <svg class="status-glyph" viewBox="0 0 20 20"><path d="m5.5 10.2 3 3 6-6.2" /></svg>
        {:else if shellStatus === "failed"}
          <svg class="status-glyph" viewBox="0 0 20 20"><path d="M10 5.2v6.2M10 14.7h.01" /></svg>
        {:else}
          <span></span>
        {/if}
      </span>
      <span class="agent-count">{activeCount}</span>
    </button>
  {:else}
    <section class="panel" transition:fly={{ y: -8, duration: 220, easing: cubicOut }}>
      <header class="panel-header" data-tauri-drag-region>
        {#if view === "sessions"}
          <div class="brand-lockup">
            <span class="lume-mark large status-{shellStatus}" aria-hidden="true">
              {#if shellStatus === "completed"}
                <svg class="status-glyph" viewBox="0 0 20 20"><path d="m5.5 10.2 3 3 6-6.2" /></svg>
              {:else if shellStatus === "failed"}
                <svg class="status-glyph" viewBox="0 0 20 20"><path d="M10 5.2v6.2M10 14.7h.01" /></svg>
              {:else}
                <span></span>
              {/if}
            </span>
            <div>
              <strong>Lume</strong>
              <span>{activeCount === 1 ? "1 agente ativo" : `${activeCount} agentes ativos`}</span>
            </div>
          </div>
        {:else}
          <button class="back-button" type="button" onclick={() => openView("sessions")}>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m12.5 5-5 5 5 5" /></svg>
            <span>{view === "history" ? "Histórico" : "Configurações"}</span>
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
              <span class="agent-avatar agent-{integration.kind}">{agentInitial(integration.kind)}</span>
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
                  <span class="agent-avatar agent-{session.agent}">{agentInitial(session.agent)}</span>
                  <span class="session-copy">
                    <span class="session-title-row">
                      <strong>{session.agentLabel}</strong>
                      <span class="source-label">{sourceLabel(session.source)}</span>
                    </span>
                    <span class="project-name">{session.project}</span>
                    <span class="status-line status-{session.status}">
                      <i></i>{session.statusLabel}
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
                  </div>
                {/if}
              </article>
            {:else}
              <div class="empty-state" transition:fade>
                <span class="quiet-orbit" aria-hidden="true"><i></i></span>
                <strong>Nenhum agente ativo</strong>
                <p>Novas sessões aparecerão aqui automaticamente.</p>
              </div>
            {/each}
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
                <span class="agent-avatar agent-{integration.kind}">{agentInitial(integration.kind)}</span>
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
              <span class="agent-avatar agent-vscode">V</span>
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
              <span class="agent-avatar agent-browser">W</span>
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

<style>
  .overlay-shell {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: flex-start;
    justify-content: center;
  }

  .overlay-shell.expanded {
    padding: 8px;
  }

  button,
  select {
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
    transition: transform 160ms ease, box-shadow 160ms ease;
  }

  .lume-orb:hover {
    transform: translateY(1px) scale(1.02);
    box-shadow: 0 9px 30px rgba(28, 43, 37, 0.2);
  }

  .lume-orb:active { transform: scale(0.97); }

  .lume-mark {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border: 1.4px solid currentColor;
    border-radius: 50%;
    transition: color 180ms ease, transform 180ms ease;
  }

  .lume-mark > span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }

  .lume-mark .status-glyph {
    width: 14px;
    height: 14px;
    stroke-width: 2;
  }

  .lume-mark.large .status-glyph { width: 17px; height: 17px; }

  .lume-mark.large { width: 27px; height: 27px; }
  .lume-mark.large > span { width: 7px; height: 7px; }
  .status-permission_required { color: #ae6b24; }
  .status-failed { color: #a84d4d; }
  .status-completed { color: #708079; }
  .status-idle { color: #829089; }

  .status-permission_required.lume-orb .lume-mark {
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
    width: 376px;
    height: 544px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid rgba(105, 124, 116, 0.18);
    border-radius: 21px;
    color: #26322e;
    background: rgba(249, 251, 250, 0.965);
    box-shadow: 0 20px 58px rgba(25, 39, 33, 0.2);
    backdrop-filter: blur(28px) saturate(125%);
  }

  .panel-header {
    min-height: 61px;
    padding: 12px 13px 10px 16px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid rgba(101, 120, 112, 0.11);
  }

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

  .panel-content { min-height: 0; flex: 1; overflow: hidden; }
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
  .settings { height: 100%; overflow-y: auto; scrollbar-width: thin; scrollbar-color: #cad2ce transparent; }

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
  .agent-codex { color: #edf5f1; background: #294138; }
  .agent-claude { color: #583928; background: #ead2c0; }
  .agent-gemini { color: #34456d; background: #dbe4f8; }
  .agent-vscode { color: #e7f3fb; background: #287aa9; }
  .agent-browser { color: #52615a; background: #e1e7e4; }
  .agent-unknown { color: #48534f; background: #e2e7e4; }

  .session-copy { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .session-title-row { display: flex; align-items: baseline; gap: 6px; }
  .session-title-row strong { color: #27342f; font-size: 11px; }
  .source-label { color: #87918d; font-size: 9px; font-weight: 720; letter-spacing: 0.045em; text-transform: uppercase; }
  .project-name { overflow: hidden; color: #56645e; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }

  .status-line { display: flex; align-items: center; gap: 5px; color: #7a8580; font-size: 10px; }
  .status-line i { width: 5px; height: 5px; border-radius: 50%; background: #82908a; }
  .status-line.status-running i { background: #4c8871; }
  .status-line.status-permission_required { color: #a46522; }
  .status-line.status-permission_required i { background: #cb8235; box-shadow: 0 0 0 3px rgba(203, 130, 53, 0.1); }
  .status-line.status-completed i { background: #78a18f; }
  .status-line.status-failed i { background: #b95454; }
  .status-line.status-waiting_for_input i { background: #6681a4; }

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
  .save-state { display: block; color: #87928d; font-size: 9px; text-align: right; opacity: 0; transition: opacity 120ms ease; }
  .save-state.visible { opacity: 1; }

  footer {
    min-height: 52px;
    padding: 6px 10px 8px;
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    align-items: center;
    border-top: 1px solid rgba(101, 120, 112, 0.11);
  }
  footer button { height: 36px; display: flex; align-items: center; justify-content: center; gap: 5px; border: 0; border-radius: 10px; color: #88928e; background: transparent; font-size: 9px; font-weight: 650; cursor: pointer; transition: color 150ms ease, background 150ms ease; }
  footer button:hover { color: #52615a; background: rgba(76, 100, 90, 0.045); }
  footer button.active { color: #476c5d; }
  footer button svg { width: 15px; height: 15px; }

  @media (prefers-reduced-motion: reduce) {
    *, *::before, *::after { animation-duration: 0.01ms !important; animation-iteration-count: 1 !important; transition-duration: 0.01ms !important; }
  }

  @media (prefers-color-scheme: dark) {
    .lume-orb,
    .panel,
    .launcher-popover { color: #dfe8e3; border-color: rgba(190, 209, 200, 0.13); background: rgba(27, 34, 31, 0.96); box-shadow: 0 20px 58px rgba(0, 0, 0, 0.34); }
    .brand-lockup strong,
    .back-button,
    .session-title-row strong,
    .history-row strong,
    .setting-row strong,
    .integration-row strong,
    .field-row strong,
    .launch-setting strong { color: #e3ebe7; }
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
    .history-row span,
    .settings-feedback { color: #adbab4; }
    .settings-feedback.error { color: #d68d8d; }
    .access-profile span,
    .empty-state strong { color: #c5d0cb; }
    code,
    .segmented { color: #bdc8c3; background: rgba(216, 229, 223, 0.06); }
    .permission-block > strong { color: #e2d0bd; }
    .permission-actions button,
    .field-row select { color: #c5d0cb; border-color: rgba(207, 223, 215, 0.12); background: rgba(222, 233, 228, 0.04); }
    .permission-actions button:hover { background: rgba(222, 233, 228, 0.09); }
    .segmented button.active { color: #dfe8e3; background: rgba(214, 229, 221, 0.1); }
  }
</style>
