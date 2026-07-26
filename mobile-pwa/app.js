const tokenKey = "lume-mobile-token-v1";
const baseKey = "lume-mobile-gateway-v1";
const credentialKey = "lume-mobile-biometric-v1";
const notificationsKey = "lume-mobile-notifications-v1";
const mobileUpdateCheckKey = "lume-mobile-update-check-v1";
const mobileManifestUrl =
  "https://github.com/tulerws/Lume/releases/latest/download/mobile-latest.json";
const mobileUpdateInterval = 6 * 60 * 60 * 1000;
const params = new URLSearchParams(location.search);
let pairingCode = params.get("code");
const pairView = document.querySelector("#pair-view");
const pairForm = document.querySelector("#pair-form");
const pairMessage = document.querySelector("#pair-message");
const emptyAuthView = document.querySelector("#empty-auth-view");
const scanPairingButton = document.querySelector("#scan-pairing-qr");
const dashboard = document.querySelector("#dashboard");
const appContent = document.querySelector("#app-content");
const loadingView = document.querySelector("#loading-view");
const sessionList = document.querySelector("#session-list");
const resultsList = document.querySelector("#results-list");
const dashboardMessage = document.querySelector("#dashboard-message");
const connectionDot = document.querySelector("#connection-dot");
const connectionLabel = document.querySelector("#connection-label");
const androidInstallCard = document.querySelector("#android-install-card");
const pairInstallPrompt = document.querySelector("#pair-install-prompt");
const openLumeMobile = document.querySelector("#open-lume-mobile");
const mobileApkDeviceDownload = document.querySelector("#mobile-apk-device-download");
const pwaInstallButton = document.querySelector("#pwa-install-button");
const mobileUpdateCard = document.querySelector("#mobile-update-card");
const mobileUpdateButton = document.querySelector("#mobile-update-button");
const mobileUpdateDetail = document.querySelector("#mobile-update-detail");
const mobileVersionLabel = document.querySelector("#mobile-version-label");
const mobileUpdateProgress = document.querySelector("#mobile-update-progress");
let token = localStorage.getItem(tokenKey);
let apiBase = localStorage.getItem(baseKey) || "";
let pollTimer;
let lastSequence = 0;
let currentDevice;
let previousStatuses = new Map();
let hasRenderedSnapshot = false;
let currentSnapshot;
let activeFilter = "all";
let deferredInstallPrompt;
let installedMobileInfo;
let availableMobileUpdate;
let mobileUpdateBusy = false;
const expandedSessions = new Set();

const escapeHtml = (value = "") =>
  String(value).replace(/[&<>"']/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#039;",
  })[character]);

const randomBytes = (length) => crypto.getRandomValues(new Uint8Array(length));
const encodeBytes = (value) =>
  btoa(String.fromCharCode(...new Uint8Array(value)))
    .replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
const decodeBytes = (value) => {
  const base64 = value.replaceAll("-", "+").replaceAll("_", "/");
  const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
  return Uint8Array.from(atob(padded), (character) => character.charCodeAt(0));
};

async function api(path, options = {}) {
  const headers = { "Content-Type": "application/json", ...(options.headers || {}) };
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await fetch(`${apiBase}${path}`, { ...options, headers, cache: "no-store" });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    if (response.status === 401 && path !== "/api/v1/pair") {
      localStorage.removeItem(tokenKey);
      token = null;
      showEntryView();
    }
    throw new Error(body.error?.message || `Lume returned ${response.status}`);
  }
  return body;
}

function showEntryView() {
  clearTimeout(pollTimer);
  appContent.hidden = true;
  loadingView.hidden = true;
  pairView.hidden = !pairingCode;
  emptyAuthView.hidden = Boolean(pairingCode);
  connectionDot.className = "";
  connectionLabel.textContent = "Not paired";
  document.querySelector("#refresh-button").hidden = true;
  updateInstallOptions();
  if (pairingCode) {
    const suggested = /iPhone|iPad/i.test(navigator.userAgent)
      ? "iPhone"
      : /Android/i.test(navigator.userAgent) ? "Android phone" : "My phone";
    document.querySelector("#device-name").value = suggested;
  }
}

function nativePlatform() {
  return window.Capacitor?.getPlatform?.() || "web";
}

function parsePairingTarget(rawValue) {
  let value = String(rawValue || "").trim();
  if (value.startsWith("intent://")) {
    value = `lume://${value
      .slice("intent://".length)
      .split("#Intent;", 1)[0]}`;
  }

  try {
    const url = new URL(value);
    const gateway = url.protocol === "lume:"
      ? url.searchParams.get("gateway")
      : url.origin;
    const code = url.searchParams.get("code");
    if (!gateway?.startsWith("https://") || !code) return null;
    return { gateway: gateway.replace(/\/+$/, ""), code };
  } catch {
    return null;
  }
}

