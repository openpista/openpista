const tokenInput = document.getElementById("token");
const connectButton = document.getElementById("connect");
const authControls = document.getElementById("auth-controls");
const appShell = document.getElementById("app-shell");
const authModal = document.getElementById("auth-modal");
const sessionList = document.getElementById("session-list");
const sessionEmpty = document.getElementById("session-empty");
const messages = document.getElementById("messages");
const chatScroll = document.getElementById("chat-scroll");
const composer = document.getElementById("composer");
const messageInput = document.getElementById("message");
const sidebarToggle = document.getElementById("sidebar-toggle");
const sidebarClose = document.getElementById("sidebar-close");
const sidebarBackdrop = document.getElementById("sidebar-backdrop");
const toastContainer = document.getElementById("toast-container");
const newChatBtn = document.getElementById("new-chat");
const modelSelectorBtn = document.getElementById("model-selector-btn");
const modelSelectorLabel = document.getElementById("model-selector-label");
const modelDropdown = document.getElementById("model-dropdown");
const authBtn = document.getElementById("auth-btn");
const requestStatus = document.getElementById("request-status");
const authStatusDot = document.getElementById("auth-status-dot");
const authStatusLabel = document.getElementById("auth-status-label");
const stopBtn = document.getElementById("stop-btn");

const MOBILE_BREAKPOINT = 760;

const STORAGE_KEY = "openpista:web:client_id";
const SESSION_STORAGE_KEY = "openpista:web:session_id";
const RECENT_SESSIONS_KEY = "openpista:web:recent_sessions";
const TOKEN_STORAGE_KEY = "openpista:web:token";
const SESSION_TOKEN_KEY = "openpista:web:session_token";
const SESSION_NAMES_KEY = "openpista:web:session_names";
const RESPONSE_TIMEOUT_MS = 20_000;
const MAX_RECENT_SESSIONS = 10;
const SESSION_REFRESH_INTERVAL_MS = 10_000;
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;
const RECONNECT_MAX_ATTEMPTS = 10;

function isMobile() {
  return window.innerWidth <= MOBILE_BREAKPOINT;
}

function openSidebar() {
  appShell.classList.add("sidebar-open");
  if (sidebarToggle) {
    sidebarToggle.setAttribute("aria-expanded", "true");
  }
}

function closeSidebar() {
  appShell.classList.remove("sidebar-open");
  if (sidebarToggle) {
    sidebarToggle.setAttribute("aria-expanded", "false");
  }
}

function toggleSidebar() {
  if (appShell.classList.contains("sidebar-open")) {
    closeSidebar();
  } else {
    openSidebar();
  }
}

if (sidebarToggle) {
  sidebarToggle.addEventListener("click", toggleSidebar);
}
if (sidebarClose) {
  sidebarClose.addEventListener("click", closeSidebar);
}
if (sidebarBackdrop) {
  sidebarBackdrop.addEventListener("click", closeSidebar);
}
if (newChatBtn) {
  newChatBtn.addEventListener("click", newChat);
}

// When resizing from mobile to desktop, auto-open the sidebar.
window.matchMedia(`(min-width: ${MOBILE_BREAKPOINT + 1}px)`).addEventListener("change", (e) => {
  if (e.matches) {
    openSidebar();
  }
});

let socket = null;
let clientId = localStorage.getItem(STORAGE_KEY) || "";
let sessionId = localStorage.getItem(SESSION_STORAGE_KEY) || "";
let recentSessions = loadRecentSessions();
let serverSessions = [];
let sessionRefreshTimer = null;
let isAuthenticated = false;
let isSwitchingSession = false;
let pendingSessionReconnect = false;
let pendingQueue = [];
const pendingById = new Map();
let isConnecting = false;
let reconnectAttempts = 0;
let reconnectTimer = null;
let authRetryCount = 0;
let wasAuthenticated = false;
let sessionNames = loadSessionNames();
let availableModels = [];

function loadSessionNames() {
  try {
    const raw = localStorage.getItem(SESSION_NAMES_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return (parsed && typeof parsed === "object" && !Array.isArray(parsed)) ? parsed : {};
  } catch {
    return {};
  }
}

function getSessionName(sessionId) {
  if (sessionNames[sessionId]) return sessionNames[sessionId];
  if (sessionId.length > 8) return sessionId.slice(0, 8) + "\u2026";
  return sessionId;
}

function setSessionName(sessionId, name) {
  const trimmed = (name || "").trim();
  if (!trimmed || trimmed === sessionId || trimmed === getSessionName(sessionId)) {
    delete sessionNames[sessionId];
  } else {
    sessionNames[sessionId] = trimmed;
  }
  try {
    localStorage.setItem(SESSION_NAMES_KEY, JSON.stringify(sessionNames));
  } catch {}
}

function deleteSession(targetSession) {
  // Remove from recent sessions
  recentSessions = recentSessions.filter((s) => s !== targetSession);
  saveRecentSessions(recentSessions);

  // Remove custom name
  delete sessionNames[targetSession];
  try { localStorage.setItem(SESSION_NAMES_KEY, JSON.stringify(sessionNames)); } catch {}

  // Remove from server sessions (local only — server list refreshes on next poll)
  serverSessions = serverSessions.filter((s) => s.id !== targetSession);

  // If deleting active session, start a new chat
  if (targetSession === sessionId) {
    newChat();
  } else {
    renderSessionList();
  }

  showToast("Session deleted", "success");
}

function loadRecentSessions() {
  const raw = localStorage.getItem(RECENT_SESSIONS_KEY);
  if (!raw) {
    return [];
  }

  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }

    const normalized = [];
    const seen = new Set();
    for (const item of parsed) {
      const candidate = typeof item === "string" ? item.trim() : "";
      if (!candidate || seen.has(candidate)) {
        continue;
      }
      normalized.push(candidate);
      seen.add(candidate);
      if (normalized.length >= MAX_RECENT_SESSIONS) {
        break;
      }
    }
    return normalized;
  } catch {
    return [];
  }
}

function saveRecentSessions(list) {
  try {
    localStorage.setItem(RECENT_SESSIONS_KEY, JSON.stringify(list));
  } catch {
    // Ignore storage errors and keep UI functional.
  }
}

function touchRecentSession(session) {
  const clean = (session || "").trim();
  if (!clean) {
    return;
  }

  const next = [clean, ...recentSessions.filter((item) => item !== clean)].slice(
    0,
    MAX_RECENT_SESSIONS,
  );
  recentSessions = next;
  saveRecentSessions(next);
}

