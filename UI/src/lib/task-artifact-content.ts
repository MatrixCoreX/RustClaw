import { safeArtifactUrl } from "./task-artifacts";
import type { ArtifactFetch } from "./artifact-session-stream";
export type { ArtifactFetch } from "./artifact-session-stream";

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
  signal?.throwIfAborted();
  const response = await artifactFetch(path, signal ? { signal } : undefined);
  if (!response.ok || signal?.aborted) {
    // Stop error/obsolete bodies without delaying the original error if stream
    // cleanup itself rejects or waits on an unresponsive producer.
    void response.body?.cancel().catch(() => undefined);
    signal?.throwIfAborted();
  }
  if (!response.ok) {
    throw new Error(`task_artifact_http_${response.status}`);
  }
  const blob = await response.blob();
  signal?.throwIfAborted();
  return blob;
}

export async function fetchTaskArtifactVideoSource(
  artifactFetch: ArtifactFetch,
  path: string,
  signal: AbortSignal,
): Promise<{ kind: "native"; url: string } | { kind: "blob"; blob: Blob }> {
  if (!safeArtifactUrl(path)) throw new Error("task_artifact_url_invalid");
  const nativeUrl = artifactFetch.nativeMediaUrl?.(path);
  if (nativeUrl) {
    // Authenticate and surface session expiry through the ordinary API boundary
    // before letting the browser issue its own cookie-authenticated Range GETs.
    const response = await artifactFetch(path, { method: "HEAD", signal });
    signal.throwIfAborted();
    if (!response.ok) throw new Error(`task_artifact_http_${response.status}`);
    if (artifactFetch.nativeMediaUrl?.(path) !== nativeUrl) throw new DOMException("Request owner changed", "AbortError");
    if (response.headers.get("accept-ranges")?.toLowerCase() === "bytes"
      && /^video\/(mp4|webm)(?:;|$)/i.test(response.headers.get("content-type") || "")) {
      return { kind: "native", url: nativeUrl };
    }
  }
  return { kind: "blob", blob: await fetchTaskArtifactBlob(artifactFetch, path, signal) };
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
