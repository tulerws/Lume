import { renderSafeMarkdown } from "./markdown.js";

const tokenKey = "lume-mobile-token-v1";
const baseKey = "lume-mobile-gateway-v1";
const deviceKey = "lume-mobile-device-v1";
const desktopKey = "lume-mobile-desktop-v1";
const credentialKey = "lume-mobile-biometric-v1";
const notificationsKey = "lume-mobile-notifications-v1";
const backgroundMonitoringKey = "lume-mobile-background-v1";
const mobileUpdateCheckKey = "lume-mobile-update-check-v1";
const mobileUpdateInterval = 6 * 60 * 60 * 1000;
const params = new URLSearchParams(location.hash.slice(1) || location.search);
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
const chatScreen = document.querySelector("#chat-screen");
const chatFeed = document.querySelector("#mobile-chat-feed");
const chatComposer = document.querySelector("#mobile-chat-composer");
const chatAgentIcon = document.querySelector("#chat-agent-icon");
const chatAgentName = document.querySelector("#chat-agent-name");
const chatAgentStatus = document.querySelector("#chat-agent-status");
const chatRateLimit = document.querySelector("#chat-rate-limit");
const chatWorkTray = document.querySelector("#chat-work-tray");
const chatStopButton = document.querySelector("#chat-stop");
const dashboardMessage = document.querySelector("#dashboard-message");
const connectionDot = document.querySelector("#connection-dot");
const connectionLabel = document.querySelector("#connection-label");
const headerMascot = document.querySelector("#header-mascot");
const androidInstallCard = document.querySelector("#android-install-card");
const pairInstallPrompt = document.querySelector("#pair-install-prompt");
const closePairInstallPrompt = document.querySelector("#close-pair-install");
const openLumeMobile = document.querySelector("#open-lume-mobile");
const openInstalledMobile = document.querySelector("#open-installed-mobile");
const mobileApkDeviceDownload = document.querySelector("#mobile-apk-device-download");
const pwaInstallButton = document.querySelector("#pwa-install-button");
const mobileUpdateCard = document.querySelector("#mobile-update-card");
const mobileUpdateButton = document.querySelector("#mobile-update-button");
const mobileUpdateDetail = document.querySelector("#mobile-update-detail");
const mobileVersionLabel = document.querySelector("#mobile-version-label");
const mobileUpdateProgress = document.querySelector("#mobile-update-progress");
let token = localStorage.getItem(tokenKey);
let deviceId = localStorage.getItem(deviceKey);
let apiBase = localStorage.getItem(baseKey) || "";
let desktopId = localStorage.getItem(desktopKey);
let nativeCredentialsAvailable = false;
let transportKeyPromise;
let pollTimer;
let lastSequence = 0;
let nativeRealtimeConnected = false;
let nativeRealtimeListenersReady = false;
let currentDevice;
let previousStatuses = new Map();
let previousPermissionIds = new Map();
let previousQuestionIds = new Map();
let hasRenderedSnapshot = false;
let currentSnapshot;
let activeFilter = "all";
let deferredInstallPrompt;
let installedMobileInfo;
let availableMobileUpdate;
let mobileUpdateBusy = false;
let mobileUpdatesReady = false;
let companionUpdateBusy = false;
const companionUpdateAttempts = new Map();
let lastMobileVersionReport = { key: "", at: 0 };
let automaticPairingTimer;
let pairInstallPromptDismissed = false;
let bannerTimer;
let openUpdateViewRequested = false;
let activeChatSessionId;
let lastChatRenderKey = "";
let mascotTransitionTimer;
const expandedResults = new Set();
const submittingPromptSessions = new Set();
const promptDrafts = new Map();
const promptAttachments = new Map();
const questionSelections = new Map();
const rateLimitRefreshes = new Map();
const sendIconMarkup = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12 14-7-4 14-3-6-7-1z" /></svg>';
const attachIconMarkup = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9 12 6-6a3 3 0 0 1 4 4l-8 8a5 5 0 1 1-7-7l8-8" /></svg>';
const sendSpinnerMarkup = '<span class="prompt-send-spinner" aria-hidden="true"></span>';

const escapeHtml = (value = "") =>
  String(value).replace(/[&<>"']/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#039;",
  })[character]);

function cleanFilePath(value, workingDirectory) {
  let path = String(value || "").trim().replace(/^["']|["']$/g, "");
  if (!path || path === "/dev/null") return null;
  if (path.startsWith("a/") || path.startsWith("b/")) path = path.slice(2);
  const root = String(workingDirectory || "").replace(/[\\/]+$/, "");
  if (root) {
    const normalizedPath = path.replace(/\\/g, "/");
    const normalizedRoot = root.replace(/\\/g, "/");
    const windowsPath = /^[a-z]:\//i.test(normalizedRoot);
    const comparedPath = windowsPath ? normalizedPath.toLowerCase() : normalizedPath;
    const comparedRoot = windowsPath ? normalizedRoot.toLowerCase() : normalizedRoot;
    if (comparedPath === comparedRoot || comparedPath.startsWith(`${comparedRoot}/`)) {
      path = normalizedPath.slice(normalizedRoot.length).replace(/^\/+/, "");
    }
  }
  return path || null;
}

function mergeFileChange(summaries, path, added = 0, removed = 0) {
  if (!path) return;
  const current = summaries.get(path);
  if (current) {
    current.added = Math.max(current.added, added);
    current.removed = Math.max(current.removed, removed);
  } else {
    summaries.set(path, { path, added, removed });
  }
}

function summarizeFileChanges(detail = "", reportedFiles = [], workingDirectory) {
  const summaries = new Map();
  let currentPath = null;
  let added = 0;
  let removed = 0;
  let counting = false;
  const flush = () => {
    mergeFileChange(summaries, currentPath, added, removed);
    added = 0;
    removed = 0;
  };
  for (const line of String(detail || "").split(/\r?\n/)) {
    const patchHeader = line.match(/^\*\*\* (?:Update|Add|Delete) File:\s+(.+)$/);
    const gitHeader = line.match(/^diff --git a\/(.+?) b\/(.+)$/);
    const nextFile = line.match(/^\+\+\+\s+(?:b\/)?(.+)$/);
    if (patchHeader || gitHeader || nextFile) {
      flush();
      currentPath = cleanFilePath(
        patchHeader?.[1] ?? gitHeader?.[2] ?? nextFile?.[1] ?? "",
        workingDirectory,
      );
      counting = Boolean(patchHeader);
      continue;
    }
    if (line.startsWith("@@")) {
      counting = true;
      continue;
    }
    if (line === "*** End Patch") {
      flush();
      currentPath = null;
      counting = false;
      continue;
    }
    if (!currentPath || !counting) continue;
    if (line.startsWith("+") && !line.startsWith("+++")) added += 1;
    if (line.startsWith("-") && !line.startsWith("---")) removed += 1;
  }
  flush();
  for (const reported of reportedFiles || []) {
    if (String(reported).includes("\n") || String(reported).includes("*** Begin Patch")) {
      for (const change of summarizeFileChanges(reported, [], workingDirectory)) {
        mergeFileChange(summaries, change.path, change.added, change.removed);
      }
    } else {
      mergeFileChange(summaries, cleanFilePath(reported, workingDirectory));
    }
  }
  return [...summaries.values()];
}

function mergeFileChanges(target, incoming) {
  for (const change of incoming) {
    const current = target.find((item) => item.path === change.path);
    if (current) {
      current.added = Math.max(current.added, change.added);
      current.removed = Math.max(current.removed, change.removed);
    } else {
      target.push({ ...change });
    }
  }
}

const randomBytes = (length) => crypto.getRandomValues(new Uint8Array(length));
const encodeBytes = (value) =>
  btoa(String.fromCharCode(...new Uint8Array(value)))
    .replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
const decodeBytes = (value) => {
  const base64 = value.replaceAll("-", "+").replaceAll("_", "/");
  const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
  return Uint8Array.from(atob(padded), (character) => character.charCodeAt(0));
};

function showBanner(message, kind = "info", source = "general") {
  clearTimeout(bannerTimer);
  dashboardMessage.textContent = String(message || "Something went wrong.");
  dashboardMessage.dataset.kind = kind;
  dashboardMessage.dataset.source = source;
  dashboardMessage.setAttribute("role", kind === "error" ? "alert" : "status");
  dashboardMessage.setAttribute("aria-live", kind === "error" ? "assertive" : "polite");
  bannerTimer = setTimeout(() => {
    if (dashboardMessage.dataset.source === source) dashboardMessage.textContent = "";
  }, kind === "error" ? 7000 : 4500);
}

function hideBanner(source) {
  if (source && dashboardMessage.dataset.source !== source) return;
  clearTimeout(bannerTimer);
  dashboardMessage.textContent = "";
}

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

async function keyFromSecret(secret) {
  const digest = await crypto.subtle.digest("SHA-256", textEncoder.encode(secret));
  return crypto.subtle.importKey("raw", digest, "AES-GCM", false, ["encrypt", "decrypt"]);
}

async function encryptPayload(key, payload, aad) {
  const nonce = randomBytes(12);
  const ciphertext = await crypto.subtle.encrypt(
    {
      name: "AES-GCM",
      iv: nonce,
      additionalData: textEncoder.encode(aad),
    },
    key,
    textEncoder.encode(JSON.stringify(payload)),
  );
  return { nonce: encodeBytes(nonce), ciphertext: encodeBytes(ciphertext) };
}

async function decryptPayload(key, envelope, aad) {
  const plaintext = await crypto.subtle.decrypt(
    {
      name: "AES-GCM",
      iv: decodeBytes(envelope.nonce),
      additionalData: textEncoder.encode(aad),
    },
    key,
    decodeBytes(envelope.ciphertext),
  );
  return JSON.parse(textDecoder.decode(plaintext));
}

function localFetch(url, options) {
  return fetch(url, { ...options, targetAddressSpace: "local" });
}

function clearMobileCredentials() {
  const realtime = nativeRealtimePlugin();
  const nativeClear = realtime
    ? realtime.clearCredentials().catch(() => undefined)
    : Promise.resolve();
  localStorage.removeItem(tokenKey);
  localStorage.removeItem(deviceKey);
  localStorage.removeItem(desktopKey);
  token = null;
  deviceId = null;
  desktopId = null;
  nativeCredentialsAvailable = false;
  nativeRealtimeConnected = false;
  transportKeyPromise = undefined;
  return nativeClear;
}

async function securePair(options) {
  const key = await keyFromSecret(pairingCode);
  const input = JSON.parse(options.body || "{}");
  const envelope = await encryptPayload(
    key,
    { deviceName: input.deviceName, timestamp: Date.now() },
    "lume-pair-request-v1",
  );
  const response = await localFetch(`${apiBase}/api/v1/pair-secure`, {
    method: "POST",
    cache: "no-store",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(envelope),
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(body.error?.message || `Lume returned ${response.status}`);
  }
  return decryptPayload(key, body, "lume-pair-response-v1");
}

async function secureApi(path, options = {}) {
  if (!token || !deviceId) {
    clearMobileCredentials();
    showEntryView();
    throw new Error("Pair this phone with Lume again.");
  }
  transportKeyPromise ||= keyFromSecret(token);
  const key = await transportKeyPromise;
  const method = String(options.method || "GET").toUpperCase();
  const requestNonce = encodeBytes(randomBytes(16));
  let requestBody = null;
  if (options.body) {
    try {
      requestBody = JSON.parse(options.body);
    } catch {
      requestBody = options.body;
    }
  }
  const envelope = await encryptPayload(
    key,
    {
      method,
      path,
      body: requestBody,
      timestamp: Date.now(),
      requestNonce,
    },
    "lume-secure-request-v1",
  );
  const response = await localFetch(`${apiBase}/api/v1/secure`, {
    method: "POST",
    cache: "no-store",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ deviceId, ...envelope }),
  });
  const outerBody = await response.json().catch(() => ({}));
  if (!response.ok) {
    if (response.status === 401) {
      clearMobileCredentials();
      showEntryView();
    }
    throw new Error(outerBody.error?.message || `Lume returned ${response.status}`);
  }
  const secureResponse = await decryptPayload(
    key,
    outerBody,
    `lume-secure-response-v1:${requestNonce}`,
  );
  if (secureResponse.status < 200 || secureResponse.status >= 300) {
    if (secureResponse.status === 401) {
      clearMobileCredentials();
      showEntryView();
    }
    throw new Error(
      secureResponse.body?.error?.message || `Lume returned ${secureResponse.status}`,
    );
  }
  return secureResponse.body;
}

async function api(path, options = {}) {
  const realtime = nativeCredentialsAvailable ? nativeRealtimePlugin() : null;
  if (realtime && path !== "/api/v1/pair") {
    let body = {};
    if (options.body) {
      try {
        body = JSON.parse(options.body);
      } catch {
        body = {};
      }
    }
    return realtime.request({
      method: String(options.method || "GET").toUpperCase(),
      path,
      body,
    });
  }
  if (apiBase.startsWith("http://")) {
    return path === "/api/v1/pair" ? securePair(options) : secureApi(path, options);
  }
  const headers = { "Content-Type": "application/json", ...(options.headers || {}) };
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await fetch(`${apiBase}${path}`, { ...options, headers, cache: "no-store" });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    if (response.status === 401 && path !== "/api/v1/pair") {
      clearMobileCredentials();
      showEntryView();
    }
    throw new Error(body.error?.message || `Lume returned ${response.status}`);
  }
  return body;
}

