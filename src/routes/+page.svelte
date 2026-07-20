<script lang="ts">
  import { onMount } from "svelte";
  import {
    getCurrentWindow,
    LogicalSize,
    PhysicalPosition,
    primaryMonitor,
  } from "@tauri-apps/api/window";
  import type {
    AgentKind,
    AgentSession,
    PermissionAction,
    SessionStatus,
  } from "$lib/domain";
  import { demoSessions } from "$lib/demo";
  import { decidePermission, loadSessions } from "$lib/lume";

  const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  let expanded = $state(!isTauri);
  let sessions = $state<AgentSession[]>(structuredClone(demoSessions));
  let selectedId = $state<string | null>(null);

  const activeCount = $derived(
    sessions.filter((session) =>
      ["running", "permission_required", "waiting_for_input"].includes(session.status),
    ).length,
  );

  const shellStatus = $derived.by<SessionStatus>(() => {
    if (sessions.some((session) => session.status === "permission_required")) {
      return "permission_required";
    }
    if (sessions.some((session) => session.status === "failed")) return "failed";
    if (sessions.some((session) => session.status === "running")) return "running";
    return "completed";
  });

  const selectedSession = $derived(
    sessions.find((session) => session.id === selectedId) ?? null,
  );

  onMount(async () => {
    sessions = await loadSessions();
    selectedId =
      sessions.find((session) => session.status === "permission_required")?.id ?? null;
    await positionWindow();
  });

  async function positionWindow() {
    if (!isTauri) return;

    try {
      const window = getCurrentWindow();
      const target = expanded
        ? { width: 392, height: 560 }
        : { width: 72, height: 44 };

      await window.setSize(new LogicalSize(target.width, target.height));
      const monitor = await primaryMonitor();
      if (!monitor) return;

      const windowSize = await window.outerSize();
      const x = monitor.position.x + Math.round((monitor.size.width - windowSize.width) / 2);
      const y = monitor.position.y + 12;
      await window.setPosition(new PhysicalPosition(x, y));
    } catch {
      // Some Wayland compositors ignore client-side positioning.
    }
  }

  async function toggleExpanded() {
    expanded = !expanded;
    if (!expanded) selectedId = null;
    await positionWindow();
  }

  function openSession(session: AgentSession) {
    selectedId = selectedId === session.id ? null : session.id;
  }

  async function handlePermission(session: AgentSession, action: PermissionAction) {
    const permission = session.pendingPermission;
    if (!permission) return;

    if (action === "open_source") return;

    await decidePermission(session.id, permission.id, action);

    sessions = sessions.map((item) => {
      if (item.id !== session.id) return item;
      return {
        ...item,
        status: action === "deny" ? "failed" : "running",
        statusLabel: action === "deny" ? "Permissão recusada" : "Continuando a tarefa",
        pendingPermission: undefined,
      };
    });
    selectedId = null;
  }

  function actionLabel(action: PermissionAction) {
    return {
      allow_once: "Permitir uma vez",
      allow_session: "Permitir nesta sessão",
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
</script>

<svelte:head>
  <title>Lume</title>
  <meta
    name="description"
    content="Monitor local e discreto para sessões de agentes de IA."
  />
</svelte:head>

<main class:expanded class="overlay-shell" aria-label="Lume, monitor de agentes">
  {#if !expanded}
    <button
      class="lume-orb status-{shellStatus}"
      type="button"
      onclick={toggleExpanded}
      aria-label="Abrir Lume, {activeCount} agentes ativos"
    >
      <span class="lume-mark" aria-hidden="true">
        <span></span>
      </span>
      <span class="agent-count">{activeCount}</span>
    </button>
  {:else}
    <section class="panel">
      <header class="panel-header" data-tauri-drag-region>
        <div class="brand-lockup">
          <span class="lume-mark large" aria-hidden="true"><span></span></span>
          <div>
            <strong>Lume</strong>
            <span>{activeCount} agentes ativos</span>
          </div>
        </div>
        <button class="collapse-button" type="button" onclick={toggleExpanded} aria-label="Recolher">
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="m5.5 8 4.5 4 4.5-4" />
          </svg>
        </button>
      </header>

      <div class="session-list">
        {#each sessions as session (session.id)}
          <article
            class:attention={session.status === "permission_required"}
            class:selected={selectedId === session.id}
            class="session-card"
          >
            <button class="session-summary" type="button" onclick={() => openSession(session)}>
              <span class="agent-avatar agent-{session.agent}">
                {agentInitial(session.agent)}
              </span>
              <span class="session-copy">
                <span class="session-title-row">
                  <strong>{session.agentLabel}</strong>
                  <span class="source-chip">{sourceLabel(session.source)}</span>
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
              <div class="session-details">
                <div class="access-profile">
                  <span>{session.permissionProfile.label}</span>
                  <small>{session.permissionProfile.approvalPolicy}</small>
                </div>

                {#if session.pendingPermission}
                  <div class="permission-card risk-{session.pendingPermission.risk}">
                    <div class="permission-heading">
                      <span class="permission-icon" aria-hidden="true">!</span>
                      <div>
                        <strong>Permissão solicitada</strong>
                        <span>{session.pendingPermission.summary}</span>
                      </div>
                    </div>
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
                  </div>
                {:else if !session.permissionProfile.canRespondFromLume}
                  <p class="integration-note">
                    Esta origem permite monitoramento. As ações continuam na interface original.
                  </p>
                {/if}
              </div>
            {/if}
          </article>
        {/each}
      </div>

      <footer>
        <button type="button">
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M4 5h12M4 10h12M4 15h8" />
          </svg>
          Histórico
        </button>
        <button type="button" aria-label="Configurações">
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <circle cx="10" cy="10" r="3" />
            <path d="M10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M15.3 4.7l-1.4 1.4M6.1 13.9l-1.4 1.4" />
          </svg>
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
    padding: 0;
  }

  .overlay-shell.expanded {
    padding: 8px;
  }

  .lume-orb {
    position: relative;
    width: 72px;
    height: 42px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    border: 1px solid rgba(111, 128, 121, 0.22);
    border-radius: 999px;
    color: #31443c;
    background: rgba(250, 252, 251, 0.94);
    box-shadow: 0 7px 24px rgba(34, 48, 42, 0.16);
    backdrop-filter: blur(18px);
    cursor: pointer;
  }

  .lume-mark {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border: 1.5px solid currentColor;
    border-radius: 50%;
  }

  .lume-mark span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }

  .lume-mark.large {
    width: 28px;
    height: 28px;
  }

  .lume-mark.large span {
    width: 8px;
    height: 8px;
  }

  .status-running .lume-mark,
  .status-running.lume-orb {
    color: #527b6d;
  }

  .status-permission_required .lume-mark,
  .status-permission_required.lume-orb {
    color: #b36d22;
  }

  .status-failed .lume-mark,
  .status-failed.lume-orb {
    color: #a94848;
  }

  .agent-count {
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    display: grid;
    place-items: center;
    border-radius: 999px;
    color: #f8faf9;
    background: #30473e;
    font-size: 10px;
    font-weight: 750;
  }

  .panel {
    width: 376px;
    max-height: 544px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid rgba(112, 128, 121, 0.2);
    border-radius: 20px;
    background: rgba(249, 251, 250, 0.96);
    box-shadow: 0 18px 55px rgba(30, 43, 37, 0.2);
    backdrop-filter: blur(24px);
  }

  .panel-header {
    min-height: 64px;
    padding: 13px 14px 11px 16px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid rgba(113, 128, 122, 0.13);
  }

  .brand-lockup {
    display: flex;
    align-items: center;
    gap: 10px;
    color: #45675a;
  }

  .brand-lockup div {
    display: grid;
    gap: 1px;
  }

  .brand-lockup strong {
    color: #1e2b26;
    font-size: 14px;
    letter-spacing: -0.01em;
  }

  .brand-lockup span:last-child {
    color: #73817b;
    font-size: 11px;
  }

  .collapse-button {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 9px;
    color: #6b7a74;
    background: transparent;
    cursor: pointer;
  }

  .collapse-button:hover {
    background: #eef2f0;
  }

  svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-width: 1.7;
  }

  .session-list {
    padding: 10px;
    display: grid;
    gap: 7px;
    overflow-y: auto;
  }

  .session-card {
    overflow: hidden;
    border: 1px solid transparent;
    border-radius: 14px;
    background: #f3f6f4;
    transition:
      border-color 140ms ease,
      background 140ms ease;
  }

  .session-card:hover,
  .session-card.selected {
    border-color: rgba(92, 119, 108, 0.2);
    background: #f7f9f8;
  }

  .session-card.attention {
    border-color: rgba(190, 120, 44, 0.28);
    background: #fbf7f0;
  }

  .session-summary {
    width: 100%;
    min-height: 78px;
    padding: 11px 12px;
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
    width: 34px;
    height: 34px;
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    border-radius: 10px;
    font-size: 12px;
    font-weight: 780;
  }

  .agent-codex {
    color: #eaf4ef;
    background: #243b32;
  }

  .agent-claude {
    color: #4f3325;
    background: #ead4c4;
  }

  .agent-gemini {
    color: #34446f;
    background: #dce5fa;
  }

  .session-copy {
    min-width: 0;
    flex: 1;
    display: grid;
    gap: 2px;
  }

  .session-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .session-title-row strong {
    color: #26332e;
    font-size: 12px;
  }

  .source-chip {
    padding: 2px 5px;
    border-radius: 5px;
    color: #77847f;
    background: rgba(104, 124, 116, 0.08);
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .project-name {
    overflow: hidden;
    color: #51605a;
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status-line {
    display: flex;
    align-items: center;
    gap: 5px;
    color: #75817c;
    font-size: 10px;
  }

  .status-line i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #82918b;
  }

  .status-line.status-running i {
    background: #4d8771;
  }

  .status-line.status-permission_required {
    color: #a66623;
  }

  .status-line.status-permission_required i {
    background: #cf8538;
  }

  .status-line.status-completed i {
    background: #75a28f;
  }

  .status-line.status-failed i {
    background: #bc5555;
  }

  .chevron {
    width: 14px;
    height: 14px;
    color: #8b9692;
  }

  .selected .chevron {
    transform: rotate(90deg);
  }

  .session-details {
    padding: 0 12px 12px 57px;
  }

  .access-profile {
    margin-bottom: 9px;
    display: grid;
    gap: 1px;
  }

  .access-profile span {
    color: #43514b;
    font-size: 10px;
    font-weight: 700;
  }

  .access-profile small {
    color: #82908a;
    font-size: 9px;
  }

  .permission-card {
    padding: 10px;
    border: 1px solid rgba(183, 113, 39, 0.18);
    border-radius: 11px;
    background: #fffdfa;
  }

  .permission-heading {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }

  .permission-icon {
    width: 18px;
    height: 18px;
    flex: 0 0 auto;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: #9b5d1d;
    background: #f4e1cb;
    font-size: 10px;
    font-weight: 800;
  }

  .permission-heading div {
    display: grid;
    gap: 2px;
  }

  .permission-heading strong {
    color: #553c28;
    font-size: 10px;
  }

  .permission-heading span {
    color: #75675c;
    font-size: 9px;
    line-height: 1.4;
  }

  code {
    margin: 8px 0;
    padding: 7px 8px;
    display: block;
    overflow: hidden;
    border-radius: 7px;
    color: #435149;
    background: #f1f3f1;
    font-family: "SFMono-Regular", Consolas, monospace;
    font-size: 9px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .permission-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .permission-actions button {
    min-height: 26px;
    padding: 0 8px;
    border: 1px solid #d9dfdc;
    border-radius: 7px;
    color: #53615b;
    background: #fff;
    font-size: 8.5px;
    font-weight: 700;
    cursor: pointer;
  }

  .permission-actions button.primary {
    border-color: #3f6254;
    color: #f7faf8;
    background: #3f6254;
  }

  .permission-actions button.danger {
    color: #a44747;
  }

  .integration-note {
    margin: 0;
    color: #78857f;
    font-size: 9px;
    line-height: 1.45;
  }

  footer {
    min-height: 48px;
    padding: 8px 12px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-top: 1px solid rgba(113, 128, 122, 0.13);
  }

  footer button {
    min-height: 30px;
    padding: 0 8px;
    display: flex;
    align-items: center;
    gap: 6px;
    border: 0;
    border-radius: 8px;
    color: #708079;
    background: transparent;
    font-size: 10px;
    cursor: pointer;
  }

  footer button:hover {
    color: #40534b;
    background: #eef2f0;
  }

  footer svg {
    width: 15px;
    height: 15px;
  }

  @media (prefers-color-scheme: dark) {
    .lume-orb,
    .panel {
      border-color: rgba(192, 209, 201, 0.12);
      color: #d7e2dd;
      background: rgba(28, 34, 32, 0.96);
      box-shadow: 0 18px 55px rgba(0, 0, 0, 0.36);
    }

    .brand-lockup strong,
    .session-title-row strong {
      color: #edf4f1;
    }

    .panel-header,
    footer {
      border-color: rgba(197, 211, 205, 0.08);
    }

    .session-card,
    .session-card:hover,
    .session-card.selected {
      border-color: transparent;
      background: #252c29;
    }

    .session-card.attention {
      border-color: rgba(205, 139, 67, 0.25);
      background: #302b25;
    }

    .project-name,
    .access-profile span {
      color: #bcc8c2;
    }

    .permission-card {
      border-color: rgba(201, 135, 64, 0.2);
      background: #27231f;
    }

    .permission-heading strong {
      color: #ead9ca;
    }

    code {
      color: #d3ded8;
      background: #1b211e;
    }

    .permission-actions button {
      border-color: #46514c;
      color: #d4ddd9;
      background: #29312e;
    }
  }
</style>