function pairingIntentUrl(target) {
  const query = new URLSearchParams({
    gateway: target.gateway,
    code: target.code,
  });
  return `intent://pair?${query.toString()}#Intent;scheme=lume;package=com.tulerws.lume.mobile;end`;
}

function prepareNativePairingLaunch(target) {
  const isAndroidBrowser = /Android/i.test(navigator.userAgent);
  const isNative = window.Capacitor?.isNativePlatform?.() || nativePlatform() !== "web";
  if (!isAndroidBrowser || isNative) return;

  const intentUrl = pairingIntentUrl(target);
  openLumeMobile.href = intentUrl;
  const attemptKey = `lume-pairing-launch-${target.code}`;
  if (sessionStorage.getItem(attemptKey)) return;
  sessionStorage.setItem(attemptKey, "1");
  setTimeout(() => {
    location.href = intentUrl;
  }, 120);
}

function applyPairingTarget(target) {
  apiBase = target.gateway;
  pairingCode = target.code;
  localStorage.setItem(baseKey, apiBase);
  pairMessage.textContent = "";
  pairMessage.className = "message";
  document.querySelector("#manual-gateway").value = apiBase;
  document.querySelector("#manual-code").value = pairingCode;
  showEntryView();
  prepareNativePairingLaunch(target);
}

function handlePairingUrl(url) {
  const target = parsePairingTarget(url);
  if (!target) return false;
  applyPairingTarget(target);
  return true;
}

async function initializePairingDeepLinks() {
  let handled = pairingCode && location.protocol === "https:"
    ? handlePairingUrl(location.href)
    : false;
  const appPlugin = window.Capacitor?.Plugins?.App;
  if (!appPlugin) return handled;

  await appPlugin.addListener("appUrlOpen", ({ url }) => {
    handlePairingUrl(url);
  });
  try {
    const launch = await appPlugin.getLaunchUrl();
    handled = handlePairingUrl(launch?.url) || handled;
  } catch {
    // The warm-start listener remains active even if no launch URL is available.
  }
  return handled;
}

function updateInstallOptions() {
  const isAndroidBrowser = /Android/i.test(navigator.userAgent);
  const isNative = window.Capacitor?.isNativePlatform?.() || nativePlatform() !== "web";
  scanPairingButton.hidden = !(isNative && nativePlatform() === "android");
  pairInstallPrompt.hidden = !pairingCode || !isAndroidBrowser || isNative;
  androidInstallCard.hidden =
    Boolean(pairingCode) || !isAndroidBrowser || isNative || Boolean(token);
  mobileApkDeviceDownload.hidden = isNative || !isAndroidBrowser;
  pwaInstallButton.hidden = !deferredInstallPrompt;
}

function statusLabel(status) {
  return {
    running: "Running",
    permission_required: "Permission required",
    waiting_for_input: "Waiting for action",
    completed: "Task completed",
    failed: "Error",
  }[status] || status;
}

function sourceLabel(session) {
  if (session.source === "web" && session.sourceApp) return session.sourceApp;
  return session.source || "agent";
}

function statusClass(status) {
  return {
    running: "running",
    permission_required: "attention",
    waiting_for_input: "waiting",
    completed: "completed",
    failed: "failed",
  }[status] || "waiting";
}

function filterSessions(sessions) {
  if (activeFilter === "running") {
    return sessions.filter((session) => session.status === "running");
  }
  if (activeFilter === "attention") {
    return sessions.filter((session) => session.status === "permission_required");
  }
  if (activeFilter === "completed") {
    return sessions.filter((session) => ["completed", "failed"].includes(session.status));
  }
  return sessions;
}

