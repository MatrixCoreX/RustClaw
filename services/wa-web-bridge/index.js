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
const bridgeStartedAtMs = Date.now();
const WA_WEB_ADAPTER_MODE = "experimental_unofficial";
const WA_WEB_TRANSPORT = "baileys";
const PROACTIVE_DELIVERY_SOURCES = new Set(["scheduled_task", "proactive_notice"]);

function loadConfig() {
  const workspaceRoot = path.resolve(process.env.APP_WORKSPACE_ROOT || path.join(__dirname, "..", ".."));
  const cfgPath = path.resolve(process.env.APP_CONFIG_PATH || path.join(workspaceRoot, "configs", "config.toml"));
  const raw = fs.readFileSync(cfgPath, "utf8");
  const baseCfg = TOML.parse(raw);
  const channelConfigDir = path.resolve(
    process.env.APP_CHANNEL_CONFIG_DIR || path.join(workspaceRoot, "configs", "channels")
  );
  const waSplitPath = path.join(channelConfigDir, "whatsapp-cloud.toml");
  let splitCfg = {};
  if (fs.existsSync(waSplitPath)) {
    splitCfg = TOML.parse(fs.readFileSync(waSplitPath, "utf8"));
  }
  const waWebSplitPath = path.join(channelConfigDir, "whatsapp-web.toml");
  let waWebCfg = {};
  if (fs.existsSync(waWebSplitPath)) {
    waWebCfg = TOML.parse(fs.readFileSync(waWebSplitPath, "utf8"));
  }
  const webdSplitPath = path.join(channelConfigDir, "webd.toml");
  let webdCfg = {};
  if (fs.existsSync(webdSplitPath)) {
    webdCfg = TOML.parse(fs.readFileSync(webdSplitPath, "utf8"));
  }
  const cfg = { ...baseCfg, ...webdCfg, ...splitCfg, ...waWebCfg };
  const clawdBaseUrl = normalizeInternalApiBaseUrl(
    process.env.APP_INTERNAL_API_BASE_URL || cfg?.webd?.upstream
  );
  const ww = cfg?.whatsapp_web || {};
  const waCloud = cfg?.whatsapp || {};
  const language = String(ww.language || "en-US").trim() || "en-US";
  const configuredI18nPath = String(ww.i18n_path || "").trim();
  const languageI18nPath = path.join(workspaceRoot, "configs", "i18n", `whatsapp-webd.${language}.toml`);
  const i18nPath = configuredI18nPath
    ? path.resolve(workspaceRoot, configuredI18nPath)
    : languageI18nPath;
  let i18n = {};
  if (fs.existsSync(languageI18nPath)) {
    i18n = TOML.parse(fs.readFileSync(languageI18nPath, "utf8"))?.dict || {};
  } else if (fs.existsSync(i18nPath)) {
    i18n = TOML.parse(fs.readFileSync(i18nPath, "utf8"))?.dict || {};
  }

  return {
    workspaceRoot,
    clawdBaseUrl,
    enabled: !!ww.enabled,
    bridgeListen: String(ww.bridge_listen || "127.0.0.1:8092"),
    authDir: path.join(workspaceRoot, String(ww.auth_dir || "data/wa-web-auth")),
    quickResultWaitSeconds: Number(ww.quick_result_wait_seconds || 3),
    allowProactiveSend: ww.allow_proactive_send === true,
    allowlist: new Set((ww.allowlist || []).map((v) => String(v).trim()).filter(Boolean)),
    admins: new Set((ww.admins || []).map((v) => String(v).trim()).filter(Boolean)),
    i18n,
    imageInboxDir: path.join(workspaceRoot, String(waCloud.image_inbox_dir || "image/upload")),
    audioInboxDir: path.join(workspaceRoot, String(waCloud.audio_inbox_dir || "audio/upload")),
    artifactOutboxDir: path.join(
      workspaceRoot,
      String(ww.artifact_outbox_dir || ".agent-runtime/artifacts/channel-outbox/whatsapp-web")
    ),
    maxOutboundImageBytes: Number(ww.max_outbound_image_bytes || 100 * 1024 * 1024),
    maxOutboundVideoBytes: Number(ww.max_outbound_video_bytes || 100 * 1024 * 1024),
    maxOutboundAudioBytes: Number(ww.max_outbound_audio_bytes || 100 * 1024 * 1024),
    maxOutboundFileBytes: Number(ww.max_outbound_file_bytes || 2 * 1024 * 1024 * 1024),
  };
}

function parseListenAddress(raw, defaultPort) {
  const value = String(raw || "").trim();
  const ipv6 = value.match(/^\[([^\]]+)\](?::([0-9]+))?$/);
  if (ipv6) {
    return { host: ipv6[1], port: Number(ipv6[2] || defaultPort) };
  }
  const separator = value.lastIndexOf(":");
  if (separator > 0) {
    return { host: value.slice(0, separator), port: Number(value.slice(separator + 1) || defaultPort) };
  }
  return { host: value || "127.0.0.1", port: Number(defaultPort) };
}

