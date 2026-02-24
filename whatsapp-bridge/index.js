#!/usr/bin/env node
// ──────────────────────────────────────────────────────────────
// openpista WhatsApp Web Bridge
//
// Spawned as a subprocess by the Rust WhatsApp adapter.
// Communicates via JSON lines over stdin (commands) / stdout (events).
//
// Usage:  node index.js <session_dir>
//
// Protocol (each message is a single JSON line):
//
//   Rust → Bridge (stdin):
//     { "type": "send", "to": "15551234567", "text": "Hello" }
//     { "type": "disconnect" }
//
//   Bridge → Rust (stdout):
//     { "type": "qr",           "data": "2@ABC..." }
//     { "type": "connected",    "phone": "15551234567", "name": "John" }
//     { "type": "message",      "from": "15551234567", "text": "Hi", "timestamp": 1700000000 }
//     { "type": "disconnected", "reason": "logged out" }
//     { "type": "error",        "message": "..." }
// ──────────────────────────────────────────────────────────────

"use strict";

const {
  default: makeWASocket,
  useMultiFileAuthState,
  DisconnectReason,
  fetchLatestBaileysVersion,
  makeCacheableSignalKeyStore,
  Browsers,
} = require("@whiskeysockets/baileys");
const pino = require("pino");
const { createInterface } = require("readline");
const path = require("path");
const fs = require("fs");

// ── Helpers ────────────────────────────────────────────────

/** Send a JSON event line to Rust (stdout). */
function emit(event) {
  process.stdout.write(JSON.stringify(event) + "\n");
}

/** Log to stderr so it doesn't interfere with the JSON protocol on stdout. */
const logger = pino(
  { level: process.env.LOG_LEVEL || "warn" },
  pino.destination(2) // stderr
);

// ── Session directory ──────────────────────────────────────

const sessionDir = process.argv[2];
if (!sessionDir) {
  emit({ type: "error", message: "Usage: node index.js <session_dir>" });
  process.exit(1);
}

// Ensure session directory exists
fs.mkdirSync(sessionDir, { recursive: true });

// ── Module-level state ─────────────────────────────────────

let cachedVersion;
async function getVersion() {
  if (!cachedVersion) {
    const { version } = await fetchLatestBaileysVersion();
    cachedVersion = version;
  }
  return cachedVersion;
}

const msgRetryCounterCache = {
  _cache: new Map(),
  get(key) { return this._cache.get(key); },
  set(key, val) { this._cache.set(key, val); },
  del(key) { this._cache.delete(key); },
};

let currentSock = null;
let shuttingDown = false;
let connectedPhone = ""; // own phone number, set on connect — used to resolve @lid reply JIDs
const sentMessageIds = new Set(); // Track bot-sent message IDs to prevent loops

// Message store for WhatsApp retry mechanism (prevents "암호화 대기중").
// When the phone fails to decrypt a message it sends a retry request;
// Baileys then calls getMessage() to re-encrypt and re-send the payload.
const sentMsgStore = new Map(); // msgId → proto message object

const rl = createInterface({ input: process.stdin });