function normalizeServerSessions(entries) {
  if (!Array.isArray(entries)) {
    return [];
  }

  const normalized = [];
  const seen = new Set();
  for (const entry of entries) {
    const id = typeof entry?.id === "string" ? entry.id.trim() : "";
    if (!id || seen.has(id)) {
      continue;
    }
    normalized.push({
      id,
      channel_id: typeof entry?.channel_id === "string" ? entry.channel_id.trim() : "",
      updated_at: typeof entry?.updated_at === "string" ? entry.updated_at.trim() : "",
      preview: typeof entry?.preview === "string" ? entry.preview.trim() : "",
    });
    seen.add(id);
  }
  return normalized;
}

function updateMainHeaderTitle() {
  const titleEl = document.getElementById("main-header-title");
  if (!titleEl) return;
  titleEl.textContent = sessionId ? getSessionName(sessionId) : "Assistant";
  titleEl.title = sessionId || "";
}

function startRenameSession(button, itemSession) {
  if (button.querySelector(".session-item-input")) return;
  const currentName = getSessionName(itemSession);
  const input = document.createElement("input");
  input.type = "text";
  input.className = "session-item-input";
  input.value = sessionNames[itemSession] || "";
  input.placeholder = itemSession.length > 8 ? itemSession.slice(0, 8) + "\u2026" : itemSession;
  button.textContent = "";
  button.classList.add("is-editing");
  button.appendChild(input);
  input.focus();
  input.select();

  function commit() {
    const val = input.value.trim();
    setSessionName(itemSession, val);
    button.classList.remove("is-editing");
    button.textContent = getSessionName(itemSession);
    updateMainHeaderTitle();
  }

  input.addEventListener("blur", commit, { once: true });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); input.blur(); }
    if (e.key === "Escape") { input.value = currentName; input.blur(); }
  });
  input.addEventListener("click", (e) => e.stopPropagation());
}

function renderSessionList() {
  if (!sessionList || !sessionEmpty) {
    return;
  }

  sessionList.textContent = "";
  const useServerSessions = serverSessions.length > 0;
  const displaySessions = useServerSessions ? serverSessions : recentSessions;

  if (displaySessions.length === 0) {
    sessionEmpty.classList.remove("hidden");
    sessionList.classList.add("hidden");
    return;
  }

  sessionEmpty.classList.add("hidden");
  sessionList.classList.remove("hidden");

  for (const item of displaySessions) {
    const itemSession = useServerSessions ? item.id : item;
    const li = document.createElement("li");
    li.className = "session-li";
    const button = document.createElement("button");
    button.type = "button";
    button.className = "session-item";
    if (itemSession === sessionId) {
      button.classList.add("is-active");
    }
    button.textContent = getSessionName(itemSession);
    button.title = itemSession;
    button.addEventListener("click", () => {
      switchSession(itemSession);
    });
    // Three-dot context menu (Claude-style)
    const menuBtn = document.createElement("button");
    menuBtn.type = "button";
    menuBtn.className = "session-menu-btn";
    menuBtn.setAttribute("aria-label", "Session options");
    menuBtn.innerHTML = '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true"><circle cx="8" cy="3" r="1.2" fill="currentColor"/><circle cx="8" cy="8" r="1.2" fill="currentColor"/><circle cx="8" cy="13" r="1.2" fill="currentColor"/></svg>';

    const dropdown = document.createElement("div");
    dropdown.className = "session-menu-dropdown";

    const renameOption = document.createElement("button");
    renameOption.type = "button";
    renameOption.className = "session-menu-option";
    renameOption.innerHTML = '<svg width="14" height="14" viewBox="0 0 14 14" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true"><path d="M10.5 1.5L12.5 3.5L4.5 11.5H2.5V9.5L10.5 1.5Z" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/></svg><span>Rename</span>';
    renameOption.addEventListener("click", (e) => {
      e.stopPropagation();
      dropdown.classList.remove("open");
      startRenameSession(button, itemSession);
    });

    const deleteOption = document.createElement("button");
    deleteOption.type = "button";
    deleteOption.className = "session-menu-option session-menu-option-danger";
    deleteOption.innerHTML = '<svg width="14" height="14" viewBox="0 0 14 14" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true"><path d="M2 4H12M5 4V2.5H9V4M5.5 6.5V10.5M8.5 6.5V10.5M3 4L3.5 12H10.5L11 4" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round"/></svg><span>Delete</span>';
    deleteOption.addEventListener("click", (e) => {
      e.stopPropagation();
      dropdown.classList.remove("open");
      const name = getSessionName(itemSession);
      if (confirm(`Delete "${name}"?`)) {
        deleteSession(itemSession);
      }
    });

    dropdown.appendChild(renameOption);
    dropdown.appendChild(deleteOption);

    menuBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      // Close any other open dropdowns first
      document.querySelectorAll(".session-menu-dropdown.open").forEach(d => d.classList.remove("open"));
      dropdown.classList.toggle("open");
    });

    li.appendChild(button);
    li.appendChild(menuBtn);
    li.appendChild(dropdown);
    sessionList.appendChild(li);
  }

  updateMainHeaderTitle();
}

document.addEventListener("click", () => {
  document.querySelectorAll(".session-menu-dropdown.open").forEach(d => d.classList.remove("open"));
});

function switchSession(nextSession) {
  const target = (nextSession || "").trim();
  if (!target) {
    return;
  }

  const changed = target !== sessionId;
  setSession(target);

  if (isMobile()) {
    closeSidebar();
  }

  if (!changed) {
    setRequestStatus("Session unchanged");
    return;
  }

  const readyState = socket ? socket.readyState : WebSocket.CLOSED;
  if (
    readyState === WebSocket.OPEN ||
    readyState === WebSocket.CONNECTING ||
    readyState === WebSocket.CLOSING
  ) {
    isSwitchingSession = true;
    pendingSessionReconnect = true;
    setRequestStatus("Switching session...");
    try {
      socket.close();
    } catch {
      pendingSessionReconnect = false;
      isSwitchingSession = false;
      connect();
    }
    return;
  }

  const token = tokenInput.value.trim();
  if (token) {
    connect();
  }
}

