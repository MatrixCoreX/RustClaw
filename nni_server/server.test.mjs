import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createServer } from "node:net";
import test from "node:test";
import { setTimeout as delay } from "node:timers/promises";

const VALID_PUBKEY = "aa".repeat(64);
const OTHER_PUBKEY = "bb".repeat(64);
const VALID_SIGNATURE = "11".repeat(64);

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
  const child = spawn(process.execPath, ["server.mjs"], {
    cwd: new URL(".", import.meta.url),
    env: {
      ...process.env,
      NNI_SERVER_HOST: "127.0.0.1",
      NNI_SERVER_PORT: String(port),
      NNI_SERVER_STATE_PATH: statePath,
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
