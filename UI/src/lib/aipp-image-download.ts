import type { ArtifactFetch } from "./task-artifact-content";

export type AippViewerImage = {
  title: string;
  filename: string;
  previewUrl: string;
  downloadUrl: string;
  initialSource?: string | null;
};

export const AIPP_IMAGE_MAX_BYTES = 25 * 1024 * 1024;
export const AIPP_GALLERY_MAX_BYTES = 128 * 1024 * 1024;
const IMAGE_EXTENSIONS: Record<string, string> = {
  "image/png": "png", "image/jpeg": "jpg", "image/webp": "webp",
  "image/gif": "gif", "image/avif": "avif",
};

export function imageDownloadName(filename: string, mime: string): string {
  const stem = filename.split(/[\\/]/).pop()!.replace(/\.[^.]*$/, "")
    .replace(/[\x00-\x1f\x7f<>:"|?*]/g, "_").replace(/^[. ]+|[. ]+$/g, "").slice(0, 120);
  return `${stem || "image"}.${IMAGE_EXTENSIONS[mime] || "png"}`;
}

export async function fetchAippImage(apiFetch: ArtifactFetch, path: string, signal: AbortSignal, maxBytes = AIPP_IMAGE_MAX_BYTES): Promise<Blob> {
  signal.throwIfAborted();
  // The host supplies relative authenticated endpoints, never platform image URLs.
  if (!path.startsWith("/") || path.startsWith("//") || path.includes("\\")) throw new Error("aipp_image_url_invalid");
  for (let attempt = 0; ; attempt++) {
    const controller = new AbortController();
    const abort = () => controller.abort();
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) controller.abort();
    const timeout = setTimeout(abort, 60_000);
    let retryable = true;
    try {
      const response = await apiFetch(path, { signal: controller.signal });
      retryable = response.status >= 500;
      if (!response.ok) {
        await response.body?.cancel();
        throw new Error(`aipp_image_http_${response.status}`);
      }
      retryable = false;
      const mime = (response.headers.get("content-type") || "").split(";")[0].trim().toLowerCase();
      if (!IMAGE_EXTENSIONS[mime] || Number(response.headers.get("content-length")) > maxBytes || !response.body) {
        await response.body?.cancel();
        throw new Error("aipp_image_content_invalid");
      }
      const reader = response.body.getReader();
      const chunks: Uint8Array<ArrayBuffer>[] = [];
      let size = 0;
      try {
        while (true) {
          signal.throwIfAborted();
          const { value, done } = await reader.read();
          if (done) break;
          size += value.byteLength;
          if (size > maxBytes) throw new Error("aipp_image_size_limit");
          chunks.push(value);
        }
      } finally { await reader.cancel(); reader.releaseLock(); }
      signal.throwIfAborted();
      if (!size) throw new Error("aipp_image_empty");
      return new Blob(chunks, { type: mime });
    } catch (error) {
      if (signal.aborted || !retryable || attempt >= 1) throw error;
    } finally {
      clearTimeout(timeout);
      signal.removeEventListener("abort", abort);
    }
  }
}

export async function downloadAippGallery(images: AippViewerImage[], apiFetch: ArtifactFetch, signal: AbortSignal, onProgress: (completed: number) => void): Promise<Blob> {
  if (!images.length || images.length > 1000) throw new Error("aipp_gallery_count_invalid");
  const { Zip, ZipPassThrough } = await import("fflate");
  signal.throwIfAborted();
  const chunks: BlobPart[] = [];
  let zipError: Error | null = null;
  const zip = new Zip((error, data) => {
    if (error) zipError = error;
    else chunks.push(new Blob([data as Uint8Array<ArrayBuffer>]));
  });
  let bytes = 0;
  try {
    for (const [index, image] of images.entries()) {
      const blob = await fetchAippImage(apiFetch, image.downloadUrl, signal, Math.min(AIPP_IMAGE_MAX_BYTES, AIPP_GALLERY_MAX_BYTES - bytes));
      bytes += blob.size;
      signal.throwIfAborted();
      // Already-compressed images need no CPU-heavy recompression on small devices.
      const entry = new ZipPassThrough(`${String(index + 1).padStart(3, "0")}-${imageDownloadName(image.filename, blob.type)}`);
      zip.add(entry);
      entry.push(new Uint8Array(await blob.arrayBuffer()), true);
      if (zipError) throw zipError;
      onProgress(index + 1);
    }
    signal.throwIfAborted();
    zip.end();
    if (zipError) throw zipError;
    return new Blob(chunks, { type: "application/zip" });
  } catch (error) { zip.terminate(); throw error; }
}