function normalizeInternalApiBaseUrl(raw) {
  const value = String(raw || "").trim();
  if (!value) throw new Error("internal API base URL is missing");
  const parsed = new URL(value);
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error("internal API base URL must use http or https");
  }
  if (parsed.username || parsed.password) {
    throw new Error("internal API base URL must not contain credentials");
  }
  if ((parsed.pathname && parsed.pathname !== "/") || parsed.search || parsed.hash) {
    throw new Error("internal API base URL must not contain a path, query, or fragment");
  }
  return parsed.toString().replace(/\/$/, "");
}

function isLoopbackHost(host) {
  const normalized = String(host || "").trim().toLowerCase();
  return normalized === "127.0.0.1" || normalized === "localhost" || normalized === "::1";
}

const cfg = loadConfig();
let sock = null;
const inboundDedup = new Map();
const DEDUP_WINDOW_MS = 10 * 60 * 1000;
const outboundMessageIds = new Map();
const pendingOutboundMessages = new Map();
const OUTBOUND_DEDUP_WINDOW_MS = 10 * 60 * 1000;
const PENDING_OUTBOUND_WINDOW_MS = 30 * 1000;
const waLoginState = {
  phase: "starting",
  connected: false,
  qrRaw: null,
  qrDataUrl: null,
  lastUpdateTs: Date.now(),
  lastErrorCode: null,
  lastDiagnosticId: null,
};
const expectingKeyReply = new Set();

function adapterDiagnosticId(operation, material) {
  const digest = crypto
    .createHash("sha256")
    .update(`whatsapp_web\u0000${operation}\u0000${String(material || "unknown")}`)
    .digest("hex")
    .slice(0, 24);
  return `whatsapp-web:${digest}`;
}

function adapterError(errorCode, operation, material, retryable = false) {
  return {
    ok: false,
    error_code: String(errorCode || "adapter_error"),
    diagnostic_id: adapterDiagnosticId(operation, material),
    retryable: Boolean(retryable),
  };
}

function updateLoginState(phase, options = {}) {
  waLoginState.phase = phase;
  waLoginState.connected = phase === "connected";
  waLoginState.lastUpdateTs = Date.now();
  if (options.clearQr === true) {
    waLoginState.qrRaw = null;
    waLoginState.qrDataUrl = null;
  }
  if (options.errorCode) {
    waLoginState.lastErrorCode = String(options.errorCode);
    waLoginState.lastDiagnosticId = adapterDiagnosticId(
      String(options.operation || "login_state"),
      String(options.diagnosticMaterial || options.errorCode)
    );
  } else {
    waLoginState.lastErrorCode = null;
    waLoginState.lastDiagnosticId = null;
  }
}

function loginStatusSnapshot() {
  return {
    ok: true,
    adapter_mode: WA_WEB_ADAPTER_MODE,
    official_bot_api: false,
    transport: WA_WEB_TRANSPORT,
    phase: waLoginState.phase,
    connected: waLoginState.connected,
    qr_ready: Boolean(waLoginState.qrDataUrl),
    qr_data_url: waLoginState.qrDataUrl,
    last_update_ts: waLoginState.lastUpdateTs,
    last_error_code: waLoginState.lastErrorCode,
    last_diagnostic_id: waLoginState.lastDiagnosticId,
    proactive_send_enabled: cfg.allowProactiveSend,
    local_safety_limits: {
      image_bytes: cfg.maxOutboundImageBytes,
      video_bytes: cfg.maxOutboundVideoBytes,
      audio_bytes: cfg.maxOutboundAudioBytes,
      file_bytes: cfg.maxOutboundFileBytes,
    },
  };
}

function normalizeDeliverySource(value) {
  return String(value || "unknown").trim().toLowerCase() || "unknown";
}

function deliverySourceAllowed(source, allowProactiveSend) {
  const normalized = normalizeDeliverySource(source);
  if (PROACTIVE_DELIVERY_SOURCES.has(normalized)) return allowProactiveSend === true;
  return normalized === "immediate_daemon" || normalized === "background_completion";
}

function tr(key, fallback, vars = {}) {
  let text = String(cfg.i18n?.[key] || fallback);
  for (const [name, value] of Object.entries(vars)) {
    text = text.split(`{${name}}`).join(String(value));
  }
  return text;
}

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

function canonicalUserJid(jid) {
  return normalizeJid(jid).replace(/^([^:@]+):\d+@/, "$1@");
}

function isSameAccountJid(remoteJid, ownId, ownLid) {
  const remote = canonicalUserJid(remoteJid);
  if (!remote) return false;
  return [ownId, ownLid]
    .map(canonicalUserJid)
    .filter(Boolean)
    .some((candidate) => candidate === remote);
}

