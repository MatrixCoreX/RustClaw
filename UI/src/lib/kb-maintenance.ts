import type { TaskQueryResponse } from "../types/api";

export type KbIdentity = {
  userKey?: string;
  userId?: number;
  chatId?: number;
};

export type KbNamespaceRow = {
  namespace: string;
  docs: number;
  chunks: number;
  updatedAt: number | null;
  embeddingVersion: string;
  activeGeneration: number | null;
};

export type KbDocumentRow = {
  path: string;
  fileType: string;
  size: number;
  chunks: number;
  parserVersion: string;
  chunkerVersion: string;
};

export type KbSearchRow = {
  evidenceId: string;
  sourceRef: string;
  text: string;
  score: number | null;
  location: Record<string, unknown>;
};

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function boundedString(value: unknown, limit: number): string {
  return typeof value === "string" ? value.trim().slice(0, limit) : "";
}

function nonnegativeInteger(value: unknown): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

export function buildKbTaskBody(
  args: Record<string, unknown>,
  identity: KbIdentity,
): Record<string, unknown> {
  const userKey = boundedString(identity.userKey, 512);
  return {
    channel: "ui",
    kind: "run_skill",
    idempotency_key: `ui:kb:${crypto.randomUUID()}`,
    payload: { skill_name: "kb", args },
    ...(userKey
      ? { user_key: userKey }
      : Number.isSafeInteger(identity.userId) && Number.isSafeInteger(identity.chatId)
        ? { user_id: identity.userId, chat_id: identity.chatId }
        : {}),
  };
}

export function kbPayload(resultJson: unknown): Record<string, unknown> | null {
  const queue: Array<{ value: unknown; depth: number }> = [{ value: resultJson, depth: 0 }];
  const seen = new Set<unknown>();
  while (queue.length > 0) {
    const current = queue.shift();
    if (!current || current.depth > 4 || seen.has(current.value)) continue;
    seen.add(current.value);
    const item = record(current.value);
    if (!item) continue;
    if (
      typeof item.status === "string"
      && [
        "namespaces", "documents", "hits", "stats", "continuation", "job",
        "document_count", "namespace_count", "complete", "maintenance",
      ].some((key) => key in item)
    ) {
      return item;
    }
    for (const key of ["extra", "result", "output", "skill_result", "data"]) {
      if (key in item) queue.push({ value: item[key], depth: current.depth + 1 });
    }
    const text = item.text;
    if (typeof text === "string" && text.length <= 1_000_000 && text.trim().startsWith("{")) {
      try {
        queue.push({ value: JSON.parse(text), depth: current.depth + 1 });
      } catch {
        // A readable fallback is not a machine payload.
      }
    }
  }
  return record(resultJson);
}

export function kbNamespaceRows(payload: unknown): KbNamespaceRow[] {
  const items = record(payload)?.namespaces;
  if (!Array.isArray(items)) return [];
  return items.flatMap((value) => {
    const item = record(value);
    const namespace = boundedString(item?.namespace, 128);
    if (!item || !namespace) return [];
    const generation = item.active_vector_generation;
    return [{
      namespace,
      docs: nonnegativeInteger(item.docs),
      chunks: nonnegativeInteger(item.chunks),
      updatedAt: typeof item.updated_at_epoch === "number" && Number.isFinite(item.updated_at_epoch)
        ? item.updated_at_epoch
        : null,
      embeddingVersion: boundedString(item.embedding_version, 128),
      activeGeneration: typeof generation === "number" && Number.isSafeInteger(generation) && generation > 0
        ? generation
        : null,
    }];
  });
}

export function kbDocumentRows(payload: unknown): KbDocumentRow[] {
  const items = record(payload)?.documents;
  if (!Array.isArray(items)) return [];
  return items.flatMap((value) => {
    const item = record(value);
    const path = boundedString(item?.path, 2_048);
    if (!item || !path) return [];
    return [{
      path,
      fileType: boundedString(item.file_type, 32),
      size: nonnegativeInteger(item.size),
      chunks: nonnegativeInteger(item.chunk_count),
      parserVersion: boundedString(item.parser_version, 128),
      chunkerVersion: boundedString(item.chunker_version, 128),
    }];
  });
}

export function kbSearchRows(payload: unknown): KbSearchRow[] {
  const items = record(payload)?.hits;
  if (!Array.isArray(items)) return [];
  return items.flatMap((value) => {
    const item = record(value);
    if (!item || item.instruction_authority !== "none") return [];
    const score = record(item.score)?.final;
    return [{
      evidenceId: boundedString(item.evidence_id, 256),
      sourceRef: boundedString(item.source_ref, 2_048),
      text: boundedString(item.text, 8_000),
      score: typeof score === "number" && Number.isFinite(score) ? score : null,
      location: record(item.location) ?? {},
    }];
  });
}

export function kbContinuationArgs(payload: unknown): Record<string, unknown> | null {
  const item = record(payload);
  if (!item || item.complete !== false) return null;
  const continuation = record(item.continuation);
  if (!continuation) return null;
  const action = boundedString(continuation.action, 64);
  if (!new Set(["resume_ingest", "resume_vector_reindex"]).has(action)) return null;
  return { ...continuation, action };
}

export function kbTaskIsTerminal(task: TaskQueryResponse): boolean {
  return ["succeeded", "failed", "canceled", "timeout"].includes(task.status);
}
