const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

test("core requests reject redirects without forwarding channel credentials", async (t) => {
  const key = "fixture-channel-credential";
  const requests = [];
  let redirectStatus = 200;
  let redirectOrigin = "";
  const handler = async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    requests.push({ url: request.url, method: request.method,
      key: request.headers["x-agent-key"], body: Buffer.concat(chunks).toString() });
    if (redirectStatus !== 200 && request.url !== "/destination") {
      response.writeHead(redirectStatus, { location: `${redirectOrigin}/destination` });
    } else {
      response.writeHead(200, { "content-type": "application/json" });
    }
    response.end(JSON.stringify({ ok: true, data: {
      task_id: "fixture-task", status: "accepted", accepted: true,
      identity: { user_key: key },
    } }));
  };
  const servers = [http.createServer(handler), http.createServer(handler)];
  t.after(async () => {
    for (const server of servers) {
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    }
  });
  for (const server of servers) {
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  }
  const origins = servers.map((server) => `http://127.0.0.1:${server.address().port}`);
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "agent-channel-http-"));
  t.after(() => fs.rmSync(workspace, { recursive: true, force: true }));
  const config = path.join(workspace, "config.toml");
  fs.writeFileSync(config, `[webd]\nupstream = "${origins[0]}"\n`);
  process.env.APP_WORKSPACE_ROOT = workspace;
  process.env.APP_CONFIG_PATH = config;
  process.env.APP_CHANNEL_CONFIG_DIR = workspace;
  process.env.APP_INTERNAL_API_BASE_URL = origins[0];
  const bridge = require("./index.js");
  const identity = { user_key: key };
  const calls = {
    resolve: () => bridge.resolveIdentity("fixture-user", "fixture-chat"),
    bind: () => bridge.bindIdentity("fixture-user", "fixture-chat", key),
    submit: () => bridge.submitTask("fixture-user", "fixture-chat", "ask",
      { text: "fixture" }, identity, "fixture-message"),
    query: () => bridge.queryTask("fixture-task", identity),
    delivery: () => bridge.requestTaskDelivery("fixture-task", identity),
  };
  for (const status of [301, 302, 303, 307, 308]) {
    for (const origin of origins) {
      for (const [name, call] of Object.entries(calls)) {
        await t.test(`${name} ${status} cross_origin=${origin !== origins[0]}`, async () => {
          requests.length = 0;
          redirectStatus = status;
          redirectOrigin = origin;
          await assert.rejects(call, "redirect must not become a successful core request");
          assert.equal(requests.length, 1, "redirected request must not be sent");
        });
      }
    }
  }
  redirectStatus = 200;
  for (const [name, call] of Object.entries(calls)) {
    await t.test(`direct ${name} succeeds without changing authentication`, async () => {
      requests.length = 0;
      await call();
      assert.equal(requests.length, 1);
      if (name === "bind") {
        assert.equal(JSON.parse(requests[0].body).user_key, key);
      } else if (name !== "resolve") {
        assert.equal(requests[0].key, key);
      }
    });
  }
});
