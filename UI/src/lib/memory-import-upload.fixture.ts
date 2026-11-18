import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { data } from "../hooks/memory-runtime.fixture";
import type { MemoryImportProgress } from "./memory-import-upload";

export const digest = (raw: Uint8Array) => `sha256:${createHash("sha256").update(raw).digest("hex")}`;
export const response = (value: unknown, status = 200, headers?: HeadersInit) => new Response(
  JSON.stringify(status === 200 ? { ok: true, data: value } : { ok: false, error: value }), { status, headers });
export class BoundedFile extends File {
  reads: Array<[number, number]> = [];
  override text(): Promise<string> { throw new Error("whole_file_text_forbidden"); }
  override arrayBuffer(): Promise<ArrayBuffer> { throw new Error("whole_file_buffer_forbidden"); }
  override slice(start = 0, end = this.size, contentType?: string): Blob {
    this.reads.push([start, Math.min(end, this.size)]);
    return super.slice(start, end, contentType);
  }
}
export function server(raw: Uint8Array, chunk = 65536) {
  let received = 0;
  let state = "uploading";
  let revision = 1;
  const calls: Array<{ path: string; method: string; init?: RequestInit }> = [];
  const puts: Array<{ offset: number; bytes: Uint8Array }> = [];
  const policy = { schema_version: 1, protocol: "memory-import-upload-v1", max_file_bytes: String(raw.byteLength * 2 + 1),
    max_chunk_bytes: String(chunk), max_value_bytes: "1048576", file_limit_scope: "memory_domain_budget" };
  const status = () => ({ schema_version: 1, upload_id: "fixture-upload", file_digest: digest(raw), total_bytes: String(raw.byteLength),
    received_bytes: String(received), chunk_bytes: String(chunk), revision: String(revision), state });
  const fixture = {
    calls, puts, policy, status,
    previews: 0, confirms: 0, cancels: 0, accepted: 1, busyPreviews: 0,
    failAfterAppend: false,
    onRequest: null as null | ((path: string, init?: RequestInit) => Promise<Response | undefined>),
    setReceived(value: number) { received = value; },
    setState(value: string) { state = value; },
    api: async (path: string, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? "GET";
      calls.push({ path, method, init });
      const intercepted = await fixture.onRequest?.(path, init);
      if (intercepted) return intercepted;
      if (path === "/v1/memory/import/policy") return response(policy);
      if (path === "/v1/memory/import/uploads") {
        assert.equal(method, "POST");
        assert.deepEqual(JSON.parse(String(init?.body)), { file_digest: digest(raw), total_bytes: raw.byteLength });
        return response(status());
      }
      const url = new URL(path, "https://fixture.invalid");
      if (url.pathname === "/v1/memory/import/uploads/fixture-upload/chunks") {
        assert.equal(method, "PUT");
        const offset = Number(url.searchParams.get("offset"));
        const bytes = new Uint8Array(init?.body as ArrayBuffer);
        assert.equal(new Headers(init?.headers).get("x-content-sha256"), digest(bytes));
        assert.equal(offset, received);
        assert.ok(bytes.byteLength > 0 && bytes.byteLength <= chunk);
        assert.deepEqual(bytes, raw.slice(offset, offset + bytes.byteLength));
        puts.push({ offset, bytes }); received += bytes.byteLength; revision++;
        if (fixture.failAfterAppend) { fixture.failAfterAppend = false; throw new TypeError("network lost after append"); }
        return response(status());
      }
      if (url.pathname.endsWith("/preview") && url.pathname.startsWith("/v1/memory/import/uploads/")) {
        assert.equal(method, "POST");
        assert.equal(received, raw.byteLength);
        fixture.previews++;
        if (fixture.busyPreviews-- > 0) {
          state = "parsing";
          return response("memory_import_upload_parser_busy", 409, { "retry-after": "1" });
        }
        state = "preview";
        return response({ ...data("/v1/memory/import/preview", "fixture"), accepted_items: fixture.accepted });
      }
      if (url.pathname === "/v1/memory/import/uploads/fixture-upload") {
        if (method === "DELETE") { fixture.cancels++; state = "cancelled"; }
        return response(status());
      }
      if (path === "/v1/memory/import/confirm") {
        fixture.confirms++; state = "confirmed";
        return response(data(path, "fixture"));
      }
      return response(data(path, "fixture"));
    },
  };
  return fixture;
}
export function options(apiFetch: ReturnType<typeof server>["api"]) {
  const controller = new AbortController();
  const progress: MemoryImportProgress[] = [];
  const uploads: string[] = [];
  return { controller, progress, uploads, apiFetch, owns: () => true, signal: controller.signal,
    onProgress: (value: MemoryImportProgress) => progress.push(value), onUpload: (id: string) => uploads.push(id) };
}
