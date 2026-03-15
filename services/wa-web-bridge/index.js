const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const express = require("express");
const TOML = require("toml");
const qrcode = require("qrcode-terminal");
const QRCode = require("qrcode");
const pino = require("pino");
const {
  default: makeWASocket,
  DisconnectReason,
  useMultiFileAuthState,
  fetchLatestBaileysVersion,
  downloadMediaMessage,
} = require("@whiskeysockets/baileys");

const log = pino({ level: process.env.WA_WEB_LOG_LEVEL || "info" });

function loadConfig() {
  const workspaceRoot = process.cwd();
  const cfgPath = path.join(workspaceRoot, "configs", "config.toml");
  const raw = fs.readFileSync(cfgPath, "utf8");
  const baseCfg = TOML.parse(raw);
  const waSplitPath = path.join(workspaceRoot, "configs", "channels", "whatsapp.toml");
  let splitCfg = {};
  if (fs.existsSync(waSplitPath)) {
    splitCfg = TOML.parse(fs.readFileSync(waSplitPath, "utf8"));
  }
  const waWebSplitPath = path.join(workspaceRoot, "configs", "channels", "whatsapp-web.toml");
  let waWebCfg = {};
  if (fs.existsSync(waWebSplitPath)) {
    waWebCfg = TOML.parse(fs.readFileSync(waWebSplitPath, "utf8"));
  }
  const cfg = { ...baseCfg, ...splitCfg, ...waWebCfg };
  const serverListen = String(cfg?.server?.listen || "127.0.0.1:8088");
  const clawdBaseUrl = `http://${serverListen}`;
  const ww = cfg?.whatsapp_web || {};
  const waCloud = cfg?.whatsapp || {};

  return {
    workspaceRoot,
    clawdBaseUrl,
    enabled: !!ww.enabled,
    bridgeListen: String(ww.bridge_listen || "127.0.0.1:8092"),
    authDir: path.join(workspaceRoot, String(ww.auth_dir || "data/wa-web-auth")),
    quickResultWaitSeconds: Number(ww.quick_result_wait_seconds || 3),
    allowlist: new Set((ww.allowlist || []).map((v) => String(v).trim()).filter(Boolean)),
    admins: new Set((ww.admins || []).map((v) => String(v).trim()).filter(Boolean)),
    imageInboxDir: path.join(workspaceRoot, String(waCloud.image_inbox_dir || "image/upload")),
    audioInboxDir: path.join(workspaceRoot, String(waCloud.audio_inbox_dir || "audio/upload")),
  };
}

const cfg = loadConfig();
let sock = null;
const inboundDedup = new Map();
const DEDUP_WINDOW_MS = 10 * 60 * 1000;
const waLoginState = {
  connected: false,
  qrRaw: null,
  qrDataUrl: null,
  lastUpdateTs: Date.now(),
  lastError: null,
};

function cleanupDedup(now = Date.now()) {
  for (const [k, ts] of inboundDedup.entries()) {
    if (now - ts > DEDUP_WINDOW_MS) {
      inboundDedup.delete(k);
    }
  }
}

function dedupInboundKey(msg) {
  const id = String(msg?.key?.id || "").trim();
  if (id) return `wa_web_msg:${id}`;
  const jid = normalizeJid(msg?.key?.remoteJid);
  const text = extractTextContent(msg?.message || {});
  const type = Object.keys(msg?.message || {}).sort().join(",");
  return `wa_web_fallback:${jid}:${type}:${text}`;
}

function shouldProcessInbound(msg) {
  const key = dedupInboundKey(msg);
  if (!key) return true;
  const now = Date.now();
  cleanupDedup(now);
  const last = inboundDedup.get(key);
  if (typeof last === "number" && now - last <= DEDUP_WINDOW_MS) {
    return false;
  }
  inboundDedup.set(key, now);
  return true;
}

function stableUserId(input) {
  const digest = crypto.createHash("sha256").update(input).digest();
  const n = digest.readBigUInt64BE(0) & BigInt("0x7fffffffffffffff");
  const maxSafe = BigInt(Number.MAX_SAFE_INTEGER);
  return Number(n % maxSafe);
}

function isAllowed(jid) {
  if (cfg.allowlist.size === 0 && cfg.admins.size === 0) return true;
  return cfg.allowlist.has(jid) || cfg.admins.has(jid);
}

function normalizeJid(jid) {
  if (!jid) return "";
  return String(jid).trim();
}

function extractTextContent(message) {
  if (!message) return "";
  return (
    message.conversation ||
    message.extendedTextMessage?.text ||
    message.imageMessage?.caption ||
    message.videoMessage?.caption ||
    ""
  ).trim();
}