function newChat() {
  const newId = window.crypto && typeof window.crypto.randomUUID === "function"
    ? window.crypto.randomUUID()
    : `web-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  clearMessages();
  switchSession(newId);
}


function setAuthMode(authenticated) {
  isAuthenticated = authenticated;
  if (authModal) {
    authModal.classList.toggle("hidden", authenticated);
  }
  if (appShell) {
    appShell.classList.toggle("is-auth-required", !authenticated);
  }
  if (document.body) {
    document.body.classList.toggle("auth-required", !authenticated);
  }
  if (authControls) {
    authControls.classList.remove("hidden");
  }
  messages.classList.toggle("hidden", !authenticated);
  composer.classList.toggle("hidden", !authenticated);

  if (authenticated) {
    messageInput.focus();
  } else {
    tokenInput.focus();
  }
}

function setSelectedModel(provider, model) {
  const cleanProvider = (provider || "").trim();
  const cleanModel = (model || "").trim();
  const displayModel = cleanModel || "(unknown)";
  if (modelSelectorLabel) {
    modelSelectorLabel.textContent = cleanProvider ? `${cleanProvider}/${displayModel}` : displayModel;
  }
}

/* ── Model Selector ── */

function requestModelList() {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify({ type: "model_list_request" }));
  }
}

let _lastModelDropdownKey = null;

function renderModelDropdown() {
  if (!modelDropdown) return;
  const key = JSON.stringify(availableModels) + JSON.stringify(
    providerAuth.providers.map(p => ({ n: p.name, a: p.authenticated, m: p.auth_mode }))
  );
  if (key === _lastModelDropdownKey) return;
  _lastModelDropdownKey = key;
  modelDropdown.innerHTML = "";
  const groups = {};
  const authenticatedProviders = new Set(
    providerAuth.providers
      .filter(p => p.authenticated || p.auth_mode === 'none')
      .map(p => p.name)
  );
  const filteredModels = authenticatedProviders.size > 0
    ? availableModels.filter(m => authenticatedProviders.has(m.provider))
    : availableModels.filter(_ => false);  // no provider info yet — show nothing
  if (filteredModels.length === 0) {
    const emptyMsg = document.createElement("div");
    emptyMsg.className = "model-group-header";
    emptyMsg.textContent = "No models available. Connect a provider first.";
    emptyMsg.style.fontStyle = "italic";
    emptyMsg.style.color = "var(--muted)";
    modelDropdown.appendChild(emptyMsg);
    return;
  }
  for (const m of filteredModels) {
    if (!groups[m.provider]) groups[m.provider] = [];
    groups[m.provider].push(m);
  }
  for (const [provider, models] of Object.entries(groups)) {
    const header = document.createElement("div");
    header.className = "model-group-header";
    header.textContent = provider;
    modelDropdown.appendChild(header);
    for (const m of models) {
      const opt = document.createElement("button");
      opt.className = "model-option";
      opt.textContent = m.model;
      if (m.recommended) opt.classList.add("recommended");
      opt.dataset.provider = m.provider;
      opt.dataset.model = m.model;
      opt.addEventListener("click", () => selectModel(m.provider, m.model));
      modelDropdown.appendChild(opt);
    }
  }
}

function toggleModelDropdown() {
  if (!modelDropdown) return;
  const isOpen = !modelDropdown.classList.contains("hidden");
  if (isOpen) {
    modelDropdown.classList.add("hidden");
    if (modelSelectorBtn) modelSelectorBtn.setAttribute("aria-expanded", "false");
  } else {
    modelDropdown.classList.remove("hidden");
    if (modelSelectorBtn) modelSelectorBtn.setAttribute("aria-expanded", "true");
  }
}

function closeModelDropdown() {
  if (modelDropdown) modelDropdown.classList.add("hidden");
  if (modelSelectorBtn) modelSelectorBtn.setAttribute("aria-expanded", "false");
}

function selectModel(provider, model) {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify({ type: "model_change", provider, model }));
  }
  closeModelDropdown();
}

if (modelSelectorBtn) {
  modelSelectorBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    toggleModelDropdown();
  });
}

document.addEventListener("click", (e) => {
  const sel = document.getElementById("model-selector");
  if (sel && !sel.contains(e.target)) {
    closeModelDropdown();
  }
});

/* ── Auth UI ── */

function updateAuthUI(online) {
  if (authStatusDot) authStatusDot.className = "auth-dot " + (online ? "online" : "offline");
  if (authStatusLabel) authStatusLabel.textContent = online ? "Online" : "Offline";
}

if (authBtn) {
  authBtn.addEventListener("click", () => {
    if (isAuthenticated) {
      const prov = modelSelectorLabel ? modelSelectorLabel.textContent : "";
      showToast(prov + " · Session: " + (sessionId || "none"), "default", 3000);
    } else {
      setAuthMode(false);
    }
  });
}

// ── Provider Auth ─────────────────────────────────────────────────────────
const providerAuth = {
  providers: [],
  pendingFlows: {},

  init() {
    const btn = document.getElementById('providerAuthBtn');
    const modal = document.getElementById('providerAuthModal');
    const closeBtn = document.getElementById('providerAuthClose');

    if (btn) {
      btn.addEventListener('click', () => {
        modal.classList.remove('hidden');
        this.requestProviders();
      });
    }

    if (closeBtn) {
      closeBtn.addEventListener('click', () => {
        modal.classList.add('hidden');
      });
    }

    if (modal) {
      modal.addEventListener('click', (e) => {
        if (e.target === modal) modal.classList.add('hidden');
      });
    }
  },

  requestProviders() {
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: 'provider_auth_request' }));
    }
  },

  handleProviderAuthStatus(data) {
    this.providers = data.providers || [];
    this.renderProviders();
    requestModelList();
    renderModelDropdown();
  },

  handleProviderAuthUrl(data) {
    const { provider, auth_url, flow_type } = data;

    if (flow_type === 'code_display') {
      window.open(auth_url, '_blank', 'noopener,noreferrer');
      this.pendingFlows[provider] = flow_type;
      this.showCodeInput(provider);
      showToast(`Opened ${provider} auth page. Enter the code when ready.`, 'default');
    } else {
      // Security note: noopener omitted intentionally — we need the popup
      // reference to detect when the OAuth window closes. Only trusted
      // provider OAuth URLs are opened here.
      const popup = window.open(auth_url, `${provider}_auth`, 'width=600,height=700');
      showToast(`Opened ${provider} login window...`, 'default');
      const check = setInterval(() => {
        if (popup && popup.closed) {
          clearInterval(check);
          setTimeout(() => this.requestProviders(), 1000);
        }
      }, 500);
    }
  },

  handleProviderAuthCompleted(data) {
    const { provider, success, message } = data;
    if (success) {
      showToast(`\u2705 ${message}`, 'success');
    } else {
      showToast(`\u274c ${message}`, 'error');
    }
    this.requestProviders();
    requestModelList();
  },

  showCodeInput(provider) {
    const item = document.querySelector(`.provider-item[data-provider="${provider}"]`);
    if (!item) return;
    const codeInput = item.querySelector('.provider-code-input');
    if (codeInput) codeInput.classList.add('visible');
  },

  submitCode(provider) {
    const item = document.querySelector(`.provider-item[data-provider="${provider}"]`);
    if (!item) return;
    const input = item.querySelector('.code-field');
    if (!input || !input.value.trim()) return;

    socket.send(JSON.stringify({
      type: 'provider_login',
      provider: provider,
      auth_code: input.value.trim(),
    }));

    input.value = '';
    const codeInput = item.querySelector('.provider-code-input');
    if (codeInput) codeInput.classList.remove('visible');
  },

  submitApiKey(provider) {
    const item = document.querySelector(`.provider-item[data-provider="${provider}"]`);
    if (!item) return;
    const keyInput = item.querySelector('.api-key-field');
    const endpointInput = item.querySelector('.endpoint-field');

    const apiKey = keyInput ? keyInput.value.trim() : '';
    if (!apiKey) {
      showToast('Please enter an API key', 'error');
      return;
    }

    const msg = {
      type: 'provider_login',
      provider: provider,
      api_key: apiKey,
    };
    if (endpointInput && endpointInput.value.trim()) {
      msg.endpoint = endpointInput.value.trim();
    }

    socket.send(JSON.stringify(msg));

    if (keyInput) keyInput.value = '';
    if (endpointInput) endpointInput.value = '';

    const form = item.querySelector('.provider-auth-form');
    if (form) form.classList.remove('visible');
  },

  startOAuthLogin(provider) {
    socket.send(JSON.stringify({
      type: 'provider_login',
      provider: provider,
    }));
  },

  renderProviders() {
    const container = document.getElementById('providerList');
    if (!container) return;

    if (this.providers.length === 0) {
      container.innerHTML = '<div class="provider-loading">No providers available</div>';
      return;
    }

    container.innerHTML = this.providers.map(p => {
      const statusDot = `<span class="provider-status-dot ${p.authenticated ? 'authenticated' : 'unauthenticated'}"></span>`;
      const statusText = p.authenticated ? 'Connected' : 'Not connected';
      const runtimeBadge = p.supports_runtime ? '' : ' <span style="font-size:11px;color:var(--muted)">(extension)</span>';

      let actionBtn = '';
      let formHtml = '';

      if (p.auth_mode === 'none') {
        actionBtn = '<span style="font-size:12px;color:var(--muted)">No auth needed</span>';
      } else if (p.auth_mode === 'oauth') {
        if (p.authenticated) {
          actionBtn = `<button class="provider-auth-btn disconnect" onclick="providerAuth.startOAuthLogin('${escAttr(p.name)}')">Reconnect</button>`;
        } else {
          actionBtn = `<button class="provider-auth-btn" onclick="providerAuth.startOAuthLogin('${escAttr(p.name)}')">Login</button>`;
        }
      } else if (p.auth_mode === 'api_key') {
        actionBtn = `<button class="provider-auth-btn" onclick="this.closest('.provider-item').querySelector('.provider-auth-form').classList.toggle('visible')">${p.authenticated ? 'Update Key' : 'Add Key'}</button>`;
        formHtml = `
          <div class="provider-auth-form">
            <input type="password" class="api-key-field" placeholder="API Key">
            <div class="form-actions">
              <button class="provider-auth-btn" onclick="providerAuth.submitApiKey('${escAttr(p.name)}')">Save</button>
            </div>
          </div>`;
      } else if (p.auth_mode === 'endpoint_and_key') {
        actionBtn = `<button class="provider-auth-btn" onclick="this.closest('.provider-item').querySelector('.provider-auth-form').classList.toggle('visible')">${p.authenticated ? 'Update' : 'Configure'}</button>`;
        formHtml = `
          <div class="provider-auth-form">
            <input type="text" class="endpoint-field" placeholder="Endpoint URL">
            <input type="password" class="api-key-field" placeholder="API Key">
            <div class="form-actions">
              <button class="provider-auth-btn" onclick="providerAuth.submitApiKey('${escAttr(p.name)}')">Save</button>
            </div>
          </div>`;
      }

      const codeInputHtml = `
        <div class="provider-code-input">
          <input type="text" class="code-field" placeholder="Paste authorization code">
          <div class="form-actions">
            <button class="provider-auth-btn" onclick="providerAuth.submitCode('${escAttr(p.name)}')">Submit Code</button>
          </div>
        </div>`;

      return `
        <div class="provider-item" data-provider="${escAttr(p.name)}">
          <div style="flex:1">
            <div style="display:flex;align-items:center;justify-content:space-between">
              <div class="provider-item-info">
                ${statusDot}
                <div>
                  <div class="provider-item-name">${escapeHtml(p.display_name)}${runtimeBadge}</div>
                  <div class="provider-item-mode">${statusText}</div>
                </div>
              </div>
              <div class="provider-item-status">
                ${actionBtn}
              </div>
            </div>
            ${formHtml}
            ${codeInputHtml}
          </div>
        </div>`;
    }).join('');
  }
};

function setSession(session) {
  const cleanSession = (session || "").trim();
  sessionId = cleanSession;

  if (cleanSession) {
    localStorage.setItem(SESSION_STORAGE_KEY, cleanSession);
    touchRecentSession(cleanSession);
    const targetPath = `/s/${encodeURIComponent(cleanSession)}`;
    if (window.location.pathname !== targetPath) {
      window.history.replaceState(null, "", targetPath);
    }
  } else {
    localStorage.removeItem(SESSION_STORAGE_KEY);
  }

  renderSessionList();
  updateMainHeaderTitle();
}

function setRequestStatus(text) {
  if (!requestStatus) {
    return;
  }
  requestStatus.textContent = text;
}

/* ── Lightweight Markdown Renderer ── */

function _mdEsc(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function _mdInline(s) {
  s = _mdEsc(s);
  const codes = [];
  s = s.replace(/`([^`]+)`/g, (_, c) => { codes.push(c); return '\x00C' + (codes.length - 1) + '\x00'; });
  s = s.replace(/\*\*\*(.+?)\*\*\*/g, '<strong><em>$1</em></strong>');
  s = s.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/\*(.+?)\*/g, '<em>$1</em>');
  s = s.replace(/~~(.+?)~~/g, '<del>$1</del>');
  s = s.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, t, u) => {
    if (/^(https?:\/\/|\/|#)/.test(u)) return '<a href="' + u + '" target="_blank" rel="noopener noreferrer">' + t + '</a>';
    return '[' + t + '](' + u + ')';
  });
  s = s.replace(/\x00C(\d+)\x00/g, (_, i) => '<code>' + codes[parseInt(i)] + '</code>');
  return s;
}

