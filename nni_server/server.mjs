#!/usr/bin/env node
import { createPublicKey, randomBytes, verify as verifySignature } from "node:crypto";
import { createServer } from "node:http";
import { appendFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { NniStore } from "./storage.mjs";

const JOIN_REQUEST_INTERVAL_SECONDS = 60;
const JOIN_TASK_TTL_SECONDS = 600;

const HOST = process.env.NNI_SERVER_HOST || "0.0.0.0";
const PORT = Number.parseInt(process.env.NNI_SERVER_PORT || "8797", 10);
const DATABASE_PATH = process.env.NNI_SERVER_DATABASE_PATH || "data/nni-server.sqlite3";
const LEGACY_STATE_PATH = process.env.NNI_SERVER_STATE_PATH || "data/nni-server-state.json";
const LOG_PATH = process.env.NNI_SERVER_LOG_PATH || "logs/nni-server.log";
const LOG_TO_STDOUT = /^(1|true|yes)$/i.test(process.env.NNI_SERVER_LOG_STDOUT || "");
const PUBLIC_KEY_WHITELIST_ENV = "NNI_SERVER_PUBLIC_KEY_WHITELIST";

function nowTs() {
  return Math.floor(Date.now() / 1000);
}

let stateMutationTail = Promise.resolve();

function serializeStateMutation(operation) {
  const pending = stateMutationTail.then(operation, operation);
  stateMutationTail = pending.then(
    () => undefined,
    () => undefined,
  );
  return pending;
}

async function appendNniServerLog(eventKind, payload = {}) {
  const parent = path.dirname(path.resolve(LOG_PATH));
  await mkdir(parent, { recursive: true });
  const line = `${JSON.stringify({
    ts: nowTs(),
    event_kind: eventKind,
    payload,
  })}\n`;
  await appendFile(LOG_PATH, line, "utf8");
  if (LOG_TO_STDOUT) process.stdout.write(line);
}

function logNniServerEvent(eventKind, payload = {}) {
  void appendNniServerLog(eventKind, payload).catch(() => {});
}

function sendJson(res, status, payload) {
  const body = Buffer.from(JSON.stringify(payload));
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": String(body.length),
  });
  res.end(body);
  if (res.nniRequestMeta) {
    logNniServerEvent("http_response", {
      method: res.nniRequestMeta.method,
      path: res.nniRequestMeta.path,
      status,
      error_code: payload && typeof payload === "object" ? payload.error || null : null,
    });
  }
}

function ok(data) {
  return { ok: true, data, error: null };
}

function fail(error, data = {}) {
  return { ok: false, data, error };
}

async function readJson(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  const raw = Buffer.concat(chunks).toString("utf8").trim() || "{}";
  const parsed = JSON.parse(raw);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("request body must be a JSON object");
  }
  return parsed;
}

function normalizeHex(value, expectedBytes, codePrefix) {
  const normalized = String(value || "").trim().toLowerCase();
  if (normalized.length !== expectedBytes * 2) {
    throw new Error(`${codePrefix}_length_invalid`);
  }
  if (!/^[0-9a-f]+$/.test(normalized)) {
    throw new Error(`${codePrefix}_hex_invalid`);
  }
  return normalized;
}

function normalizePublicKeyHex(value) {
  return normalizeHex(value, 64, "nni_pubkey");
}

function normalizePublicKeyWhitelist(values) {
  const normalized = [];
  const seen = new Set();
  for (const value of values) {
    const pubkey = normalizePublicKeyHex(value);
    if (!seen.has(pubkey)) {
      seen.add(pubkey);
      normalized.push(pubkey);
    }
  }
  return normalized;
}

function parsePublicKeyWhitelistEnv() {
  const raw = process.env[PUBLIC_KEY_WHITELIST_ENV] || "";
  if (!raw.trim()) return [];
  return normalizePublicKeyWhitelist(raw.split(/[\s,;]+/).filter(Boolean));
}

function configuredPublicKeyWhitelist() {
  return parsePublicKeyWhitelistEnv();
}

const store = new NniStore({
  databasePath: DATABASE_PATH,
  legacyStatePath: LEGACY_STATE_PATH,
  configuredPublicKeys: configuredPublicKeyWhitelist(),
});

function publicKeyWhitelistDecision(devicePubkey) {
  if (store.publicKeyWhitelistCount() === 0) {
    return {
      allowed: false,
      error_code: "nni_public_key_whitelist_empty",
      status: "public_key_whitelist_empty",
      message_key: "nni.join.public_key_whitelist_empty",
    };
  }
  if (!store.isPublicKeyAllowed(devicePubkey)) {
    return {
      allowed: false,
      error_code: "nni_pubkey_not_allowlisted",
      status: "public_key_not_allowlisted",
      message_key: "nni.join.public_key_not_allowlisted",
    };
  }
  return {
    allowed: true,
    error_code: null,
    status: "public_key_allowed",
    message_key: "nni.join.public_key_allowed",
  };
}

