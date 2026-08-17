import assert from "node:assert/strict";
import test from "node:test";

import {
  fetchResilientRead,
  runCoalescedRead,
  runCoalescedResponseRead,
} from "./resilient-read";

test("resilient read retries transient network failures", async () => {
  let attempts = 0;
  const response = await fetchResilientRead(
    async () => {
      attempts += 1;
      if (attempts < 3) throw new TypeError("network unavailable");
      return new Response("ok", { status: 200 });
    },
    "/v1/status",
    undefined,
    { retryDelaysMs: [0, 0] },
  );

  assert.equal(await response.text(), "ok");
  assert.equal(attempts, 3);
});

test("resilient read does not retry application JSON errors", async () => {
  let attempts = 0;
  const response = await fetchResilientRead(
    async () => {
      attempts += 1;
      return new Response(JSON.stringify({ ok: false, error: "remote_unavailable" }), {
        status: 502,
        headers: { "content-type": "application/json" },
      });
    },
    "/v1/status",
    undefined,
    { retryDelaysMs: [0, 0] },
  );

  assert.equal(response.status, 502);
  assert.equal(attempts, 1);
});

test("resilient read retries temporary proxy responses", async () => {
  let attempts = 0;
  const response = await fetchResilientRead(
    async () => {
      attempts += 1;
      return attempts === 1
        ? new Response("upstream restarting", { status: 502, headers: { "content-type": "text/html" } })
        : new Response("ok", { status: 200 });
    },
    "/v1/status",
    undefined,
    { retryDelaysMs: [0] },
  );

  assert.equal(await response.text(), "ok");
  assert.equal(attempts, 2);
});

test("coalesced reads share one in-flight request", async () => {
  const inFlight = new Map<string, Promise<unknown>>();
  let starts = 0;
  let resolveRequest: ((value: number) => void) | undefined;
  const start = () => {
    starts += 1;
    return new Promise<number>((resolve) => {
      resolveRequest = resolve;
    });
  };

  const first = runCoalescedRead(inFlight, "market", start);
  const second = runCoalescedRead(inFlight, "market", start);
  assert.equal(first, second);
  assert.equal(starts, 1);
  resolveRequest?.(7);
  assert.equal(await second, 7);
  await Promise.resolve();
  assert.equal(inFlight.size, 0);
});

test("coalesced response reads clone the shared response for every consumer", async () => {
  const inFlight = new Map<string, Promise<Response>>();
  let starts = 0;
  let resolveRequest: ((response: Response) => void) | undefined;
  const start = () => {
    starts += 1;
    return new Promise<Response>((resolve) => {
      resolveRequest = resolve;
    });
  };

  const first = runCoalescedResponseRead(inFlight, "skills-config", start);
  const second = runCoalescedResponseRead(inFlight, "skills-config", start);
  resolveRequest?.(Response.json({ enabled: true }));
  const [firstResponse, secondResponse] = await Promise.all([first, second]);

  assert.equal(starts, 1);
  assert.deepEqual(await firstResponse.json(), { enabled: true });
  assert.deepEqual(await secondResponse.json(), { enabled: true });
  assert.equal(inFlight.size, 0);
});