function pairingFailureMessage(error) {
  const detail = String(error?.message || error);
  if (/failed to fetch|networkerror|load failed/i.test(detail)) {
    if (nativePlatform() === "ios") {
      return "Could not reach Lume. In iPhone Settings, allow Lume to access the local network and confirm that both devices use the same Wi-Fi.";
    }
    return "Could not reach Lume. Allow local network access and confirm that both devices use the same Wi-Fi.";
  }
  return /não está ativo|expirou|not active|expired/i.test(detail)
    ? "Pairing code expired — generate a new QR code in Lume Desktop."
    : detail;
}

function showEntryView() {
  clearTimeout(pollTimer);
  appContent.hidden = true;
  loadingView.hidden = true;
  pairView.hidden = !pairingCode;
  emptyAuthView.hidden = Boolean(pairingCode);
  connectionDot.className = "";
  connectionLabel.textContent = "Not paired";
  setHeaderMascotState("sleeping");
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

function nativeRealtimePlugin() {
  return nativePlatform() === "android"
    ? window.Capacitor?.Plugins?.LumeNative || null
    : null;
}

function hasMobileSession() {
  return Boolean(token || nativeCredentialsAvailable);
}

async function restoreNativeCredentials() {
  const realtime = nativeRealtimePlugin();
  if (!realtime || token) return false;
  try {
    const stored = await realtime.getStoredConnection();
    if (!stored.paired) return false;
    apiBase = stored.gateway;
    deviceId = stored.deviceId;
    desktopId = stored.desktopId || null;
    nativeCredentialsAvailable = true;
    localStorage.setItem(baseKey, apiBase);
    localStorage.setItem(deviceKey, deviceId);
    if (desktopId) localStorage.setItem(desktopKey, desktopId);
    return true;
  } catch {
    return false;
  }
}

function applyRealtimeSnapshot(snapshot, sequence, cached = false) {
  if (!snapshot?.sessions) return;
  if (Number.isFinite(Number(sequence))) {
    lastSequence = Math.max(lastSequence, Number(sequence));
  }
  renderSessions(snapshot);
  if (cached) {
    connectionDot.className = "offline";
    connectionLabel.textContent = "Cached";
    setHeaderMascotState("sleeping");
    showBanner("Showing the last state saved on this phone.", "info", "connection");
  } else {
    connectionDot.className = "online";
    connectionLabel.textContent = "Connected";
    hideBanner("connection");
  }
}

function applyRealtimeConnectionState(state = {}) {
  const connected = state.status === "connected";
  const changed = nativeRealtimeConnected !== connected;
  nativeRealtimeConnected = connected;
  if (connected) {
    connectionDot.className = "online";
    connectionLabel.textContent = "Connected";
    hideBanner("connection");
    setHeaderMascotState(mascotStateForSessions(currentSnapshot?.sessions || []));
  } else if (state.status === "reconnecting") {
    connectionDot.className = "offline";
    connectionLabel.textContent = "Reconnecting";
    setHeaderMascotState("sleeping");
  } else {
    setHeaderMascotState("sleeping");
  }
  if (changed && hasMobileSession()) {
    clearTimeout(pollTimer);
    pollTimer = setTimeout(pollEvents, connected ? 10_000 : 250);
  }
}

async function initializeNativeRealtime() {
  const realtime = nativeRealtimePlugin();
  if (!realtime || !deviceId || !apiBase || (!token && !nativeCredentialsAvailable)) return false;
  if (!nativeRealtimeListenersReady) {
    await realtime.addListener("connectionChanged", applyRealtimeConnectionState);
    await realtime.addListener("sessionSnapshot", ({ snapshot, sequence, cached }) => {
      applyRealtimeSnapshot(snapshot, sequence, cached);
    });
    await realtime.addListener("sessionDelta", ({ snapshot, events = [], cached }) => {
      for (const event of events || []) {
        lastSequence = Math.max(lastSequence, Number(event.sequence || 0));
      }
      applyRealtimeSnapshot(snapshot, undefined, cached);
    });
    await realtime.addListener("streamError", ({ code, message, retryable }) => {
      if (code === "authentication_failed") {
        nativeCredentialsAvailable = false;
        nativeRealtimeConnected = false;
        localStorage.removeItem(deviceKey);
        localStorage.removeItem(desktopKey);
        deviceId = null;
        desktopId = null;
        showEntryView();
      }
      if (code === "background_service_unavailable") {
        void realtime.getStatus()
          .then(applyNativeMonitoringStatus)
          .catch(() => undefined);
        showBanner(
          message || "Android could not start background monitoring.",
          "error",
          "connection",
        );
      } else if (!retryable) {
        showBanner(message || "The live connection failed.", "error", "connection");
      }
    });
    nativeRealtimeListenersReady = true;
  }
  try {
    const state = token
      ? await realtime.connect({ gateway: apiBase, desktopId, deviceId, token })
      : await realtime.connect();
    nativeCredentialsAvailable = Boolean(state.credentialsStored || nativeCredentialsAvailable);
    if (nativeCredentialsAvailable && token) {
      localStorage.removeItem(tokenKey);
      token = null;
      transportKeyPromise = undefined;
    }
    applyRealtimeConnectionState(state);
    return true;
  } catch {
    nativeRealtimeConnected = false;
    return false;
  }
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
    const fragmentParams = new URLSearchParams(url.hash.slice(1));
    const gateway = url.searchParams.get("gateway")
      || fragmentParams.get("gateway")
      || (url.protocol === "lume:" ? null : url.origin);
    const code = url.searchParams.get("code") || fragmentParams.get("code");
    const importedToken = url.searchParams.get("token") || fragmentParams.get("token");
    const importedDeviceId = url.searchParams.get("deviceId") || fragmentParams.get("deviceId");
    const importedDesktopId = url.searchParams.get("desktopId")
      || fragmentParams.get("desktopId");
    if (
      !/^https?:\/\//.test(gateway || "")
      || (!code && !(importedToken && importedDeviceId))
    ) return null;
    return {
      gateway: gateway.replace(/\/+$/, ""),
      code,
      token: importedToken,
      deviceId: importedDeviceId,
      desktopId: importedDesktopId,
    };
  } catch {
    return null;
  }
}

