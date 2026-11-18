import { sha256 } from "@noble/hashes/sha2.js";
import { readJsonApiResponse } from "./api-response";
import type { ApiResponse, MemoryImportPreviewResult } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type Owner = () => boolean;
export type MemoryImportPhase = "hashing" | "uploading" | "parsing" | "preview" | "confirming" | "paused" | "cancelling";
export interface MemoryImportProgress { phase: MemoryImportPhase; received: number; total: number }
interface Options {
  apiFetch: ApiFetch;
  owns: Owner;
  signal: AbortSignal;
  onProgress: (progress: MemoryImportProgress) => void;
  onUpload: (id: string) => void;
  resumeId?: string;
}
interface UploadStatus {
  schema_version: 1; upload_id: string; file_digest: string;
  total_bytes: string; received_bytes: string; chunk_bytes: string; revision: string;
  state: "uploading" | "parsing" | "preview" | "confirmed" | "cancelled";
}
interface Policy {
  schema_version: 1; protocol: "memory-import-upload-v1";
  max_file_bytes: string; max_chunk_bytes: string; max_value_bytes: string;
  file_limit_scope: "memory_domain_budget";
}

// An I/O quantum, not a file, record-count or text-length ceiling. A file may
// contain any number of these slices within the host's advertised resource policy.
export const MEMORY_IMPORT_IO_BYTES = 1024 * 1024;
const ID = /^[A-Za-z0-9_-]+$/;
const DIGEST = /^sha256:[0-9a-f]{64}$/;
const STATES = new Set(["uploading", "parsing", "preview", "confirmed", "cancelled"]);
export class MemoryImportTransportError extends Error {
  constructor(readonly code: string, readonly status = 0, readonly retryAfterMs = 1000) {
    super(code); this.name = "MemoryImportTransportError";
  }
}
function check(options: Pick<Options, "owns" | "signal">) {
  if (!options.owns()) throw new MemoryImportTransportError("memory_import_owner_changed");
  if (options.signal.aborted) throw options.signal.reason ?? new DOMException("memory_import_aborted", "AbortError");
}
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new MemoryImportTransportError("memory_import_response_invalid");
  return value as Record<string, unknown>;
}
function integer(value: unknown): bigint {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]*)$/.test(value) || value.length > 20) {
    throw new MemoryImportTransportError("memory_import_response_invalid");
  }
  const result = BigInt(value);
  if (result > 18446744073709551615n) throw new MemoryImportTransportError("memory_import_response_invalid");
  return result;
}
function policy(value: unknown): Policy {
  const row = object(value);
  if (row.schema_version !== 1 || row.protocol !== "memory-import-upload-v1" || row.file_limit_scope !== "memory_domain_budget"
    || integer(row.max_file_bytes) === 0n || integer(row.max_chunk_bytes) === 0n || integer(row.max_value_bytes) === 0n) {
    throw new MemoryImportTransportError("memory_import_policy_unavailable");
  }
  return row as unknown as Policy;
}
function status(value: unknown, digest: string, size: number, id?: string): UploadStatus {
  const row = object(value);
  if (row.schema_version !== 1 || typeof row.upload_id !== "string" || !ID.test(row.upload_id)
    || (id !== undefined && row.upload_id !== id) || row.file_digest !== digest
    || integer(row.total_bytes) !== BigInt(size) || integer(row.received_bytes) > BigInt(size)
    || integer(row.chunk_bytes) === 0n || integer(row.revision) === 0n || !STATES.has(String(row.state))) {
    throw new MemoryImportTransportError("memory_import_response_invalid");
  }
  return row as unknown as UploadStatus;
}
function preview(value: unknown): MemoryImportPreviewResult {
  const row = object(value);
  if (row.schema_version !== 1 || typeof row.import_id !== "string" || !ID.test(row.import_id)
    || typeof row.payload_digest !== "string" || !DIGEST.test(row.payload_digest)
    || row.scope_kind !== "principal" || row.trust_tier !== "imported_legacy"
    || [row.accepted_items, row.skipped_items, row.duplicate_items].some(n => !Number.isSafeInteger(n) || Number(n) < 0)) {
    throw new MemoryImportTransportError("memory_import_response_invalid");
  }
  return row as unknown as MemoryImportPreviewResult;
}
async function request(options: Pick<Options, "apiFetch" | "owns" | "signal">, path: string, init?: RequestInit): Promise<unknown> {
  check(options);
  const response = await options.apiFetch(path, { ...init, signal: options.signal });
  check(options);
  const body = await readJsonApiResponse<ApiResponse<unknown>>(response);
  check(options);
  if (!response.ok || body?.ok !== true || body.data === undefined) {
    const code = typeof body?.error === "string" && /^[a-z][a-z0-9_]+$/.test(body.error)
      ? body.error : "memory_import_request_failed";
    const retry = response.headers.get("retry-after");
    const seconds = retry && /^[0-9]+$/.test(retry) ? Number(retry) : 1;
    throw new MemoryImportTransportError(code, response.status,
      Number.isSafeInteger(seconds) ? Math.max(1, Math.min(60, seconds)) * 1000 : 1000);
  }
  return body.data;
}
const json = (value: unknown): RequestInit => ({ method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(value) });
const hex = (bytes: Uint8Array) => `sha256:${Array.from(bytes, b => b.toString(16).padStart(2, "0")).join("")}`;
async function slice(file: Blob, start: number, end: number, options: Options) {
  check(options);
  const raw = await file.slice(start, end).arrayBuffer();
  check(options);
  if (raw.byteLength !== end - start) throw new MemoryImportTransportError("memory_import_file_read_failed");
  return new Uint8Array(raw);
}
async function hash(file: Blob, options: Options) {
  const digest = sha256.create();
  try {
    for (let offset = 0; offset < file.size; offset += MEMORY_IMPORT_IO_BYTES) {
      const end = Math.min(file.size, offset + MEMORY_IMPORT_IO_BYTES);
      digest.update(await slice(file, offset, end, options));
      options.onProgress({ phase: "hashing", received: end, total: file.size });
    }
    return hex(digest.digest());
  } finally { digest.destroy(); }
}
async function delay(milliseconds: number, options: Options) {
  check(options);
  await new Promise<void>((resolve, reject) => {
    const abort = () => { clearTimeout(timer); reject(options.signal.reason ?? new DOMException("memory_import_aborted", "AbortError")); };
    const timer = setTimeout(() => { options.signal.removeEventListener("abort", abort); resolve(); }, milliseconds);
    options.signal.addEventListener("abort", abort, { once: true });
    if (options.signal.aborted) abort();
  });
  check(options);
}