function _mdTable(lines) {
  const hdr = lines[0].replace(/^\|/, '').replace(/\|$/, '').split('|').map(c => c.trim());
  const rows = lines.slice(2);
  let h = '<table><thead><tr>' + hdr.map(c => '<th>' + _mdInline(c) + '</th>').join('') + '</tr></thead><tbody>';
  for (const r of rows) {
    const cells = r.replace(/^\|/, '').replace(/\|$/, '').split('|').map(c => c.trim());
    h += '<tr>' + cells.map(c => '<td>' + _mdInline(c) + '</td>').join('') + '</tr>';
  }
  return h + '</tbody></table>';
}

function _mdBlocks(text) {
  const lines = text.split('\n');
  const out = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Skip empty lines
    if (!line.trim()) { i++; continue; }

    // Horizontal rule
    if (/^[-*_]{3,}$/.test(line.trim())) { out.push('<hr>'); i++; continue; }

    // Header
    const hm = line.match(/^(#{1,6})\s+(.+)$/);
    if (hm) { out.push('<h' + hm[1].length + '>' + _mdInline(hm[2]) + '</h' + hm[1].length + '>'); i++; continue; }

    // Table: current line has pipes AND next line is separator (|---|---|)
    if (/\|/.test(line) && i + 1 < lines.length && /^[\s|:-]+$/.test(lines[i + 1]) && lines[i + 1].includes('---')) {
      const tableLines = [line, lines[i + 1]];
      i += 2;
      while (i < lines.length && /\|/.test(lines[i]) && lines[i].trim()) { tableLines.push(lines[i]); i++; }
      out.push(_mdTable(tableLines));
      continue;
    }

    // Blockquote
    if (/^>\s?/.test(line)) {
      const bqLines = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) { bqLines.push(lines[i].replace(/^>\s?/, '')); i++; }
      out.push('<blockquote>' + _mdInline(bqLines.join('\n')) + '</blockquote>');
      continue;
    }

    // Unordered list
    if (/^\s*[-*+]\s/.test(line)) {
      const items = [];
      while (i < lines.length && /^\s*[-*+]\s/.test(lines[i])) { items.push(lines[i].replace(/^\s*[-*+]\s+/, '')); i++; }
      out.push('<ul>' + items.map(t => '<li>' + _mdInline(t) + '</li>').join('') + '</ul>');
      continue;
    }

    // Ordered list
    if (/^\s*\d+[.)]\s/.test(line)) {
      const items = [];
      while (i < lines.length && /^\s*\d+[.)]\s/.test(lines[i])) { items.push(lines[i].replace(/^\s*\d+[.)]\s+/, '')); i++; }
      out.push('<ol>' + items.map(t => '<li>' + _mdInline(t) + '</li>').join('') + '</ol>');
      continue;
    }

    // Paragraph: collect consecutive non-special lines
    const pLines = [];
    while (i < lines.length && lines[i].trim() && !/^(#{1,6}\s|[-*_]{3,}$|>\s?|\s*[-*+]\s|\s*\d+[.)]\s)/.test(lines[i]) && !(i + 1 < lines.length && /^[\s|:-]+$/.test(lines[i + 1]) && lines[i + 1].includes('---') && /\|/.test(lines[i]))) {
      pLines.push(lines[i]); i++;
    }
    if (pLines.length) out.push('<p>' + _mdInline(pLines.join('\n')).replace(/\n/g, '<br>') + '</p>');
  }

  return out.join('');
}

