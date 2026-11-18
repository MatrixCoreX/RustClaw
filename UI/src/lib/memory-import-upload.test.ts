import assert from "node:assert/strict";
import test from "node:test";
import { setImmediate } from "node:timers/promises";
import { cancelMemoryImport, MEMORY_IMPORT_IO_BYTES, MemoryImportTransportError, prepareMemoryImport } from "./memory-import-upload";
import { BoundedFile, options, response, server } from "./memory-import-upload.fixture";

test("staged import reads a large Unicode file in bounded slices without whole-file text or buffers", async () => {
  const raw = new TextEncoder().encode(JSON.stringify({ items: Array.from({ length: 48 }, (_, i) => ({ content: `${i}${"文".repeat(18000)}` })) }));
  assert.ok(raw.byteLength > 2 * 1024 * 1024);
  const file = new BoundedFile([raw], "memory.json");
  const host = server(raw); const control = options(host.api);
  const preview = await prepareMemoryImport(file, control);
  assert.equal(preview.import_id, "fixture-import");
  assert.equal(host.previews, 1); assert.equal(host.confirms, 0);
  assert.equal(host.puts.reduce((n, p) => n + p.bytes.byteLength, 0), raw.byteLength);
  assert.ok(file.reads.every(([a, b]) => b - a <= MEMORY_IMPORT_IO_BYTES));
  assert.ok(host.calls.every(c => c.init?.signal === control.signal));
  assert.deepEqual(control.uploads, ["fixture-upload"]);
});

test("staged import does not impose a ten-thousand-item client ceiling", async () => {
  const raw = new TextEncoder().encode(JSON.stringify({ items: Array.from({ length: 10001 }, (_, i) => ({ content: String(i) })) }));
  const host = server(raw); host.accepted = 10001;
  const result = await prepareMemoryImport(new BoundedFile([raw], "many.json"), options(host.api));
  assert.equal(result.accepted_items, 10001); assert.equal(host.previews, 1); assert.equal(host.confirms, 0);
});

test("staged import resumes a persisted byte offset instead of reuploading the prefix", async () => {
  const raw = new Uint8Array(201).fill(65), host = server(raw, 31);
  host.setReceived(62);
  await prepareMemoryImport(new BoundedFile([raw], "memory.json"), options(host.api));
  assert.equal(host.puts[0]?.offset, 62);
  assert.equal(host.puts.reduce((n, p) => n + p.bytes.length, 0), 139);
  assert.equal(host.previews, 1);
});

test("staged import reconciles an append whose response was lost through the original upload identity", async () => {
  const raw = new Uint8Array(51).fill(66), host = server(raw, 17);
  const first = options(host.api); host.failAfterAppend = true;
  const file = new BoundedFile([raw], "memory.json");
  await assert.rejects(prepareMemoryImport(file, first), TypeError);
  assert.equal(host.puts.length, 1);
  const second = { ...options(host.api), resumeId: first.uploads[0] };
  await prepareMemoryImport(file, second);
  assert.deepEqual(host.puts.map(p => p.offset), [0, 17, 34]);
  assert.equal(host.calls.filter(c => c.path === "/v1/memory/import/uploads").length, 1);
});

test("staged import recognizes a confirmed original receipt without creating another upload or confirming twice", async () => {
  const raw = new TextEncoder().encode("{}"), host = server(raw);
  host.setReceived(raw.length); host.setState("confirmed");
  await assert.rejects(prepareMemoryImport(new BoundedFile([raw], "memory.json"), {
    ...options(host.api), resumeId: "fixture-upload",
  }), (e: unknown) => e instanceof MemoryImportTransportError && e.code === "memory_import_already_confirmed");
  assert.equal(host.calls.filter(c => c.method !== "GET").length, 0);
  assert.equal(host.confirms, 0);
});

test("staged import reuses an already prepared whole preview without uploading plaintext again", async () => {
  const raw = new TextEncoder().encode("{}"), host = server(raw);
  host.setReceived(raw.length); host.setState("preview");
  await prepareMemoryImport(new BoundedFile([raw], "memory.json"), { ...options(host.api), resumeId: "fixture-upload" });
  assert.equal(host.puts.length, 0); assert.equal(host.previews, 1); assert.equal(host.confirms, 0);
});

test("staged import honors the advertised file budget without creating a rejected session", async () => {
  const raw = new TextEncoder().encode("{}"), host = server(raw); host.policy.max_file_bytes = "1";
  await assert.rejects(prepareMemoryImport(new BoundedFile([raw], "memory.json"), options(host.api)),
    (e: unknown) => e instanceof MemoryImportTransportError && e.code === "memory_import_upload_file_too_large");
  assert.deepEqual(host.calls.map(c => c.path), ["/v1/memory/import/policy"]);
});