function recordWhitelistBlock(
  { task = null, userKey, devicePubkey, signature = null, ts, errorCode, requestKind = task?.task_kind || "nni_join" },
) {
  store.recordRequest({
    request_kind: requestKind,
    task_id: task?.task_id || null,
    user_key: userKey,
    device_pubkey: devicePubkey,
    challenge: task?.challenge || null,
    signature,
    compliant: false,
    status: "blocked",
    error_code: errorCode,
    created_at_ts: ts,
  });
}

function sendWhitelistBlock(res, decision, devicePubkey) {
  sendJson(
    res,
    403,
    fail(decision.error_code, {
      status: decision.status,
      message_key: decision.message_key,
      device_pubkey: devicePubkey,
    }),
  );
}

function base64url(bytes) {
  return Buffer.from(bytes).toString("base64url");
}

function rawEcdsaSignatureToDer(signature) {
  const r = signature.subarray(0, 32);
  const s = signature.subarray(32, 64);
  const derR = derInteger(r);
  const derS = derInteger(s);
  const length = derR.length + derS.length;
  return Buffer.concat([Buffer.from([0x30, length]), derR, derS]);
}

function derInteger(raw) {
  let value = Buffer.from(raw);
  while (value.length > 1 && value[0] === 0x00 && (value[1] & 0x80) === 0) {
    value = value.subarray(1);
  }
  if (value[0] & 0x80) {
    value = Buffer.concat([Buffer.from([0x00]), value]);
  }
  return Buffer.concat([Buffer.from([0x02, value.length]), value]);
}

function verifyJoinSignature(pubkeyHex, challenge, signatureHex) {
  const pubkey = Buffer.from(normalizeHex(pubkeyHex, 64, "nni_pubkey"), "hex");
  const signature = Buffer.from(normalizeHex(signatureHex, 64, "nni_signature"), "hex");
  const publicKey = createPublicKey({
    key: {
      kty: "EC",
      crv: "P-256",
      x: base64url(pubkey.subarray(0, 32)),
      y: base64url(pubkey.subarray(32, 64)),
    },
    format: "jwk",
  });
  const derSignature = rawEcdsaSignatureToDer(signature);
  if (!verifySignature("sha256", Buffer.from(challenge, "utf8"), publicKey, derSignature)) {
    throw new Error("nni_signature_verify_failed");
  }
}

async function handleJoinRequest(res, body) {
  let devicePubkey;
  try {
    devicePubkey = normalizePublicKeyHex(body.device_pubkey);
  } catch (error) {
    sendJson(res, 400, fail(error.message, { status: "device_pubkey_invalid" }));
    return;
  }

  const userKey = String(body.client_user_key || "anonymous").trim() || "anonymous";
  const ts = nowTs();
  const whitelistDecision = publicKeyWhitelistDecision(devicePubkey);
  if (!whitelistDecision.allowed) {
    recordWhitelistBlock({
      userKey,
      devicePubkey,
      ts,
      errorCode: whitelistDecision.error_code,
    });
    sendWhitelistBlock(res, whitelistDecision, devicePubkey);
    return;
  }

  const lastTs = store.latestJoinTaskTs(userKey, devicePubkey);
  if (lastTs != null && ts - lastTs < JOIN_REQUEST_INTERVAL_SECONDS) {
    const nextAllowedTs = lastTs + JOIN_REQUEST_INTERVAL_SECONDS;
    sendJson(
      res,
      429,
      fail("nni_join_request_interval_active", {
        status: "request_interval_active",
        message_key: "nni.join.request_interval_active",
        request_interval_seconds: JOIN_REQUEST_INTERVAL_SECONDS,
        retry_after_seconds: Math.max(nextAllowedTs - ts, 0),
        next_allowed_ts: nextAllowedTs,
        device_pubkey: devicePubkey,
      }),
    );
    return;
  }

  const taskId = `nni-join-${randomBytes(16).toString("hex")}`;
  const challenge = randomBytes(32).toString("hex");
  const expiresAtTs = ts + JOIN_TASK_TTL_SECONDS;
  store.createTask({
    task_id: taskId,
    user_key: userKey,
    device_pubkey: devicePubkey,
    challenge,
    status: "pending",
    task_kind: "nni_join",
    created_at_ts: ts,
    expires_at_ts: expiresAtTs,
    verified_at_ts: null,
    error_code: null,
  });

  sendJson(
    res,
    200,
    ok({
      status: "challenge_created",
      message_key: "nni.join.challenge_created",
      task_id: taskId,
      challenge,
      device_pubkey: devicePubkey,
      expires_at_ts: expiresAtTs,
      request_interval_seconds: JOIN_REQUEST_INTERVAL_SECONDS,
      task_kind: "nni_join",
      task_payload: {},
    }),
  );
}