function renderMarkdown(src) {
  if (!src) return '';
  const parts = [];
  let cursor = 0;
  const re = /```(\w*)\n?([\s\S]*?)```/g;
  let m;
  while ((m = re.exec(src)) !== null) {
    if (m.index > cursor) parts.push({ t: 0, v: src.slice(cursor, m.index) });
    parts.push({ t: 1, lang: m[1], v: m[2].replace(/\n$/, '') });
    cursor = m.index + m[0].length;
  }
  if (cursor < src.length) parts.push({ t: 0, v: src.slice(cursor) });
  return parts.map(p => p.t === 1
    ? '<pre><code' + (p.lang ? ' class="language-' + _mdEsc(p.lang) + '"' : '') + '>' + _mdEsc(p.v) + '</code></pre>'
    : _mdBlocks(p.v)
  ).join('');
}

function tryRenderInlineImages(text) {
  // Try parsing the entire text as a JSON image payload
  try {
    const parsed = JSON.parse(text.trim());
    if (parsed && parsed.data_b64 && parsed.mime && parsed.mime.startsWith("image/")) {
      return `<img src="data:${parsed.mime};base64,${parsed.data_b64}" style="max-width:100%;border-radius:8px;margin:8px 0;" alt="screen capture" />`;
    }
  } catch {}

  // Also check if JSON image payload is embedded in surrounding text
  const jsonMatch = text.match(/\{[\s\S]*"data_b64"\s*:[\s\S]*\}/);
  if (jsonMatch) {
    try {
      const parsed = JSON.parse(jsonMatch[0]);
      if (parsed && parsed.data_b64 && parsed.mime && parsed.mime.startsWith("image/")) {
        const idx = text.indexOf(jsonMatch[0]);
        const before = text.slice(0, idx);
        const after = text.slice(idx + jsonMatch[0].length);
        const imgHtml = `<img src="data:${parsed.mime};base64,${parsed.data_b64}" style="max-width:100%;border-radius:8px;margin:8px 0;" alt="screen capture" />`;
        return (before ? `<p>${escapeHtml(before)}</p>` : '') + imgHtml + (after ? `<p>${escapeHtml(after)}</p>` : '');
      }
    } catch {}
  }

  return null;
}