function agentVisual(session) {
  if (session.agent === "codex") {
    return '<svg viewBox="0 0 256 260" aria-hidden="true"><path d="M239.184 106.203a64.716 64.716 0 0 0-5.576-53.103C219.452 28.459 191 15.784 163.213 21.74A65.586 65.586 0 0 0 52.096 45.22a64.716 64.716 0 0 0-43.23 31.36c-14.31 24.602-11.061 55.634 8.033 76.74a64.665 64.665 0 0 0 5.525 53.102c14.174 24.65 42.644 37.324 70.446 31.36a64.72 64.72 0 0 0 48.754 21.744c28.481.025 53.714-18.361 62.414-45.481a64.767 64.767 0 0 0 43.229-31.36c14.137-24.558 10.875-55.423-8.083-76.483Zm-97.56 136.338a48.397 48.397 0 0 1-31.105-11.255l1.535-.87 51.67-29.825a8.595 8.595 0 0 0 4.247-7.367v-72.85l21.845 12.636c.218.111.37.32.409.563v60.367c-.056 26.818-21.783 48.545-48.601 48.601Zm-104.466-44.61a48.345 48.345 0 0 1-5.781-32.589l1.534.921 51.722 29.826a8.339 8.339 0 0 0 8.441 0l63.181-36.425v25.221a.87.87 0 0 1-.358.665l-52.335 30.184c-23.257 13.398-52.97 5.431-66.404-17.803ZM23.549 85.38a48.499 48.499 0 0 1 25.58-21.333v61.39a8.288 8.288 0 0 0 4.195 7.316l62.874 36.272-21.845 12.636a.819.819 0 0 1-.767 0L41.353 151.53c-23.211-13.454-31.171-43.144-17.804-66.405Zm179.466 41.695-63.08-36.63L161.73 77.86a.819.819 0 0 1 .768 0l52.233 30.184a48.6 48.6 0 0 1-7.316 87.635v-61.391a8.544 8.544 0 0 0-4.4-7.213Zm21.742-32.69-1.535-.922-51.619-30.081a8.39 8.39 0 0 0-8.492 0L99.98 99.808V74.587a.716.716 0 0 1 .307-.665l52.233-30.133a48.652 48.652 0 0 1 72.236 50.391ZM88.061 139.097l-21.845-12.585a.87.87 0 0 1-.41-.614V65.685a48.652 48.652 0 0 1 79.757-37.346l-1.535.87-51.67 29.825a8.595 8.595 0 0 0-4.246 7.367l-.051 72.697Zm11.868-25.58L128.067 97.3l28.188 16.218v32.434l-28.086 16.218-28.188-16.218Z"/></svg>';
  }
  if (session.agent === "claude") {
    return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m4.714 15.956 4.717-2.648.079-.23-.079-.128H9.2l-.789-.049-2.696-.073-2.337-.097-2.265-.121-.57-.122-.535-.704.055-.352.48-.322.686.061 5.517.358.055-.158L3.088 7.176l-.723-.492L2 6.223l-.158-1.008.656-.722.88.06.225.061 6.544 5.01.146-.103.018-.073-.164-.273-3.442-5.968-.17-.62-.103-.728.255-.862L6.7 0l.996.134.419.364.619 1.415 2.556 5.258.455.899.243.832.091.255h.158V9.01l.674-6.496.079-.759.376-.91.747-.492.583.279.48.686-.067.443-1.208 6.849h.212l.243-.243 4.025-4.789.85-.904.546-.431h1.032l.759 1.129-.34 1.166-4.055 5.499.073.109.188-.019 6.235-1.202.832.389.091.394-.328.808-7.71 1.761-.043.03.049.061 5.848.407.789.522.474.638-.079.486-1.214.619-6.776-1.627h-.182v.109l7.032 6.278.128.577-.322.455-.34-.049-6.29-5.026h-.128v.17l2.787 4.171.122 1.081-.17.352-.607.212-.668-.121-4.61-6.684-.14.079-.674 7.255-.315.37-.729.28-.607-.462-.322-.747.322-1.475 1.159-5.718-.012-.042-.14.018-5.336 6.758-.413.164-.716-.37.067-.662.4-.589 4.754-6.005-.006-.158h-.054l-6.339 4.116-1.129.146-.486-.456.061-.746.23-.243Z"/></svg>';
  }
  if (session.agent === "gemini") {
    return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81"/></svg>';
  }
  return `<span>${escapeHtml((session.agentLabel || "AI").slice(0, 2).toUpperCase())}</span>`;
}

