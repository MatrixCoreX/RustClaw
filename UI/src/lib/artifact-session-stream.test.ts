import assert from "node:assert/strict";
import test from "node:test";
import { withSessionArtifactStreaming } from "./artifact-session-stream";
import { fetchTaskArtifactVideoSource } from "./task-artifact-content";

const path = "/v1/tasks/stream-test/artifacts/video/content?disposition=inline";
const defaults = { authMode: "webd" as const, apiBase: "https://console.example", pageUrl: "https://console.example/ui", isCurrent: () => true };

test("same-origin Web login streams only canonical, original artifact URLs without credentials in the URL", async () => {
  let calls = 0;
  const fetcher = withSessionArtifactStreaming(async () => { calls++; return new Response(); }, defaults);
  assert.equal(fetcher.nativeMediaUrl?.(path), `https://console.example${path}`);
  assert.equal(calls, 0);
  for (const invalid of ["//other.example/file", "https://other.example/file", "/v1/auth/me", `${path}&preview=browser`, `${path}&preview=poster`, `${path}&key=secret`, `${path}#fragment`]) {
    assert.equal(fetcher.nativeMediaUrl?.(invalid), null, invalid);
  }
});

test("key login, unauthenticated, stale, cross-origin, credentialed and non-HTTP transports never expose native URLs", () => {
  for (const changes of [
    { authMode: "key" as const }, { authMode: null }, { isCurrent: () => false },
    { apiBase: "https://other.example" }, { apiBase: "http://console.example" },
    { apiBase: "https://key@console.example" }, { apiBase: "https://console.example/?key=secret" },
    { apiBase: "https://console.example/#secret" }, { pageUrl: "tauri://localhost/" }, { pageUrl: "not a URL" },
  ]) {
    const fetcher = withSessionArtifactStreaming(async () => new Response(), { ...defaults, ...changes });
    assert.equal(fetcher.nativeMediaUrl?.(path), null, JSON.stringify(changes));
  }
});

test("relative API bases and same-origin LAN HTTP can reuse an existing cookie session", () => {
  for (const apiBase of ["", "/", "http://192.168.1.8/"]) {
    const fetcher = withSessionArtifactStreaming(async () => new Response(), { ...defaults, pageUrl: "http://192.168.1.8/", apiBase });
    assert.equal(fetcher.nativeMediaUrl?.(path), `http://192.168.1.8${path}`);
  }
});

test("streaming authenticates using HEAD without reading video bytes and carries cancellation", async () => {
  const controller = new AbortController();
  const calls: RequestInit[] = [];
  const fetcher = withSessionArtifactStreaming(async (endpoint, init) => {
    assert.equal(endpoint, path); calls.push(init!);
    const response = new Response(null, { headers: { "accept-ranges": "bytes", "content-type": "video/webm" } });
    response.blob = async () => { throw new Error("must not buffer native video"); };
    return response;
  }, defaults);
  assert.deepEqual(await fetchTaskArtifactVideoSource(fetcher, path, controller.signal), { kind: "native", url: `https://console.example${path}` });
  assert.equal(calls.length, 1); assert.equal(calls[0].method, "HEAD"); assert.equal(calls[0].signal, controller.signal);
});

test("servers without Range support retain authenticated blob fetch, and key login adds no HEAD", async () => {
  for (const native of [true, false]) {
    const methods: string[] = [];
    const base = async (_path: string, init?: RequestInit) => {
      methods.push(init?.method || "GET"); return new Response("video", { headers: { "content-type": "video/mp4" } });
    };
    const fetcher = native ? withSessionArtifactStreaming(base, defaults) : base;
    const source = await fetchTaskArtifactVideoSource(fetcher, path, new AbortController().signal);
    assert.equal(source.kind, "blob"); if (source.kind === "blob") assert.equal(await source.blob.text(), "video");
    assert.deepEqual(methods, native ? ["HEAD", "GET"] : ["GET"]);
  }
});

test("failed authorization never hands a native source to the player or retries a body fetch", async () => {
  for (const status of [401, 403, 404, 503]) {
    let calls = 0;
    const fetcher = withSessionArtifactStreaming(async () => { calls++; return new Response(null, { status }); }, defaults);
    await assert.rejects(fetchTaskArtifactVideoSource(fetcher, path, new AbortController().signal), new RegExp(`task_artifact_http_${status}`));
    assert.equal(calls, 1);
  }
});

test("ownership changes and cancellation while HEAD is pending cannot publish a native source", async () => {
  for (const cancel of [false, true]) {
    let current = true;
    const controller = new AbortController();
    const fetcher = withSessionArtifactStreaming(async () => {
      if (cancel) controller.abort(); else current = false;
      return new Response(null, { headers: { "accept-ranges": "bytes", "content-type": "video/mp4" } });
    }, { ...defaults, isCurrent: () => current });
    await assert.rejects(fetchTaskArtifactVideoSource(fetcher, path, controller.signal), { name: "AbortError" });
  }
});

test("transformed previews keep the existing authenticated content path", async () => {
  const methods: string[] = [];
  const fetcher = withSessionArtifactStreaming(async (_path, init) => { methods.push(init?.method || "GET"); return new Response("converted"); }, defaults);
  assert.equal((await fetchTaskArtifactVideoSource(fetcher, `${path}&preview=browser`, new AbortController().signal)).kind, "blob");
  assert.deepEqual(methods, ["GET"]);
});
