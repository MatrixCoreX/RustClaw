import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { generateKeyPairSync, sign as signMessage } from "node:crypto";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createServer } from "node:net";
import test from "node:test";
import { setTimeout as delay } from "node:timers/promises";

const VALID_PUBKEY = "aa".repeat(64);
const OTHER_PUBKEY = "bb".repeat(64);
const VALID_SIGNATURE = "11".repeat(64);

function base64urlToBuffer(value) {
  return Buffer.from(value, "base64url");
}

function derIntegerToRaw(der, offset) {
  assert.equal(der[offset], 0x02);
  const len = der[offset + 1];
  let value = der.subarray(offset + 2, offset + 2 + len);
  while (value.length > 32 && value[0] === 0x00) value = value.subarray(1);
  if (value.length < 32) value = Buffer.concat([Buffer.alloc(32 - value.length), value]);
  assert.equal(value.length, 32);
  return { value, nextOffset: offset + 2 + len };
}

function derSignatureToRawHex(derSignature) {
  const der = Buffer.from(derSignature);
  assert.equal(der[0], 0x30);
  const r = derIntegerToRaw(der, 2);
  const s = derIntegerToRaw(der, r.nextOffset);
  return Buffer.concat([r.value, s.value]).toString("hex");
}

function generateSigningFixture() {
  const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "prime256v1" });
  const jwk = publicKey.export({ format: "jwk" });
  const pubkey = Buffer.concat([base64urlToBuffer(jwk.x), base64urlToBuffer(jwk.y)]).toString("hex");
  return {
    pubkey,
    signChallenge(challenge) {
      return derSignatureToRawHex(signMessage("sha256", Buffer.from(challenge, "utf8"), privateKey));
    },
  };
}