function renderSessions(snapshot, trackChanges = true) {
  currentSnapshot = snapshot;
  const sessions = snapshot.sessions || [];
  if (trackChanges && hasRenderedSnapshot && localStorage.getItem(notificationsKey) === "on") {
    for (const session of sessions) {
      if (previousStatuses.get(session.id) !== session.status) notifySession(session);
    }
  }
  if (trackChanges) {
    previousStatuses = new Map(sessions.map((session) => [session.id, session.status]));
    hasRenderedSnapshot = true;
  }
  const active = sessions.filter((session) =>
    ["running", "permission_required", "waiting_for_input"].includes(session.status)
  ).length;
  const attention = sessions.filter((session) => session.status === "permission_required").length;
  document.querySelector("#active-count").textContent = String(active);
  document.querySelector("#attention-count").textContent = String(attention);
  document.querySelector("#updated-at").textContent =
    `Updated ${new Date(snapshot.generatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  renderResults(sessions);
  renderDevice();
  const visibleSessions = filterSessions(sessions);
  if (!sessions.length) {
    sessionList.innerHTML = '<div class="empty-list"><img src="/lume-mobile-icon.svg" alt=""><strong>No agents are open</strong><p>Start an agent on your computer. Lume will bring it here automatically.</p></div>';
    return;
  }
  if (!visibleSessions.length) {
    sessionList.innerHTML = '<div class="empty-list compact"><strong>Nothing in this view</strong><p>Try another filter.</p></div>';
    return;
  }
  sessionList.innerHTML = visibleSessions.map((session) => {
    const scopes = currentDevice?.scopes || [];
    const expanded = expandedSessions.has(session.id) || session.status === "permission_required";
    const response = session.lastResponse
      ? `<div class="response"><span class="content-label">Final response</span><p>${escapeHtml(session.lastResponse)}</p></div>`
      : "";
    const permission = session.pendingPermission
      ? `<div class="permission"><span class="content-label">Approval required</span><strong>${escapeHtml(session.pendingPermission.summary)}</strong><code>${escapeHtml(session.pendingPermission.resource)}</code>
          ${scopes.includes("approve") && session.capabilities?.canApprove ? `<span class="permission-actions">
            <button data-command="allow_once" data-session="${escapeHtml(session.id)}" data-permission="${escapeHtml(session.pendingPermission.id)}">Allow once</button>
            <button data-command="deny" data-session="${escapeHtml(session.id)}" data-permission="${escapeHtml(session.pendingPermission.id)}">Deny</button>
          </span>` : ""}</div>`
      : "";
    const files = [...new Set([
      ...(session.activities || []).flatMap((activity) => activity.files || []),
      ...(session.results || []).flatMap((result) => result.files || []),
    ])];
    const changes = files.length
      ? `<details class="changes"><summary><span>Changed files</span><em>${files.length}</em></summary><div>${files.map((file) => `<code>${escapeHtml(file)}</code>`).join("")}</div></details>`
      : "";
    const canPrompt = scopes.includes("prompt") && session.capabilities?.canPrompt &&
      ["completed", "failed", "waiting_for_input"].includes(session.status);
    const prompt = canPrompt
      ? `<form class="prompt-form" data-session="${escapeHtml(session.id)}"><textarea maxlength="16384" aria-label="Message ${escapeHtml(session.agentLabel)}" placeholder="Continue with a new prompt…" required></textarea><button type="submit" aria-label="Send prompt"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12 14-7-4 14-3-6-7-1z" /></svg></button></form>`
      : "";
    const stop = scopes.includes("terminate") && session.capabilities?.canTerminate
      ? `<button class="stop-agent" data-command="terminate" data-session="${escapeHtml(session.id)}">Stop agent</button>`
      : "";
    return `
      <article class="session tone-${statusClass(session.status)} ${expanded ? "expanded" : ""}">
        <button class="session-summary" data-expand="${escapeHtml(session.id)}" type="button" aria-expanded="${expanded}">
          <span class="agent-icon agent-${escapeHtml(session.agent)}">${agentVisual(session)}</span>
          <span class="session-heading"><strong>${escapeHtml(session.agentLabel)}</strong><small>${escapeHtml(session.project)}</small></span>
          <span class="source-badge">${escapeHtml(sourceLabel(session))}</span>
          <span class="status-badge"><i></i>${escapeHtml(statusLabel(session.status))}</span>
          <svg class="chevron" viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4" /></svg>
        </button>
        <div class="session-details" ${expanded ? "" : "hidden"}>
          ${permission}${response}${changes}${prompt}${stop}
        </div>
      </article>`;
  }).join("");
}

function renderResults(sessions) {
  const results = sessions
    .flatMap((session) => (session.results || []).map((result) => ({ session, result })))
    .sort((left, right) => right.result.createdAt - left.result.createdAt);
  if (!results.length) {
    resultsList.innerHTML = '<div class="empty-list"><img src="/lume-mobile-icon.svg" alt=""><strong>No results yet</strong><p>Finished responses and changed files will be collected here.</p></div>';
    return;
  }
  resultsList.innerHTML = results.map(({ session, result }) => `
    <article class="result-card">
      <header><span class="agent-icon agent-${escapeHtml(session.agent)}">${agentVisual(session)}</span><span><strong>${escapeHtml(session.agentLabel)}</strong><small>${escapeHtml(session.project)}</small></span><time>${new Date(result.createdAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time></header>
      <p>${escapeHtml(result.response || "Task completed.")}</p>
      ${(result.files || []).length ? `<div class="result-meta"><span>${result.files.length} changed file${result.files.length === 1 ? "" : "s"}</span></div>` : ""}
    </article>
  `).join("");
}