rl.on("line", async (line) => {
  line = line.trim();
  if (!line) return;

  let cmd;
  try {
    cmd = JSON.parse(line);
  } catch (e) {
    emit({ type: "error", message: `Invalid JSON command: ${e.message}` });
    return;
  }

  if (!currentSock) {
    emit({ type: "error", message: "Socket not connected yet" });
    return;
  }

  switch (cmd.type) {
    case "send": {
      // @lid is receive-only in WhatsApp's multi-device protocol.
      // Replies must be sent to the real phone-number JID (@s.whatsapp.net).
      const resolvedTo = (cmd.to.endsWith("@lid") && connectedPhone)
        ? `${connectedPhone}@s.whatsapp.net`
        : cmd.to;
      const jid = resolvedTo.includes("@") ? resolvedTo : `${resolvedTo}@s.whatsapp.net`;
      try {
        // Force-refresh Signal session before every send to prevent "암호화 대기중".
        // assertSessions(jids, true) triggers a fresh SKDM (Sender-Key Distribution
        // Message) so the phone always has the current sender key before delivery.
        if (currentSock?.assertSessions) {
          await currentSock.assertSessions([jid], true);
        }
        const sent = await currentSock.sendMessage(jid, { text: cmd.text });
        // Track sent message ID to prevent self-chat infinite loops
        if (sent?.key?.id) {
          sentMessageIds.add(sent.key.id);
          // Also cache in sentMsgStore for phone retry / re-encryption
          if (sent.message) {
            sentMsgStore.set(sent.key.id, sent.message);
            setTimeout(() => sentMsgStore.delete(sent.key.id), 86_400_000);
          }
        }
      } catch (e) {
        emit({
          type: "error",
          message: `Failed to send message to ${cmd.to}: ${e.message}`,
        });
      }
      break;
    }

    case "disconnect": {
      await currentSock.logout().catch(() => {});
      process.exit(0);
      break;
    }

    case "shutdown": {
      shuttingDown = true;
      if (currentSock) {
        currentSock.ws.close();
      }
      setTimeout(() => process.exit(0), 2000);
      break;
    }

    default:
      emit({ type: "error", message: `Unknown command type: ${cmd.type}` });
  }
});

rl.on("close", () => {
  process.exit(0);
});

// ── Main ───────────────────────────────────────────────────