function messageTimestampSeconds(msg) {
  const value = msg?.messageTimestamp;
  if (typeof value === "number") return Number.isFinite(value) ? Math.floor(value) : null;
  if (typeof value === "bigint") return Number(value);
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? Math.floor(parsed) : null;
  }
  if (value && typeof value.toNumber === "function") {
    const parsed = value.toNumber();
    return Number.isFinite(parsed) ? Math.floor(parsed) : null;
  }
  return null;
}

function shouldProcessUpsertMessage(msg, type, ownId, ownLid, startedAtMs, nowMs = Date.now()) {
  if (type === "notify") return true;
  if (type !== "append" || !msg?.key?.fromMe) return false;
  if (!isSameAccountJid(msg.key.remoteJid, ownId, ownLid)) return false;
  const timestampSeconds = messageTimestampSeconds(msg);
  if (timestampSeconds === null) return false;
  const earliestSeconds = Math.floor(startedAtMs / 1000) - 10;
  const latestSeconds = Math.floor(nowMs / 1000) + 300;
  return timestampSeconds >= earliestSeconds && timestampSeconds <= latestSeconds;
}

function outboundContentKey(jid, content) {
  const target = canonicalUserJid(jid);
  if (typeof content?.text === "string") return `${target}:text:${content.text}`;
  if (content?.image) return `${target}:image`;
  if (content?.audio) return `${target}:audio`;
  if (content?.document) return `${target}:document`;
  return "";
}

function inboundContentKey(msg) {
  const target = canonicalUserJid(msg?.key?.remoteJid);
  const message = msg?.message || {};
  const text = extractTextContent(message);
  if (text) return `${target}:text:${text}`;
  if (message.imageMessage) return `${target}:image`;
  if (message.audioMessage) return `${target}:audio`;
  if (message.documentMessage) return `${target}:document`;
  return "";
}

function cleanupOutboundTracking(now = Date.now()) {
  for (const [id, ts] of outboundMessageIds.entries()) {
    if (now - ts > OUTBOUND_DEDUP_WINDOW_MS) outboundMessageIds.delete(id);
  }
  for (const [key, timestamps] of pendingOutboundMessages.entries()) {
    const fresh = timestamps.filter((ts) => now - ts <= PENDING_OUTBOUND_WINDOW_MS);
    if (fresh.length > 0) pendingOutboundMessages.set(key, fresh);
    else pendingOutboundMessages.delete(key);
  }
}

function rememberPendingOutbound(key, now = Date.now()) {
  if (!key) return;
  cleanupOutboundTracking(now);
  const timestamps = pendingOutboundMessages.get(key) || [];
  timestamps.push(now);
  pendingOutboundMessages.set(key, timestamps);
}

function consumePendingOutbound(key, now = Date.now()) {
  if (!key) return false;
  cleanupOutboundTracking(now);
  const timestamps = pendingOutboundMessages.get(key);
  if (!timestamps?.length) return false;
  timestamps.shift();
  if (timestamps.length === 0) pendingOutboundMessages.delete(key);
  return true;
}

function isBridgeOutboundMessage(msg) {
  const now = Date.now();
  cleanupOutboundTracking(now);
  const id = String(msg?.key?.id || "").trim();
  if (id && outboundMessageIds.has(id)) {
    outboundMessageIds.delete(id);
    consumePendingOutbound(inboundContentKey(msg), now);
    return true;
  }
  return consumePendingOutbound(inboundContentKey(msg), now);
}

function isSelfChatMessage(msg) {
  if (!msg?.key?.fromMe || !sock?.user?.id) return false;
  return isSameAccountJid(msg.key.remoteJid, sock.user.id, sock.user.lid);
}

async function sendWaMessage(jid, content) {
  if (!sock) throw new Error("wa socket not ready");
  const pendingKey = outboundContentKey(jid, content);
  rememberPendingOutbound(pendingKey);
  try {
    const sent = await sock.sendMessage(jid, content);
    const id = String(sent?.key?.id || "").trim();
    if (id) outboundMessageIds.set(id, Date.now());
    return sent;
  } catch (error) {
    consumePendingOutbound(pendingKey);
    throw error;
  }
}

function isGroupJid(jid) {
  return normalizeJid(jid).endsWith("@g.us");
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

function extractBindKeyCandidate(text, expectReply) {
  const trimmed = String(text || "").trim();
  if (trimmed.toLowerCase().startsWith("/key")) {
    const candidate = trimmed.slice(4).trim();
    return candidate || null;
  }
  if (expectReply && trimmed && !trimmed.startsWith("/")) return trimmed;
  return null;
}

async function resolveIdentity(externalUserId, externalChatId) {
  const resp = await fetch(`${cfg.clawdBaseUrl}/v1/auth/channel/resolve`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      channel: "whatsapp",
      external_user_id: externalUserId,
      external_chat_id: externalChatId,
    }),
  });
  const body = await resp.json();
  if (!resp.ok || !body.ok) {
    throw new Error(`resolve channel identity failed: ${body.error || resp.status}`);
  }
  return body.data?.identity || null;
}

