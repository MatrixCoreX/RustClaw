import { useEffect, useRef, useState } from "react";
import { Download, Image as ImageIcon, LoaderCircle, RefreshCw, Video, ZoomIn } from "lucide-react";

import {
  fetchTaskArtifactBlob,
  MAX_AUTOMATIC_ARTIFACT_PREVIEW_BYTES,
  saveTaskArtifactBlob,
  taskArtifactBrowserVideoUrl,
  taskArtifactVideoPosterUrl,
} from "../lib/task-artifact-content";
import type { AippTaskActivityArtifact, AippTaskActivityItem } from "../types/api";
import type { AippViewerImage } from "./AippImageViewer";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export function activityImageArtifacts(item: AippTaskActivityItem): AippTaskActivityArtifact[] {
  return item.artifacts.filter((artifact) => artifact.kind === "image" && Boolean(artifact.preview_url || artifact.download_url));
}

export function activityVideoArtifacts(item: AippTaskActivityItem): AippTaskActivityArtifact[] {
  return item.artifacts.filter((artifact) => artifact.kind === "video" && Boolean(artifact.preview_url || artifact.download_url));
}

export function activityViewerImage(artifact: AippTaskActivityArtifact, initialSource?: string | null): AippViewerImage {
  return {
    title: artifact.filename,
    filename: artifact.filename,
    previewUrl: artifact.preview_url || artifact.download_url,
    downloadUrl: artifact.download_url,
    initialSource,
  };
}