/** Reselecting the same immutable file resumes the principal-owned server upload.
 * No file, plaintext, credential or upload identity is stored in browser storage.
 * No canonical item is written here; the caller must obtain one explicit approval.
 */
export async function prepareMemoryImport(file: Blob, options: Options): Promise<MemoryImportPreviewResult> {
  check(options);
  if (!Number.isSafeInteger(file.size) || file.size <= 0) throw new MemoryImportTransportError("memory_import_file_invalid");
  const fileDigest = await hash(file, options);
  const limits = policy(await request(options, "/v1/memory/import/policy"));
  if (BigInt(file.size) > integer(limits.max_file_bytes)) throw new MemoryImportTransportError("memory_import_upload_file_too_large");
  if (options.resumeId !== undefined && !ID.test(options.resumeId)) throw new MemoryImportTransportError("memory_import_response_invalid");
  let upload = status(options.resumeId
    ? await request(options, `/v1/memory/import/uploads/${encodeURIComponent(options.resumeId)}`)
    : await request(options, "/v1/memory/import/uploads", json({ file_digest: fileDigest, total_bytes: file.size })),
  fileDigest, file.size, options.resumeId);
  check(options); options.onUpload(upload.upload_id);
  const path = `/v1/memory/import/uploads/${encodeURIComponent(upload.upload_id)}`;
  upload = status(await request(options, path), fileDigest, file.size, upload.upload_id);
  if (upload.state === "confirmed") throw new MemoryImportTransportError("memory_import_already_confirmed");
  const quantum = Number([integer(limits.max_chunk_bytes), integer(upload.chunk_bytes), BigInt(MEMORY_IMPORT_IO_BYTES)]
    .reduce((a, b) => a < b ? a : b));
  while (upload.state === "uploading" && integer(upload.received_bytes) < BigInt(file.size)) {
    const offset = Number(integer(upload.received_bytes));
    options.onProgress({ phase: "uploading", received: offset, total: file.size });
    const bytes = await slice(file, offset, Math.min(file.size, offset + quantum), options);
    const next = status(await request(options, `${path}/chunks?offset=${offset}`, {
      method: "PUT", headers: { "Content-Type": "application/octet-stream", "x-content-sha256": hex(sha256(bytes)) },
      body: bytes.buffer as ArrayBuffer,
    }), fileDigest, file.size, upload.upload_id);
    if (integer(next.received_bytes) < BigInt(offset + bytes.byteLength)) throw new MemoryImportTransportError("memory_import_response_invalid");
    upload = next;
  }
  if (!["uploading", "parsing", "preview"].includes(upload.state) || integer(upload.received_bytes) !== BigInt(file.size)) {
    throw new MemoryImportTransportError("memory_import_upload_state_conflict");
  }
  options.onProgress({ phase: "parsing", received: file.size, total: file.size });
  // Another tab or a disconnected request may already be parsing. This waits
  // cooperatively without a total task deadline or replaying confirmation.
  for (;;) {
    try { return preview(await request(options, `${path}/preview`, { method: "POST" })); }
    catch (error) {
      if (!(error instanceof MemoryImportTransportError) || error.status !== 409 || error.code !== "memory_import_upload_parser_busy") throw error;
      await delay(error.retryAfterMs, options);
      upload = status(await request(options, path), fileDigest, file.size, upload.upload_id);
      if (!["parsing", "preview", "uploading"].includes(upload.state)) throw new MemoryImportTransportError("memory_import_upload_state_conflict");
    }
  }
}

export async function cancelMemoryImport(id: string, options: Pick<Options, "apiFetch" | "owns" | "signal">): Promise<void> {
  if (!ID.test(id)) throw new MemoryImportTransportError("memory_import_response_invalid");
  const row = object(await request(options, `/v1/memory/import/uploads/${encodeURIComponent(id)}`, { method: "DELETE" }));
  if (row.schema_version !== 1 || row.upload_id !== id || row.state !== "cancelled") {
    throw new MemoryImportTransportError("memory_import_response_invalid");
  }
}