function renderDevice() {
  if (!currentDevice) return;
  document.querySelector("#paired-device-name").textContent = currentDevice.name || "This phone";
  document.querySelector("#device-connection").textContent =
    apiBase ? apiBase.replace(/^https?:\/\//, "") : location.host;
  document.querySelector("#device-scopes").innerHTML = (currentDevice.scopes || [])
    .map((scope) => `<span>${escapeHtml(scope)}</span>`)
    .join("");
}

async function notifySession(session) {
  const messages = {
    permission_required: `${session.agentLabel} needs a permission decision.`,
    completed: `${session.agentLabel} finished a task.`,
    failed: `${session.agentLabel} reported an error.`,
  };
  const body = messages[session.status];
  if (!body || Notification.permission !== "granted") return;
  const registration = await navigator.serviceWorker?.ready.catch(() => null);
  if (registration) {
    registration.showNotification("Lume", {
      body,
      tag: `lume-${session.id}-${session.status}`,
      icon: "/lume-mobile-icon.svg",
    });
  } else {
    new Notification("Lume", { body, tag: `lume-${session.id}-${session.status}` });
  }
}

function updateSecurityControls() {
  const biometric = document.querySelector("#biometric-button");
  const notifications = document.querySelector("#notification-button");
  biometric.querySelector("em").textContent =
    localStorage.getItem(credentialKey) ? "On" : "Off";
  notifications.querySelector("em").textContent =
    localStorage.getItem(notificationsKey) === "on" ? "On" : "Off";
  biometric.classList.toggle("active", Boolean(localStorage.getItem(credentialKey)));
  notifications.classList.toggle(
    "active",
    localStorage.getItem(notificationsKey) === "on",
  );
}

async function toggleBiometric() {
  if (localStorage.getItem(credentialKey)) {
    if (confirm("Turn off device verification for remote actions?")) {
      localStorage.removeItem(credentialKey);
      updateSecurityControls();
    }
    return;
  }
  if (!window.PublicKeyCredential) {
    dashboardMessage.textContent = "Device verification is not supported in this browser.";
    return;
  }
  try {
    const credential = await navigator.credentials.create({
      publicKey: {
        challenge: randomBytes(32),
        rp: { name: "Lume Local" },
        user: {
          id: randomBytes(16),
          name: "lume-mobile",
          displayName: currentDevice?.name || "Lume Mobile",
        },
        pubKeyCredParams: [{ type: "public-key", alg: -7 }, { type: "public-key", alg: -257 }],
        authenticatorSelection: {
          authenticatorAttachment: "platform",
          residentKey: "preferred",
          userVerification: "required",
        },
        timeout: 60000,
        attestation: "none",
      },
    });
    localStorage.setItem(credentialKey, encodeBytes(credential.rawId));
    dashboardMessage.textContent = "Device verification enabled for remote actions.";
    updateSecurityControls();
  } catch (error) {
    dashboardMessage.textContent = error.name === "NotAllowedError"
      ? "Device verification was cancelled."
      : "Could not enable device verification.";
  }
}

async function verifySensitiveAction() {
  const encodedId = localStorage.getItem(credentialKey);
  if (!encodedId) return true;
  try {
    await navigator.credentials.get({
      publicKey: {
        challenge: randomBytes(32),
        allowCredentials: [{
          type: "public-key",
          id: decodeBytes(encodedId),
          transports: ["internal"],
        }],
        userVerification: "required",
        timeout: 60000,
      },
    });
    return true;
  } catch {
    dashboardMessage.textContent = "Device verification was not completed.";
    return false;
  }
}

async function toggleNotifications() {
  if (localStorage.getItem(notificationsKey) === "on") {
    localStorage.removeItem(notificationsKey);
    updateSecurityControls();
    return;
  }
  if (!("Notification" in window)) {
    dashboardMessage.textContent = "Notifications are not supported in this browser.";
    return;
  }
  const permission = await Notification.requestPermission();
  if (permission === "granted") {
    localStorage.setItem(notificationsKey, "on");
    dashboardMessage.textContent = "Notifications enabled while Lume Mobile is active.";
  } else {
    dashboardMessage.textContent = "Notification permission was not granted.";
  }
  updateSecurityControls();
}

function compareVersions(left, right) {
  const normalize = (value) => String(value || "")
    .split("-")[0]
    .split(".")
    .map((part) => Number.parseInt(part, 10) || 0);
  const leftParts = normalize(left);
  const rightParts = normalize(right);
  const length = Math.max(leftParts.length, rightParts.length, 4);
  for (let index = 0; index < length; index += 1) {
    const difference = (leftParts[index] || 0) - (rightParts[index] || 0);
    if (difference) return difference;
  }
  return 0;
}

function nativeUpdater() {
  if (nativePlatform() !== "android") return null;
  return window.Capacitor?.Plugins?.LumeUpdater || null;
}

function setMobileUpdateBusy(busy) {
  mobileUpdateBusy = busy;
  mobileUpdateCard.classList.toggle("busy", busy);
  mobileUpdateButton.disabled = busy;
  mobileUpdateProgress.hidden = !busy;
}

function showCurrentMobileVersion() {
  if (installedMobileInfo?.version) {
    mobileVersionLabel.textContent = `Installed version ${installedMobileInfo.version}`;
  }
}

async function checkMobileUpdate({ manual = false } = {}) {
  if (mobileUpdateBusy) return;
  const platform = nativePlatform();
  if (platform === "web") {
    setMobileUpdateBusy(true);
    mobileUpdateDetail.textContent = "Checking the installed web app…";
    try {
      const registration = await navigator.serviceWorker?.ready;
      await registration?.update();
      mobileUpdateDetail.textContent = "The web app updates automatically.";
      mobileVersionLabel.textContent = "Web app";
    } catch {
      mobileUpdateDetail.textContent = "Could not check the web app right now.";
    } finally {
      setMobileUpdateBusy(false);
    }
    return;
  }
  if (platform === "ios") {
    mobileUpdateDetail.textContent = "Updates are managed securely by TestFlight or the App Store.";
    mobileVersionLabel.textContent = "iOS application";
    mobileUpdateButton.hidden = true;
    return;
  }

  const updater = nativeUpdater();
  if (!updater) {
    mobileUpdateDetail.textContent = "The native updater is unavailable in this build.";
    mobileUpdateButton.hidden = true;
    return;
  }

  setMobileUpdateBusy(true);
  mobileUpdateDetail.textContent = "Checking for a new version…";
  try {
    installedMobileInfo = await updater.getInfo();
    showCurrentMobileVersion();
    const response = await fetch(mobileManifestUrl, { cache: "no-store" });
    if (!response.ok) throw new Error(`Update manifest returned ${response.status}`);
    const manifest = await response.json();
    if (
      !manifest.version ||
      !manifest.android?.url ||
      !/^[a-f0-9]{64}$/i.test(manifest.android?.sha256 || "")
    ) {
      throw new Error("Invalid mobile update manifest");
    }
    localStorage.setItem(mobileUpdateCheckKey, String(Date.now()));
    if (compareVersions(manifest.version, installedMobileInfo.version) > 0) {
      availableMobileUpdate = manifest;
      mobileUpdateCard.classList.add("available");
      mobileUpdateDetail.textContent = `Version ${manifest.version} is ready to install.`;
      mobileUpdateButton.textContent = "Install";
    } else {
      availableMobileUpdate = undefined;
      mobileUpdateCard.classList.remove("available");
      mobileUpdateDetail.textContent = "You are using the latest version.";
      mobileUpdateButton.textContent = "Check now";
    }
  } catch {
    mobileUpdateDetail.textContent = manual
      ? "Could not check for updates right now."
      : "Automatic update checks will retry later.";
  } finally {
    setMobileUpdateBusy(false);
  }
}

async function installMobileUpdate() {
  const updater = nativeUpdater();
  if (!updater || !availableMobileUpdate || mobileUpdateBusy) return;
  setMobileUpdateBusy(true);
  mobileUpdateDetail.textContent = "Downloading and verifying the update…";
  mobileUpdateButton.textContent = "Installing…";
  try {
    await updater.installUpdate({
      url: availableMobileUpdate.android.url,
      sha256: availableMobileUpdate.android.sha256,
    });
    mobileUpdateDetail.textContent = "Complete the update in the Android installer.";
  } catch (error) {
    mobileUpdateDetail.textContent = error.code === "INSTALL_PERMISSION_REQUIRED"
      ? "Allow Lume to install updates, then return and tap Install again."
      : (error.message || "The update could not be installed.");
  } finally {
    mobileUpdateButton.textContent = "Install";
    setMobileUpdateBusy(false);
  }
}

async function initializeMobileUpdates() {
  const platform = nativePlatform();
  if (platform === "android" && nativeUpdater()) {
    try {
      installedMobileInfo = await nativeUpdater().getInfo();
      showCurrentMobileVersion();
    } catch {
      mobileVersionLabel.textContent = "Android application";
    }
  }
  const lastCheck = Number(localStorage.getItem(mobileUpdateCheckKey) || 0);
  if (Date.now() - lastCheck >= mobileUpdateInterval) {
    await checkMobileUpdate();
  } else if (platform === "web") {
    mobileVersionLabel.textContent = "Web app";
    mobileUpdateDetail.textContent = "The web app updates automatically.";
  }
  setInterval(() => void checkMobileUpdate(), mobileUpdateInterval);
}

async function refreshSnapshot() {
  try {
    const [snapshot, device] = await Promise.all([
      api("/api/v1/snapshot"),
      api("/api/v1/me"),
    ]);
    currentDevice = device;
    renderSessions(snapshot);
    connectionDot.className = "online";
    connectionLabel.textContent = "Connected";
    dashboardMessage.textContent = "";
  } catch (error) {
    connectionDot.className = "offline";
    connectionLabel.textContent = "Offline";
    dashboardMessage.textContent = error.message;
  }
}

async function executeCommand(command) {
  const requestId = crypto.randomUUID?.() || `${Date.now()}-${Math.random()}`;
  const response = await api("/api/v1/commands", {
    method: "POST",
    body: JSON.stringify({ requestId, ...command }),
  });
  if (!response.ok) throw new Error(response.error?.message || "The command failed");
  await refreshSnapshot();
}

async function pollEvents() {
  if (!token) return;
  try {
    const payload = await api(`/api/v1/events?since=${lastSequence}`);
    const events = payload.events || [];
    if (events.length) {
      lastSequence = Math.max(...events.map((event) => event.sequence || 0), lastSequence);
      await refreshSnapshot();
    }
  } catch (error) {
    connectionDot.className = "offline";
    dashboardMessage.textContent = error.message;
  } finally {
    pollTimer = setTimeout(pollEvents, 1400);
  }
}

async function showDashboard() {
  pairView.hidden = true;
  emptyAuthView.hidden = true;
  appContent.hidden = false;
  loadingView.hidden = false;
  updateInstallOptions();
  document.querySelector("#refresh-button").hidden = false;
  updateSecurityControls();
  await refreshSnapshot();
  loadingView.hidden = true;
  pollEvents();
}

function setView(view) {
  const screens = {
    sessions: dashboard,
    results: document.querySelector("#results-screen"),
    device: document.querySelector("#device-screen"),
  };
  for (const [name, screen] of Object.entries(screens)) {
    screen.hidden = name !== view;
  }
  document.querySelectorAll(".bottom-nav button").forEach((button) => {
    button.classList.toggle("active", button.dataset.view === view);
  });
  window.scrollTo({ top: 0, behavior: "smooth" });
}

pairForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = pairForm.querySelector("button");
  button.disabled = true;
  pairMessage.textContent = "Pairing…";
  try {
    const credentials = await api("/api/v1/pair", {
      method: "POST",
      body: JSON.stringify({
        code: pairingCode,
        deviceName: document.querySelector("#device-name").value.trim(),
      }),
    });
    token = credentials.token;
    localStorage.setItem(tokenKey, token);
    history.replaceState({}, "", "/");
    await showDashboard();
  } catch (error) {
    pairMessage.textContent = error.message;
    pairMessage.className = "message error";
  } finally {
    button.disabled = false;
  }
});