async function start() {
  const { state, saveCreds } = await useMultiFileAuthState(
    path.join(sessionDir, "auth")
  );

  const version = await getVersion();

  const sock = makeWASocket({
    version,
    auth: {
      creds: state.creds,
      keys: makeCacheableSignalKeyStore(state.keys, logger),
    },
    logger,
    browser: Browsers.macOS("Chrome"),
    printQRInTerminal: false,
    generateHighQualityLinkPreview: false,
    syncFullHistory: false,
    markOnlineOnConnect: true,
    defaultQueryTimeoutMs: 60_000,
    connectTimeoutMs: 60_000,
    keepAliveIntervalMs: 25_000,
    msgRetryCounterCache,
    // Baileys has a double-increment bug in the retry counter, so the effective
    // number of resends is floor(maxMsgRetryCount / 2).  Setting 10 → ~5 actual
    // resends, enough to survive the initial session-establishment window.
    maxMsgRetryCount: 10,
    // CRITICAL: lets Baileys re-encrypt messages on phone retry requests.
    // Without this callback "암호화 대기중" never resolves after a retry.
    getMessage: async (key) => {
      return sentMsgStore.get(key.id) || undefined;
    },
  });

  currentSock = sock;

  // ── QR code event ───────────────────────────────────────
  sock.ev.on("connection.update", (update) => {
    const { connection, lastDisconnect, qr } = update;

    if (qr) {
      emit({ type: "qr", data: qr });
    }

    if (connection === "open") {
      const me = sock.user;
      const phone = me?.id?.split(":")[0] || me?.id?.split("@")[0] || "";
      const name = me?.name || null;
      connectedPhone = phone; // store for @lid → @s.whatsapp.net resolution in send handler
      emit({ type: "connected", phone, name });
      // Mark as available so WhatsApp pushes messages to this device
      sock.sendPresenceUpdate("available").catch((e) => {
        logger.warn({ err: e.message }, "Failed to send presence update");
      });
      // Pre-warm Signal sessions for own devices to prevent "암호화 대기중".
      // We assert both the @s.whatsapp.net JID and (if known) the @lid JID so
      // that Baileys sends a fresh SKDM to the phone on first connect.
      setTimeout(async () => {
        try {
          const myJid = `${connectedPhone}@s.whatsapp.net`;
          const lidJid = sock.user?.lid || sock.authState?.creds?.me?.lid || null;
          const jidsToWarm = [myJid];
          if (lidJid && lidJid !== myJid) jidsToWarm.push(lidJid);
          if (sock?.assertSessions) {
            await sock.assertSessions(jidsToWarm, true);
            logger.info({ jidsToWarm }, "Signal sessions pre-warmed");
          }
        } catch (e) {
          logger.warn({ err: e.message }, "Failed to pre-warm Signal sessions");
        }
      }, 1000); // 1 s — session must be ready before the first retry storm (~2 s)
    }

    if (connection === "close") {
      const statusCode =
        lastDisconnect?.error?.output?.statusCode ||
        lastDisconnect?.error?.statusCode;
      const reason =
        lastDisconnect?.error?.message || `status ${statusCode}`;

      if (statusCode === DisconnectReason.loggedOut) {
        // Session invalidated — clean up auth and exit
        emit({ type: "disconnected", reason: "logged out" });
        const authDir = path.join(sessionDir, "auth");
        if (fs.existsSync(authDir)) {
          fs.rmSync(authDir, { recursive: true, force: true });
        }
        process.exit(0);
      } else {
        try { sock.ev.removeAllListeners(); } catch (_) {}
        // If shutting down gracefully, don't reconnect
        if (shuttingDown) {
          emit({ type: "disconnected", reason: "shutdown" });
          return;
        }
        // Transient disconnect — notify and attempt reconnect
        emit({ type: "disconnected", reason });
        logger.info({ statusCode, reason }, "Reconnecting...");
        setTimeout(() => start(), 3000);
      }
    }
  });

  // ── Persist credentials on update ───────────────────────
  sock.ev.on("creds.update", saveCreds);

  // ── Incoming messages ───────────────────────────────────
  sock.ev.on("messages.upsert", ({ messages, type }) => {
    // Only process new incoming messages, not history sync
    if (type !== "notify") {
      return;
    }
    // Resolve self JID for self-chat detection
    const selfPhone = sock.user?.id?.split(":")[0] || sock.user?.id?.split("@")[0] || "";
    const selfJid = selfPhone ? `${selfPhone}@s.whatsapp.net` : "";
    const selfLid = sock.user?.id || "";
    // Cache ALL messages for retry mechanism (must run before any filtering).
    // When the phone fails to decrypt it sends a retry; getMessage() returns
    // the cached proto so Baileys can re-encrypt and re-deliver.
    for (const msg of messages) {
      if (msg.key?.id && msg.message) {
        sentMsgStore.set(msg.key.id, msg.message);
        setTimeout(() => sentMsgStore.delete(msg.key.id), 86_400_000); // 24 h TTL
      }
    }

    for (const msg of messages) {
      // Skip status broadcasts and reactions
      if (msg.key.remoteJid === "status@broadcast") continue;
      if (msg.key.remoteJid?.endsWith("@broadcast")) continue;
      if (msg.message?.reactionMessage) continue;
      // Skip bot's own sent messages (prevent infinite loops)
      if (sentMessageIds.has(msg.key.id)) {
        sentMessageIds.delete(msg.key.id);
        continue;
      }
      const remoteJid = msg.key.remoteJid || "";
      const isFromMe = Boolean(msg.key.fromMe);
      // Newer WhatsApp versions use @lid (Linked ID) for self-chat
      // instead of @s.whatsapp.net — must check both formats
      const isSelfChat = isFromMe && (
        remoteJid === selfJid ||
        remoteJid === selfLid ||
        remoteJid.split("@")[0] === selfPhone ||
        remoteJid.endsWith("@lid")
      );
      // fromMe but NOT self-chat => outbound sync, skip
      if (isFromMe && !isSelfChat) {
        continue;
      }
      // Extract text content
      const text =
        msg.message?.conversation ||
        msg.message?.extendedTextMessage?.text ||
        null;
      if (!text) {
        continue;
      }
      const from = remoteJid; // preserve full JID (@lid, @s.whatsapp.net, etc.) for correct reply routing
      const timestamp = msg.messageTimestamp
        ? Number(msg.messageTimestamp)
        : undefined;
      emit({ type: "message", from, text, timestamp, selfChat: isSelfChat });
    }
  });
}

// ── Entrypoint ─────────────────────────────────────────────

start().catch((err) => {
  emit({ type: "error", message: `Bridge startup failed: ${err.message}` });
  process.exit(1);
});