test("staged import rejects malformed resource policies rather than silently applying a client default", async () => {
  for (const value of ["0", "-1", "1.5", "18446744073709551616", "9007199254740993x"]) {
    const raw = new TextEncoder().encode("{}"), host = server(raw); host.policy.max_chunk_bytes = value;
    await assert.rejects(prepareMemoryImport(new BoundedFile([raw], "memory.json"), options(host.api)), MemoryImportTransportError);
    assert.equal(host.calls.length, 1);
  }
});

test("staged import rejects a status belonging to a different file before sending any chunk", async () => {
  const raw = new TextEncoder().encode("{}"), host = server(raw);
  host.onRequest = async path => path === "/v1/memory/import/uploads" ? response({ ...host.status(), file_digest: `sha256:${"0".repeat(64)}` }) : undefined;
  await assert.rejects(prepareMemoryImport(new BoundedFile([raw], "memory.json"), options(host.api)), MemoryImportTransportError);
  assert.equal(host.puts.length, 0); assert.equal(host.previews, 0);
});

test("staged import owner revocation during file hashing prevents every network request", async () => {
  let owns = true;
  const raw = new TextEncoder().encode("{}"), host = server(raw);
  class ChangedFile extends BoundedFile {
    override slice(start?: number, end?: number) { owns = false; return super.slice(start, end); }
  }
  await assert.rejects(prepareMemoryImport(new ChangedFile([raw], "memory.json"), { ...options(host.api), owns: () => owns }), MemoryImportTransportError);
  assert.equal(host.calls.length, 0);
});

test("staged import owner revocation after a chunk response prevents later chunks and preview", async () => {
  let owns = true;
  const raw = new Uint8Array(60), host = server(raw, 20), base = host.api;
  const api = async (path: string, init?: RequestInit) => {
    const reply = await base(path, init); if (path.includes("/chunks?")) owns = false; return reply;
  };
  await assert.rejects(prepareMemoryImport(new BoundedFile([raw], "memory.json"), { ...options(api), owns: () => owns }), MemoryImportTransportError);
  assert.equal(host.puts.length, 1); assert.equal(host.previews, 0);
});

test("staged import cooperatively waits for an existing parser without a whole-task timeout", async () => {
  const raw = new TextEncoder().encode("{}"), host = server(raw); host.busyPreviews = 1;
  const preview = await prepareMemoryImport(new BoundedFile([raw], "memory.json"), options(host.api));
  assert.equal(preview.import_id, "fixture-import"); assert.equal(host.previews, 2); assert.equal(host.confirms, 0);
});

test("staged import parser wait can be aborted without replaying or confirming work", { timeout: 15000 }, async () => {
  const raw = new TextEncoder().encode("{}"), host = server(raw); host.busyPreviews = 100;
  const control = options(host.api);
  const pending = prepareMemoryImport(new BoundedFile([raw], "memory.json"), control);
  while (!host.previews) await setImmediate();
  await setImmediate(); control.controller.abort();
  await assert.rejects(pending);
  assert.equal(host.previews, 1); assert.equal(host.confirms, 0);
});

test("staged import does not retry a structured checksum rejection as a busy parser", async () => {
  const raw = new TextEncoder().encode("{}"), host = server(raw);
  host.onRequest = async path => path.endsWith("/preview") ? response("memory_import_checksum_invalid", 400) : undefined;
  await assert.rejects(prepareMemoryImport(new BoundedFile([raw], "memory.json"), options(host.api)),
    (e: unknown) => e instanceof MemoryImportTransportError && e.code === "memory_import_checksum_invalid");
  assert.equal(host.calls.filter(c => c.path.endsWith("/preview")).length, 1); assert.equal(host.confirms, 0);
});

test("staged import cancellation is explicit and checks the server's cancelled state", async () => {
  const host = server(new Uint8Array(2)), control = options(host.api);
  await cancelMemoryImport("fixture-upload", control);
  assert.equal(host.cancels, 1); assert.equal(host.status().state, "cancelled");
  await assert.rejects(cancelMemoryImport("../other-owner", control), MemoryImportTransportError);
  assert.equal(host.cancels, 1);
});

test("staged import cancellation cannot issue a request through a departed owner", async () => {
  const host = server(new Uint8Array(2));
  await assert.rejects(cancelMemoryImport("fixture-upload", { ...options(host.api), owns: () => false }), MemoryImportTransportError);
  assert.equal(host.calls.length, 0);
});
