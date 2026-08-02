import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { generateKeyPairSync, sign as signMessage } from "node:crypto";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createServer } from "node:net";
import { DatabaseSync } from "node:sqlite";
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

async function startServer({ publicKeyWhitelist = "", initialState = null, dataDir = null } = {}) {
  const dir = dataDir || (await mkdtemp(path.join(tmpdir(), "agent-runtime-nni-server-test-")));
  const statePath = path.join(dir, "state.json");
  if (initialState) {
    await writeFile(statePath, `${JSON.stringify(initialState, null, 2)}\n`, "utf8");
  }
  const databasePath = path.join(dir, "nni-server.sqlite3");
  const port = await freePort();
  const logPath = path.join(dir, "nni-server.log");
  const child = spawn(process.execPath, ["server.mjs"], {
    cwd: new URL(".", import.meta.url),
    env: {
      ...process.env,
      NNI_SERVER_HOST: "127.0.0.1",
      NNI_SERVER_PORT: String(port),
      NNI_SERVER_DATABASE_PATH: databasePath,
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
          dataDir: dir,
          databasePath,
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

function readDatabaseSnapshot(databasePath) {
  const database = new DatabaseSync(databasePath);
  try {
    return {
      tasks: database.prepare("SELECT * FROM tasks ORDER BY created_at_ts, task_id").all(),
      devices: database.prepare("SELECT * FROM devices ORDER BY device_pubkey, user_key").all(),
      requests: database.prepare("SELECT * FROM request_records ORDER BY id").all(),
      heartbeats: database
        .prepare(`
          SELECT
            heartbeat_records.id,
            devices.user_key,
            devices.device_pubkey,
            heartbeat_records.heartbeat_at_unix
          FROM heartbeat_records
          JOIN devices ON devices.id = heartbeat_records.device_id
          ORDER BY heartbeat_records.id
        `)
        .all(),
      whitelist: database
        .prepare("SELECT device_pubkey FROM public_key_whitelist ORDER BY device_pubkey")
        .all()
        .map((row) => row.device_pubkey),
      integrityCheck: database.prepare("PRAGMA integrity_check").get().integrity_check,
      journalMode: database.prepare("PRAGMA journal_mode").get().journal_mode,
      heartbeatIndexes: database
        .prepare("PRAGMA index_list('heartbeat_records')")
        .all()
        .map((row) => row.name),
    };
  } finally {
    database.close();
  }
}

async function sendSignedHeartbeat(baseUrl, fixture, userKey = "clawd-nni-heartbeat") {
  const request = await postJson(baseUrl, "/v1/nni/server/heartbeat/request", {
    device_pubkey: fixture.pubkey,
    client_user_key: userKey,
  });
  assert.equal(request.status, 200);
  assert.equal(request.body.ok, true);
  const signature = fixture.signChallenge(request.body.data.challenge);
  const verify = await postJson(baseUrl, "/v1/nni/server/heartbeat/verify", {
    task_id: request.body.data.task_id,
    signature,
  });
  assert.equal(verify.status, 200);
  assert.equal(verify.body.ok, true);
  return verify.body.data;
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

  const health = await getJson(server.baseUrl, "/v1/health");
  assert.equal(health.status, 200);
  assert.equal(health.body.data.storage, "sqlite");

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

  const snapshot = readDatabaseSnapshot(server.databasePath);
  assert.deepEqual(snapshot.whitelist, [VALID_PUBKEY]);
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

  const snapshot = readDatabaseSnapshot(server.databasePath);
  assert.equal(snapshot.tasks[0].status, "rejected");
  assert.equal(snapshot.tasks[0].error_code, "nni_pubkey_not_allowlisted");
  assert.equal(snapshot.requests[0].status, "blocked");
});

test("heartbeat verify records public key request time and count", async (t) => {
  const fixture = generateSigningFixture();
  const server = await startServer({ publicKeyWhitelist: fixture.pubkey });
  t.after(() => server.stop());
  const beforeUnix = Math.floor(Date.now() / 1000);

  const heartbeat = await sendSignedHeartbeat(server.baseUrl, fixture);
  const afterUnix = Math.floor(Date.now() / 1000);
  assert.equal(heartbeat.status, "heartbeat_accepted");
  assert.equal(heartbeat.device_pubkey, fixture.pubkey);
  assert.equal(heartbeat.heartbeat_count, 1);
  assert.equal(Number.isSafeInteger(heartbeat.heartbeat_at_unix), true);
  assert.equal(heartbeat.heartbeat_at_unix >= beforeUnix, true);
  assert.equal(heartbeat.heartbeat_at_unix <= afterUnix, true);
  assert.equal(heartbeat.request_time_ts, heartbeat.heartbeat_at_unix);

  const snapshot = readDatabaseSnapshot(server.databasePath);
  const device = snapshot.devices[0];
  assert.equal(device.device_pubkey, fixture.pubkey);
  assert.equal(device.heartbeat_count, 1);
  assert.equal(device.first_heartbeat_ts, heartbeat.heartbeat_at_unix);
  assert.equal(device.last_heartbeat_ts, heartbeat.heartbeat_at_unix);
  assert.equal(snapshot.heartbeats.length, 1);
  assert.equal(snapshot.heartbeats[0].device_pubkey, fixture.pubkey);
  assert.equal(snapshot.heartbeats[0].heartbeat_at_unix, heartbeat.heartbeat_at_unix);
  assert.equal(snapshot.requests[0].request_kind, "nni_heartbeat");
  assert.equal(snapshot.requests[0].device_pubkey, fixture.pubkey);
  assert.equal(snapshot.requests[0].created_at_ts, heartbeat.heartbeat_at_unix);
  assert.equal(snapshot.requests[0].status, "accepted");
  assert.equal(snapshot.integrityCheck, "ok");
  assert.equal(snapshot.journalMode, "wal");
  assert(snapshot.heartbeatIndexes.includes("idx_heartbeat_records_device_time"));
});

test("every concurrent heartbeat is retained per device public key as UNIX seconds", async (t) => {
  const firstDevice = generateSigningFixture();
  const secondDevice = generateSigningFixture();
  const server = await startServer({
    publicKeyWhitelist: `${firstDevice.pubkey},${secondDevice.pubkey}`,
  });
  t.after(() => server.stop());
  const beforeUnix = Math.floor(Date.now() / 1000);

  const heartbeats = await Promise.all([
    sendSignedHeartbeat(server.baseUrl, firstDevice),
    sendSignedHeartbeat(server.baseUrl, firstDevice),
    sendSignedHeartbeat(server.baseUrl, secondDevice),
  ]);
  const afterUnix = Math.floor(Date.now() / 1000);

  const snapshot = readDatabaseSnapshot(server.databasePath);
  const first = snapshot.devices.find((device) => device.device_pubkey === firstDevice.pubkey);
  const second = snapshot.devices.find((device) => device.device_pubkey === secondDevice.pubkey);
  assert.equal(first.device_pubkey, firstDevice.pubkey);
  assert.equal(first.heartbeat_count, 2);
  assert.equal(second.device_pubkey, secondDevice.pubkey);
  assert.equal(second.heartbeat_count, 1);
  assert.equal(snapshot.heartbeats.length, 3);
  assert.equal(snapshot.requests.filter((record) => record.request_kind === "nni_heartbeat").length, 3);
  for (const timestamp of snapshot.heartbeats.map((record) => record.heartbeat_at_unix)) {
    assert.equal(Number.isSafeInteger(timestamp), true);
    assert.equal(timestamp >= beforeUnix, true);
    assert.equal(timestamp <= afterUnix, true);
  }
  assert.deepEqual(
    heartbeats.map((heartbeat) => heartbeat.heartbeat_at_unix).sort(),
    snapshot.heartbeats.map((record) => record.heartbeat_at_unix).sort(),
  );
});

test("SQLite keeps one indexed heartbeat row for each of many devices", async (t) => {
  const devices = Array.from({ length: 16 }, () => generateSigningFixture());
  const server = await startServer({
    publicKeyWhitelist: devices.map((device) => device.pubkey).join(","),
  });
  t.after(() => server.stop());

  await Promise.all(devices.map((device) => sendSignedHeartbeat(server.baseUrl, device)));

  const snapshot = readDatabaseSnapshot(server.databasePath);
  assert.equal(snapshot.devices.length, devices.length);
  assert.equal(snapshot.heartbeats.length, devices.length);
  assert.deepEqual(
    snapshot.heartbeats.map((record) => record.device_pubkey).sort(),
    devices.map((device) => device.pubkey).sort(),
  );
  assert(snapshot.devices.every((device) => device.heartbeat_count === 1));
});

test("heartbeat rows survive an NNI server restart", async (t) => {
  const fixture = generateSigningFixture();
  const firstServer = await startServer({ publicKeyWhitelist: fixture.pubkey });
  await sendSignedHeartbeat(firstServer.baseUrl, fixture);
  await firstServer.stop();

  const restartedServer = await startServer({
    publicKeyWhitelist: fixture.pubkey,
    dataDir: firstServer.dataDir,
  });
  t.after(() => restartedServer.stop());
  const secondHeartbeat = await sendSignedHeartbeat(restartedServer.baseUrl, fixture);

  const snapshot = readDatabaseSnapshot(restartedServer.databasePath);
  assert.equal(snapshot.heartbeats.length, 2);
  assert.equal(snapshot.devices[0].heartbeat_count, 2);
  assert.equal(snapshot.devices[0].last_heartbeat_ts, secondHeartbeat.heartbeat_at_unix);
});

test("legacy accepted heartbeat requests backfill the per-device UNIX history", async (t) => {
  const fixture = generateSigningFixture();
  const userKey = "clawd-nni-heartbeat";
  const legacyUnixTimes = [1_700_000_001, 1_700_000_002];
  const legacyRequests = legacyUnixTimes.map((createdAtTs, index) => ({
    id: index + 1,
    request_kind: "nni_heartbeat",
    task_id: `legacy-heartbeat-${index + 1}`,
    user_key: userKey,
    device_pubkey: fixture.pubkey,
    challenge: "00".repeat(32),
    signature: "11".repeat(64),
    compliant: true,
    status: "accepted",
    error_code: null,
    created_at_ts: createdAtTs,
  }));
  const server = await startServer({
    publicKeyWhitelist: fixture.pubkey,
    initialState: {
      tasks: {},
      devices: {
        [`${userKey}:${fixture.pubkey}`]: {
          user_key: userKey,
          device_pubkey: fixture.pubkey,
          heartbeat_count: 2,
          last_heartbeat_ts: legacyUnixTimes.at(-1),
          status: "heartbeat",
        },
      },
      requests: legacyRequests,
      public_key_whitelist: [fixture.pubkey],
    },
  });
  t.after(() => server.stop());

  const heartbeat = await sendSignedHeartbeat(server.baseUrl, fixture, userKey);
  const snapshot = readDatabaseSnapshot(server.databasePath);
  const device = snapshot.devices[0];
  assert.equal(device.heartbeat_count, 3);
  assert.deepEqual(
    snapshot.heartbeats.map((record) => record.heartbeat_at_unix),
    [...legacyUnixTimes, heartbeat.heartbeat_at_unix],
  );
  assert.equal(device.first_heartbeat_ts, legacyUnixTimes[0]);
  assert.equal(device.last_heartbeat_ts, heartbeat.heartbeat_at_unix);

  await server.stop();
  const restartedServer = await startServer({
    publicKeyWhitelist: fixture.pubkey,
    dataDir: server.dataDir,
  });
  t.after(() => restartedServer.stop());
  const afterRestart = readDatabaseSnapshot(restartedServer.databasePath);
  assert.equal(afterRestart.heartbeats.length, 3);
  assert.equal(afterRestart.requests.length, 3);
  assert.equal(afterRestart.integrityCheck, "ok");
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

  const snapshot = readDatabaseSnapshot(server.databasePath);
  assert.equal(snapshot.requests.length, 1);
  assert.equal(snapshot.requests[0].task_id, "join-visible");
});
