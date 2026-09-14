import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronLeft, ChevronRight, Download, ImageOff, LoaderCircle, RefreshCw, X } from "lucide-react";

import { saveTaskArtifactBlob, type ArtifactFetch } from "../lib/task-artifact-content";
import { downloadAippGallery, fetchAippImage, imageDownloadName, type AippViewerImage } from "../lib/aipp-image-download";
export type { AippViewerImage } from "../lib/aipp-image-download";

export function AippImageViewer({ image: initialImage, images, apiFetch, t, onClose }: {
  image: AippViewerImage;
  images?: AippViewerImage[];
  apiFetch: ArtifactFetch;
  t: (zh: string, en: string) => string;
  onClose: () => void;
}) {
  // Keep a stable gallery while its collection is refreshed in the background.
  const [gallery] = useState(() => images?.length ? images : [initialImage]);
  const [index, setIndex] = useState(() => Math.max(0, gallery.findIndex((image) => image.previewUrl === initialImage.previewUrl)));
  const image = gallery[index];
  const dialogRef = useRef<HTMLDialogElement>(null);
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;
  const titleId = useId();
  const [source, setSource] = useState(image.initialSource || null);
  const [previewFailed, setPreviewFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  const [downloadState, setDownloadState] = useState<"idle" | "working" | "failed">("idle");
  const downloadRequest = useRef<AbortController | null>(null);
  const [downloadMode, setDownloadMode] = useState<"single" | "all">("single");
  const [completed, setCompleted] = useState(0);
  const touchStart = useRef<{ x: number; y: number } | null>(null);
  const move = (delta: number) => { setIndex((value) => Math.max(0, Math.min(gallery.length - 1, value + delta))); setRetry(0); };

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
    void fetchAippImage(apiFetchRef.current, image.previewUrl, controller.signal)
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

  const download = async (mode: "single" | "all") => {
    if (downloadRequest.current && !downloadRequest.current.signal.aborted) return;
    // Disabling the focused download button must not move focus outside the modal.
    dialogRef.current?.focus({ preventScroll: true });
    const controller = new AbortController();
    downloadRequest.current = controller;
    setDownloadState("working");
    setDownloadMode(mode);
    setCompleted(0);
    try {
      const blob = mode === "all"
        ? await downloadAippGallery(gallery, apiFetchRef.current, controller.signal, setCompleted)
        : await fetchAippImage(apiFetchRef.current, image.downloadUrl, controller.signal);
      if (controller.signal.aborted) return;
      saveTaskArtifactBlob(blob, mode === "all" ? `${imageDownloadName(gallery[0].filename, "image/png").replace(/\.png$/, "")}-all.zip` : imageDownloadName(image.filename, blob.type));
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
      tabIndex={-1}
      aria-labelledby={titleId}
      className="theme-dialog-panel m-auto h-[min(90dvh,900px)] max-h-[calc(100dvh-1rem)] w-[calc(100vw-1rem)] max-w-6xl overflow-hidden rounded-lg p-0 text-[var(--theme-text-strong)] backdrop:bg-black/70"
      onCancel={(event) => { event.preventDefault(); onClose(); }}
      onKeyDown={(event) => {
        if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
          event.preventDefault(); move(event.key === "ArrowLeft" ? -1 : 1);
        }
      }}
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
        <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden p-2 sm:p-4"
          onTouchStart={(event) => { const touch = event.touches[0]; touchStart.current = event.touches.length === 1 ? { x: touch.clientX, y: touch.clientY } : null; }}
          onTouchEnd={(event) => { const start = touchStart.current; touchStart.current = null; const touch = event.changedTouches[0]; if (start && touch && Math.abs(touch.clientX - start.x) > 60 && Math.abs(touch.clientX - start.x) > Math.abs(touch.clientY - start.y) * 2) move(touch.clientX < start.x ? 1 : -1); }}>
          {previewFailed ? (
            <div role="alert" className="flex flex-col items-center gap-3 text-center text-sm text-[var(--theme-text-muted)]">
              <ImageOff className="h-8 w-8" />
              <p>{t("图片加载失败，请重试。", "Could not load the image. Try again.")}</p>
              <button type="button" className="theme-secondary-btn px-3 py-2" onClick={() => setRetry((value) => value + 1)}>
                <RefreshCw className="h-4 w-4" />{t("重试", "Retry")}
              </button>
            </div>
          ) : source ? (
            <img key={source} src={source} alt={image.title} referrerPolicy="no-referrer" className="h-full w-full object-contain" onError={() => setPreviewFailed(true)} />
          ) : (
            <LoaderCircle role="status" aria-label={t("正在加载图片", "Loading image")} className="h-7 w-7 animate-spin text-[var(--theme-text-muted)]" />
          )}
          {gallery.length > 1 ? <>
            <button type="button" className="absolute left-2 flex h-10 w-10 items-center justify-center rounded-md border border-white/40 bg-black/65 text-white disabled:opacity-30" disabled={index === 0} onClick={() => move(-1)} aria-label={t("上一张图片", "Previous image")} title={t("上一张图片", "Previous image")}><ChevronLeft className="h-6 w-6" /></button>
            <button type="button" className="absolute right-2 flex h-10 w-10 items-center justify-center rounded-md border border-white/40 bg-black/65 text-white disabled:opacity-30" disabled={index === gallery.length - 1} onClick={() => move(1)} aria-label={t("下一张图片", "Next image")} title={t("下一张图片", "Next image")}><ChevronRight className="h-6 w-6" /></button>
          </> : null}
        </div>
        <footer className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-[var(--theme-border)] px-3 py-2 sm:px-4">
          {downloadState === "failed" ? <p role="alert" className="w-full text-sm text-[var(--theme-text-body)]">{downloadMode === "all" ? t(`第 ${completed + 1} 张下载失败或图片组过大，未生成压缩包。可重试或单张下载。`, `Image ${completed + 1} failed or the gallery is too large. No ZIP was saved. Retry or download individually.`) : t("图片下载失败，请重试。", "Image download failed. Try again.")}</p> : null}
          {gallery.length > 1 ? <span className="mr-auto text-xs tabular-nums" aria-live="polite">{index + 1} / {gallery.length}</span> : null}
          {gallery.length > 1 ? <button type="button" className="theme-secondary-btn px-3 py-2 text-xs" disabled={downloadState === "working"} onClick={() => void download("all")}>
            {downloadState === "working" && downloadMode === "all" ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
            {downloadState === "working" && downloadMode === "all" ? `${completed} / ${gallery.length}` : t("下载全部", "Download all")}
          </button> : null}
          <button type="button" className="theme-secondary-btn px-3 py-2 text-xs" disabled={downloadState === "working"} onClick={() => void download("single")}>
            {downloadState === "working" && downloadMode === "single" ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
            {downloadState === "working" && downloadMode === "single" ? t("正在下载", "Downloading") : t("下载图片", "Download image")}
          </button>
          {downloadState === "working" ? <button type="button" className="theme-icon-btn h-8 w-8" title={t("取消下载", "Cancel download")} aria-label={t("取消下载", "Cancel download")} onClick={() => { downloadRequest.current?.abort(); setDownloadState("idle"); }}><X className="h-4 w-4" /></button> : null}
        </footer>
      </div>
    </dialog>,
    document.body,
  );
}
