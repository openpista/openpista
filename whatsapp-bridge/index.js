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

// ── Main ───────────────────────────────────────────────────

async function start() {
  const { state, saveCreds } = await useMultiFileAuthState(
    path.join(sessionDir, "auth")
  );

  const { version } = await fetchLatestBaileysVersion();

  const sock = makeWASocket({
    version,
    auth: {
      creds: state.creds,
      keys: makeCacheableSignalKeyStore(state.keys, logger),
    },
    logger,
    printQRInTerminal: false, // We handle QR ourselves
    generateHighQualityLinkPreview: false,
    syncFullHistory: false,
  });

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
      emit({ type: "connected", phone, name });
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
  sock.ev.on("messages.upsert", ({ messages }) => {
    for (const msg of messages) {
      // Skip status broadcasts, reactions, and our own messages
      if (msg.key.remoteJid === "status@broadcast") continue;
      if (msg.key.fromMe) continue;
      if (msg.message?.reactionMessage) continue;

      const text =
        msg.message?.conversation ||
        msg.message?.extendedTextMessage?.text ||
        null;

      if (!text) continue; // Skip non-text messages for now

      const from = msg.key.remoteJid?.split("@")[0] || "";
      const timestamp = msg.messageTimestamp
        ? Number(msg.messageTimestamp)
        : undefined;

      emit({ type: "message", from, text, timestamp });
    }
  });

  // ── Stdin command handler ───────────────────────────────
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

    switch (cmd.type) {
      case "send": {
        const jid = cmd.to.includes("@") ? cmd.to : `${cmd.to}@s.whatsapp.net`;
        try {
          await sock.sendMessage(jid, { text: cmd.text });
        } catch (e) {
          emit({
            type: "error",
            message: `Failed to send message to ${cmd.to}: ${e.message}`,
          });
        }
        break;
      }

      case "disconnect": {
        logger.info("Disconnect command received");
        await sock.logout().catch(() => {});
        process.exit(0);
        break;
      }

      default:
        emit({ type: "error", message: `Unknown command type: ${cmd.type}` });
    }
  });

  rl.on("close", () => {
    logger.info("stdin closed, shutting down");
    sock.end(undefined);
    process.exit(0);
  });
}

// ── Entrypoint ─────────────────────────────────────────────

start().catch((err) => {
  emit({ type: "error", message: `Bridge startup failed: ${err.message}` });
  process.exit(1);
});
