#!/usr/bin/env node
import { createHash, createPublicKey, randomBytes, verify as verifySignature } from "node:crypto";
import { createServer } from "node:http";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const JOIN_REQUEST_INTERVAL_SECONDS = 3600;
const JOIN_TASK_TTL_SECONDS = 600;

const HOST = process.env.NNI_SERVER_HOST || "0.0.0.0";
const PORT = Number.parseInt(process.env.NNI_SERVER_PORT || "8797", 10);
const STATE_PATH = process.env.NNI_SERVER_STATE_PATH || "data/nni-server-state.json";

function nowTs() {
  return Math.floor(Date.now() / 1000);
}

function emptyState() {
  return {
    tasks: {},
    devices: {},
    requests: [],
  };
}

async function loadState() {
  try {
    const raw = await readFile(STATE_PATH, "utf8");
    const parsed = JSON.parse(raw);
    return {
      tasks: parsed.tasks && typeof parsed.tasks === "object" ? parsed.tasks : {},
      devices: parsed.devices && typeof parsed.devices === "object" ? parsed.devices : {},
      requests: Array.isArray(parsed.requests) ? parsed.requests : [],
    };
  } catch (error) {
    if (error && error.code === "ENOENT") return emptyState();
    throw error;
  }
}

async function saveState(state) {
  const parent = path.dirname(path.resolve(STATE_PATH));
  await mkdir(parent, { recursive: true });
  await writeFile(STATE_PATH, `${JSON.stringify(state, null, 2)}\n`, "utf8");
}

function sendJson(res, status, payload) {
  const body = Buffer.from(JSON.stringify(payload));
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": String(body.length),
  });
  res.end(body);
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
  const digest = createHash("sha256").update(challenge, "utf8").digest();
  const derSignature = rawEcdsaSignatureToDer(signature);
  if (!verifySignature(null, digest, publicKey, derSignature)) {
    throw new Error("nni_signature_verify_failed");
  }
}

function latestTaskTs(state, userKey, devicePubkey) {
  let latest = null;
  for (const task of Object.values(state.tasks)) {
    if (task.user_key === userKey && task.device_pubkey === devicePubkey) {
      latest = latest == null ? task.created_at_ts : Math.max(latest, task.created_at_ts);
    }
  }
  return latest;
}

function deviceKey(userKey, devicePubkey) {
  return `${userKey}:${devicePubkey}`;
}

async function handleJoinRequest(res, body) {
  let devicePubkey;
  try {
    devicePubkey = normalizeHex(body.device_pubkey, 64, "nni_pubkey");
  } catch (error) {
    sendJson(res, 400, fail(error.message, { status: "device_pubkey_invalid" }));
    return;
  }

  const userKey = String(body.client_user_key || "anonymous").trim() || "anonymous";
  const state = await loadState();
  const ts = nowTs();
  const lastTs = latestTaskTs(state, userKey, devicePubkey);
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
  state.tasks[taskId] = {
    task_id: taskId,
    user_key: userKey,
    device_pubkey: devicePubkey,
    challenge,
    status: "pending",
    created_at_ts: ts,
    expires_at_ts: expiresAtTs,
    verified_at_ts: null,
    error_code: null,
  };
  await saveState(state);

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

  const state = await loadState();
  const task = state.tasks[taskId];
  if (!task) {
    sendJson(res, 404, fail("nni_join_task_not_found", { status: "task_not_found" }));
    return;
  }

  const ts = nowTs();
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
    task.status = "expired";
    task.error_code = "task_expired";
    recordRequest(state, task, signature, ts, false, "expired", "task_expired");
    await saveState(state);
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
    task.status = "rejected";
    task.error_code = error.message || "nni_signature_verify_failed";
    recordRequest(state, task, signature, ts, false, "rejected", task.error_code);
    await saveState(state);
    sendJson(
      res,
      401,
      fail(task.error_code, {
        status: "signature_rejected",
        task_id: task.task_id,
        device_pubkey: task.device_pubkey,
        compliant: false,
        joined: false,
      }),
    );
    return;
  }

  task.status = "verified";
  task.verified_at_ts = ts;
  task.error_code = null;
  const key = deviceKey(task.user_key, task.device_pubkey);
  const currentDevice = state.devices[key];
  state.devices[key] = {
    user_key: task.user_key,
    device_pubkey: task.device_pubkey,
    first_joined_at_ts: currentDevice?.first_joined_at_ts || ts,
    last_compliant_request_ts: ts,
    join_count: (currentDevice?.join_count || 0) + 1,
    status: "joined",
  };
  recordRequest(state, task, signature, ts, true, "accepted", null);
  await saveState(state);

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

function recordRequest(state, task, signature, ts, compliant, status, errorCode) {
  state.requests.push({
    id: state.requests.length + 1,
    task_id: task.task_id,
    user_key: task.user_key,
    device_pubkey: task.device_pubkey,
    challenge: task.challenge,
    signature,
    compliant,
    status,
    error_code: errorCode,
    created_at_ts: ts,
  });
}

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url || "/", `http://${req.headers.host || "localhost"}`);
    if (req.method === "GET" && url.pathname === "/v1/health") {
      sendJson(res, 200, ok({ service: "nni-server", status: "ok" }));
      return;
    }
    if (req.method !== "POST") {
      sendJson(res, 404, fail("not_found", { status: "not_found" }));
      return;
    }
    const body = await readJson(req);
    if (url.pathname === "/v1/nni/server/join/request") {
      await handleJoinRequest(res, body);
      return;
    }
    if (url.pathname === "/v1/nni/server/join/verify") {
      await handleJoinVerify(res, body);
      return;
    }
    sendJson(res, 404, fail("not_found", { status: "not_found" }));
  } catch (error) {
    sendJson(res, 500, fail("nni_server_internal_error", { status: "internal_error", error: String(error?.message || error) }));
  }
});

server.listen(PORT, HOST, () => {
  console.log(`[nni-server] listening on ${HOST}:${PORT}, state=${STATE_PATH}`);
});