function pairingIntentUrl(target) {
  const query = new URLSearchParams({ gateway: target.gateway });
  if (target.code) query.set("code", target.code);
  if (target.token && target.deviceId) {
    query.set("token", target.token);
    query.set("deviceId", target.deviceId);
  }
  if (target.desktopId) query.set("desktopId", target.desktopId);
  const fallback = encodeURIComponent(location.href);
  return `intent://pair?${query.toString()}#Intent;scheme=lume;package=com.tulerws.lume.mobile;S.browser_fallback_url=${fallback};end`;
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

function scheduleAutomaticPairing() {
  clearTimeout(automaticPairingTimer);
  automaticPairingTimer = setTimeout(() => {
    if (!pairingCode || hasMobileSession() || document.hidden || pairForm.querySelector("button").disabled) return;
    pairForm.requestSubmit();
  }, nativePlatform() === "web" ? 900 : 80);
}

function applyPairingTarget(target) {
  apiBase = target.gateway;
  if (target.token && target.deviceId) {
    token = target.token;
    deviceId = target.deviceId;
    desktopId = target.desktopId || null;
    pairingCode = null;
    transportKeyPromise = undefined;
    localStorage.setItem(baseKey, apiBase);
    localStorage.setItem(tokenKey, token);
    localStorage.setItem(deviceKey, deviceId);
    if (desktopId) localStorage.setItem(desktopKey, desktopId);
    history.replaceState({}, "", new URL("./", location.href).pathname);
    void showDashboard();
    return;
  }
  pairingCode = target.code;
  desktopId = target.desktopId || null;
  pairInstallPromptDismissed = false;
  localStorage.setItem(baseKey, apiBase);
  pairMessage.textContent = "";
  pairMessage.className = "message";
  document.querySelector("#manual-gateway").value = apiBase;
  document.querySelector("#manual-code").value = pairingCode;
  showEntryView();
  prepareNativePairingLaunch(target);
  scheduleAutomaticPairing();
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
  const showPairInstallPrompt =
    Boolean(pairingCode) && isAndroidBrowser && !isNative && !pairInstallPromptDismissed;
  scanPairingButton.hidden = !(isNative && nativePlatform() === "android");
  pairInstallPrompt.hidden = !showPairInstallPrompt;
  document.body.classList.toggle("install-modal-open", showPairInstallPrompt);
  androidInstallCard.hidden =
    Boolean(pairingCode) || !isAndroidBrowser || isNative;
  mobileApkDeviceDownload.hidden = isNative || !isAndroidBrowser;
  openInstalledMobile.hidden = isNative || !isAndroidBrowser || !token || !deviceId;
  if (!openInstalledMobile.hidden) {
    openInstalledMobile.href = pairingIntentUrl({ gateway: apiBase, token, deviceId });
  }
  pwaInstallButton.hidden = !deferredInstallPrompt;
}

function statusLabel(status, hasQuestion = false) {
  if (status === "waiting_for_input" && hasQuestion) return "Question waiting";
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

function mascotStateForSessions(sessions) {
  const statuses = new Set(sessions.map((session) => session.status));
  if (statuses.has("permission_required")) return "permission";
  if (statuses.has("running")) return "running";
  if (statuses.has("failed")) return "failed";
  if (statuses.has("completed")) return "completed";
  if (statuses.has("waiting_for_input")) return "waiting";
  return sessions.length ? "awake" : "sleeping";
}

function setHeaderMascotState(state) {
  if (!headerMascot || headerMascot.dataset.state === state) return;
  clearTimeout(mascotTransitionTimer);
  headerMascot.dataset.state = state;
  headerMascot.className = `lume-mobile-mascot state-${state}`;
  void headerMascot.offsetWidth;
  headerMascot.classList.add("is-changing");
  mascotTransitionTimer = setTimeout(() => headerMascot.classList.remove("is-changing"), 360);
}

function mascotMarkup(state, extraClass = "") {
  return `<span class="lume-mobile-mascot state-${state} ${extraClass}" aria-hidden="true">${headerMascot?.innerHTML || ""}</span>`;
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

function activityTime(createdAt) {
  return new Date(createdAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function displayFilePath(path) {
  const value = String(path || "").trim();
  if (
    /^(?:[a-z]:[\\/]|[\\/]{2}|\/)/i.test(value)
    || /^(?:\.\.[\\/])/.test(value)
  ) {
    return value;
  }
  return value.split(/[\\/]/).filter(Boolean).pop() || value;
}

function fileChangeRows(files) {
  return files.map((file) => `
    <code>
      <span class="file-path" title="${escapeHtml(file.path)}">${escapeHtml(displayFilePath(file.path))}</span>
      <span class="added">+${file.added}</span>
      <span class="removed">-${file.removed}</span>
    </code>
  `).join("");
}

function chatTextKey(value) {
  return String(value || "")
    .replace(/\r\n?/g, "\n")
    .replace(/[ \t]+$/gm, "")
    .trim();
}

function comparableResponseText(value) {
  return chatTextKey(value).replace(/(?:…|\.\.\.)$/, "").trimEnd();
}

function sameResponseText(left, right) {
  const leftKey = comparableResponseText(left);
  const rightKey = comparableResponseText(right);
  if (!leftKey || !rightKey) return false;
  if (leftKey === rightKey) return true;
  const shorter = leftKey.length <= rightKey.length ? leftKey : rightKey;
  const longer = leftKey.length <= rightKey.length ? rightKey : leftKey;
  return shorter.length >= 256 && longer.startsWith(shorter);
}

function keepFullerResponse(item, response) {
  if (chatTextKey(response).length > chatTextKey(item.detail).length) {
    item.detail = response;
  }
}

function buildChatTurns(session) {
  const turns = [];
  const ensureTurn = (id) => {
    const turn = { id, items: [], files: [] };
    turns.push(turn);
    return turn;
  };
  let current;
  for (const activity of session.activities || []) {
    if (activity.kind === "prompt") {
      current = ensureTurn(activity.id);
      current.prompt = activity;
      continue;
    }
    current ||= ensureTurn(`turn:${activity.id}`);
    mergeFileChanges(
      current.files,
      summarizeFileChanges(activity.detail || "", activity.files || [], session.workingDirectory),
    );
    const messageKey = activity.kind === "message" ? chatTextKey(activity.detail) : "";
    const matchingMessage = messageKey
      ? current.items.find(
          (item) => item.kind === "message" && sameResponseText(item.detail, activity.detail),
        )
      : undefined;
    if (matchingMessage) {
      keepFullerResponse(matchingMessage, activity.detail);
      continue;
    }
    current.items.push({ ...activity });
  }
  for (const result of session.results || []) {
    const resultTurn = [...turns]
      .reverse()
      .find((turn) => !turn.prompt || turn.prompt.createdAt <= result.createdAt)
      || ensureTurn(`result:${result.id}`);
    current = resultTurn;
    mergeFileChanges(
      resultTurn.files,
      summarizeFileChanges("", result.files || [], session.workingDirectory),
    );
    const matchingMessage = result.response
      ? resultTurn.items.find(
          (item) =>
            item.kind === "message"
            && sameResponseText(item.detail, result.response),
        )
      : undefined;
    if (matchingMessage) {
      keepFullerResponse(matchingMessage, result.response);
    } else if (result.response) {
      resultTurn.items.push({
        id: `response:${result.id}`,
        kind: "message",
        title: "Agent response",
        detail: result.response,
        status: "completed",
        createdAt: result.createdAt,
        files: [],
      });
    }
  }
  const matchingLastResponse = session.lastResponse
    ? turns
        .flatMap((turn) => turn.items)
        .find(
          (item) =>
            item.kind === "message"
            && sameResponseText(item.detail, session.lastResponse),
        )
    : undefined;
  if (matchingLastResponse) {
    keepFullerResponse(matchingLastResponse, session.lastResponse);
  } else if (session.lastResponse) {
    current ||= ensureTurn(`response:${session.id}`);
    current.items.push({
      id: `response:${session.id}:${session.updatedAt}`,
      kind: "message",
      title: "Agent response",
      detail: session.lastResponse,
      status: "completed",
      createdAt: session.updatedAt,
      files: [],
    });
  }
  return turns;
}

function safeImagePreview(value) {
  const preview = String(value || "");
  return /^data:image\/(?:png|jpeg|webp|gif);base64,[a-z0-9+/=\s]+$/i.test(preview)
    ? preview
    : "";
}

function messageImagesMarkup(attachments = []) {
  const images = attachments
    .map((attachment) => ({
      name: attachment.name || "Attached image",
      preview: safeImagePreview(attachment.previewDataUrl),
    }))
    .filter((attachment) => attachment.preview);
  if (!images.length) return "";
  return `<div class="mobile-message-images">${images.map((attachment) =>
    `<img src="${attachment.preview}" alt="${escapeHtml(attachment.name)}" />`
  ).join("")}</div>`;
}

function renderRateLimit(session) {
  const limits = (session.rateLimits || [])
    .filter((limit) => Number.isFinite(Number(limit.usedPercent)));
  if (!limits.length) {
    chatRateLimit.hidden = true;
    chatRateLimit.innerHTML = "";
    return;
  }
  const limit = [...limits].sort((left, right) =>
    Number(right.usedPercent) - Number(left.usedPercent)
  )[0];
  const used = Math.max(0, Math.min(100, Math.round(Number(limit.usedPercent))));
  const remaining = 100 - used;
  const tone = remaining <= 20 ? "danger" : remaining <= 50 ? "warning" : "healthy";
  const reset = limit.resetsAt
    ? ` · resets ${new Date(limit.resetsAt).toLocaleString([], { dateStyle: "short", timeStyle: "short" })}`
    : "";
  chatRateLimit.hidden = false;
  chatRateLimit.innerHTML = `
    <span class="rate-donut ${tone}" style="--remaining-angle:${remaining * 3.6}deg" role="img"
      aria-label="${escapeHtml(`${limit.label}: ${remaining}% remaining${reset}`)}">
      <b>${remaining}</b>
    </span>
    <small>${escapeHtml(limit.label || "Limit")}</small>`;
  chatRateLimit.title = `${limit.label}: ${remaining}% remaining${reset}`;
}

async function refreshRateLimitsIfNeeded(session) {
  if (session.agent !== "codex") return;
  const lastRefresh = rateLimitRefreshes.get(session.agent) || 0;
  if (Date.now() - lastRefresh < 60_000) return;
  rateLimitRefreshes.set(session.agent, Date.now());
  try {
    await executeCommand({ type: "refresh_rate_limits", agent: session.agent });
  } catch {
    rateLimitRefreshes.delete(session.agent);
  }
}

function elapsedWorkTime(startedAt, updatedAt, active = true) {
  const end = active ? Date.now() : Number(updatedAt || Date.now());
  const seconds = Math.max(0, Math.floor((end - Number(startedAt || end)) / 1_000));
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes}m`;
  if (minutes) return `${minutes}m`;
  return `${seconds}s`;
}

function goalStatusLabel(status) {
  return { active: "Active", complete: "Complete", blocked: "Blocked" }[status] || "Active";
}

function renderWorkTray(session) {
  const todo = session.workSummary?.todo;
  const goal = session.workSummary?.goal;
  if (!todo && !goal) {
    chatWorkTray.hidden = true;
    chatWorkTray.innerHTML = "";
    return;
  }

  const completed = todo?.items.filter((item) => item.status === "completed").length || 0;
  const current = todo?.items.find((item) => item.status === "in_progress")
    || todo?.items.find((item) => item.status === "pending")
    || todo?.items.at(-1);
  const todoMarkup = todo ? `
    <details class="chat-work-card todo">
      <summary>
        <span><strong>TO DO</strong><small>${escapeHtml(current?.label || "Plan complete")}</small></span>
        <b>${completed}/${todo.items.length}</b>
      </summary>
      <i class="chat-todo-progress"><em style="width:${(completed / todo.items.length) * 100}%"></em></i>
      <ul>${todo.items.map((item) => `
        <li class="${escapeHtml(item.status)}"><i></i><span>${escapeHtml(item.label)}</span></li>
      `).join("")}</ul>
    </details>` : "";
  const goalMarkup = goal ? `
    <article class="chat-work-card goal">
      <div>
        <strong>GOAL</strong>
        <b class="${escapeHtml(goal.status)}">${goalStatusLabel(goal.status)}</b>
      </div>
      <p>${escapeHtml(goal.objective)}</p>
      <small
        data-goal-started="${Number(goal.startedAt)}"
        data-goal-updated="${Number(goal.updatedAt)}"
        data-goal-active="${goal.status === "active"}"
      >${elapsedWorkTime(goal.startedAt, goal.updatedAt, goal.status === "active")}</small>
    </article>` : "";

  chatWorkTray.innerHTML = todoMarkup + goalMarkup;
  chatWorkTray.hidden = false;
}

function updateGoalElapsedTimes() {
  document.querySelectorAll("[data-goal-started]").forEach((element) => {
    element.textContent = elapsedWorkTime(
      Number(element.dataset.goalStarted),
      Number(element.dataset.goalUpdated),
      element.dataset.goalActive === "true",
    );
  });
}

function renderChat(sessions) {
  if (!activeChatSessionId) return;
  const session = sessions.find((item) => item.id === activeChatSessionId);
  if (!session) {
    chatAgentName.textContent = "Agent closed";
    chatAgentStatus.textContent = "This session is no longer open";
    chatStopButton.hidden = true;
    chatRateLimit.hidden = true;
    chatRateLimit.innerHTML = "";
    chatWorkTray.hidden = true;
    chatWorkTray.innerHTML = "";
    chatFeed.innerHTML = '<div class="empty-list compact"><strong>Session closed</strong><p>Return to the agent list to choose another session.</p></div>';
    chatComposer.innerHTML = "";
    return;
  }

  chatAgentIcon.className = `agent-icon agent-${session.agent}`;
  chatAgentIcon.innerHTML = agentVisual(session);
  chatAgentName.textContent = session.agentLabel;
  chatAgentStatus.textContent = `${session.project} · ${statusLabel(session.status, Boolean(session.pendingQuestion))}`;
  renderRateLimit(session);
  const scopes = currentDevice?.scopes || [];
  const canTerminate = scopes.includes("terminate") && session.capabilities?.canTerminate;
  chatStopButton.hidden = !canTerminate;
  chatStopButton.dataset.session = session.id;
  chatStopButton.dataset.command = "terminate";

  const renderKey = JSON.stringify([
    session.status,
    session.updatedAt,
    session.pendingPermission,
    session.pendingQuestion,
    session.activities,
    session.results,
    session.lastResponse,
    session.rateLimits,
    session.workSummary,
    scopes,
    submittingPromptSessions.has(session.id),
    promptAttachments.get(session.id),
  ]);
  if (renderKey === lastChatRenderKey) return;
  const shouldFollow =
    !lastChatRenderKey
    || chatFeed.scrollHeight - chatFeed.scrollTop - chatFeed.clientHeight < 90;
  lastChatRenderKey = renderKey;
  renderWorkTray(session);

  const permission = session.pendingPermission
    ? `<article class="mobile-chat-permission">
        <small>Approval required</small>
        <strong>${escapeHtml(session.pendingPermission.summary)}</strong>
        <code>${escapeHtml(session.pendingPermission.resource)}</code>
        ${scopes.includes("approve") && session.capabilities?.canApprove
          ? `<div>${(session.permissionProfile?.availableActions || ["deny"]).map((action) => `
              <button
                data-command="${escapeHtml(action)}"
                data-session="${escapeHtml(session.id)}"
                data-permission="${escapeHtml(session.pendingPermission.id)}"
              >${action === "allow_once" ? "Allow once" : action === "allow_session" ? "Allow session" : "Deny"}</button>
            `).join("")}</div>`
          : ""}
      </article>`
    : "";
  const questionRequest = session.pendingQuestion
    ? `<article class="mobile-chat-question">
        <small>Agent question</small>
        ${(session.pendingQuestion.questions || []).map((question) => `
          <section>
            <strong>${escapeHtml(question.header || "Question")}</strong>
            <p>${escapeHtml(question.question)}</p>
            ${(question.options || []).length ? `<div class="mobile-question-options">
              ${question.options.map((option, index) => {
                const selectionKey = `${session.pendingQuestion.id}:${question.id}`;
                const selected = questionSelections.get(selectionKey) === option.label;
                return `<button
                  class="${selected ? "selected" : ""}"
                  type="button"
                  data-question-option="${escapeHtml(option.label)}"
                  data-question-item="${escapeHtml(question.id)}"
                  data-question-request="${escapeHtml(session.pendingQuestion.id)}"
                  data-session="${escapeHtml(session.id)}"
                ><b>${index + 1}</b><span>${escapeHtml(option.label)}${option.description ? `<small>${escapeHtml(option.description)}</small>` : ""}</span></button>`;
              }).join("")}
            </div>` : ""}
            <em>Tap an option or type its number below.</em>
          </section>
        `).join("")}
        ${(session.pendingQuestion.questions || []).length > 1
          ? `<button class="mobile-question-submit" type="button" data-question-submit data-session="${escapeHtml(session.id)}">Answer</button>`
          : ""}
      </article>`
    : "";
  const turns = buildChatTurns(session);
  const conversation = turns.map((turn) => `
    <article class="mobile-chat-turn">
      ${turn.prompt?.detail || turn.prompt?.attachments?.length ? `
        <div class="mobile-chat-message user">
          <header><strong>You</strong><time>${activityTime(turn.prompt.createdAt)}</time></header>
          ${turn.prompt.detail ? `<p>${escapeHtml(turn.prompt.detail)}</p>` : ""}
          ${messageImagesMarkup(turn.prompt.attachments)}
        </div>` : ""}
      ${(turn.items || []).map((item) => {
        if (item.kind === "message" && item.detail) {
          return `<div class="mobile-chat-message agent">
            <header><strong>${escapeHtml(session.agentLabel)}</strong><time>${activityTime(item.createdAt)}</time></header>
            <div class="mobile-markdown">${renderSafeMarkdown(item.detail)}</div>
          </div>`;
        }
        if (item.kind === "file") return "";
        return `<details class="mobile-chat-trace">
          <summary><i></i><span>${escapeHtml(item.title || "Agent activity")}</span><time>${activityTime(item.createdAt)}</time></summary>
          ${item.detail ? `<pre>${escapeHtml(item.detail)}</pre>` : ""}
        </details>`;
      }).join("")}
      ${turn.files.length ? `
        <details class="mobile-turn-files">
          <summary><span>Files changed in this prompt</span><em>${turn.files.length}</em></summary>
          <div>${fileChangeRows(turn.files)}</div>
        </details>` : ""}
    </article>
  `).join("");
  const typing = session.status === "running"
    ? `<div class="mobile-agent-typing" aria-label="${escapeHtml(session.agentLabel)} is working"><span></span><span></span><span></span></div>`
    : "";
  chatFeed.innerHTML = permission + questionRequest + (conversation || '<p class="mobile-chat-empty">Messages and live agent activity will appear here.</p>') + typing;

  const promptReady = ["completed", "failed", "waiting_for_input"].includes(session.status);
  const agentWorking = session.status === "running";
  const canPrompt = scopes.includes("prompt")
    && session.capabilities?.canPrompt
    && (promptReady || agentWorking);
  const promptSubmitting = submittingPromptSessions.has(session.id);
  const sendLocked = promptSubmitting || agentWorking;
  const promptDraft = promptDrafts.get(session.id) || "";
  const attachments = promptAttachments.get(session.id) || [];
  const canAttachImages = Boolean(session.capabilities?.canAttachImages && !session.pendingQuestion);
  chatComposer.innerHTML = canPrompt
    ? `<form class="mobile-chat-form${canAttachImages ? " can-attach" : ""}${promptSubmitting ? " is-sending" : ""}" data-session="${escapeHtml(session.id)}" aria-busy="${promptSubmitting}">
        ${attachments.length ? `<div class="mobile-pending-images">
          <small>${attachments.length === 1 ? "Photo attached" : `${attachments.length} photos attached`}</small>
          ${attachments.map((attachment, index) => `
          <span title="${escapeHtml(attachment.name || "Attached image")}"><img src="${safeImagePreview(attachment.previewDataUrl)}" alt="${escapeHtml(attachment.name || "Attached image")}" /><button type="button" data-remove-image="${index}" aria-label="Remove image" ${promptSubmitting ? "disabled" : ""}>×</button></span>
        `).join("")}</div>` : ""}
        ${canAttachImages ? `<input class="mobile-image-input" type="file" accept="image/*" multiple hidden />
          <button class="mobile-attach-button" type="button" data-attach-image aria-label="Attach image" ${promptSubmitting || attachments.length >= 4 ? "disabled" : ""}>${attachIconMarkup}</button>` : ""}
        <textarea maxlength="16384" placeholder="Message ${escapeHtml(session.agentLabel)}…" ${promptSubmitting ? "disabled" : ""}>${escapeHtml(promptDraft)}</textarea>
        <button class="mobile-send-button" type="submit" aria-label="${promptSubmitting ? "Sending prompt" : "Send prompt"}" ${sendLocked || (!promptDraft.trim() && !attachments.length) ? "disabled" : ""}>${promptSubmitting ? sendSpinnerMarkup : sendIconMarkup}</button>
        <span class="prompt-send-state" role="status" aria-live="polite">${promptSubmitting ? "Sending prompt…" : ""}</span>
      </form>`
    : '<p class="mobile-composer-unavailable">Sending is unavailable for this session.</p>';

  if (shouldFollow) {
    requestAnimationFrame(() => chatFeed.scrollTo({ top: chatFeed.scrollHeight, behavior: "smooth" }));
  }
}

function openChat(sessionId) {
  activeChatSessionId = sessionId;
  lastChatRenderKey = "";
  setView("chat");
  const sessions = currentSnapshot?.sessions || [];
  renderChat(sessions);
  const session = sessions.find((item) => item.id === sessionId);
  if (session) void refreshRateLimitsIfNeeded(session);
}

function renderSessions(snapshot, trackChanges = true) {
  currentSnapshot = snapshot;
  void coordinateCompanionUpdates(snapshot);
  const sessions = snapshot.sessions || [];
  setHeaderMascotState(mascotStateForSessions(sessions));
  if (trackChanges && hasRenderedSnapshot && localStorage.getItem(notificationsKey) === "on") {
    for (const session of sessions) {
      const permissionId = session.pendingPermission?.id;
      const questionId = session.pendingQuestion?.id;
      const isNewPermission =
        session.status === "permission_required"
        && permissionId
        && previousPermissionIds.get(session.id) !== permissionId;
      const isStatusTransition =
        session.status !== "permission_required"
        && previousStatuses.get(session.id) !== session.status;
      const isNewQuestion =
        questionId && previousQuestionIds.get(session.id) !== questionId;
      if (isNewPermission || isNewQuestion || isStatusTransition) {
        void notifySession(session).catch((error) => {
          showBanner(error?.message || "Could not deliver the notification.", "error");
        });
      }
    }
  }
  if (trackChanges) {
    previousStatuses = new Map(sessions.map((session) => [session.id, session.status]));
    const openSessionIds = new Set(sessions.map((session) => session.id));
    for (const session of sessions) {
      if (session.pendingPermission?.id) {
        previousPermissionIds.set(session.id, session.pendingPermission.id);
      }
      if (session.pendingQuestion?.id) {
        previousQuestionIds.set(session.id, session.pendingQuestion.id);
      }
    }
    for (const sessionId of previousPermissionIds.keys()) {
      if (!openSessionIds.has(sessionId)) previousPermissionIds.delete(sessionId);
    }
    for (const sessionId of previousQuestionIds.keys()) {
      if (!openSessionIds.has(sessionId)) previousQuestionIds.delete(sessionId);
    }
    hasRenderedSnapshot = true;
  }
  document.querySelector("#updated-at").textContent =
    `Updated ${new Date(snapshot.generatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  renderResults(sessions);
  renderDevice();
  renderChat(sessions);
  const visibleSessions = filterSessions(sessions);
  if (!sessions.length) {
    sessionList.innerHTML = `<div class="empty-list">${mascotMarkup("sleeping")}<strong>No agents are open</strong><p>Start an agent on your computer. Lume will bring it here automatically.</p></div>`;
    return;
  }
  if (!visibleSessions.length) {
    sessionList.innerHTML = '<div class="empty-list compact"><strong>Nothing in this view</strong><p>Try another filter.</p></div>';
    return;
  }
  sessionList.innerHTML = visibleSessions.map((session) => {
    const scopes = currentDevice?.scopes || [];
    const expanded = session.status === "permission_required";
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
    const promptSubmitting = submittingPromptSessions.has(session.id);
    const promptDraft = promptDrafts.get(session.id) || "";
    const prompt = canPrompt
      ? `<form class="prompt-form${promptSubmitting ? " is-sending" : ""}" data-session="${escapeHtml(session.id)}" aria-busy="${promptSubmitting}"><textarea maxlength="16384" aria-label="Message ${escapeHtml(session.agentLabel)}" placeholder="Continue with a new prompt…" ${promptSubmitting ? "disabled" : ""} required>${escapeHtml(promptDraft)}</textarea><button type="submit" aria-label="${promptSubmitting ? "Sending prompt" : "Send prompt"}" ${promptSubmitting ? "disabled" : ""}>${promptSubmitting ? sendSpinnerMarkup : sendIconMarkup}</button><span class="prompt-send-state" role="status" aria-live="polite">${promptSubmitting ? "Sending prompt…" : ""}</span></form>`
      : "";
    const stop = scopes.includes("terminate") && session.capabilities?.canTerminate
      ? `<button class="stop-agent" data-command="terminate" data-session="${escapeHtml(session.id)}">Stop agent</button>`
      : "";
    return `
      <article class="session tone-${statusClass(session.status)} ${expanded ? "expanded" : ""}">
        <button class="session-summary" data-chat-session="${escapeHtml(session.id)}" type="button" aria-label="Open chat with ${escapeHtml(session.agentLabel)}">
          <span class="agent-icon agent-${escapeHtml(session.agent)}">${agentVisual(session)}</span>
          <span class="session-heading"><strong>${escapeHtml(session.agentLabel)}</strong><small>${escapeHtml(session.project)}</small></span>
          <span class="source-badge">${escapeHtml(sourceLabel(session))}</span>
          <span class="status-badge"><i></i>${escapeHtml(statusLabel(session.status, Boolean(session.pendingQuestion)))}</span>
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
    resultsList.innerHTML = `<div class="empty-list">${mascotMarkup("awake")}<strong>No results yet</strong><p>Finished responses and changed files will be collected here.</p></div>`;
    return;
  }
  resultsList.innerHTML = results.map(({ session, result }) => {
    const resultKey = `${session.id}:${result.id}`;
    const expanded = expandedResults.has(resultKey);
    const files = summarizeFileChanges("", result.files || [], session.workingDirectory);
    const previousPrompt = [...(session.activities || [])]
      .reverse()
      .find((activity) => activity.kind === "prompt" && activity.createdAt <= result.createdAt);
    for (const activity of session.activities || []) {
      if (
        activity.createdAt > result.createdAt
        || (previousPrompt && activity.createdAt < previousPrompt.createdAt)
      ) {
        continue;
      }
      mergeFileChanges(
        files,
        summarizeFileChanges(
          activity.detail || "",
          activity.files || [],
          session.workingDirectory,
        ),
      );
    }
    const added = files.reduce((total, file) => total + file.added, 0);
    const removed = files.reduce((total, file) => total + file.removed, 0);
    return `
      <article class="result-card ${expanded ? "expanded" : ""}">
        <button class="result-summary" data-result="${escapeHtml(resultKey)}" type="button" aria-expanded="${expanded}">
          <span class="agent-icon agent-${escapeHtml(session.agent)}">${agentVisual(session)}</span>
          <span><strong>${escapeHtml(session.agentLabel)}</strong><small>${escapeHtml(session.project)}</small></span>
          <time>${activityTime(result.createdAt)}</time>
          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4" /></svg>
        </button>
        <p class="result-response">${escapeHtml(result.response || "Task completed.")}</p>
        ${files.length ? `
          <details class="result-changes">
            <summary>
              <span>${files.length} changed file${files.length === 1 ? "" : "s"}</span>
              <i class="added">+${added}</i>
              <i class="removed">-${removed}</i>
              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4" /></svg>
            </summary>
            <div>${fileChangeRows(files)}</div>
          </details>` : ""}
      </article>`;
  }).join("");
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
  const questionBody = session.pendingQuestion
    ? `${session.agentLabel} is waiting for your answer.`
    : null;
  const notificationKey = session.pendingQuestion?.id || session.status;
  if (!body && !questionBody) return;
  if (
    nativePlatform() === "android"
    && nativeCredentialsAvailable
    && nativeRealtimePlugin()
  ) {
    return;
  }
  const localNotifications = window.Capacitor?.Plugins?.LocalNotifications;
  if (nativePlatform() === "android" && localNotifications) {
    let identifier = 17;
    for (const character of `${session.id}-${notificationKey}`) {
      identifier = ((identifier * 31) + character.charCodeAt(0)) & 0x7fffffff;
    }
    await localNotifications.schedule({
      notifications: [{
        id: identifier || 1,
        title: "Lume",
        body: questionBody || body,
        channelId: "lume-agent-events",
        extra: { sessionId: session.id, status: session.status },
      }],
    });
    return;
  }
  if (!("Notification" in window) || Notification.permission !== "granted") return;
  const registration = await navigator.serviceWorker?.ready.catch(() => null);
  if (registration) {
    registration.showNotification("Lume", {
      body: questionBody || body,
      tag: `lume-${session.id}-${notificationKey}`,
      icon: "./lume-mobile-icon.svg",
    });
  } else {
    new Notification("Lume", {
      body: questionBody || body,
      tag: `lume-${session.id}-${notificationKey}`,
    });
  }
}

async function syncNotificationPreference() {
  const localNotifications = window.Capacitor?.Plugins?.LocalNotifications;
  if (
    nativePlatform() === "android"
    && localNotifications
    && localStorage.getItem(notificationsKey) === "on"
  ) {
    const permission = await localNotifications.checkPermissions();
    if (permission.display !== "granted") {
      localStorage.removeItem(notificationsKey);
      localStorage.removeItem(backgroundMonitoringKey);
    }
  }
  await syncNativeMonitoringPreferences();
}

function updateSecurityControls() {
  const biometric = document.querySelector("#biometric-button");
  const notifications = document.querySelector("#notification-button");
  const background = document.querySelector("#background-monitoring-button");
  biometric.querySelector("em").textContent =
    localStorage.getItem(credentialKey) ? "On" : "Off";
  notifications.querySelector("em").textContent =
    localStorage.getItem(notificationsKey) === "on" ? "On" : "Off";
  biometric.classList.toggle("active", Boolean(localStorage.getItem(credentialKey)));
  notifications.classList.toggle(
    "active",
    localStorage.getItem(notificationsKey) === "on",
  );
  const nativeBackgroundAvailable = nativePlatform() === "android" && nativeRealtimePlugin();
  background.hidden = !nativeBackgroundAvailable;
  background.querySelector("em").textContent =
    localStorage.getItem(backgroundMonitoringKey) === "on" ? "On" : "Off";
  background.classList.toggle(
    "active",
    localStorage.getItem(backgroundMonitoringKey) === "on",
  );
}

async function syncNativeMonitoringPreferences() {
  const plugin = nativeRealtimePlugin();
  if (nativePlatform() !== "android" || !plugin?.setMonitoringPreferences) return;
  const status = await plugin.setMonitoringPreferences({
    notificationsEnabled: localStorage.getItem(notificationsKey) === "on",
    backgroundEnabled: localStorage.getItem(backgroundMonitoringKey) === "on",
  });
  return applyNativeMonitoringStatus(status);
}

function applyNativeMonitoringStatus(status = {}) {
  if (status.notificationsEnabled) {
    localStorage.setItem(notificationsKey, "on");
  } else {
    localStorage.removeItem(notificationsKey);
  }
  if (status.backgroundEnabled) {
    localStorage.setItem(backgroundMonitoringKey, "on");
  } else {
    localStorage.removeItem(backgroundMonitoringKey);
  }
  updateSecurityControls();
  return status;
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
    showBanner("Device verification is not supported in this browser.", "error");
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
    showBanner("Device verification enabled for remote actions.", "success");
    updateSecurityControls();
  } catch (error) {
    showBanner(error.name === "NotAllowedError"
      ? "Device verification was cancelled."
      : "Could not enable device verification.", "error");
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
    showBanner("Device verification was not completed.", "error");
    return false;
  }
}

async function toggleNotifications() {
  if (localStorage.getItem(notificationsKey) === "on") {
    const backgroundWasEnabled = localStorage.getItem(backgroundMonitoringKey) === "on";
    localStorage.removeItem(notificationsKey);
    localStorage.removeItem(backgroundMonitoringKey);
    try {
      await syncNativeMonitoringPreferences();
    } catch (error) {
      localStorage.setItem(notificationsKey, "on");
      if (backgroundWasEnabled) localStorage.setItem(backgroundMonitoringKey, "on");
      showBanner(error?.message || "Could not turn off notifications.", "error");
    }
    updateSecurityControls();
    return;
  }
  const localNotifications = window.Capacitor?.Plugins?.LocalNotifications;
  if (nativePlatform() === "android" && localNotifications) {
    try {
      let permission = await localNotifications.checkPermissions();
      if (permission.display !== "granted") {
        permission = await localNotifications.requestPermissions();
      }
      if (permission.display !== "granted") {
        localStorage.removeItem(notificationsKey);
        showBanner(
          "Notification access is blocked. Enable it in Android Settings → Apps → Lume → Notifications.",
          "error",
        );
        updateSecurityControls();
        return;
      }
      await localNotifications.createChannel({
        id: "lume-agent-events",
        name: "Agent activity",
        description: "Task completion, errors and permission requests",
        importance: 4,
        visibility: 1,
      });
      localStorage.setItem(notificationsKey, "on");
      await syncNativeMonitoringPreferences();
      showBanner("Notifications enabled for tasks, errors and approvals.", "success");
      updateSecurityControls();
    } catch (error) {
      localStorage.removeItem(notificationsKey);
      localStorage.removeItem(backgroundMonitoringKey);
      void syncNativeMonitoringPreferences().catch(() => undefined);
      showBanner(error?.message || "Could not enable Android notifications.", "error");
      updateSecurityControls();
    }
    return;
  }
  if (!("Notification" in window)) {
    showBanner("Notifications are not supported in this browser.", "error");
    return;
  }
  const permission = await Notification.requestPermission();
  if (permission === "granted") {
    localStorage.setItem(notificationsKey, "on");
    showBanner("Notifications enabled while Lume Mobile is active.", "success");
  } else {
    showBanner("Notification permission was not granted.", "error");
  }
  updateSecurityControls();
}

async function toggleBackgroundMonitoring() {
  if (nativePlatform() !== "android" || !nativeRealtimePlugin()) return;
  if (localStorage.getItem(backgroundMonitoringKey) === "on") {
    localStorage.removeItem(backgroundMonitoringKey);
    try {
      await syncNativeMonitoringPreferences();
      showBanner("Background monitoring turned off.", "success");
    } catch (error) {
      localStorage.setItem(backgroundMonitoringKey, "on");
      showBanner(error?.message || "Could not turn off background monitoring.", "error");
    }
    updateSecurityControls();
    return;
  }
  if (localStorage.getItem(notificationsKey) !== "on") {
    await toggleNotifications();
    if (localStorage.getItem(notificationsKey) !== "on") return;
  }
  localStorage.setItem(backgroundMonitoringKey, "on");
  try {
    const status = await syncNativeMonitoringPreferences();
    if (!status?.backgroundEnabled) {
      throw new Error("Android did not enable background monitoring.");
    }
    showBanner(
      "Background monitoring is on. Android will keep a quiet Lume notification visible.",
      "success",
    );
  } catch (error) {
    localStorage.removeItem(backgroundMonitoringKey);
    showBanner(error?.message || "Could not enable background monitoring.", "error");
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
  if (mobileUpdateBusy) return "busy";
  const platform = nativePlatform();
  if (platform === "web") {
    setMobileUpdateBusy(true);
    mobileUpdateDetail.textContent = "Checking the installed web app…";
    try {
      const registration = await navigator.serviceWorker?.ready;
      await registration?.update();
      mobileUpdateDetail.textContent = "The web app updates automatically.";
      mobileVersionLabel.textContent = "Web app";
      return "up_to_date";
    } catch {
      mobileUpdateDetail.textContent = "Could not check the web app right now.";
      return "error";
    } finally {
      setMobileUpdateBusy(false);
    }
  }
  if (platform === "ios") {
    mobileUpdateDetail.textContent = "Updates are managed securely by TestFlight or the App Store.";
    mobileVersionLabel.textContent = "iOS application";
    mobileUpdateButton.hidden = true;
    return "managed";
  }

  const updater = nativeUpdater();
  if (!updater) {
    mobileUpdateDetail.textContent = "The native updater is unavailable in this build.";
    mobileUpdateButton.hidden = true;
    return "unavailable";
  }

  setMobileUpdateBusy(true);
  mobileUpdateDetail.textContent = "Checking for a new version…";
  try {
    installedMobileInfo = await updater.getInfo();
    if (installedMobileInfo.openedFromUpdateNotification) {
      openUpdateViewRequested = true;
      if (hasMobileSession() && !appContent.hidden) {
        setView("device");
        openUpdateViewRequested = false;
      }
    }
    showCurrentMobileVersion();
    const manifest = await updater.getUpdateManifest();
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
      return "available";
    } else {
      availableMobileUpdate = undefined;
      mobileUpdateCard.classList.remove("available");
      mobileUpdateDetail.textContent = "You are using the latest version.";
      mobileUpdateButton.textContent = "Check now";
      return "up_to_date";
    }
  } catch (error) {
    mobileUpdateDetail.textContent = manual
      ? (error?.message || "Could not check for updates right now.")
      : "Automatic update checks will retry later.";
    return "error";
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
      if (installedMobileInfo.openedFromUpdateNotification) {
        openUpdateViewRequested = true;
        if (hasMobileSession() && !appContent.hidden) {
          setView("device");
          openUpdateViewRequested = false;
        }
      }
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
  mobileUpdatesReady = true;
  if (currentSnapshot) void coordinateCompanionUpdates(currentSnapshot);
}

async function coordinateCompanionUpdates(snapshot) {
  if (
    nativePlatform() !== "android"
    || !mobileUpdatesReady
    || companionUpdateBusy
    || !snapshot?.desktopVersion
  ) return;
  const updater = nativeUpdater();
  if (!updater) return;
  try {
    installedMobileInfo ||= await updater.getInfo();
  } catch {
    return;
  }
  const mobileVersion = installedMobileInfo?.version;
  const desktopVersion = snapshot.desktopVersion;
  if (!mobileVersion) return;

  if (compareVersions(desktopVersion, mobileVersion) > 0) {
    const key = `mobile:${mobileVersion}->${desktopVersion}`;
    const attemptedAt = companionUpdateAttempts.get(key) || 0;
    if (Date.now() - attemptedAt < 5 * 60_000) return;
    companionUpdateAttempts.set(key, Date.now());
    companionUpdateBusy = true;
    try {
      const result = await checkMobileUpdate({ manual: true });
      if (result === "available") {
        setView("device");
        await installMobileUpdate();
      }
    } finally {
      companionUpdateBusy = false;
    }
    return;
  }

  if (compareVersions(mobileVersion, desktopVersion) > 0) {
    const key = `${mobileVersion}->${desktopVersion}`;
    if (lastMobileVersionReport.key === key && Date.now() - lastMobileVersionReport.at < 60_000) {
      return;
    }
    lastMobileVersionReport = { key, at: Date.now() };
    try {
      await executeCommand({
        type: "report_mobile_version",
        version: mobileVersion,
      });
    } catch {
      // The next snapshot retries after the short throttle.
    }
  }
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
    hideBanner("connection");
  } catch (error) {
    connectionDot.className = "offline";
    connectionLabel.textContent = "Offline";
    setHeaderMascotState("sleeping");
    showBanner(error.message, "error", "connection");
  }
}

async function executeCommand(command) {
  const requestId = crypto.randomUUID?.() || `${Date.now()}-${Math.random()}`;
  const request = { requestId, ...command };
  const realtime = nativeRealtimeConnected ? nativeRealtimePlugin() : null;
  const response = realtime
    ? await realtime.sendCommand({ command: request })
    : await api("/api/v1/commands", {
        method: "POST",
        body: JSON.stringify(request),
      });
  if (!response.ok) throw new Error(response.error?.message || "The command failed");
  await refreshSnapshot();
}

async function pollEvents() {
  if (!hasMobileSession()) return;
  if (nativeRealtimeConnected) {
    pollTimer = setTimeout(pollEvents, 10_000);
    return;
  }
  try {
    const payload = await api(`/api/v1/events?since=${lastSequence}`);
    const events = payload.events || [];
    if (events.length) {
      lastSequence = Math.max(...events.map((event) => event.sequence || 0), lastSequence);
      await refreshSnapshot();
    }
  } catch (error) {
    connectionDot.className = "offline";
    showBanner(error.message, "error", "connection");
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
  await syncNotificationPreference();
  updateSecurityControls();
  await initializeNativeRealtime();
  await refreshSnapshot();
  if (openUpdateViewRequested) {
    setView("device");
    openUpdateViewRequested = false;
  }
  loadingView.hidden = true;
  pollEvents();
}

function setView(view) {
  const screens = {
    sessions: dashboard,
    results: document.querySelector("#results-screen"),
    device: document.querySelector("#device-screen"),
    chat: chatScreen,
  };
  for (const [name, screen] of Object.entries(screens)) {
    screen.hidden = name !== view;
  }
  document.querySelectorAll(".bottom-nav button").forEach((button) => {
    button.classList.toggle("active", button.dataset.view === view);
  });
  appContent.classList.toggle("chat-open", view === "chat");
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
    deviceId = credentials.device.id;
    desktopId = credentials.desktopId || desktopId;
    pairingCode = null;
    localStorage.setItem(tokenKey, token);
    localStorage.setItem(deviceKey, deviceId);
    if (desktopId) localStorage.setItem(desktopKey, desktopId);
    history.replaceState({}, "", new URL("./", location.href).pathname);
    await showDashboard();
  } catch (error) {
    pairMessage.textContent = "";
    showBanner(pairingFailureMessage(error), "error");
  } finally {
    button.disabled = false;
  }
});

document.querySelector("#refresh-button").addEventListener("click", refreshSnapshot);
document.querySelectorAll(".bottom-nav button").forEach((button) => {
  button.addEventListener("click", () => setView(button.dataset.view));
});
closePairInstallPrompt.addEventListener("click", () => {
  pairInstallPromptDismissed = true;
  updateInstallOptions();
});
pairInstallPrompt.addEventListener("click", (event) => {
  if (event.target !== pairInstallPrompt) return;
  pairInstallPromptDismissed = true;
  updateInstallOptions();
});
document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape" || pairInstallPrompt.hidden) return;
  pairInstallPromptDismissed = true;
  updateInstallOptions();
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
    message.textContent = "";
    showBanner("QR scanner is unavailable. Restart the app and try again.", "error");
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
    if (/cancel/i.test(detail)) {
      message.textContent = "Scan cancelled.";
      message.className = "message";
    } else {
      message.textContent = "";
      showBanner(detail, "error");
    }
  } finally {
    scanPairingButton.disabled = false;
  }
});
document.querySelector("#manual-pair-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const gateway = document.querySelector("#manual-gateway").value.trim().replace(/\/+$/, "");
  const code = document.querySelector("#manual-code").value.trim();
  if (!/^https?:\/\//.test(gateway) || !code) return;
  const button = event.currentTarget.querySelector("button");
  const message = event.currentTarget.querySelector(".message");
  button.disabled = true;
  message.textContent = "Pairing…";
  try {
    apiBase = gateway;
    pairingCode = code;
    const body = await api("/api/v1/pair", {
      method: "POST",
      body: JSON.stringify({
        code,
        deviceName: document.querySelector("#manual-device-name").value.trim(),
      }),
    });
    token = body.token;
    deviceId = body.device.id;
    desktopId = body.desktopId || desktopId;
    pairingCode = null;
    localStorage.setItem(baseKey, apiBase);
    localStorage.setItem(tokenKey, token);
    localStorage.setItem(deviceKey, deviceId);
    if (desktopId) localStorage.setItem(desktopKey, desktopId);
    await showDashboard();
  } catch (error) {
    message.textContent = "";
    showBanner(pairingFailureMessage(error), "error");
  } finally {
    button.disabled = false;
  }
});

function readMobileFileDataUrl(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => typeof reader.result === "string"
      ? resolve(reader.result)
      : reject(new Error("Could not read this image."));
    reader.onerror = () => reject(new Error("Could not read this image."));
    reader.readAsDataURL(file);
  });
}

function loadMobileImage(source) {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("Could not decode this image."));
    image.src = source;
  });
}

function imageDataUrl(image, maxDimension, quality) {
  const scale = Math.min(1, maxDimension / Math.max(image.naturalWidth, image.naturalHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
  const context = canvas.getContext("2d");
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.drawImage(image, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/jpeg", quality);
}

async function prepareMobileImage(file) {
  const fileName = file.name || `clipboard-image-${Date.now()}.png`;
  if (file.type && !file.type.startsWith("image/")) throw new Error(`${fileName} is not an image.`);
  if (file.size > 20 * 1024 * 1024) throw new Error(`${fileName} is larger than 20 MB.`);
  const source = await readMobileFileDataUrl(file);
  const nativeImages = nativePlatform() === "android"
    ? window.Capacitor?.Plugins?.LumeImages
    : null;
  if (nativeImages?.prepareImage) {
    const prepared = await nativeImages.prepareImage({
      dataBase64: source.slice(source.indexOf(",") + 1),
    });
    return {
      name: fileName.replace(/\.[^.]+$/, "") + ".jpg",
      mimeType: prepared.mimeType,
      dataBase64: prepared.dataBase64,
      previewDataUrl: prepared.previewDataUrl,
    };
  }
  const image = await loadMobileImage(source);
  let dataUrl = "";
  for (const [dimension, quality] of [[1600, 0.82], [1400, 0.74], [1200, 0.68], [960, 0.6]]) {
    dataUrl = imageDataUrl(image, dimension, quality);
    if (dataUrl.length <= 1_800_000) break;
  }
  if (dataUrl.length > 1_800_000) throw new Error(`${fileName} could not be prepared for secure transfer.`);
  return {
    name: fileName.replace(/\.[^.]+$/, "") + ".jpg",
    mimeType: "image/jpeg",
    dataBase64: dataUrl.slice(dataUrl.indexOf(",") + 1),
    previewDataUrl: imageDataUrl(image, 360, 0.68),
  };
}

function clipboardImageFiles(event) {
  const itemFiles = [...(event.clipboardData?.items || [])]
    .filter((item) => item.kind === "file" && item.type.startsWith("image/"))
    .map((item) => item.getAsFile())
    .filter(Boolean);
  if (itemFiles.length) return itemFiles;
  return [...(event.clipboardData?.files || [])]
    .filter((file) => !file.type || file.type.startsWith("image/"));
}

async function attachMobileImages(sessionId, files) {
  const existing = promptAttachments.get(sessionId) || [];
  const selected = [...files].slice(0, 4 - existing.length);
  const prepared = [];
  for (const file of selected) prepared.push(await prepareMobileImage(file));
  attachPreparedMobileImages(sessionId, prepared);
}

function attachPreparedMobileImages(sessionId, prepared) {
  const existing = promptAttachments.get(sessionId) || [];
  const available = Math.max(0, 4 - existing.length);
  if (!prepared.length || !available) return;
  promptAttachments.set(sessionId, [...existing, ...prepared.slice(0, available)]);
  lastChatRenderKey = "";
  renderChat(currentSnapshot?.sessions || []);
}

async function readNativeClipboardImage() {
  const images = nativePlatform() === "android"
    ? window.Capacitor?.Plugins?.LumeImages
    : null;
  if (!images?.readClipboardImage) return null;
  return images.readClipboardImage();
}

async function submitPromptForm(form) {
  const sessionId = form.dataset.session;
  const session = currentSnapshot?.sessions?.find((item) => item.id === sessionId);
  if (session?.status === "running") return;
  if (!(await verifySensitiveAction())) return;
  const textarea = form.querySelector("textarea");
  const button = form.querySelector(".mobile-send-button, button[type='submit']");
  const status = form.querySelector(".prompt-send-state");
  const submittedPrompt = textarea.value.trim();
  const attachments = promptAttachments.get(sessionId) || [];
  if ((!submittedPrompt && !attachments.length) || submittingPromptSessions.has(sessionId)) return;
  submittingPromptSessions.add(sessionId);
  promptDrafts.set(sessionId, textarea.value);
  form.classList.add("is-sending");
  form.setAttribute("aria-busy", "true");
  textarea.disabled = true;
  button.disabled = true;
  button.setAttribute("aria-label", "Sending prompt");
  button.innerHTML = sendSpinnerMarkup;
  if (status) status.textContent = "Sending prompt…";
  let promptAccepted = false;
  try {
    await executeCommand({
      type: "submit_prompt",
      sessionId,
      prompt: submittedPrompt,
      attachments,
    });
    promptAccepted = true;
    promptDrafts.delete(sessionId);
    promptAttachments.delete(sessionId);
    textarea.value = "";
  } catch (error) {
    showBanner(error.message, "error");
  } finally {
    submittingPromptSessions.delete(sessionId);
    if (form.isConnected) {
      form.classList.remove("is-sending");
      form.setAttribute("aria-busy", "false");
      textarea.disabled = false;
      const currentSession = currentSnapshot?.sessions?.find((item) => item.id === sessionId);
      button.disabled =
        promptAccepted
        || currentSession?.status === "running"
        || (!textarea.value.trim() && !(promptAttachments.get(sessionId) || []).length);
      button.setAttribute("aria-label", "Send prompt");
      button.innerHTML = sendIconMarkup;
      if (status) status.textContent = "";
    } else if (currentSnapshot) {
      renderSessions(currentSnapshot, false);
    }
  }
}

function rememberPromptDraft(event) {
  const textarea = event.target.closest?.("textarea");
  const form = textarea?.closest(".prompt-form, .mobile-chat-form");
  if (!form?.dataset.session) return;
  promptDrafts.set(form.dataset.session, textarea.value);
  const sendButton = form.querySelector(".mobile-send-button");
  if (sendButton && !submittingPromptSessions.has(form.dataset.session)) {
    const session = currentSnapshot?.sessions?.find((item) => item.id === form.dataset.session);
    sendButton.disabled =
      session?.status === "running"
      || (!textarea.value.trim() && !(promptAttachments.get(form.dataset.session) || []).length);
  }
}

async function runSessionCommand(button) {
  const command = button.dataset.command;
  if (command === "terminate" && !confirm("Stop this agent and its commands?")) return;
  if (!(await verifySensitiveAction())) return;
  button.disabled = true;
  try {
    if (command === "terminate") {
      await executeCommand({ type: "terminate_session", sessionId: button.dataset.session });
      showBanner("Agent stopped.", "success");
    } else {
      await executeCommand({
        type: "resolve_permission",
        sessionId: button.dataset.session,
        permissionId: button.dataset.permission,
        action: command,
      });
    }
  } catch (error) {
    showBanner(error.message, "error");
  } finally {
    button.disabled = false;
  }
}

async function answerMobileQuestion(button) {
  const session = currentSnapshot?.sessions?.find(
    (item) => item.id === button.dataset.session,
  );
  const request = session?.pendingQuestion;
  if (!request) return;

  if (button.dataset.questionOption) {
    const key = `${request.id}:${button.dataset.questionItem}`;
    questionSelections.set(key, button.dataset.questionOption);
    if (request.questions.length > 1) {
      lastChatRenderKey = "";
      renderChat(currentSnapshot?.sessions || []);
      return;
    }
  }

  const answers = request.questions.map((question) => {
    const value = questionSelections.get(`${request.id}:${question.id}`);
    return {
      questionId: question.id,
      answers: value ? [value] : [],
    };
  });
  if (answers.some((answer) => answer.answers.length === 0)) {
    showBanner("Choose one option for each question.", "error");
    return;
  }
  if (!(await verifySensitiveAction())) return;
  button.disabled = true;
  try {
    await executeCommand({
      type: "resolve_question",
      sessionId: session.id,
      questionId: request.id,
      answers,
    });
    for (const question of request.questions) {
      questionSelections.delete(`${request.id}:${question.id}`);
    }
    await refreshSnapshot();
  } catch (error) {
    showBanner(error.message, "error");
  } finally {
    button.disabled = false;
  }
}

sessionList.addEventListener("submit", async (event) => {
  const form = event.target.closest(".prompt-form");
  if (!form) return;
  event.preventDefault();
  await submitPromptForm(form);
});
sessionList.addEventListener("input", rememberPromptDraft);
sessionList.addEventListener("click", async (event) => {
  const chatButton = event.target.closest("button[data-chat-session]");
  if (chatButton) {
    openChat(chatButton.dataset.chatSession);
    return;
  }
  const button = event.target.closest("button[data-command]");
  if (!button) return;
  await runSessionCommand(button);
});
chatScreen.addEventListener("submit", async (event) => {
  const form = event.target.closest(".mobile-chat-form");
  if (!form) return;
  event.preventDefault();
  await submitPromptForm(form);
});
chatScreen.addEventListener("input", rememberPromptDraft);
chatScreen.addEventListener("paste", async (event) => {
  const form = event.target.closest?.(".mobile-chat-form");
  const sessionId = form?.dataset.session;
  const session = currentSnapshot?.sessions?.find((item) => item.id === sessionId);
  const files = clipboardImageFiles(event);
  const clipboardTypes = [...(event.clipboardData?.types || [])];
  const hasImageHint = clipboardTypes.some(
    (type) => type === "Files" || type.toLowerCase().startsWith("image/"),
  );
  if (
    !sessionId
    || submittingPromptSessions.has(sessionId)
    || !session?.capabilities?.canAttachImages
    || (!files.length && !hasImageHint)
  ) return;
  event.preventDefault();
  try {
    if (files.length) {
      await attachMobileImages(sessionId, files);
    } else {
      const prepared = await readNativeClipboardImage();
      if (!prepared) throw new Error("Could not read the image from the clipboard.");
      attachPreparedMobileImages(sessionId, [prepared]);
    }
  } catch (error) {
    showBanner(error?.message || "Could not attach this image.", "error");
  }
});
chatScreen.addEventListener("change", async (event) => {
  const input = event.target.closest(".mobile-image-input");
  if (!input) return;
  const form = input.closest(".mobile-chat-form");
  const sessionId = form?.dataset.session;
  if (!sessionId) return;
  try {
    await attachMobileImages(sessionId, input.files || []);
  } catch (error) {
    showBanner(error?.message || "Could not attach this image.", "error");
  }
});
chatScreen.addEventListener("click", async (event) => {
  const attachButton = event.target.closest("button[data-attach-image]");
  if (attachButton) {
    attachButton.closest(".mobile-chat-form")?.querySelector(".mobile-image-input")?.click();
    return;
  }
  const removeButton = event.target.closest("button[data-remove-image]");
  if (removeButton) {
    const form = removeButton.closest(".mobile-chat-form");
    const sessionId = form?.dataset.session;
    if (!sessionId) return;
    const index = Number(removeButton.dataset.removeImage);
    const attachments = [...(promptAttachments.get(sessionId) || [])];
    attachments.splice(index, 1);
    if (attachments.length) promptAttachments.set(sessionId, attachments);
    else promptAttachments.delete(sessionId);
    lastChatRenderKey = "";
    renderChat(currentSnapshot?.sessions || []);
    return;
  }
  const questionButton = event.target.closest(
    "button[data-question-option], button[data-question-submit]",
  );
  if (questionButton) {
    await answerMobileQuestion(questionButton);
    return;
  }
  const button = event.target.closest("button[data-command]");
  if (!button) return;
  await runSessionCommand(button);
});
document.querySelector("#chat-back").addEventListener("click", () => {
  setView("sessions");
});
resultsList.addEventListener("click", (event) => {
  const button = event.target.closest("button[data-result]");
  if (!button) return;
  const resultId = button.dataset.result;
  if (expandedResults.has(resultId)) expandedResults.delete(resultId);
  else expandedResults.add(resultId);
  if (currentSnapshot) renderResults(currentSnapshot.sessions || []);
});
document.querySelector("#biometric-button").addEventListener("click", toggleBiometric);
document.querySelector("#notification-button").addEventListener("click", toggleNotifications);
document.querySelector("#background-monitoring-button")
  .addEventListener("click", toggleBackgroundMonitoring);
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
document.querySelector("#disconnect-button").addEventListener("click", async () => {
  if (!confirm("Disconnect this phone from Lume?")) return;
  await clearMobileCredentials();
  localStorage.removeItem(baseKey);
  apiBase = "";
  currentDevice = undefined;
  showEntryView();
});
document.addEventListener("visibilitychange", () => {
  if (document.hidden) return;
  if (pairingCode && !hasMobileSession()) scheduleAutomaticPairing();
  if (nativePlatform() === "android") {
    const realtime = nativeRealtimePlugin();
    if (realtime?.getStatus) {
      void realtime.getStatus()
        .then(applyNativeMonitoringStatus)
        .catch(() => undefined);
    }
    const lastCheck = Number(localStorage.getItem(mobileUpdateCheckKey) || 0);
    if (Date.now() - lastCheck >= mobileUpdateInterval) {
      void checkMobileUpdate();
    } else {
      const updater = nativeUpdater();
      if (!updater) return;
      void updater.getInfo().then((info) => {
        if (!info.openedFromUpdateNotification) return;
        openUpdateViewRequested = true;
        if (hasMobileSession() && !appContent.hidden) {
          setView("device");
          openUpdateViewRequested = false;
        }
        void checkMobileUpdate({ manual: true });
      }).catch(() => undefined);
    }
  }
});
if ("serviceWorker" in navigator) {
  let refreshing = false;
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (refreshing || !navigator.serviceWorker.controller) return;
    refreshing = true;
    location.reload();
  });
  navigator.serviceWorker.register("./sw.js")
    .then((registration) => registration.update())
    .catch(() => undefined);
}
async function startApp() {
  if (token && !deviceId) {
    localStorage.removeItem(tokenKey);
    token = null;
    transportKeyPromise = undefined;
  }
  await restoreNativeCredentials();
  const pairingHandled = await initializePairingDeepLinks();
  void initializeMobileUpdates();
  if (pairingHandled) return;
  if (hasMobileSession()) await showDashboard();
  else showEntryView();
}

void startApp();
setInterval(updateGoalElapsedTimes, 30_000);
