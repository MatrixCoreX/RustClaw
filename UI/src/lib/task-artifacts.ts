import type { TaskArtifact, TaskQueryResponse } from "../types/api";

const MACHINE_ID = /^[A-Za-z0-9_.:-]{1,128}$/;
const SHA256 = /^[0-9a-f]{64}$/i;

export type TaskArtifactPreviewKind = "image" | "audio" | "video" | "pdf" | "none";

export interface TaskArtifactDeliverySummary {
  schema_version: 1;
  candidate_count: number;
  delivered_count: number;
  truncated: boolean;
  max_items: number;
}

export function extractTaskArtifacts(result: TaskQueryResponse): TaskArtifact[] {
  if (!result.result_json || typeof result.result_json !== "object") return [];
  return normalizeTaskArtifacts((result.result_json as { artifacts?: unknown }).artifacts);
}

export function extractTaskArtifactDeliverySummary(
  result: TaskQueryResponse,
): TaskArtifactDeliverySummary | undefined {
  if (!result.result_json || typeof result.result_json !== "object") return undefined;
  const value = (result.result_json as { artifact_delivery?: unknown }).artifact_delivery;
  return normalizeTaskArtifactDeliverySummary(value);
}

export function normalizeTaskArtifactDeliverySummary(
  value: unknown,
): TaskArtifactDeliverySummary | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const summary = value as Partial<TaskArtifactDeliverySummary>;
  if (
    summary.schema_version !== 1 ||
    !Number.isSafeInteger(summary.candidate_count) ||
    !Number.isSafeInteger(summary.delivered_count) ||
    !Number.isSafeInteger(summary.max_items) ||
    Number(summary.candidate_count) < 0 ||
    Number(summary.delivered_count) < 0 ||
    Number(summary.max_items) < 1 ||
    typeof summary.truncated !== "boolean"
  ) {
    return undefined;
  }
  return summary as TaskArtifactDeliverySummary;
}

export function normalizeTaskArtifacts(value: unknown): TaskArtifact[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  const artifacts: TaskArtifact[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object" || Array.isArray(item)) continue;
    const record = item as Partial<TaskArtifact>;
    const artifactRef = canonicalArtifactRef(record.download_url, record.id);
    if (
      (record.schema_version !== 1 && record.schema_version !== 2) ||
      typeof record.id !== "string" ||
      !MACHINE_ID.test(record.id) ||
      typeof record.filename !== "string" ||
      !record.filename.trim() ||
      typeof record.kind !== "string" ||
      typeof record.mime_type !== "string" ||
      !record.mime_type.includes("/") ||
      typeof record.size_bytes !== "number" ||
      !Number.isSafeInteger(record.size_bytes) ||
      record.size_bytes < 0 ||
      typeof record.sha256 !== "string" ||
      !SHA256.test(record.sha256) ||
      !artifactRef ||
      (record.schema_version === 2 && record.artifact_ref !== artifactRef)
    ) {
      continue;
    }
    if (seen.has(record.id)) continue;
    seen.add(record.id);
    artifacts.push({
      schema_version: 2,
      id: record.id,
      artifact_ref: artifactRef,
      filename: record.filename.trim(),
      kind: record.kind,
      mime_type: record.mime_type,
      size_bytes: record.size_bytes,
      sha256: record.sha256.toLowerCase(),
      download_url: record.download_url,
      preview_url: safeArtifactUrl(record.preview_url) ? record.preview_url : null,
    });
  }
  return artifacts.slice(0, 128);
}

function canonicalArtifactRef(downloadUrl: unknown, artifactId: unknown): string | null {
  if (!safeArtifactUrl(downloadUrl) || typeof artifactId !== "string") return null;
  const matched = /^\/v1\/tasks\/([A-Za-z0-9-]+)\/artifacts\/([^/]+)\/content$/.exec(
    new URL(downloadUrl, "http://agent.invalid").pathname,
  );
  if (!matched) return null;
  let decodedId: string;
  try {
    decodedId = decodeURIComponent(matched[2]);
  } catch {
    return null;
  }
  if (decodedId !== artifactId) return null;
  return `artifact:task/${matched[1]}/${artifactId}`;
}

export function artifactPreviewKind(artifact: TaskArtifact): TaskArtifactPreviewKind {
  if (!artifact.preview_url) return "none";
  const mime = artifact.mime_type.split(";", 1)[0].trim().toLowerCase();
  if (["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"].includes(mime)) {
    return "image";
  }
  if (mime.startsWith("audio/")) return "audio";
  if (mime.startsWith("video/")) return "video";
  if (mime === "application/pdf") return "pdf";
  return "none";
}

export function safeArtifactUrl(value: unknown): value is string {
  if (typeof value !== "string" || !value.startsWith("/v1/tasks/") || value.includes("\\")) {
    return false;
  }
  try {
    const parsed = new URL(value, "http://agent.invalid");
    return (
      parsed.origin === "http://agent.invalid" &&
      /^\/v1\/tasks\/[A-Za-z0-9-]+\/artifacts\/[A-Za-z0-9_.:%-]+\/content$/.test(
        parsed.pathname,
      )
    );
  } catch {
    return false;
  }
}