async function handleJoinVerify(res, body) {
  const taskId = String(body.task_id || "").trim();
  if (!taskId) {
    sendJson(res, 400, fail("nni_join_task_id_required", { status: "task_id_required" }));
    return;
  }

  let signature;
  try {
    signature = normalizeHex(body.signature, 64, "nni_signature");
  } catch (error) {
    sendJson(res, 400, fail(error.message, { status: "signature_invalid" }));
    return;
  }

  const task = store.getTask(taskId);
  if (!task || task.task_kind !== "nni_join") {
    sendJson(res, 404, fail("nni_join_task_not_found", { status: "task_not_found" }));
    return;
  }

  const ts = nowTs();
  const whitelistDecision = publicKeyWhitelistDecision(task.device_pubkey);
  if (!whitelistDecision.allowed) {
    store.finishTaskWithRequest(
      task,
      signature,
      ts,
      "rejected",
      false,
      "blocked",
      whitelistDecision.error_code,
    );
    sendWhitelistBlock(res, whitelistDecision, task.device_pubkey);
    return;
  }

  if (task.status === "verified") {
    sendJson(
      res,
      409,
      fail("nni_join_task_already_verified", {
        status: "task_already_verified",
        task_id: task.task_id,
        device_pubkey: task.device_pubkey,
      }),
    );
    return;
  }

  if (ts > task.expires_at_ts) {
    store.finishTaskWithRequest(
      task,
      signature,
      ts,
      "expired",
      false,
      "expired",
      "task_expired",
    );
    sendJson(
      res,
      410,
      fail("nni_join_task_expired", {
        status: "task_expired",
        task_id: task.task_id,
        expires_at_ts: task.expires_at_ts,
      }),
    );
    return;
  }

  try {
    verifyJoinSignature(task.device_pubkey, task.challenge, signature);
  } catch (error) {
    const errorCode = error.message || "nni_signature_verify_failed";
    store.finishTaskWithRequest(task, signature, ts, "rejected", false, "rejected", errorCode);
    sendJson(
      res,
      401,
      fail(errorCode, {
        status: "signature_rejected",
        task_id: task.task_id,
        device_pubkey: task.device_pubkey,
        compliant: false,
        joined: false,
      }),
    );
    return;
  }

  store.acceptJoin(task, signature, ts);

  sendJson(
    res,
    200,
    ok({
      status: "joined",
      message_key: "nni.join.verified",
      task_id: task.task_id,
      device_pubkey: task.device_pubkey,
      compliant: true,
      joined: true,
      verified_at_ts: ts,
      request_interval_seconds: JOIN_REQUEST_INTERVAL_SECONDS,
      next_allowed_ts: ts + JOIN_REQUEST_INTERVAL_SECONDS,
    }),
  );
}

async function handleHeartbeatRequest(res, body) {
  let devicePubkey;
  try {
    devicePubkey = normalizePublicKeyHex(body.device_pubkey);
  } catch (error) {
    sendJson(res, 400, fail(error.message, { status: "device_pubkey_invalid" }));
    return;
  }

  const userKey = String(body.client_user_key || "clawd-nni-heartbeat").trim() || "clawd-nni-heartbeat";
  const ts = nowTs();
  const whitelistDecision = publicKeyWhitelistDecision(devicePubkey);
  if (!whitelistDecision.allowed) {
    recordWhitelistBlock({
      userKey,
      devicePubkey,
      ts,
      errorCode: whitelistDecision.error_code,
      requestKind: "nni_heartbeat",
    });
    sendWhitelistBlock(res, whitelistDecision, devicePubkey);
    return;
  }

  const taskId = `nni-heartbeat-${randomBytes(16).toString("hex")}`;
  const challenge = randomBytes(32).toString("hex");
  const expiresAtTs = ts + JOIN_TASK_TTL_SECONDS;
  store.createTask({
    task_id: taskId,
    user_key: userKey,
    device_pubkey: devicePubkey,
    challenge,
    status: "pending",
    task_kind: "nni_heartbeat",
    created_at_ts: ts,
    expires_at_ts: expiresAtTs,
    verified_at_ts: null,
    error_code: null,
  });

  sendJson(
    res,
    200,
    ok({
      status: "heartbeat_challenge_created",
      message_key: "nni.heartbeat.challenge_created",
      task_id: taskId,
      challenge,
      device_pubkey: devicePubkey,
      expires_at_ts: expiresAtTs,
      task_kind: "nni_heartbeat",
      task_payload: {},
    }),
  );
}

