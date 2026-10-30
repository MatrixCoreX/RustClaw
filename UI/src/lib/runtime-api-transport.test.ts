import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { once } from "node:events";
import test from "node:test";
import { runtimeApiFetch } from "./runtime-api-transport";

const KEY = "fixture-browser-api-key";
const PASSWORD = "fixture-browser-api-password";
const CSRF = "ab".repeat(16);

async function fixture(status: number, destination?: string) {
  const requests: { path: string; method: string; key?: string; csrf?: string; body: string }[] = [];
  const server: Server = createServer(async (req, res) => {
    let body = "";
    for await (const chunk of req) body += chunk;
    requests.push({ path: req.url!, method: req.method!, body,
      key: req.headers["x-agent-key"] as string | undefined,
      csrf: req.headers["x-webd-csrf"] as string | undefined });
    const actualStatus = req.url === "/destination" ? 200 : status;
    if (actualStatus >= 300 && actualStatus < 400) {
      res.setHeader("Location", `${destination || base}/destination`);
    }
    res.writeHead(actualStatus, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ ok: actualStatus === 200, data: { value: "fixture-result" } }));
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const base = `http://127.0.0.1:${address.port}`;
  return { base, requests, close: () => new Promise<void>((resolve, reject) => {
    server.close(error => error ? reject(error) : resolve());
    server.closeAllConnections();
  }) };
}

test("API requests reject every redirect without forwarding keys or login bodies", async () => {
  const failures: object[] = [];
  for (const status of [301, 302, 303, 307, 308]) {
    for (const crossOrigin of [false, true]) {
      for (const mode of ["key", "password"]) {
        const target = await fixture(200);
        const origin = await fixture(status, crossOrigin ? target.base : undefined);
        try {
          let rejected = false;
          try {
            await runtimeApiFetch(`${origin.base}/api`, {
              method: mode === "key" ? "GET" : "POST",
              headers: mode === "key" ? { "x-agent-key": KEY } : { "Content-Type": "application/json" },
              body: mode === "key" ? undefined : JSON.stringify({ username: "fixture-user", password: PASSWORD }),
              redirect: "follow",
            });
          } catch (error) {
            assert.ok(error instanceof TypeError);
            rejected = true;
          }
          if (!rejected || origin.requests.length !== 1 || target.requests.length !== 0) {
            const forwarded = [...origin.requests.slice(1), ...target.requests];
            failures.push({ status, crossOrigin, mode, rejected,
              originRequests: origin.requests.length, targetRequests: target.requests.length,
              forwardedKey: forwarded.some(request => request.key === KEY),
              forwardedPassword: forwarded.some(request => request.body.includes(PASSWORD)) });
          }
        } finally {
          await origin.close();
          await target.close();
        }
      }
    }
  }
  assert.deepEqual(failures, []);
});

test("direct API responses retain headers, JSON, HTTP status and body semantics", async () => {
  for (const status of [200, 401, 403, 500]) {
    const origin = await fixture(status);
    try {
      const response = await runtimeApiFetch(`${origin.base}/api`, {
        method: "POST", credentials: "include",
        headers: { "Content-Type": "application/json", "x-agent-key": KEY, "x-webd-csrf": CSRF },
        body: JSON.stringify({ input: "fixture-input" }),
      });
      assert.equal(response.status, status);
      assert.equal(response.ok, status === 200);
      assert.deepEqual(await response.json(), { ok: status === 200, data: { value: "fixture-result" } });
      assert.deepEqual(origin.requests, [{ path: "/api", method: "POST", key: KEY, csrf: CSRF,
        body: JSON.stringify({ input: "fixture-input" }) }]);
    } finally {
      await origin.close();
    }
  }
});

test("transport fixes redirect policy after caller options without changing ownership or cancellation", async context => {
  const abort = new AbortController();
  const headers = new Headers({ "x-webd-csrf": CSRF });
  const request = new Request("https://fixture.invalid/api", { redirect: "follow" });
  const response = new Response("fixture-result");
  const intercepted = context.mock.method(globalThis, "fetch", async (input, init) => {
    assert.equal(input, request);
    assert.equal(init?.credentials, "include");
    assert.equal(init?.headers, headers);
    assert.equal(init?.signal, abort.signal);
    assert.equal(init?.redirect, "error");
    return response;
  });
  try {
    assert.equal(await runtimeApiFetch(request, {
      credentials: "include", headers, signal: abort.signal, redirect: "manual",
    }), response);
    assert.equal(intercepted.mock.callCount(), 1);
  } finally {
    intercepted.mock.restore();
  }
});

test("aborted API calls do not reach the backend", async () => {
  const origin = await fixture(200);
  const abort = new AbortController();
  abort.abort();
  try {
    await assert.rejects(runtimeApiFetch(`${origin.base}/api`, { signal: abort.signal }),
      { name: "AbortError" });
    assert.equal(origin.requests.length, 0);
  } finally {
    await origin.close();
  }
});

test("ordinary uncredentialed media requests keep their own redirect behavior", async () => {
  const target = await fixture(200);
  const origin = await fixture(302, target.base);
  try {
    const response = await fetch(`${origin.base}/media`);
    assert.equal(response.status, 200);
    assert.equal(target.requests.length, 1);
    assert.equal(target.requests[0].key, undefined);
    await response.text();
  } finally {
    await origin.close();
    await target.close();
  }
});