function appendMessage(role, text, isError = false) {
  const node = document.createElement("div");
  node.className = `msg ${role}${isError ? " error" : ""}`;
  if (role === "agent" && !isError) {
    const imgHtml = tryRenderInlineImages(text);
    if (imgHtml) {
      node.innerHTML = imgHtml;
    } else {
      node.innerHTML = renderMarkdown(text);
    }
  } else {
    node.textContent = text;
  }
  messages.appendChild(node);
  chatScroll.scrollTop = chatScroll.scrollHeight;
}

function showThinkingIndicator() {
  hideThinkingIndicator();
  const el = document.createElement("div");
  el.className = "thinking-indicator";
  el.id = "thinking-indicator";
  for (let i = 0; i < 3; i++) {
    const dot = document.createElement("span");
    dot.className = "thinking-dot";
    el.appendChild(dot);
  }
  messages.appendChild(el);
  chatScroll.scrollTop = chatScroll.scrollHeight;
}

function hideThinkingIndicator() {
  const el = document.getElementById("thinking-indicator");
  if (el) el.remove();
}

function showToast(text, variant = "default", durationMs = 3000) {
  const el = document.createElement("div");
  el.className = `toast${variant !== "default" ? ` toast-${variant}` : ""}`;
  el.textContent = text;
  toastContainer.appendChild(el);

  if (durationMs > 0) {
    setTimeout(() => {
      el.classList.add("toast-out");
      el.addEventListener("animationend", () => el.remove());
    }, durationMs);
  }
}

function clearMessages() {
  messages.textContent = "";
}

function renderHistory(historyMessages) {
  clearMessages();
  if (!Array.isArray(historyMessages) || historyMessages.length === 0) {
    return;
  }
  for (const msg of historyMessages) {
    const role = msg.role === "user" ? "user" : "agent";
    // Skip empty assistant/tool messages (e.g. tool-call-only assistant turns)
    if (!msg.content) {
      continue;
    }
    appendMessage(role, msg.content, false);
  }
}

async function authenticate(token) {
  const resp = await fetch("/auth", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ token, client_id: clientId }),
  });
  const data = await resp.json();
  if (!data.success) throw new Error(data.error || "Authentication failed");
  localStorage.setItem(SESSION_TOKEN_KEY, data.session_token);
  if (data.client_id) {
    clientId = data.client_id;
    localStorage.setItem(STORAGE_KEY, clientId);
  }
  return data;
}

function wsUrl() {
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  const client = encodeURIComponent(clientId);
  const session = encodeURIComponent(sessionId);
  const sessionToken = localStorage.getItem(SESSION_TOKEN_KEY) || "";
  const rawToken = (localStorage.getItem(TOKEN_STORAGE_KEY) || tokenInput.value || "").trim();
  if (sessionToken) {
    const params = new URLSearchParams({
      session_token: sessionToken,
      client_id: clientId,
      session_id: sessionId,
    });
    // Always include raw token as fallback for stale session_token recovery
    if (rawToken) params.set("token", rawToken);
    return `${protocol}://${location.host}/ws?${params}`;
  }
  // Fallback: raw token parameter (legacy or direct connect)
  const token = encodeURIComponent(rawToken);
  return `${protocol}://${location.host}/ws?token=${token}&client_id=${client}&session_id=${session}`;
}

