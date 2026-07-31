import { safeArtifactUrl } from "./task-artifacts";

export type ArtifactFetch = (path: string, init?: RequestInit) => Promise<Response>;

export const MAX_AUTOMATIC_ARTIFACT_PREVIEW_BYTES = 25 * 1024 * 1024;

export function taskArtifactBrowserVideoUrl(path: string): string | null {
  if (!safeArtifactUrl(path)) return null;
  const parsed = new URL(path, "http://agent.invalid");
  parsed.searchParams.set("disposition", "inline");
  parsed.searchParams.set("preview", "browser");
  return `${parsed.pathname}?${parsed.searchParams.toString()}`;
}

export function taskArtifactVideoPosterUrl(path: string): string | null {
  if (!safeArtifactUrl(path)) return null;
  const parsed = new URL(path, "http://agent.invalid");
  parsed.searchParams.set("disposition", "inline");
  parsed.searchParams.set("preview", "poster");
  return `${parsed.pathname}?${parsed.searchParams.toString()}`;
}

export async function fetchTaskArtifactBlob(
  artifactFetch: ArtifactFetch,
  path: string,
  signal?: AbortSignal,
): Promise<Blob> {
  if (!safeArtifactUrl(path)) {
    throw new Error("task_artifact_url_invalid");
  }
  const response = await artifactFetch(path, signal ? { signal } : undefined);
  if (!response.ok) {
    throw new Error(`task_artifact_http_${response.status}`);
  }
  return response.blob();
}

export function saveTaskArtifactBlob(blob: Blob, filename: string): void {
  const objectUrl = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = objectUrl;
  link.download = filename;
  link.rel = "noopener";
  link.style.display = "none";
  document.body.appendChild(link);
  link.click();
  link.remove();
  window.setTimeout(() => URL.revokeObjectURL(objectUrl), 0);
}
