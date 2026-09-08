import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Download, ImageOff, LoaderCircle, RefreshCw, X } from "lucide-react";

import { saveTaskArtifactBlob, type ArtifactFetch } from "../lib/task-artifact-content";

export type AippViewerImage = {
  title: string;
  filename: string;
  previewUrl: string;
  downloadUrl: string;
  initialSource?: string | null;
};

export function AippImageViewer({ image, apiFetch, t, onClose }: {
  image: AippViewerImage;
  apiFetch: ArtifactFetch;
  t: (zh: string, en: string) => string;
  onClose: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;
  const titleId = useId();
  const [source, setSource] = useState(image.initialSource || null);
  const [previewFailed, setPreviewFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  const [downloadState, setDownloadState] = useState<"idle" | "working" | "failed">("idle");
  const downloadRequest = useRef<AbortController | null>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement;
    const previousOverflow = document.body.style.overflow;
    dialog?.showModal();
    document.body.style.overflow = "hidden";
    return () => {
      downloadRequest.current?.abort();
      dialog?.close();
      document.body.style.overflow = previousOverflow;
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, []);

  useEffect(() => {
    setPreviewFailed(false);
    if (retry === 0 && image.initialSource) {
      setSource(image.initialSource);
      return;
    }
    setSource(null);
    const controller = new AbortController();
    let objectUrl: string | null = null;
    void apiFetchRef.current(image.previewUrl, { signal: controller.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error(`aipp_image_http_${response.status}`);
        return response.blob();
      })
      .then((blob) => {
        if (controller.signal.aborted) return;
        objectUrl = URL.createObjectURL(blob);
        setSource(objectUrl);
      })
      .catch(() => {
        if (!controller.signal.aborted) setPreviewFailed(true);
      });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [image.previewUrl, image.initialSource, retry]);

  const download = async () => {
    if (downloadRequest.current && !downloadRequest.current.signal.aborted) return;
    const controller = new AbortController();
    downloadRequest.current = controller;
    setDownloadState("working");
    try {
      const response = await apiFetchRef.current(image.downloadUrl, { signal: controller.signal });
      if (!response.ok) throw new Error(`aipp_image_download_http_${response.status}`);
      const blob = await response.blob();
      if (controller.signal.aborted) return;
      saveTaskArtifactBlob(blob, image.filename);
      setDownloadState("idle");
    } catch {
      if (!controller.signal.aborted) setDownloadState("failed");
    } finally {
      if (downloadRequest.current === controller) downloadRequest.current = null;
    }
  };

  return createPortal(
    <dialog
      ref={dialogRef}
      aria-labelledby={titleId}
      className="theme-dialog-panel m-auto h-[min(90dvh,900px)] max-h-[calc(100dvh-1rem)] w-[calc(100vw-1rem)] max-w-6xl overflow-hidden rounded-lg p-0 text-[var(--theme-text-strong)] backdrop:bg-black/70"
      onCancel={(event) => { event.preventDefault(); onClose(); }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        const rect = event.currentTarget.getBoundingClientRect();
        if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) onClose();
      }}
    >
      <div className="flex h-full min-h-0 flex-col">
        <header className="flex shrink-0 items-center gap-3 border-b border-[var(--theme-border)] px-3 py-2 sm:px-4">
          <h2 id={titleId} className="min-w-0 flex-1 truncate text-sm font-medium" title={image.title}>{image.title}</h2>
          <button type="button" autoFocus className="theme-icon-btn h-9 w-9 shrink-0" onClick={onClose} title={t("关闭", "Close")} aria-label={t("关闭", "Close")}>
            <X className="h-5 w-5" />
          </button>
        </header>
        <div className="flex min-h-0 flex-1 items-center justify-center overflow-hidden p-2 sm:p-4">
          {previewFailed ? (
            <div role="alert" className="flex flex-col items-center gap-3 text-center text-sm text-[var(--theme-text-muted)]">
              <ImageOff className="h-8 w-8" />
              <p>{t("图片加载失败，请重试。", "Could not load the image. Try again.")}</p>
              <button type="button" className="theme-secondary-btn px-3 py-2" onClick={() => setRetry((value) => value + 1)}>
                <RefreshCw className="h-4 w-4" />{t("重试", "Retry")}
              </button>
            </div>
          ) : source ? (
            <img src={source} alt={image.title} referrerPolicy="no-referrer" className="h-full w-full object-contain" onError={() => setPreviewFailed(true)} />
          ) : (
            <LoaderCircle role="status" aria-label={t("正在加载图片", "Loading image")} className="h-7 w-7 animate-spin text-[var(--theme-text-muted)]" />
          )}
        </div>
        <footer className="flex shrink-0 flex-wrap items-center justify-end gap-3 border-t border-[var(--theme-border)] px-3 py-3 sm:px-4">
          {downloadState === "failed" ? <p role="alert" className="min-w-0 flex-1 text-sm text-[var(--theme-text-body)]">{t("图片下载失败，请重试。", "Image download failed. Try again.")}</p> : null}
          <button type="button" className="theme-secondary-btn shrink-0 px-4 py-2 text-sm" disabled={downloadState === "working"} onClick={() => void download()}>
            {downloadState === "working" ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
            {downloadState === "working" ? t("正在下载", "Downloading") : t("下载图片", "Download image")}
          </button>
        </footer>
      </div>
    </dialog>,
    document.body,
  );
}