function buildRelPath(absPath) {
  return path.relative(cfg.workspaceRoot, absPath).split(path.sep).join("/");
}

function ensureParentDir(filePath) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
}

function resetWaLoginState() {
  waLoginState.connected = false;
  waLoginState.qrRaw = null;
  waLoginState.qrDataUrl = null;
  waLoginState.lastUpdateTs = Date.now();
  waLoginState.lastError = null;
}

async function submitTask(jid, kind, payload) {
  const userId = stableUserId(jid);
  const body = {
    user_id: userId,
    chat_id: userId,
    channel: "whatsapp",
    external_user_id: jid,
    external_chat_id: jid,
    kind,
    payload: {
      adapter: "whatsapp_web",
      ...(payload || {}),
    },
  };
  const resp = await fetch(`${cfg.clawdBaseUrl}/v1/tasks`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await resp.text();
  if (!resp.ok) {
    throw new Error(`submit task http ${resp.status}: ${text}`);
  }
  const parsed = JSON.parse(text);
  if (!parsed.ok || !parsed.data?.task_id) {
    throw new Error(`submit task rejected: ${parsed.error || "unknown"}`);
  }
  return String(parsed.data.task_id);
}

async function queryTask(taskId) {
  const resp = await fetch(`${cfg.clawdBaseUrl}/v1/tasks/${taskId}`);
  const text = await resp.text();
  if (!resp.ok) {
    throw new Error(`query task http ${resp.status}: ${text}`);
  }
  const parsed = JSON.parse(text);
  if (!parsed.ok || !parsed.data) {
    throw new Error(`query task failed: ${parsed.error || "unknown"}`);
  }
  return parsed.data;
}

async function pollTaskResult(taskId, waitSeconds) {
  const pollMs = 500;
  const rounds = Math.max(1, Math.floor((waitSeconds * 1000) / pollMs));
  for (let i = 0; i < rounds; i += 1) {
    const task = await queryTask(taskId);
    if (task.status === "queued" || task.status === "running") {
      await new Promise((r) => setTimeout(r, pollMs));
      continue;
    }
    if (task.status === "succeeded") {
      return String(task.result_json?.text || "done");
    }
    throw new Error(task.error_text || `task status=${task.status}`);
  }
  throw new Error("task_result_wait_timeout");
}

function extractTokenPaths(answer, prefix) {
  return answer
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.startsWith(prefix))
    .map((line) => line.slice(prefix.length).trim().replace(/^['"`]+|['"`]+$/g, ""))
    .filter((p) => p.length > 0)
    .filter((p) => fs.existsSync(path.resolve(cfg.workspaceRoot, p)) || fs.existsSync(p))
    .map((p) => (path.isAbsolute(p) ? p : path.resolve(cfg.workspaceRoot, p)));
}

function stripTokens(answer) {
  return answer
    .split(/\r?\n/)
    .filter((line) => {
      const t = line.trimStart();
      return !(
        t.startsWith("IMAGE_FILE:") ||
        t.startsWith("FILE:") ||
        t.startsWith("VOICE_FILE:")
      );
    })
    .join("\n")
    .trim();
}

async function sendAnswer(jid, answer) {
  const text = stripTokens(answer);
  const imagePaths = extractTokenPaths(answer, "IMAGE_FILE:");
  const filePaths = extractTokenPaths(answer, "FILE:");
  const voicePaths = extractTokenPaths(answer, "VOICE_FILE:");

  if (text) {
    await sock.sendMessage(jid, { text });
  }
  for (const p of imagePaths) {
    await sock.sendMessage(jid, { image: { url: p } });
  }
  for (const p of filePaths) {
    await sock.sendMessage(jid, {
      document: { url: p },
      fileName: path.basename(p),
    });
  }
  for (const p of voicePaths) {
    await sock.sendMessage(jid, {
      audio: { url: p },
      ptt: true,
      mimetype: "audio/ogg; codecs=opus",
    });
  }
  if (!text && imagePaths.length === 0 && filePaths.length === 0 && voicePaths.length === 0) {
    await sock.sendMessage(jid, { text: answer });
  }
}

async function runTaskFlow(jid, kind, payload, quickWait = cfg.quickResultWaitSeconds) {
  const taskId = await submitTask(jid, kind, payload);
  try {
    const out = await pollTaskResult(taskId, quickWait);
    await sendAnswer(jid, out);
  } catch (err) {
    if (String(err.message || err) === "task_result_wait_timeout") {
      setTimeout(async () => {
        try {
          const finalOut = await pollTaskResult(taskId, 600);
          await sendAnswer(jid, finalOut);
        } catch (e) {
          await sock.sendMessage(jid, { text: `处理失败: ${String(e.message || e)}` });
        }
      }, 200);
      return;
    }
    await sock.sendMessage(jid, { text: `处理失败: ${String(err.message || err)}` });
  }
}

function getMediaType(message) {
  if (message?.imageMessage) return "image";
  if (message?.audioMessage) return "audio";
  if (message?.documentMessage && String(message.documentMessage?.mimetype || "").startsWith("image/")) {
    return "image";
  }
  return "";
}

function pickExtFromMime(mime, fallback) {
  const m = String(mime || "").toLowerCase();
  if (m.includes("jpeg")) return "jpg";
  if (m.includes("png")) return "png";
  if (m.includes("webp")) return "webp";
  if (m.includes("ogg")) return "ogg";
  if (m.includes("mp3") || m.includes("mpeg")) return "mp3";
  if (m.includes("wav")) return "wav";
  return fallback;
}

async function saveInboundMedia(message, jid, userId) {
  const mediaType = getMediaType(message);
  if (!mediaType) return null;
  const ts = Math.floor(Date.now() / 1000);
  const baseDir = mediaType === "image" ? cfg.imageInboxDir : cfg.audioInboxDir;
  const mime =
    message?.imageMessage?.mimetype ||
    message?.audioMessage?.mimetype ||
    message?.documentMessage?.mimetype ||
    "";
  const ext = pickExtFromMime(mime, mediaType === "image" ? "jpg" : "ogg");
  const safe = jid.replace(/[^a-zA-Z0-9]/g, "");
  const absPath = path.join(baseDir, `waweb_${safe}_${userId}_${ts}.${ext}`);
  ensureParentDir(absPath);

  const buffer = await downloadMediaMessage(
    { message },
    "buffer",
    {},
    { logger: log, reuploadRequest: sock.updateMediaMessage }
  );
  fs.writeFileSync(absPath, buffer);
  return { mediaType, absPath, relPath: buildRelPath(absPath) };
}

async function handleInboundMessage(msg) {
  if (!msg?.key || msg.key.fromMe) return;
  if (!shouldProcessInbound(msg)) {
    log.info({ id: msg?.key?.id, jid: msg?.key?.remoteJid }, "skip duplicated inbound message");
    return;
  }
  const jid = normalizeJid(msg.key.remoteJid);
  if (!jid) return;
  if (!isAllowed(jid)) {
    await sock.sendMessage(jid, { text: "Unauthorized user" });
    return;
  }

  const text = extractTextContent(msg.message);
  if (text.startsWith("/run")) {
    const rest = text.slice(4).trim();
    const firstSpace = rest.indexOf(" ");
    const skill = (firstSpace >= 0 ? rest.slice(0, firstSpace) : rest).trim();
    const args = (firstSpace >= 0 ? rest.slice(firstSpace + 1) : "").trim();
    if (!skill) {
      await sock.sendMessage(jid, { text: "Usage: /run <skill_name> <args>" });
      return;
    }
    await runTaskFlow(jid, "run_skill", { skill_name: skill, args });
    return;
  }

  const userId = stableUserId(jid);
  const media = await saveInboundMedia(msg.message, jid, userId);
  if (media?.mediaType === "image") {
    await runTaskFlow(jid, "run_skill", {
      skill_name: "image_vision",
      args: {
        action: "describe",
        images: [{ path: media.relPath }],
        detail_level: "normal",
      },
    });
    return;
  }
  if (media?.mediaType === "audio") {
    await runTaskFlow(jid, "run_skill", {
      skill_name: "audio_transcribe",
      args: {
        audio: { path: media.relPath },
      },
    }, 120);
    return;
  }

  if (text) {
    await runTaskFlow(jid, "ask", { text, agent_mode: true });
  }
}

async function connectWhatsApp() {
  fs.mkdirSync(cfg.authDir, { recursive: true });
  const { state, saveCreds } = await useMultiFileAuthState(cfg.authDir);
  const { version } = await fetchLatestBaileysVersion();
  sock = makeWASocket({
    auth: state,
    version,
    logger: pino({ level: "silent" }),
    printQRInTerminal: false,
    syncFullHistory: false,
  });

  sock.ev.on("creds.update", saveCreds);
  sock.ev.on("connection.update", async (update) => {
    const { connection, lastDisconnect, qr } = update;
    if (qr) {
      waLoginState.connected = false;
      waLoginState.qrRaw = qr;
      waLoginState.lastUpdateTs = Date.now();
      waLoginState.lastError = null;
      try {
        waLoginState.qrDataUrl = await QRCode.toDataURL(qr, {
          width: 320,
          margin: 1,
          errorCorrectionLevel: "M",
        });
      } catch (err) {
        waLoginState.qrDataUrl = null;
        waLoginState.lastError = `render qr failed: ${String(err?.message || err)}`;
      }
      console.log("\n[wa-web-bridge] 请扫码登录 WhatsApp:");
      qrcode.generate(qr, { small: true });
    }
    if (connection === "open") {
      log.info("wa-web connected");
      waLoginState.connected = true;
      waLoginState.qrRaw = null;
      waLoginState.qrDataUrl = null;
      waLoginState.lastUpdateTs = Date.now();
      waLoginState.lastError = null;
    }
    if (connection === "close") {
      waLoginState.connected = false;
      waLoginState.lastUpdateTs = Date.now();
      const statusCode = lastDisconnect?.error?.output?.statusCode;
      const shouldReconnect = statusCode !== DisconnectReason.loggedOut;
      waLoginState.lastError = shouldReconnect
        ? `connection closed: status=${String(statusCode || "unknown")}`
        : "logged_out";
      log.warn({ statusCode, shouldReconnect }, "wa-web connection closed");
      if (shouldReconnect) {
        setTimeout(connectWhatsApp, 2000);
      } else {
        log.error("wa-web logged out, remove auth dir and login again");
      }
    }
  });

  sock.ev.on("messages.upsert", async ({ messages, type }) => {
    if (type !== "notify" || !Array.isArray(messages)) return;
    for (const m of messages) {
      try {
        await handleInboundMessage(m);
      } catch (err) {
        log.error({ err: String(err?.stack || err) }, "handle inbound failed");
      }
    }
  });
}

function startHttpServer() {
  const app = express();
  app.use(express.json({ limit: "1mb" }));

  app.get("/health", (_req, res) => {
    res.json({ ok: true, connected: waLoginState.connected, socket_ready: !!sock });
  });

  app.get("/v1/login-status", (_req, res) => {
    res.json({
      ok: true,
      connected: waLoginState.connected,
      qr_ready: Boolean(waLoginState.qrDataUrl),
      qr_data_url: waLoginState.qrDataUrl,
      last_update_ts: waLoginState.lastUpdateTs,
      last_error: waLoginState.lastError,
    });
  });

  app.post("/v1/send-text", async (req, res) => {
    try {
      const to = String(req.body?.to || "").trim();
      const text = String(req.body?.text || "").trim();
      if (!to || !text) {
        return res.status(400).json({ ok: false, error: "missing to/text" });
      }
      if (!sock) {
        return res.status(503).json({ ok: false, error: "wa socket not ready" });
      }
      await sock.sendMessage(to, { text });
      return res.json({ ok: true });
    } catch (err) {
      return res.status(500).json({ ok: false, error: String(err.message || err) });
    }
  });

  app.post("/v1/logout", async (_req, res) => {
    try {
      if (sock) {
        try {
          await sock.logout();
        } catch (err) {
          log.warn({ err: String(err?.message || err) }, "wa-web logout error");
        }
      }
      sock = null;
      resetWaLoginState();
      // Force next login to require QR by removing local auth cache.
      try {
        fs.rmSync(cfg.authDir, { recursive: true, force: true });
      } catch (err) {
        log.warn({ err: String(err?.message || err) }, "remove auth dir failed");
      }
      fs.mkdirSync(cfg.authDir, { recursive: true });
      setTimeout(() => {
        connectWhatsApp().catch((err) => {
          waLoginState.lastError = `reconnect after logout failed: ${String(err?.message || err)}`;
          waLoginState.lastUpdateTs = Date.now();
          log.error({ err: String(err?.stack || err) }, "reconnect after logout failed");
        });
      }, 500);
      return res.json({ ok: true });
    } catch (err) {
      return res.status(500).json({ ok: false, error: String(err?.message || err) });
    }
  });

  const [host, portRaw] = cfg.bridgeListen.split(":");
  const port = Number(portRaw || 8092);
  app.listen(port, host || "127.0.0.1", () => {
    log.info(`wa-web bridge listening on ${host || "127.0.0.1"}:${port}`);
  });
}

async function main() {
  if (!cfg.enabled) {
    log.warn("whatsapp_web.enabled=false, bridge exits");
    process.exit(0);
  }
  startHttpServer();
  await connectWhatsApp();
}

main().catch((err) => {
  log.error({ err: String(err?.stack || err) }, "wa-web bridge fatal");
  process.exit(1);
});