async function freePort() {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

async function startServer({ publicKeyWhitelist = "", initialState = null } = {}) {
  const dir = await mkdtemp(path.join(tmpdir(), "rustclaw-nni-server-test-"));
  const statePath = path.join(dir, "state.json");
  if (initialState) {
    await writeFile(statePath, `${JSON.stringify(initialState, null, 2)}\n`, "utf8");
  }
  const port = await freePort();
  const logPath = path.join(dir, "nni-server.log");
  const child = spawn(process.execPath, ["server.mjs"], {
    cwd: new URL(".", import.meta.url),
    env: {
      ...process.env,
      NNI_SERVER_HOST: "127.0.0.1",
      NNI_SERVER_PORT: String(port),
      NNI_SERVER_STATE_PATH: statePath,
      NNI_SERVER_LOG_PATH: logPath,
      NNI_SERVER_PUBLIC_KEY_WHITELIST: publicKeyWhitelist,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });

  const baseUrl = `http://127.0.0.1:${port}`;
  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (child.exitCode != null) {
      throw new Error(`nni server exited early: stdout=${stdout} stderr=${stderr}`);
    }
    try {
      const res = await fetch(`${baseUrl}/v1/health`);
      if (res.ok) {
        return {
          baseUrl,
          statePath,
          logPath,
          async stop() {
            if (child.exitCode != null) return;
            child.kill("SIGTERM");
            await Promise.race([
              new Promise((resolve) => child.once("exit", resolve)),
              delay(1000).then(() => child.kill("SIGKILL")),
            ]);
          },
        };
      }
    } catch {
      // Keep polling until the process has bound the port.
    }
    await delay(50);
  }

  child.kill("SIGKILL");
  throw new Error(`nni server did not become ready: stdout=${stdout} stderr=${stderr}`);
}

async function postJson(baseUrl, pathName, body) {
  const res = await fetch(`${baseUrl}${pathName}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  return {
    status: res.status,
    body: await res.json(),
  };
}

async function getJson(baseUrl, pathName) {
  const res = await fetch(`${baseUrl}${pathName}`);
  return {
    status: res.status,
    body: await res.json(),
  };
}

async function readLogLines(logPath) {
  const raw = await readFile(logPath, "utf8");
  return raw
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

test("join request rejects public keys when the whitelist is empty", async (t) => {
  const server = await startServer();
  t.after(() => server.stop());

  const res = await postJson(server.baseUrl, "/v1/nni/server/join/request", {
    device_pubkey: VALID_PUBKEY,
    client_user_key: "ui-user",
  });

  assert.equal(res.status, 403);
  assert.equal(res.body.ok, false);
  assert.equal(res.body.error, "nni_public_key_whitelist_empty");
  assert.equal(res.body.data.status, "public_key_whitelist_empty");
  assert.equal(res.body.data.device_pubkey, VALID_PUBKEY);
});

test("server writes nni events to configured log file", async (t) => {
  const server = await startServer();
  t.after(() => server.stop());

  await getJson(server.baseUrl, "/v1/health");

  let lines = [];
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      lines = await readLogLines(server.logPath);
    } catch {
      lines = [];
    }
    if (
      lines.some((line) => line.event_kind === "server_listening") &&
      lines.some(
        (line) =>
          line.event_kind === "http_response" &&
          line.payload?.path === "/v1/health" &&
          line.payload?.status === 200,
      )
    ) {
      break;
    }
    await delay(50);
  }

  assert(lines.some((line) => line.event_kind === "server_listening"));
  assert(
    lines.some(
      (line) =>
        line.event_kind === "http_response" &&
        line.payload?.path === "/v1/health" &&
        line.payload?.status === 200,
    ),
  );
});

test("join request accepts public keys injected through the whitelist env", async (t) => {
  const server = await startServer({ publicKeyWhitelist: VALID_PUBKEY });
  t.after(() => server.stop());

  const res = await postJson(server.baseUrl, "/v1/nni/server/join/request", {
    device_pubkey: VALID_PUBKEY,
    client_user_key: "ui-user",
  });

  assert.equal(res.status, 200);
  assert.equal(res.body.ok, true);
  assert.equal(res.body.data.status, "challenge_created");
  assert.equal(res.body.data.device_pubkey, VALID_PUBKEY);
  assert.match(res.body.data.challenge, /^[0-9a-f]{64}$/);

  const state = JSON.parse(await readFile(server.statePath, "utf8"));
  assert.deepEqual(state.public_key_whitelist, [VALID_PUBKEY]);
});

test("join request retry interval is one minute", async (t) => {
  const server = await startServer({ publicKeyWhitelist: VALID_PUBKEY });
  t.after(() => server.stop());

  const request = {
    device_pubkey: VALID_PUBKEY,
    client_user_key: "ui-user",
  };
  const first = await postJson(server.baseUrl, "/v1/nni/server/join/request", request);
  assert.equal(first.status, 200);

  const second = await postJson(server.baseUrl, "/v1/nni/server/join/request", request);
  assert.equal(second.status, 429);
  assert.equal(second.body.ok, false);
  assert.equal(second.body.error, "nni_join_request_interval_active");
  assert.equal(second.body.data.request_interval_seconds, 60);
  assert.equal(second.body.data.retry_after_seconds <= 60, true);
  assert.equal(second.body.data.status, "request_interval_active");
});

test("join verify rejects tasks whose public key is no longer whitelisted", async (t) => {
  const now = Math.floor(Date.now() / 1000);
  const taskId = "nni-join-test";
  const server = await startServer({
    initialState: {
      tasks: {
        [taskId]: {
          task_id: taskId,
          user_key: "ui-user",
          device_pubkey: VALID_PUBKEY,
          challenge: "00".repeat(32),
          status: "pending",
          created_at_ts: now,
          expires_at_ts: now + 600,
          verified_at_ts: null,
          error_code: null,
        },
      },
      devices: {},
      requests: [],
      public_key_whitelist: [OTHER_PUBKEY],
    },
  });
  t.after(() => server.stop());

  const res = await postJson(server.baseUrl, "/v1/nni/server/join/verify", {
    task_id: taskId,
    signature: VALID_SIGNATURE,
  });

  assert.equal(res.status, 403);
  assert.equal(res.body.ok, false);
  assert.equal(res.body.error, "nni_pubkey_not_allowlisted");
  assert.equal(res.body.data.status, "public_key_not_allowlisted");

  const state = JSON.parse(await readFile(server.statePath, "utf8"));
  assert.equal(state.tasks[taskId].status, "rejected");
  assert.equal(state.tasks[taskId].error_code, "nni_pubkey_not_allowlisted");
  assert.equal(state.requests[0].status, "blocked");
});

test("heartbeat verify records public key request time and count", async (t) => {
  const fixture = generateSigningFixture();
  const server = await startServer({ publicKeyWhitelist: fixture.pubkey });
  t.after(() => server.stop());

  const request = await postJson(server.baseUrl, "/v1/nni/server/heartbeat/request", {
    device_pubkey: fixture.pubkey,
    client_user_key: "clawd-nni-heartbeat",
  });
  assert.equal(request.status, 200);
  assert.equal(request.body.ok, true);
  assert.equal(request.body.data.status, "heartbeat_challenge_created");
  assert.equal(request.body.data.device_pubkey, fixture.pubkey);

  const signature = fixture.signChallenge(request.body.data.challenge);
  const verify = await postJson(server.baseUrl, "/v1/nni/server/heartbeat/verify", {
    task_id: request.body.data.task_id,
    signature,
  });
  assert.equal(verify.status, 200);
  assert.equal(verify.body.ok, true);
  assert.equal(verify.body.data.status, "heartbeat_accepted");
  assert.equal(verify.body.data.device_pubkey, fixture.pubkey);
  assert.equal(verify.body.data.heartbeat_count, 1);
  assert.equal(typeof verify.body.data.request_time_ts, "number");

  const state = JSON.parse(await readFile(server.statePath, "utf8"));
  const device = state.devices[`clawd-nni-heartbeat:${fixture.pubkey}`];
  assert.equal(device.device_pubkey, fixture.pubkey);
  assert.equal(device.heartbeat_count, 1);
  assert.equal(device.last_heartbeat_ts, verify.body.data.request_time_ts);
  assert.equal(state.requests[0].request_kind, "nni_heartbeat");
  assert.equal(state.requests[0].device_pubkey, fixture.pubkey);
  assert.equal(state.requests[0].created_at_ts, verify.body.data.request_time_ts);
  assert.equal(state.requests[0].status, "accepted");
});

test("request records are stored but not exposed through public query endpoints", async (t) => {
  const requests = [
    {
      id: 1,
      request_kind: "nni_join",
      task_id: "join-visible",
      user_key: "ui-user",
      device_pubkey: VALID_PUBKEY,
      challenge: "00".repeat(32),
      signature: "11".repeat(64),
      compliant: true,
      status: "accepted",
      error_code: null,
      created_at_ts: 1_800_000_000,
    },
  ];
  const server = await startServer({
    initialState: {
      tasks: {},
      devices: {},
      requests,
      public_key_whitelist: [VALID_PUBKEY],
    },
  });
  t.after(() => server.stop());

  const records = await getJson(server.baseUrl, "/v1/nni/server/records?page=1&per_page=10");
  assert.equal(records.status, 404);
  assert.equal(records.body.ok, false);
  assert.equal(records.body.error, "not_found");

  const legacyRecords = await getJson(server.baseUrl, "/v1/nni/server/heartbeat/records?page=1&per_page=10");
  assert.equal(legacyRecords.status, 404);
  assert.equal(legacyRecords.body.ok, false);
  assert.equal(legacyRecords.body.error, "not_found");

  const state = JSON.parse(await readFile(server.statePath, "utf8"));
  assert.equal(state.requests.length, 1);
  assert.equal(state.requests[0].task_id, "join-visible");
});