document.querySelector("#refresh-button").addEventListener("click", refreshSnapshot);
document.querySelectorAll(".bottom-nav button").forEach((button) => {
  button.addEventListener("click", () => setView(button.dataset.view));
});
document.querySelectorAll(".filter-bar button").forEach((button) => {
  button.addEventListener("click", () => {
    activeFilter = button.dataset.filter;
    document.querySelectorAll(".filter-bar button").forEach((item) => {
      item.classList.toggle("active", item === button);
    });
    if (currentSnapshot) renderSessions(currentSnapshot, false);
  });
});
scanPairingButton.addEventListener("click", async () => {
  const scanner = window.Capacitor?.Plugins?.CapacitorBarcodeScanner;
  const message = document.querySelector("#manual-pair-form .message");
  if (!scanner) {
    message.textContent = "QR scanner is unavailable. Restart the app and try again.";
    message.className = "message error";
    return;
  }

  scanPairingButton.disabled = true;
  message.textContent = "";
  message.className = "message";
  try {
    const result = await scanner.scanBarcode({
      hint: 0,
      scanInstructions: "Scan the QR code shown in Lume Desktop",
      scanButton: false,
      cameraDirection: 1,
      scanOrientation: 3,
      cancelButtonAccessibilityLabel: "Cancel",
      android: { scanningLibrary: "mlkit" },
    });
    if (!handlePairingUrl(result.ScanResult)) {
      throw new Error("This is not a valid Lume pairing QR code.");
    }
  } catch (error) {
    const detail = String(error?.message || error);
    message.textContent = /cancel/i.test(detail) ? "Scan cancelled." : detail;
    message.className = /cancel/i.test(detail) ? "message" : "message error";
  } finally {
    scanPairingButton.disabled = false;
  }
});
document.querySelector("#manual-pair-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const gateway = document.querySelector("#manual-gateway").value.trim().replace(/\/+$/, "");
  const code = document.querySelector("#manual-code").value.trim();
  if (!gateway.startsWith("https://") || !code) return;
  const button = event.currentTarget.querySelector("button");
  const message = event.currentTarget.querySelector(".message");
  button.disabled = true;
  message.textContent = "Pairing…";
  try {
    const response = await fetch(`${gateway}/api/v1/pair`, {
      method: "POST",
      cache: "no-store",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        code,
        deviceName: document.querySelector("#manual-device-name").value.trim(),
      }),
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error?.message || "Pairing failed");
    apiBase = gateway;
    token = body.token;
    localStorage.setItem(baseKey, apiBase);
    localStorage.setItem(tokenKey, token);
    await showDashboard();
  } catch (error) {
    message.textContent = error.message;
    message.className = "message error";
  } finally {
    button.disabled = false;
  }
});
sessionList.addEventListener("submit", async (event) => {
  const form = event.target.closest(".prompt-form");
  if (!form) return;
  event.preventDefault();
  if (!(await verifySensitiveAction())) return;
  const textarea = form.querySelector("textarea");
  const button = form.querySelector("button");
  button.disabled = true;
  try {
    await executeCommand({
      type: "submit_prompt",
      sessionId: form.dataset.session,
      prompt: textarea.value,
    });
    textarea.value = "";
  } catch (error) {
    dashboardMessage.textContent = error.message;
  } finally {
    button.disabled = false;
  }
});
sessionList.addEventListener("click", async (event) => {
  const summary = event.target.closest("button[data-expand]");
  if (summary) {
    const sessionId = summary.dataset.expand;
    if (expandedSessions.has(sessionId)) expandedSessions.delete(sessionId);
    else expandedSessions.add(sessionId);
    if (currentSnapshot) renderSessions(currentSnapshot, false);
    return;
  }
  const button = event.target.closest("button[data-command]");
  if (!button) return;
  const command = button.dataset.command;
  if (command === "terminate" && !confirm("Stop this agent and its commands?")) return;
  if (!(await verifySensitiveAction())) return;
  button.disabled = true;
  try {
    if (command === "terminate") {
      await executeCommand({ type: "terminate_session", sessionId: button.dataset.session });
    } else {
      await executeCommand({
        type: "resolve_permission",
        sessionId: button.dataset.session,
        permissionId: button.dataset.permission,
        action: command,
      });
    }
  } catch (error) {
    dashboardMessage.textContent = error.message;
  } finally {
    button.disabled = false;
  }
});
document.querySelector("#biometric-button").addEventListener("click", toggleBiometric);
document.querySelector("#notification-button").addEventListener("click", toggleNotifications);
mobileUpdateButton.addEventListener("click", () => {
  if (availableMobileUpdate) void installMobileUpdate();
  else void checkMobileUpdate({ manual: true });
});
pwaInstallButton.addEventListener("click", async () => {
  if (!deferredInstallPrompt) return;
  deferredInstallPrompt.prompt();
  await deferredInstallPrompt.userChoice;
  deferredInstallPrompt = undefined;
  updateInstallOptions();
});
window.addEventListener("beforeinstallprompt", (event) => {
  event.preventDefault();
  deferredInstallPrompt = event;
  updateInstallOptions();
});
window.addEventListener("appinstalled", () => {
  deferredInstallPrompt = undefined;
  updateInstallOptions();
});
document.querySelector("#disconnect-button").addEventListener("click", () => {
  if (!confirm("Disconnect this phone from Lume?")) return;
  localStorage.removeItem(tokenKey);
  localStorage.removeItem(baseKey);
  token = null;
  apiBase = "";
  currentDevice = undefined;
  showEntryView();
});
if ("serviceWorker" in navigator) {
  let refreshing = false;
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (refreshing || !navigator.serviceWorker.controller) return;
    refreshing = true;
    location.reload();
  });
  navigator.serviceWorker.register("/sw.js")
    .then((registration) => registration.update())
    .catch(() => undefined);
}
async function startApp() {
  const pairingHandled = await initializePairingDeepLinks();
  void initializeMobileUpdates();
  if (pairingHandled) return;
  if (token) await showDashboard();
  else showEntryView();
}

void startApp();