async function bindIdentity(externalUserId, externalChatId, userKey) {
  const resp = await fetch(`${cfg.clawdBaseUrl}/v1/auth/channel/bind`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      channel: "whatsapp",
      external_user_id: externalUserId,
      external_chat_id: externalChatId,
      user_key: String(userKey || "").trim(),
    }),
  });
  const body = await resp.json();
  if (resp.status === 401) return null;
  if (!resp.ok) {
    throw new Error(`bind channel identity failed: ${body.error || resp.status}`);
  }
  if (!body.ok || !body.data?.user_key) return null;
  return body.data;
}

function buildRelPath(absPath) {
  return path.relative(cfg.workspaceRoot, absPath).split(path.sep).join("/");
}

function ensureParentDir(filePath) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
}

function resetWaLoginState() {
  updateLoginState("logged_out", { clearQr: true });
}

function buildSubmitTaskBody(externalUserId, externalChatId, kind, payload, identity) {
  const userId = Number(identity?.user_id || stableUserId(externalUserId));
  const userKey = String(identity?.user_key || "").trim();
  if (!userKey) throw new Error("bound user key is required");
  return {
    user_id: userId,
    chat_id: userId,
    user_key: userKey,
    channel: "whatsapp",
    external_user_id: externalUserId,
    external_chat_id: externalChatId,
    kind,
    payload: {
      adapter: "whatsapp_web",
      ...(payload || {}),
    },
  };
}

