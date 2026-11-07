import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react";
import { begin, deferred, mount } from "./memory-runtime.fixture";
import { BoundedFile, response, server } from "../lib/memory-import-upload.fixture";

const input = () => {
  const raw = new TextEncoder().encode("{\"items\":[]}");
  return { raw, file: new BoundedFile([raw], "memory.json") };
};

test("memory staged import asks once for whole-operation approval before one canonical confirmation", async () => {
  const { raw, file } = input(), remote = server(raw, 3), approval = deferred<boolean>();
  let approvals = 0;
  const host = await mount(remote.api, "A", () => { approvals++; return approval.promise; });
  try {
    const task = await begin(() => host.runtime.importMemory(file));
    assert.equal(host.runtime.memoryImportProgress?.phase, "preview");
    assert.equal(remote.previews, 1); assert.equal(remote.confirms, 0); assert.equal(approvals, 1);
    await act(async () => { approval.resolve(true); await task.pending; });
    assert.equal(remote.confirms, 1); assert.equal(approvals, 1);
    assert.equal(host.runtime.memoryImportProgress, null); assert.equal(host.runtime.memoryActionLoading, null);
    assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});

test("memory staged import declined approval cancels its pending plaintext without canonical confirmation", async () => {
  const { raw, file } = input(), remote = server(raw);
  const host = await mount(remote.api, "A", async () => false);
  try {
    await act(async () => { await host.runtime.importMemory(file); });
    assert.equal(remote.previews, 1); assert.equal(remote.confirms, 0); assert.equal(remote.cancels, 1);
    assert.equal(host.runtime.memoryImportProgress, null); assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});

test("memory staged import cancellation aborts the pending request and fences a late chunk response", async () => {
  const { raw, file } = input(), remote = server(raw), held = deferred<Response>();
  remote.onRequest = async path => path.includes("/chunks?") ? held.promise : undefined;
  const host = await mount(remote.api);
  try {
    const task = await begin(() => host.runtime.importMemory(file));
    const chunk = remote.calls.find(c => c.method === "PUT");
    assert.ok(chunk); assert.equal(chunk.init?.signal?.aborted, false);
    await act(async () => { await host.runtime.cancelMemoryImport(); });
    assert.equal(chunk.init?.signal?.aborted, true); assert.equal(remote.cancels, 1);
    await act(async () => { held.resolve(response(remote.status())); await task.pending; });
    assert.equal(remote.previews, 0); assert.equal(remote.confirms, 0);
    assert.equal(host.runtime.memoryImportProgress, null); assert.equal(host.runtime.memoryError, null);
  } finally { held.resolve(response(remote.status())); await host.close(); }
});

test("memory staged import A-to-B-to-A cannot revive old uploads or issue cancellation through the new owner", async () => {
  const { raw, file } = input(), remote = server(raw), held = deferred<Response>();
  remote.onRequest = async path => path.includes("/chunks?") ? held.promise : undefined;
  const host = await mount(remote.api);
  try {
    const task = await begin(() => host.runtime.importMemory(file));
    const count = remote.calls.length;
    assert.ok(remote.calls.some(c => c.method === "PUT"));
    await host.scope("B"); await host.scope("A");
    assert.equal(host.runtime.memoryImportProgress, null);
    await act(async () => { held.resolve(response(remote.status())); await task.pending; });
    assert.equal(remote.calls.length, count); assert.equal(remote.confirms, 0); assert.equal(remote.cancels, 0);
    assert.equal(host.runtime.memoryImportProgress, null); assert.equal(host.runtime.memoryError, null);
    assert.equal(host.runtime.memoryMessage, null);
  } finally { held.resolve(response(remote.status())); await host.close(); }
});

test("memory staged import retries a lost append response from its durable offset", async () => {
  const { raw, file } = input(), remote = server(raw, 3); remote.failAfterAppend = true;
  const host = await mount(remote.api);
  try {
    await act(async () => { await host.runtime.importMemory(file); });
    assert.equal(host.runtime.memoryImportProgress?.phase, "paused");
    assert.equal(remote.puts.length, 1); assert.equal(remote.confirms, 0);
    await act(async () => { await host.runtime.resumeMemoryImport(); });
    assert.deepEqual(remote.puts.map(p => p.offset), [0, 3, 6, 9]);
    assert.equal(remote.calls.filter(c => c.path === "/v1/memory/import/uploads").length, 1);
    assert.equal(remote.confirms, 1); assert.equal(host.runtime.memoryImportProgress, null);
    assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});

test("memory staged import reconciles a lost confirmation response without confirming again", async () => {
  const { raw, file } = input(), remote = server(raw), api = remote.api;
  let lose = true;
  const host = await mount(async (path, init) => {
    const result = await api(path, init);
    if (path === "/v1/memory/import/confirm" && lose) { lose = false; throw new TypeError("lost reply"); }
    return result;
  });
  try {
    await act(async () => { await host.runtime.importMemory(file); });
    assert.equal(host.runtime.memoryImportProgress?.phase, "paused"); assert.equal(remote.confirms, 1);
    await act(async () => { await host.runtime.resumeMemoryImport(); });
    assert.equal(remote.confirms, 1); assert.equal(remote.previews, 1);
    assert.equal(remote.calls.filter(c => c.path === "/v1/memory/import/uploads").length, 1);
    assert.equal(host.runtime.memoryImportProgress, null); assert.equal(host.runtime.memoryError, null);
    assert.ok(host.runtime.memoryMessage);
  } finally { await host.close(); }
});

test("memory staged import failed cancellation remains visible and recoverable rather than claiming removal", async () => {
  const { raw, file } = input(), remote = server(raw, 3); remote.failAfterAppend = true;
  const host = await mount(remote.api);
  try {
    await act(async () => { await host.runtime.importMemory(file); });
    remote.onRequest = async (_path, init) => init?.method === "DELETE" ? response("memory_import_storage_busy", 503) : undefined;
    await act(async () => { await host.runtime.cancelMemoryImport(); });
    assert.equal(remote.cancels, 0); assert.equal(remote.confirms, 0);
    assert.equal(host.runtime.memoryImportProgress?.phase, "paused"); assert.ok(host.runtime.memoryError);
    remote.onRequest = null;
    await act(async () => { await host.runtime.resumeMemoryImport(); });
    assert.equal(remote.confirms, 1); assert.equal(host.runtime.memoryImportProgress, null);
    assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});

test("memory staged import cancellation cannot clear a newer unrelated operation's busy state", async () => {
  const { raw, file } = input(), remote = server(raw), chunk = deferred<Response>(), correction = deferred<Response>();
  remote.onRequest = async path => path.includes("/chunks?") ? chunk.promise
    : path.endsWith("/correct") ? correction.promise : undefined;
  const host = await mount(remote.api);
  try {
    const uploading = await begin(() => host.runtime.importMemory(file));
    const writing = await begin(() => host.runtime.correctMemoryItem("fresh", 1, "new fact"));
    assert.equal(host.runtime.memoryActionLoading, "correct:fresh");
    await act(async () => { await host.runtime.cancelMemoryImport(); });
    await act(async () => { chunk.resolve(response(remote.status())); await uploading.pending; });
    assert.equal(host.runtime.memoryActionLoading, "correct:fresh");
    await act(async () => { correction.resolve(response({ revision_id: "fixture-undo" })); await writing.pending; });
    assert.equal(host.runtime.memoryActionLoading, null);
  } finally { chunk.resolve(response(remote.status())); correction.resolve(response({ revision_id: "fixture-undo" })); await host.close(); }
});