function generateMessageId() {
  if (window.crypto && typeof window.crypto.randomUUID === "function") {
    return window.crypto.randomUUID();
  }
  return `m-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function clearPendingMessage(messageId) {
  const timeoutId = pendingById.get(messageId);
  if (timeoutId !== undefined) {
    clearTimeout(timeoutId);
    pendingById.delete(messageId);
  }
}

function resetPending() {
  for (const timeoutId of pendingById.values()) {
    clearTimeout(timeoutId);
  }
  pendingById.clear();
  pendingQueue = [];
  setRequestStatus("Idle");
  hideThinkingIndicator();
  hideStopButton();
}

function beginPending(messageId) {
  setRequestStatus("Sending...");
  const timeoutId = window.setTimeout(() => {
    if (!pendingById.has(messageId)) {
      return;
    }
    setRequestStatus("Response delayed. Check connection status.");
    appendMessage("agent", "Response is delayed. Still processing, or connection may be unstable.", true);
  }, RESPONSE_TIMEOUT_MS);

  pendingById.set(messageId, timeoutId);
  pendingQueue.push(messageId);
  showThinkingIndicator();
  showStopButton();
}

function markAcked(messageId) {
  if (messageId && pendingById.has(messageId)) {
    setRequestStatus("Queued. Waiting for response...");
    return;
  }
  if (pendingById.size > 0) {
    setRequestStatus("Queued. Waiting for response...");
  }
}

function resolvePendingForResponse() {
  while (pendingQueue.length > 0) {
    const next = pendingQueue.shift();
    if (!pendingById.has(next)) {
      continue;
    }
    clearPendingMessage(next);
    break;
  }

  if (pendingById.size === 0) {
    setRequestStatus("Idle");
  }
  hideThinkingIndicator();
  hideStopButton();
}

function failPending(messageId, errorText) {
  if (messageId) {
    clearPendingMessage(messageId);
  }
  if (pendingById.size === 0) {
    setRequestStatus("Idle");
  }
  hideThinkingIndicator();
  hideStopButton();
  appendMessage("agent", errorText || "Failed to queue message.", true);
}

function cancelGeneration() {
  if (pendingById.size === 0) return; // nothing to cancel
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify({ type: "cancel_generation" }));
  }
  resetPending();
  appendMessage("agent", "\u23f9 Generation stopped.", false);
  hideStopButton();
}

function showStopButton() {
  if (stopBtn) stopBtn.classList.remove("hidden");
}

function hideStopButton() {
  if (stopBtn) stopBtn.classList.add("hidden");
}

function sendSessionsRequest() {
  if (!socket || socket.readyState !== WebSocket.OPEN || !isAuthenticated) {
    return;
  }
  try {
    socket.send(JSON.stringify({ type: "sessions_request" }));
  } catch {
    // Ignore request refresh failures and retry on next interval.
  }
}

function stopSessionRefresh() {
  if (sessionRefreshTimer !== null) {
    clearInterval(sessionRefreshTimer);
    sessionRefreshTimer = null;
  }
}

function startSessionRefresh() {
  stopSessionRefresh();
  sessionRefreshTimer = setInterval(() => {
    sendSessionsRequest();
  }, SESSION_REFRESH_INTERVAL_MS);
}

function cancelReconnect() {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
}

function scheduleReconnect() {
  cancelReconnect();
  if (reconnectAttempts >= RECONNECT_MAX_ATTEMPTS) {
    setRequestStatus("Reconnect failed after max attempts");
    showToast("Unable to reconnect. Please refresh the page.", "error", 0);
    wasAuthenticated = false;
    setAuthMode(false);
    return;
  }
  const delay = Math.min(RECONNECT_BASE_MS * Math.pow(2, reconnectAttempts), RECONNECT_MAX_MS);
  reconnectAttempts += 1;
  setRequestStatus(`Reconnecting in ${Math.round(delay / 1000)}s (attempt ${reconnectAttempts}/${RECONNECT_MAX_ATTEMPTS})...`);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connect();
  }, delay);
}

async function connect() {
  if (isConnecting) {
    return;
  }
  if (socket && socket.readyState === WebSocket.OPEN) {
    return;
  }

  isConnecting = true;
  cancelReconnect();

  stopSessionRefresh();
  isAuthenticated = false;

  // Close lingering socket before opening a new one
  if (socket && socket.readyState !== WebSocket.CLOSED) {
    try {
      socket.close();
    } catch {
      // Ignore close errors on stale socket.
    }
    socket = null;
  }

  // Authenticate via POST /auth — always re-auth if we have a raw token
  // This handles server restarts (stale session_token) and fresh connections.
  const token = tokenInput.value.trim();
  if (token) {
    try {
      setRequestStatus("Authenticating...");
      await authenticate(token);
    } catch (err) {
      setRequestStatus("Authentication failed");
      showToast(err.message || "Authentication failed.", "error");
      isConnecting = false;
      setAuthMode(false);
      return;
    }
  }

  socket = new WebSocket(wsUrl());

  socket.addEventListener("open", () => {
    isConnecting = false;
    updateAuthUI(true);
    setRequestStatus("Connected. Waiting for authentication...");
    showToast("Connected", "success");
  });

  socket.addEventListener("close", () => {
    isConnecting = false;
    updateAuthUI(false);
    resetPending();
    stopSessionRefresh();

    if (isSwitchingSession) {
      setRequestStatus("Reconnecting with selected session...");
      isSwitchingSession = false;
      if (pendingSessionReconnect) {
        pendingSessionReconnect = false;
        clearMessages();
        connect();
      }
      return;
    }

    if (wasAuthenticated) {
      setRequestStatus("Disconnected. Reconnecting...");
      showToast("Disconnected. Reconnecting...", "error");
      scheduleReconnect();
      return;
    }

    if (!isAuthenticated) {
      localStorage.removeItem(SESSION_TOKEN_KEY);
      // If we have a saved raw token, auto-retry (stale session_token recovery)
      const savedRawToken = (localStorage.getItem(TOKEN_STORAGE_KEY) || "").trim();
      if (savedRawToken && authRetryCount < 2) {
        authRetryCount += 1;
        setRequestStatus("Re-authenticating...");
        connect();
        return;
      }
      setAuthMode(false);
      setRequestStatus("Disconnected");
      return;
    }

    setRequestStatus("Disconnected");
    showToast("Disconnected", "error");
  });

  socket.addEventListener("error", () => {
    isConnecting = false;
    updateAuthUI(false);
    setRequestStatus("Connection error");
    stopSessionRefresh();
    if (!isAuthenticated && !isSwitchingSession && !wasAuthenticated) {
      setAuthMode(false);
    }
    showToast("Connection error", "error");
  });

  socket.addEventListener("message", (event) => {
    let payload;
    try {
      payload = JSON.parse(event.data);
    } catch {
      appendMessage("agent", "Received invalid JSON message.", true);
      return;
    }

    if (payload.type === "auth_result") {
      if (!payload.success) {
        wasAuthenticated = false;
        cancelReconnect();
        stopSessionRefresh();
        if (payload.error && payload.error.toLowerCase().includes("invalid token")) {
          localStorage.removeItem(TOKEN_STORAGE_KEY);
          tokenInput.value = "";
        }
        localStorage.removeItem(SESSION_TOKEN_KEY);
        setAuthMode(false);
        setRequestStatus("Authentication failed");
        showToast(payload.error || "Authentication failed.", "error");
        return;
      }

      if (payload.client_id) {
        clientId = payload.client_id;
        localStorage.setItem(STORAGE_KEY, clientId);
      }
      const resolvedSession = (
        payload.session_id ||
        sessionId ||
        (payload.client_id ? `web:${payload.client_id}` : "")
      ).trim();
      if (resolvedSession) {
        sessionId = resolvedSession;
      }
      setSession(resolvedSession);

      const token = tokenInput.value.trim();
      if (token) {
        localStorage.setItem(TOKEN_STORAGE_KEY, token);
      }
      wasAuthenticated = true;
      authRetryCount = 0;
      reconnectAttempts = 0;
      cancelReconnect();
      setAuthMode(true);
      setSelectedModel(payload.provider, payload.model);
      setRequestStatus("Idle");
      sendSessionsRequest();
      startSessionRefresh();
      requestModelList();
      providerAuth.requestProviders();
      return;
    }

    if (payload.type === "sessions_list") {
      serverSessions = normalizeServerSessions(payload.sessions);
      renderSessionList();
      return;
    }

    if (payload.type === "message_ack") {
      if (payload.session_id) {
        setSession(payload.session_id);
      }
      markAcked(payload.message_id || null);
      return;
    }

    if (payload.type === "message_error") {
      failPending(payload.message_id || null, payload.error || "Failed to queue message.");
      return;
    }

    if (payload.type === "response") {
      resolvePendingForResponse();
      appendMessage("agent", payload.content || "", !!payload.is_error);
      return;
    }

    if (payload.type === "pong") {
      return;
    }

    if (payload.type === "session_history") {
      renderHistory(payload.messages || []);
      return;
    }

    if (payload.type === "model_list") {
      availableModels = payload.models || [];
      renderModelDropdown();
      return;
    }

    if (payload.type === "model_changed") {
      setSelectedModel(payload.provider, payload.model);
      showToast(`Model changed to ${payload.provider}/${payload.model}`, "success");
      return;
    }

    if (payload.type === "provider_auth_status") {
      providerAuth.handleProviderAuthStatus(payload);
      return;
    }

    if (payload.type === "provider_auth_url") {
      providerAuth.handleProviderAuthUrl(payload);
      return;
    }

    if (payload.type === "provider_auth_completed") {
      providerAuth.handleProviderAuthCompleted(payload);
      return;
    }

    if (payload.type === "generation_cancelled") {
      // Server confirmed cancel — already handled client-side
      return;
    }

    if (payload.type === "tool_approval_request") {
      // Pause all pending timeouts while waiting for user approval
      for (const [mid, tid] of pendingById.entries()) {
        clearTimeout(tid);
        pendingById.set(mid, -1);  // mark as paused (no active timer)
      }
      showInlineToolApproval(payload);
      return;
    }
  });
}

// ── Tool Approval (Inline Chat) ──
let _activeApprovalCallId = null;

function showInlineToolApproval(payload) {
  const { call_id, tool_name, arguments: args } = payload;

  // Hide the thinking dots — approval message replaces them
  hideThinkingIndicator();

  _activeApprovalCallId = call_id;

  const argsParts = [];
  if (typeof args === 'object' && args !== null) {
    Object.entries(args).forEach(([k, v]) => {
      const val = typeof v === 'string' ? v : JSON.stringify(v);
      argsParts.push(`${k}: ${val}`);
    });
  } else {
    argsParts.push(String(args));
  }
  const argsText = argsParts.join('\n');

  const node = document.createElement("div");
  node.className = "msg agent tool-approval";
  node.dataset.callId = call_id;
  node.innerHTML = `
    <div class="tool-approval-header">
      \uD83D\uDD27 도구 실행 승인 필요
      <span class="tool-approval-name">${escapeHtml(tool_name)}</span>
    </div>
    <div class="tool-approval-args"><span>${escapeHtml(argsText)}</span></div>
    <div class="tool-approval-actions">
      <button class="btn-approve">승인</button>
      <button class="btn-allow-session">이 세션 허용</button>
      <button class="btn-reject">거부</button>
    </div>
  `;

  messages.appendChild(node);
  chatScroll.scrollTop = chatScroll.scrollHeight;

  // Wire up button clicks
  const actionsDiv = node.querySelector(".tool-approval-actions");

  function respond(decision, label) {
    _activeApprovalCallId = null;
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({
        type: "tool_approval_response",
        call_id: call_id,
        decision: decision,
      }));
    }
    // Replace buttons with decided status
    actionsDiv.innerHTML = `<div class="tool-approval-decided">${label}</div>`;
    // Resume pending timeouts — agent will continue working
    showThinkingIndicator();
    for (const [mid, tid] of pendingById.entries()) {
      if (tid === -1) {
        const newTid = window.setTimeout(() => {
          if (!pendingById.has(mid)) return;
          setRequestStatus("Response delayed. Check connection status.");
          appendMessage("agent", "Response is delayed. Still processing, or connection may be unstable.", true);
        }, RESPONSE_TIMEOUT_MS);
        pendingById.set(mid, newTid);
      }
    }
  }

  actionsDiv.querySelector(".btn-approve").addEventListener("click", () =>
    respond("approve", "\u2705 승인됨")
  );
  actionsDiv.querySelector(".btn-allow-session").addEventListener("click", () =>
    respond("allow_for_session", "\u2705 이 세션에서 허용됨")
  );
  actionsDiv.querySelector(".btn-reject").addEventListener("click", () =>
    respond("reject", "\u274C 거부됨")
  );
}

// ESC rejects the currently active inline approval
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && _activeApprovalCallId) {
    const node = document.querySelector(
      `.tool-approval[data-call-id="${_activeApprovalCallId}"]`
    );
    if (node) {
      const actionsDiv = node.querySelector(".tool-approval-actions");
      if (actionsDiv && actionsDiv.querySelector("button")) {
        _activeApprovalCallId = null;
        if (socket && socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify({
            type: "tool_approval_response",
            call_id: node.dataset.callId,
            decision: "reject",
          }));
        }
        actionsDiv.innerHTML = `<div class="tool-approval-decided">\u274C 거부됨</div>`;
      }
    }
  }
});

function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}

function escAttr(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

connectButton.addEventListener("click", connect);
tokenInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    connect();
  }
});

composer.addEventListener("submit", (event) => {
  event.preventDefault();

  const text = messageInput.value.trim();
  if (!text) {
    return;
  }

  if (!socket || socket.readyState !== WebSocket.OPEN) {
    appendMessage("agent", "Not connected.", true);
    return;
  }

  const messageId = generateMessageId();
  beginPending(messageId);

  try {
    socket.send(
      JSON.stringify({
        type: "message",
        content: text,
        message_id: messageId,
      }),
    );
  } catch {
    failPending(messageId, "Failed to send message.");
    return;
  }

  appendMessage("user", text);
  messageInput.value = "";
});

updateAuthUI(false);
setSelectedModel("", "");
setSession(sessionId);
setRequestStatus("Idle");
renderSessionList();

// Sidebar starts open (set in HTML). Close it on mobile by default.
if (isMobile()) {
  closeSidebar();
}

// Parse session from URL: /s/SESSION_ID path or ?session= query param
const pathMatch = window.location.pathname.match(/^\/s\/(.+)/);
const urlParams = new URLSearchParams(window.location.search);
const urlSession = (
  (pathMatch ? decodeURIComponent(pathMatch[1]) : "") ||
  urlParams.get("session") ||
  ""
).trim();
if (urlSession) {
  sessionId = urlSession;
  localStorage.setItem(SESSION_STORAGE_KEY, urlSession);
  setSession(urlSession);
}

const savedToken = (localStorage.getItem(TOKEN_STORAGE_KEY) || "").trim();
if (savedToken) {
  tokenInput.value = savedToken;
  setAuthMode(false);
  connect();
} else {
  setAuthMode(false);
}


providerAuth.init();

// ── Stop Generation ──
if (stopBtn) {
  stopBtn.addEventListener("click", cancelGeneration);
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && pendingById.size > 0) {
    e.preventDefault();
    cancelGeneration();
  }
});