async function submitTask(externalUserId, externalChatId, kind, payload, identity) {
  const body = buildSubmitTaskBody(externalUserId, externalChatId, kind, payload, identity);
  const userKey = body.user_key;
  const resp = await fetch(`${cfg.clawdBaseUrl}/v1/tasks`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-agent-key": userKey },
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

async function queryTask(taskId, identity) {
  const userKey = String(identity?.user_key || "").trim();
  const headers = userKey ? { "x-agent-key": userKey } : {};
  const resp = await fetch(`${cfg.clawdBaseUrl}/v1/tasks/${taskId}`, { headers });
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

function taskSuccessMessages(task) {
  const messages = task.result_json?.messages;
  if (Array.isArray(messages)) {
    const out = messages
      .filter((v) => typeof v === "string")
      .map((v) => v.trim())
      .filter((v) => v.length > 0);
    if (out.length > 0) return out;
  }
  const fallback = String(task.result_json?.text || "").trim();
  if (fallback) return [fallback];
  return taskSuccessArtifacts(task).length > 0 ? [] : ["done"];
}

function taskSuccessArtifacts(task) {
  const artifacts = task?.result_json?.artifacts;
  if (!Array.isArray(artifacts)) return [];
  return artifacts.filter((artifact) => {
    return (
      artifact &&
      typeof artifact === "object" &&
      typeof artifact.download_url === "string" &&
      artifact.download_url.trim().length > 0
    );
  });
}

function taskArtifactDeliveryEnabled(task) {
  const results = task?.result_json?.task_journal?.trace?.capability_results;
  if (!Array.isArray(results)) return true;
  const flags = results
    .map((result) => result?.data?.extra?.delivery?.deliver_to_user)
    .filter((value) => typeof value === "boolean");
  return flags.length === 0 || flags.some(Boolean);
}

async function pollTaskResult(taskId, waitSeconds, identity) {
  const pollMs = 500;
  const rounds = Math.max(1, Math.floor((waitSeconds * 1000) / pollMs));
  for (let i = 0; i < rounds; i += 1) {
    const task = await queryTask(taskId, identity);
    if (task.status === "queued" || task.status === "running") {
      await new Promise((r) => setTimeout(r, pollMs));
      continue;
    }
    if (task.status === "succeeded") {
      return task;
    }
    throw new Error(task.error_text || `task status=${task.status}`);
  }
  throw new Error("task_result_wait_timeout");
}

function outboundLimitForArtifact(artifact) {
  const kind = String(artifact?.kind || "").toLowerCase();
  const mime = String(artifact?.mime_type || "").toLowerCase();
  if (kind === "image" || mime.startsWith("image/")) return cfg.maxOutboundImageBytes;
  if (kind === "video" || mime.startsWith("video/")) return cfg.maxOutboundVideoBytes;
  if (kind === "audio" || mime.startsWith("audio/")) return cfg.maxOutboundAudioBytes;
  return cfg.maxOutboundFileBytes;
}

function taskArtifactDownloadUrl(taskId, artifact) {
  const raw = String(artifact?.download_url || "").trim();
  if (!raw.startsWith("/")) {
    throw new Error("WhatsApp Web 附件投送失败：任务附件地址无效");
  }
  const base = new URL(cfg.clawdBaseUrl);
  const resolved = new URL(raw, base);
  const expectedPrefix = `/v1/tasks/${encodeURIComponent(String(taskId))}/artifacts/`;
  if (resolved.origin !== base.origin || !resolved.pathname.startsWith(expectedPrefix)) {
    throw new Error("WhatsApp Web 附件投送失败：任务附件地址不属于当前任务");
  }
  return resolved.toString();
}

function safeArtifactFilename(artifact) {
  const raw = path.basename(String(artifact?.filename || "attachment.bin").trim());
  const safe = raw.replace(/[^a-zA-Z0-9._-]/g, "_").replace(/^\.+/, "");
  return safe || "attachment.bin";
}

async function materializeTaskArtifact(taskId, artifact, identity) {
  const maxBytes = outboundLimitForArtifact(artifact);
  const declaredBytes = Number(artifact?.size_bytes || 0);
  if (Number.isFinite(declaredBytes) && declaredBytes > maxBytes) {
    validateOutboundFileSize(declaredBytes, String(artifact?.kind || "文件"), maxBytes);
  }
  const userKey = String(identity?.user_key || "").trim();
  if (!userKey) throw new Error("WhatsApp Web 附件投送失败：缺少已绑定的访问凭据");
  const response = await fetch(taskArtifactDownloadUrl(taskId, artifact), {
    headers: { "x-agent-key": userKey },
  });
  if (!response.ok) {
    throw new Error(`WhatsApp Web 附件读取失败（HTTP ${response.status}）`);
  }
  const contentLength = Number(response.headers?.get?.("content-length") || 0);
  if (Number.isFinite(contentLength) && contentLength > maxBytes) {
    validateOutboundFileSize(contentLength, String(artifact?.kind || "文件"), maxBytes);
  }
  const body = Buffer.from(await response.arrayBuffer());
  validateOutboundFileSize(body.length, String(artifact?.kind || "文件"), maxBytes);
  const artifactId = String(artifact?.id || crypto.randomUUID()).replace(/[^a-zA-Z0-9._-]/g, "_");
  const outputPath = path.join(
    cfg.artifactOutboxDir,
    String(taskId).replace(/[^a-zA-Z0-9._-]/g, "_"),
    artifactId,
    safeArtifactFilename(artifact)
  );
  ensureParentDir(outputPath);
  fs.writeFileSync(outputPath, body);
  return outputPath;
}

function validateOutboundFileSize(size, mediaLabel, maxBytes) {
  if (!Number.isFinite(size) || size <= 0) {
    throw new Error(`WhatsApp Web ${mediaLabel}投送失败：附件为空`);
  }
  if (Number.isFinite(maxBytes) && maxBytes > 0 && size > maxBytes) {
    const actualMiB = (size / 1024 / 1024).toFixed(2);
    const maxMiB = (maxBytes / 1024 / 1024).toFixed(0);
    throw new Error(
      `WhatsApp Web ${mediaLabel}过大：${actualMiB} MiB，本地安全上限为 ${maxMiB} MiB。请压缩后重试，或改为在 UI 中下载原文件。`
    );
  }
  return size;
}

async function sendTaskArtifact(jid, taskId, artifact, identity) {
  const localPath = await materializeTaskArtifact(taskId, artifact, identity);
  const kind = String(artifact?.kind || "").toLowerCase();
  const mime = String(artifact?.mime_type || "application/octet-stream").toLowerCase();
  try {
    validateOutboundFile(localPath, kind || "文件", outboundLimitForArtifact(artifact));
    if (kind === "image" || mime.startsWith("image/")) {
      await sendWaMessage(jid, { image: { url: localPath }, mimetype: mime });
    } else if (kind === "video" || mime.startsWith("video/")) {
      await sendWaMessage(jid, { video: { url: localPath }, mimetype: mime });
    } else if (kind === "audio" || mime.startsWith("audio/")) {
      await sendWaMessage(jid, { audio: { url: localPath }, mimetype: mime, ptt: false });
    } else {
      await sendWaMessage(jid, {
        document: { url: localPath },
        fileName: safeArtifactFilename(artifact),
        mimetype: mime,
      });
    }
  } finally {
    fs.rmSync(localPath, { force: true });
  }
}

function stripStructuredArtifactTokens(message, artifacts) {
  const deliveredNames = new Set(artifacts.map(safeArtifactFilename));
  return String(message || "")
    .split(/\r?\n/)
    .filter((line) => {
      const trimmed = line.trim();
      for (const prefix of ["IMAGE_FILE:", "VIDEO_FILE:", "FILE:", "VOICE_FILE:"]) {
        if (!trimmed.startsWith(prefix)) continue;
        const rawPath = trimmed
          .slice(prefix.length)
          .trim()
          .replace(/^['"`]+|['"`]+$/g, "");
        const normalized = path
          .basename(rawPath)
          .replace(/[^a-zA-Z0-9._-]/g, "_")
          .replace(/^\.+/, "");
        return !deliveredNames.has(normalized);
      }
      return true;
    })
    .join("\n")
    .trim();
}

async function sendTaskResult(jid, taskId, task, identity) {
  const messages = taskSuccessMessages(task);
  const artifacts = taskArtifactDeliveryEnabled(task) ? taskSuccessArtifacts(task) : [];
  for (const artifact of artifacts) {
    await sendTaskArtifact(jid, taskId, artifact, identity);
  }
  for (const answer of messages) {
    const remaining = stripStructuredArtifactTokens(answer, artifacts);
    if (remaining) await sendAnswer(jid, remaining);
  }
}

function extractTokenPaths(answer, prefix) {
  return answer
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.startsWith(prefix))
    .map((line) => line.slice(prefix.length).trim().replace(/^['"`]+|['"`]+$/g, ""))
    .filter((p) => p.length > 0)
    .map((p) => (path.isAbsolute(p) ? p : path.resolve(cfg.workspaceRoot, p)));
}

function validateOutboundFile(filePath, mediaLabel, maxBytes) {
  let stat;
  try {
    stat = fs.statSync(filePath);
  } catch (err) {
    throw new Error(`WhatsApp Web ${mediaLabel}文件无法读取：${filePath}（${err.message || err}）`);
  }
  if (!stat.isFile()) {
    throw new Error(`WhatsApp Web ${mediaLabel}投送失败：${filePath} 不是普通文件`);
  }
  if (stat.size === 0) {
    throw new Error(`WhatsApp Web ${mediaLabel}投送失败：${filePath} 是空文件`);
  }
  if (Number.isFinite(maxBytes) && maxBytes > 0 && stat.size > maxBytes) {
    const actualMiB = (stat.size / 1024 / 1024).toFixed(2);
    const maxMiB = (maxBytes / 1024 / 1024).toFixed(0);
    throw new Error(
      `WhatsApp Web ${mediaLabel}过大：${actualMiB} MiB，本地安全上限为 ${maxMiB} MiB。请压缩后重试，或改为在 UI 中下载原文件。`
    );
  }
  return stat.size;
}

function stripTokens(answer) {
  return answer
    .split(/\r?\n/)
    .filter((line) => {
      const t = line.trimStart();
      return !(
        t.startsWith("IMAGE_FILE:") ||
        t.startsWith("VIDEO_FILE:") ||
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
  const videoPaths = extractTokenPaths(answer, "VIDEO_FILE:");
  const filePaths = extractTokenPaths(answer, "FILE:");
  const voicePaths = extractTokenPaths(answer, "VOICE_FILE:");

  if (text) {
    await sendWaMessage(jid, { text });
  }
  for (const p of imagePaths) {
    validateOutboundFile(p, "图片", cfg.maxOutboundImageBytes);
    await sendWaMessage(jid, { image: { url: p } });
  }
  for (const p of videoPaths) {
    validateOutboundFile(p, "视频", cfg.maxOutboundVideoBytes);
    await sendWaMessage(jid, { video: { url: p } });
  }
  for (const p of filePaths) {
    validateOutboundFile(p, "文件", cfg.maxOutboundFileBytes);
    await sendWaMessage(jid, {
      document: { url: p },
      fileName: path.basename(p),
    });
  }
  for (const p of voicePaths) {
    validateOutboundFile(p, "音频", cfg.maxOutboundAudioBytes);
    await sendWaMessage(jid, {
      audio: { url: p },
      ptt: true,
      mimetype: "audio/ogg; codecs=opus",
    });
  }
  if (
    !text &&
    imagePaths.length === 0 &&
    videoPaths.length === 0 &&
    filePaths.length === 0 &&
    voicePaths.length === 0
  ) {
    await sendWaMessage(jid, { text: answer });
  }
}

async function runTaskFlow(
  jid,
  externalUserId,
  kind,
  payload,
  identity,
  quickWait = cfg.quickResultWaitSeconds
) {
  const taskId = await submitTask(externalUserId, jid, kind, payload, identity);
  try {
    const task = await pollTaskResult(taskId, quickWait, identity);
    await sendTaskResult(jid, taskId, task, identity);
  } catch (err) {
    if (String(err.message || err) === "task_result_wait_timeout") {
      setTimeout(async () => {
        try {
          const task = await pollTaskResult(taskId, 600, identity);
          await sendTaskResult(jid, taskId, task, identity);
        } catch (e) {
          await sendWaMessage(jid, {
            text: tr("whatsapp_web.msg.process_failed", "Processing failed: {error}", {
              error: String(e.message || e),
            }),
          });
        }
      }, 200);
      return;
    }
    await sendWaMessage(jid, {
      text: tr("whatsapp_web.msg.process_failed", "Processing failed: {error}", {
        error: String(err.message || err),
      }),
    });
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

async function handleInboundMessage(msg, upsertType = "notify") {
  if (!msg?.key) return;
  if (
    !shouldProcessUpsertMessage(
      msg,
      upsertType,
      sock?.user?.id,
      sock?.user?.lid,
      bridgeStartedAtMs
    )
  ) return;
  if (msg.key.fromMe && isBridgeOutboundMessage(msg)) return;
  if (msg.key.fromMe && !isSelfChatMessage(msg)) return;
  if (!shouldProcessInbound(msg)) {
    log.info({ id: msg?.key?.id, jid: msg?.key?.remoteJid }, "skip duplicated inbound message");
    return;
  }
  const jid = normalizeJid(msg.key.remoteJid);
  if (!jid) return;
  const externalUserId = normalizeJid(msg.key.participant || jid);
  const bindingScope = `${externalUserId}\u0000${jid}`;
  if (!isAllowed(externalUserId) && !isAllowed(jid)) {
    await sendWaMessage(jid, {
      text: tr("whatsapp_web.msg.access_denied", "This account is not allowed to use this channel."),
    });
    return;
  }

  const text = extractTextContent(msg.message);
  let identity = await resolveIdentity(externalUserId, jid);
  if (!identity && isGroupJid(jid)) {
    identity = await resolveIdentity(externalUserId, externalUserId);
  }
  if (!identity) {
    if (isGroupJid(jid)) {
      await sendWaMessage(jid, {
        text: tr(
          "whatsapp_web.msg.bind_private",
          "For security, bind your key in a private chat with this account before using it in a group."
        ),
      });
      return;
    }
    const candidate = extractBindKeyCandidate(text, expectingKeyReply.has(bindingScope));
    if (candidate) {
      identity = await bindIdentity(externalUserId, jid, candidate);
      if (identity) {
        expectingKeyReply.delete(bindingScope);
        await sendWaMessage(jid, {
          text: tr(
            "whatsapp_web.msg.bind_success",
            "Key bound successfully. Please send your previous message again."
          ),
        });
      } else {
        expectingKeyReply.add(bindingScope);
        await sendWaMessage(jid, {
          text: tr("whatsapp_web.msg.bind_invalid", "Invalid key. Please try again."),
        });
      }
      return;
    }
    expectingKeyReply.add(bindingScope);
    await sendWaMessage(jid, {
      text: tr(
        "whatsapp_web.msg.bind_help",
        "Please send /key <your_key> first to bind this account before chatting or using features."
      ),
    });
    return;
  }
  expectingKeyReply.delete(bindingScope);

  if (text.startsWith("/run")) {
    const rest = text.slice(4).trim();
    const firstSpace = rest.indexOf(" ");
    const skill = (firstSpace >= 0 ? rest.slice(0, firstSpace) : rest).trim();
    const args = (firstSpace >= 0 ? rest.slice(firstSpace + 1) : "").trim();
    if (!skill) {
      await sendWaMessage(jid, {
        text: tr(
          "whatsapp_web.msg.run_usage",
          "Usage: /run <skill_name> <args>"
        ),
      });
      return;
    }
    await runTaskFlow(jid, externalUserId, "run_skill", { skill_name: skill, args }, identity);
    return;
  }

  const userId = Number(identity.user_id || stableUserId(externalUserId));
  const media = await saveInboundMedia(msg.message, jid, userId);
  if (media?.mediaType === "image") {
    await runTaskFlow(jid, externalUserId, "run_skill", {
      skill_name: "image_vision",
      args: {
        action: "describe",
        images: [{ path: media.relPath }],
        detail_level: "normal",
      },
    }, identity);
    return;
  }
  if (media?.mediaType === "audio") {
    await runTaskFlow(jid, externalUserId, "run_skill", {
      skill_name: "audio_transcribe",
      args: {
        audio: { path: media.relPath },
      },
    }, identity, 120);
    return;
  }

  if (text) {
    await runTaskFlow(jid, externalUserId, "ask", { text }, identity);
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
      updateLoginState("qr_ready", { clearQr: true });
      waLoginState.qrRaw = qr;
      try {
        waLoginState.qrDataUrl = await QRCode.toDataURL(qr, {
          width: 320,
          margin: 1,
          errorCorrectionLevel: "M",
        });
      } catch (err) {
        waLoginState.qrDataUrl = null;
        updateLoginState("error", {
          errorCode: "qr_render_failed",
          operation: "render_qr",
          diagnosticMaterial: String(err?.message || err),
        });
      }
      console.log("\n[wa-web-bridge] 请扫码登录 WhatsApp:");
      qrcode.generate(qr, { small: true });
    }
    if (connection === "open") {
      log.info("wa-web connected");
      updateLoginState("connected", { clearQr: true });
    }
    if (connection === "close") {
      const statusCode = lastDisconnect?.error?.output?.statusCode;
      const shouldReconnect = statusCode !== DisconnectReason.loggedOut;
      updateLoginState(shouldReconnect ? "reconnecting" : "logged_out", {
        clearQr: true,
        errorCode: shouldReconnect ? "connection_closed" : "logged_out",
        operation: "connection_update",
        diagnosticMaterial: String(statusCode || "unknown"),
      });
      log.warn({ statusCode, shouldReconnect }, "wa-web connection closed");
      if (shouldReconnect) {
        setTimeout(connectWhatsApp, 2000);
      } else {
        log.error("wa-web logged out, remove auth dir and login again");
      }
    }
  });

  sock.ev.on("messages.upsert", async ({ messages, type }) => {
    if (!Array.isArray(messages)) return;
    const ownMessages = messages.filter((message) => message?.key?.fromMe);
    if (ownMessages.length > 0) {
      const acceptedSelfMessages = ownMessages.filter((message) =>
        shouldProcessUpsertMessage(
          message,
          type,
          sock?.user?.id,
          sock?.user?.lid,
          bridgeStartedAtMs
        )
      ).length;
      log.info(
        { type, own_message_count: ownMessages.length, accepted_self_message_count: acceptedSelfMessages },
        "wa-web own message upsert"
      );
    }
    for (const m of messages) {
      try {
        await handleInboundMessage(m, type);
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
    res.json(loginStatusSnapshot());
  });

  app.post("/v1/send-text", async (req, res) => {
    try {
      const to = String(req.body?.to || "").trim();
      const text = String(req.body?.text || "").trim();
      const deliverySource = normalizeDeliverySource(req.body?.delivery_source);
      if (!to || !text) {
        return res.status(400).json(adapterError("invalid_request", "send_text", "missing_to_or_text"));
      }
      if (!deliverySourceAllowed(deliverySource, cfg.allowProactiveSend)) {
        return res
          .status(403)
          .json(adapterError("proactive_send_disabled", "send_text", deliverySource));
      }
      if (!sock) {
        return res.status(503).json(adapterError("socket_not_ready", "send_text", "socket_not_ready", true));
      }
      const sent = await sendWaMessage(to, { text });
      const messageId = String(sent?.key?.id || "").trim();
      return res.json({ ok: true, message_ids: messageId ? [messageId] : [] });
    } catch (err) {
      const material = String(err?.message || err);
      log.error({ diagnostic_id: adapterDiagnosticId("send_text", material) }, "wa-web send failed");
      return res.status(500).json(adapterError("adapter_send_failed", "send_text", material, true));
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
        updateLoginState("starting");
        connectWhatsApp().catch((err) => {
          updateLoginState("error", {
            errorCode: "reconnect_failed",
            operation: "reconnect_after_logout",
            diagnosticMaterial: String(err?.message || err),
          });
          log.error({ err: String(err?.stack || err) }, "reconnect after logout failed");
        });
      }, 500);
      return res.json({ ok: true });
    } catch (err) {
      const material = String(err?.message || err);
      return res.status(500).json(adapterError("logout_failed", "logout", material));
    }
  });

  const { host, port } = parseListenAddress(cfg.bridgeListen, 8092);
  if (!isLoopbackHost(host)) {
    throw new Error("WhatsApp Web bridge_listen must use a loopback host");
  }
  app.listen(port, host, () => {
    log.info(`wa-web bridge listening on ${host}:${port}`);
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

if (require.main === module) {
  main().catch((err) => {
    log.error({ err: String(err?.stack || err) }, "wa-web bridge fatal");
    process.exit(1);
  });
}

module.exports = {
  adapterDiagnosticId,
  adapterError,
  bindIdentity,
  buildSubmitTaskBody,
  canonicalUserJid,
  extractBindKeyCandidate,
  extractTokenPaths,
  extractTextContent,
  isGroupJid,
  isLoopbackHost,
  isSameAccountJid,
  deliverySourceAllowed,
  loginStatusSnapshot,
  messageTimestampSeconds,
  normalizeDeliverySource,
  normalizeInternalApiBaseUrl,
  parseListenAddress,
  queryTask,
  resolveIdentity,
  stableUserId,
  submitTask,
  stripTokens,
  stripStructuredArtifactTokens,
  taskArtifactDeliveryEnabled,
  taskArtifactDownloadUrl,
  taskSuccessArtifacts,
  taskSuccessMessages,
  updateLoginState,
  materializeTaskArtifact,
  validateOutboundFile,
  validateOutboundFileSize,
  shouldProcessUpsertMessage,
};