async function handleHeartbeatVerify(res, body) {
  const taskId = String(body.task_id || "").trim();
  if (!taskId) {
    sendJson(res, 400, fail("nni_heartbeat_task_id_required", { status: "task_id_required" }));
    return;
  }

  let signature;
  try {
    signature = normalizeHex(body.signature, 64, "nni_signature");
  } catch (error) {
    sendJson(res, 400, fail(error.message, { status: "signature_invalid" }));
    return;
  }

  const task = store.getTask(taskId);
  if (!task || task.task_kind !== "nni_heartbeat") {
    sendJson(res, 404, fail("nni_heartbeat_task_not_found", { status: "task_not_found" }));
    return;
  }

  const ts = nowTs();
  const whitelistDecision = publicKeyWhitelistDecision(task.device_pubkey);
  if (!whitelistDecision.allowed) {
    store.finishTaskWithRequest(
      task,
      signature,
      ts,
      "rejected",
      false,
      "blocked",
      whitelistDecision.error_code,
    );
    sendWhitelistBlock(res, whitelistDecision, task.device_pubkey);
    return;
  }

  if (task.status === "verified") {
    sendJson(
      res,
      409,
      fail("nni_heartbeat_task_already_verified", {
        status: "task_already_verified",
        task_id: task.task_id,
        device_pubkey: task.device_pubkey,
      }),
    );
    return;
  }

  if (ts > task.expires_at_ts) {
    store.finishTaskWithRequest(
      task,
      signature,
      ts,
      "expired",
      false,
      "expired",
      "task_expired",
    );
    sendJson(
      res,
      410,
      fail("nni_heartbeat_task_expired", {
        status: "task_expired",
        task_id: task.task_id,
        expires_at_ts: task.expires_at_ts,
      }),
    );
    return;
  }

  try {
    verifyJoinSignature(task.device_pubkey, task.challenge, signature);
  } catch (error) {
    const errorCode = error.message || "nni_signature_verify_failed";
    store.finishTaskWithRequest(task, signature, ts, "rejected", false, "rejected", errorCode);
    sendJson(
      res,
      401,
      fail(errorCode, {
        status: "signature_rejected",
        task_id: task.task_id,
        device_pubkey: task.device_pubkey,
        compliant: false,
      }),
    );
    return;
  }

  const heartbeatCount = store.acceptHeartbeat(task, signature, ts);

  sendJson(
    res,
    200,
    ok({
      status: "heartbeat_accepted",
      message_key: "nni.heartbeat.verified",
      task_id: task.task_id,
      device_pubkey: task.device_pubkey,
      compliant: true,
      heartbeat_count: heartbeatCount,
      heartbeat_at_unix: ts,
      request_time_ts: ts,
      verified_at_ts: ts,
    }),
  );
}

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url || "/", `http://${req.headers.host || "localhost"}`);
    res.nniRequestMeta = {
      method: req.method || "",
      path: url.pathname,
    };
    if (req.method === "GET" && url.pathname === "/v1/health") {
      sendJson(res, 200, ok({ service: "nni-server", status: "ok", storage: "sqlite" }));
      return;
    }
    if (req.method !== "POST") {
      sendJson(res, 404, fail("not_found", { status: "not_found" }));
      return;
    }
    const body = await readJson(req);
    if (url.pathname === "/v1/nni/server/join/request") {
      await serializeStateMutation(() => handleJoinRequest(res, body));
      return;
    }
    if (url.pathname === "/v1/nni/server/join/verify") {
      await serializeStateMutation(() => handleJoinVerify(res, body));
      return;
    }
    if (url.pathname === "/v1/nni/server/heartbeat/request") {
      await serializeStateMutation(() => handleHeartbeatRequest(res, body));
      return;
    }
    if (url.pathname === "/v1/nni/server/heartbeat/verify") {
      await serializeStateMutation(() => handleHeartbeatVerify(res, body));
      return;
    }
    sendJson(res, 404, fail("not_found", { status: "not_found" }));
  } catch (error) {
    sendJson(res, 500, fail("nni_server_internal_error", { status: "internal_error", error: String(error?.message || error) }));
  }
});

server.listen(PORT, HOST, () => {
  logNniServerEvent("server_listening", {
    host: HOST,
    port: PORT,
    database_path: DATABASE_PATH,
    legacy_state_path: LEGACY_STATE_PATH,
    log_path: LOG_PATH,
  });
});

function shutdown() {
  server.close(() => {
    store.close();
    process.exit(0);
  });
}

process.once("SIGINT", shutdown);
process.once("SIGTERM", shutdown);
