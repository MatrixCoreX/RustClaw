import assert from "node:assert/strict";
import test from "node:test";

import {
  fetchTaskArtifactBlob,
  MAX_AUTOMATIC_ARTIFACT_PREVIEW_BYTES,
  taskArtifactBrowserVideoUrl,
  taskArtifactVideoPosterUrl,
} from "./task-artifact-content";

test("loads protected artifact content through the authenticated API fetcher", async () => {
  const calls: Array<{ path: string; signal?: AbortSignal }> = [];
  const controller = new AbortController();
  const blob = await fetchTaskArtifactBlob(
    async (path, init) => {
      calls.push({ path, signal: init?.signal ?? undefined });
      return new Response(new Uint8Array([1, 2, 3]), {
        status: 200,
        headers: { "content-type": "video/mp4" },
      });
    },
    "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline",
    controller.signal,
  );

  assert.equal(blob.size, 3);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path.includes("disposition=inline"), true);
  assert.equal(calls[0].signal, controller.signal);
  assert.equal(MAX_AUTOMATIC_ARTIFACT_PREVIEW_BYTES, 25 * 1024 * 1024);
});

test("rejects external artifact URLs before calling the API fetcher", async () => {
  let called = false;
  await assert.rejects(
    fetchTaskArtifactBlob(async () => {
      called = true;
      return new Response();
    }, "https://example.com/video.mp4"),
    /task_artifact_url_invalid/,
  );
  assert.equal(called, false);
});

test("derives a protected video poster URL without exposing an external origin", () => {
  assert.equal(
    taskArtifactVideoPosterUrl(
      "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline",
    ),
    "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline&preview=poster",
  );
  assert.equal(taskArtifactVideoPosterUrl("https://example.com/video.mp4"), null);
});

test("derives a browser-compatible video preview URL while preserving the download URL", () => {
  const original = "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline";
  assert.equal(
    taskArtifactBrowserVideoUrl(original),
    "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline&preview=browser",
  );
  assert.equal(
    original,
    "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline",
  );
  assert.equal(taskArtifactBrowserVideoUrl("https://example.com/video.mp4"), null);
});

test("an already cancelled artifact read never starts a request", async () => {
  const controller = new AbortController(); controller.abort();
  let calls = 0;
  await assert.rejects(fetchTaskArtifactBlob(async () => { calls++; return new Response("obsolete"); },
    "/v1/tasks/task-1/artifacts/artifact-1/content", controller.signal), { name: "AbortError" });
  assert.equal(calls, 0);
});

test("a response arriving after cancellation is stopped without reading its body", async () => {
  const controller = new AbortController(); let cancelled = 0, reads = 0;
  const body = new ReadableStream({ cancel() { cancelled++; } });
  const response = new Response(body);
  response.blob = async () => { reads++; return new Blob(["obsolete"]); };
  await assert.rejects(fetchTaskArtifactBlob(async () => { controller.abort(); return response; },
    "/v1/tasks/task-1/artifacts/artifact-1/content", controller.signal), { name: "AbortError" });
  assert.equal(cancelled, 1); assert.equal(reads, 0);
});

test("cancellation during body decoding cannot return obsolete bytes", async () => {
  const controller = new AbortController();
  const response = new Response("video");
  response.blob = async () => { controller.abort(); return new Blob(["obsolete"]); };
  await assert.rejects(fetchTaskArtifactBlob(async () => response,
    "/v1/tasks/task-1/artifacts/artifact-1/content", controller.signal), { name: "AbortError" });
});

test("HTTP error bodies are cancelled and cleanup failures cannot mask the HTTP error", async () => {
  for (const cleanup of ["ok", "reject", "pending"]) {
    let cancelled = 0;
    const body = new ReadableStream({ cancel() {
      cancelled++;
      if (cleanup === "reject") return Promise.reject(new Error("cleanup failed"));
      if (cleanup === "pending") return new Promise<void>(() => undefined);
    } });
    await assert.rejects(fetchTaskArtifactBlob(async () => new Response(body, { status: 503 }),
      "/v1/tasks/task-1/artifacts/artifact-1/content"), /task_artifact_http_503/);
    assert.equal(cancelled, 1);
  }
});