export function ActivityImagePreview({ artifact, apiFetch, t, onOpen }: {
  artifact: AippTaskActivityArtifact;
  apiFetch: ApiFetch;
  t: Translate;
  onOpen: (source: string | null) => void;
}) {
  const [source, setSource] = useState<string | null>(null);
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;
  const endpoint = artifact.preview_url || artifact.download_url;
  useEffect(() => {
    setSource(null);
    if (!endpoint) return;
    let disposed = false;
    let objectUrl: string | null = null;
    void apiFetchRef.current(endpoint)
      .then(async (response) => {
        if (!response.ok) throw new Error(`aipp_activity_preview_http_${response.status}`);
        return response.blob();
      })
      .then((blob) => {
        if (disposed) return;
        objectUrl = URL.createObjectURL(blob);
        setSource(objectUrl);
      })
      .catch(() => {
        if (!disposed) setSource(null);
      });
    return () => {
      disposed = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [endpoint]);
  return (
    <button type="button" className="group relative block h-full w-full cursor-zoom-in" onClick={() => onOpen(source)} title={t("放大图片", "Enlarge image")} aria-label={t("放大图片", "Enlarge image")}>
      {source ? <img src={source} alt={artifact.filename} className="h-full w-full object-contain" /> : <span className="flex h-full w-full items-center justify-center text-white/35"><ImageIcon className="h-8 w-8" /></span>}
      <span className="absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-md border border-white/20 bg-black/60 text-white/85"><ZoomIn className="h-4 w-4" /></span>
    </button>
  );
}

export function ActivityVideoPreview({ artifact, apiFetch, t }: {
  artifact: AippTaskActivityArtifact;
  apiFetch: ApiFetch;
  t: Translate;
}) {
  const previewUrl = artifact.preview_url || artifact.download_url;
  const browserPreviewUrl = taskArtifactBrowserVideoUrl(previewUrl);
  const originalPreviewUrl = previewUrl;
  const videoPosterUrl = taskArtifactVideoPosterUrl(previewUrl);
  const playableInline = /^(video\/mp4|video\/webm)$/i.test(artifact.mime_type.split(";", 1)[0].trim());
  const automaticPreview = (artifact.size_bytes ?? Number.MAX_SAFE_INTEGER) <= MAX_AUTOMATIC_ARTIFACT_PREVIEW_BYTES;
  const [sourceMode, setSourceMode] = useState<"browser" | "original">(playableInline ? "original" : "browser");
  const mediaPreviewUrl = sourceMode === "browser" ? (browserPreviewUrl || originalPreviewUrl) : originalPreviewUrl;
  const [previewRequested, setPreviewRequested] = useState(automaticPreview);
  const [previewState, setPreviewState] = useState<"idle" | "loading" | "ready" | "error">(automaticPreview ? "loading" : "idle");
  const [previewObjectUrl, setPreviewObjectUrl] = useState<string | null>(null);
  const [posterObjectUrl, setPosterObjectUrl] = useState<string | null>(null);
  const [unsupported, setUnsupported] = useState(false);
  const [downloadLoading, setDownloadLoading] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;
  const fallbackTried = useRef(false);
  useEffect(() => {
    fallbackTried.current = false;
  }, [artifact.id]);

  const switchSource = () => {
    if (fallbackTried.current) return false;
    if (sourceMode === "original" && browserPreviewUrl && browserPreviewUrl !== originalPreviewUrl) {
      fallbackTried.current = true;
      setSourceMode("browser");
      return true;
    }
    if (sourceMode === "browser" && originalPreviewUrl && originalPreviewUrl !== mediaPreviewUrl) {
      fallbackTried.current = true;
      setSourceMode("original");
      return true;
    }
    return false;
  };

  useEffect(() => {
    if (!previewRequested || !mediaPreviewUrl) {
      setPreviewState(mediaPreviewUrl ? "idle" : "error");
      setPreviewObjectUrl(null);
      setUnsupported(false);
      return;
    }
    const controller = new AbortController();
    let objectUrl: string | null = null;
    setPreviewState("loading");
    setPreviewObjectUrl(null);
    setUnsupported(false);
    void fetchTaskArtifactBlob(apiFetchRef.current, mediaPreviewUrl, controller.signal)
      .then((blob) => {
        if (controller.signal.aborted) return;
        objectUrl = URL.createObjectURL(blob);
        setPreviewObjectUrl(objectUrl);
        setPreviewState("ready");
      })
      .catch(() => {
        if (controller.signal.aborted) return;
        if (switchSource()) return;
        setPreviewState("error");
      });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [artifact.id, mediaPreviewUrl, originalPreviewUrl, previewRequested, sourceMode]);

  useEffect(() => {
    if (!videoPosterUrl) {
      setPosterObjectUrl(null);
      return;
    }
    const controller = new AbortController();
    let objectUrl: string | null = null;
    setPosterObjectUrl(null);
    void fetchTaskArtifactBlob(apiFetchRef.current, videoPosterUrl, controller.signal)
      .then((blob) => {
        if (controller.signal.aborted) return;
        objectUrl = URL.createObjectURL(blob);
        setPosterObjectUrl(objectUrl);
      })
      .catch(() => {
        if (!controller.signal.aborted) setPosterObjectUrl(null);
      });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [artifact.id, videoPosterUrl]);

  const download = async () => {
    if (downloadLoading) return;
    setDownloadLoading(true);
    setDownloadError(null);
    try {
      const blob = await fetchTaskArtifactBlob(apiFetchRef.current, artifact.download_url);
      saveTaskArtifactBlob(blob, artifact.filename);
    } catch {
      setDownloadError(t("视频下载失败，请重试。", "Could not download the video. Try again."));
    } finally {
      setDownloadLoading(false);
    }
  };

  const requestPreview = () => {
    setPreviewState("loading");
    setPreviewRequested(true);
  };

  return (
    <div className="relative flex h-full min-h-28 w-full flex-col bg-black/20">
      {previewObjectUrl && !unsupported ? (
        <video
          controls
          playsInline
          preload="metadata"
          src={previewObjectUrl}
          poster={posterObjectUrl ?? undefined}
          className="h-full w-full flex-1 object-contain"
          title={artifact.filename}
          onLoadedData={(event) => {
            const video = event.currentTarget;
            if (video.videoWidth > 0 && video.videoHeight > 0) return;
            if (!switchSource()) setUnsupported(true);
          }}
          onError={() => {
            if (!switchSource()) setUnsupported(true);
          }}
        />
      ) : posterObjectUrl && (unsupported || previewState === "error") ? (
        <div className="flex h-full min-h-28 flex-1 flex-col">
          <img src={posterObjectUrl} alt={artifact.filename} className="min-h-0 flex-1 object-contain" />
          <p className="border-t border-white/10 px-2 py-1.5 text-[11px] leading-4 text-white/70">
            {t("网页里暂时播不了，原视频仍可下载。", "This copy cannot play in the browser. The original video can still be downloaded.")}
          </p>
        </div>
      ) : (
        <div className="flex min-h-28 flex-1 flex-col items-center justify-center gap-2 px-3 py-4 text-center text-xs text-white/65">
          {previewState === "error" ? (
            <>
              <p>{t("视频预览加载失败。", "The video preview could not load.")}</p>
              <button type="button" className="theme-secondary-btn px-2.5 py-1.5 text-xs" onClick={requestPreview}>
                <RefreshCw className="h-3.5 w-3.5" />{t("重试", "Retry")}
              </button>
            </>
          ) : previewRequested ? (
            <span className="inline-flex items-center gap-2">
              <LoaderCircle className="h-4 w-4 animate-spin" />
              {t("正在准备可播放预览…", "Preparing a playable preview…")}
            </span>
          ) : (
            <>
              <Video className="h-7 w-7 text-white/40" />
              <button type="button" className="theme-secondary-btn px-2.5 py-1.5 text-xs" onClick={requestPreview}>
                {t("预览视频", "Preview video")}
              </button>
            </>
          )}
        </div>
      )}
      <button
        type="button"
        className="absolute right-2 top-2 theme-icon-btn h-8 w-8 border border-white/20 bg-black/65 text-white/90"
        onClick={() => void download()}
        disabled={downloadLoading}
        title={t("下载视频", "Download video")}
        aria-label={t("下载视频", "Download video")}
      >
        {downloadLoading ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
      </button>
      {downloadError ? <p className="px-2 pb-2 text-[11px] text-red-200">{downloadError}</p> : null}
    </div>
  );
}
