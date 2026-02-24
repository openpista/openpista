const tokenInput = document.getElementById("token");
const connectButton = document.getElementById("connect");
const statusBadge = document.getElementById("status");
const messages = document.getElementById("messages");
const composer = document.getElementById("composer");
const messageInput = document.getElementById("message");

const STORAGE_KEY = "openpista:web:client_id";

let socket = null;
let clientId = localStorage.getItem(STORAGE_KEY) || "";

function setStatus(online) {
  statusBadge.textContent = online ? "Online" : "Offline";
  statusBadge.classList.toggle("online", online);
  statusBadge.classList.toggle("offline", !online);
}

function appendMessage(role, text, isError = false) {
  const node = document.createElement("div");
  node.className = `msg ${role}${isError ? " error" : ""}`;
  node.textContent = text;
  messages.appendChild(node);
  messages.scrollTop = messages.scrollHeight;
}

function wsUrl() {
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  const token = encodeURIComponent(tokenInput.value.trim());
  const client = encodeURIComponent(clientId);
  return `${protocol}://${location.host}/ws?token=${token}&client_id=${client}`;
}

function connect() {
  if (socket && socket.readyState === WebSocket.OPEN) {
    return;
  }

  socket = new WebSocket(wsUrl());

  socket.addEventListener("open", () => {
    setStatus(true);
    appendMessage("agent", "Connected.");
  });

  socket.addEventListener("close", () => {
    setStatus(false);
    appendMessage("agent", "Disconnected.");
  });

  socket.addEventListener("error", () => {
    setStatus(false);
    appendMessage("agent", "Connection error.", true);
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
        appendMessage("agent", payload.error || "Authentication failed.", true);
        return;
      }

      if (payload.client_id) {
        clientId = payload.client_id;
        localStorage.setItem(STORAGE_KEY, clientId);
      }
      return;
    }

    if (payload.type === "response") {
      appendMessage("agent", payload.content || "", !!payload.is_error);
      return;
    }

    if (payload.type === "pong") {
      return;
    }
  });
}

connectButton.addEventListener("click", connect);

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

  socket.send(
    JSON.stringify({
      type: "message",
      content: text,
    }),
  );
  appendMessage("user", text);
  messageInput.value = "";
});

setStatus(false);
