import { useEffect, useRef, useState } from "react";
import { Download, Image as ImageIcon, LoaderCircle, Play, RefreshCw, Video, ZoomIn } from "lucide-react";

import {
  fetchTaskArtifactBlob,
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

export function ActivityVideoPreview({ artifact, apiFetch, t, onOpen }: {
  artifact: AippTaskActivityArtifact;
  apiFetch: ApiFetch;
  t: Translate;
  onOpen: () => void;
}) {
  const [poster, setPoster] = useState<string | null>(null);
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;
  const endpoint = taskArtifactVideoPosterUrl(artifact.preview_url || artifact.download_url);
  useEffect(() => {
    setPoster(null);
    if (!endpoint) return;
    const controller = new AbortController();
    let objectUrl: string | null = null;
    void fetchTaskArtifactBlob(apiFetchRef.current, endpoint, controller.signal).then((blob) => {
      if (controller.signal.aborted) return;
      objectUrl = URL.createObjectURL(blob);
      setPoster(objectUrl);
    }).catch(() => { /* A missing poster must not prevent opening the video. */ });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [endpoint]);
  return (
    <button type="button" className="relative block h-full min-h-24 w-full cursor-zoom-in bg-black/20" onClick={onOpen} title={t("展开视频", "Expand video")} aria-label={t("展开视频", "Expand video")}>
      {poster ? <img src={poster} alt={artifact.filename} className="h-full w-full object-contain" onError={() => setPoster(null)} /> : <span className="flex h-full w-full items-center justify-center text-[var(--theme-text-muted)]"><Video className="h-8 w-8" /></span>}
      <span className="absolute inset-0 flex items-center justify-center" aria-hidden="true"><span className="flex h-10 w-10 items-center justify-center rounded-full border border-white/40 bg-black/65 text-white"><Play className="ml-0.5 h-5 w-5" /></span></span>
    </button>
  );
}

// Mounted only inside the expanded viewer; the list never fetches the video body.
export function ActivityVideoPlayer({ artifact, apiFetch, t }: {
  artifact: AippTaskActivityArtifact;
  apiFetch: ApiFetch;
  t: Translate;
}) {
  const previewUrl = artifact.preview_url || artifact.download_url;
  const browserPreviewUrl = taskArtifactBrowserVideoUrl(previewUrl);
  const originalPreviewUrl = previewUrl;
  const videoPosterUrl = taskArtifactVideoPosterUrl(previewUrl);
  const playableInline = /^(video\/mp4|video\/webm)$/i.test(artifact.mime_type.split(";", 1)[0].trim());
  const [sourceMode, setSourceMode] = useState<"browser" | "original">(playableInline ? "original" : "browser");
  const mediaPreviewUrl = sourceMode === "browser" ? (browserPreviewUrl || originalPreviewUrl) : originalPreviewUrl;
  const [retry, setRetry] = useState(0);
  const [previewState, setPreviewState] = useState<"loading" | "ready" | "error">("loading");
  const [previewObjectUrl, setPreviewObjectUrl] = useState<string | null>(null);
  const [posterObjectUrl, setPosterObjectUrl] = useState<string | null>(null);
  const [unsupported, setUnsupported] = useState(false);
  const [downloadLoading, setDownloadLoading] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;
  const fallbackTried = useRef(false);
  const videoRef = useRef<HTMLVideoElement>(null);
  const downloadRequest = useRef<AbortController | null>(null);
  useEffect(() => () => { downloadRequest.current?.abort(); }, []);
  useEffect(() => {
    const video = videoRef.current;
    return () => {
      video?.pause();
      video?.removeAttribute("src");
      video?.load();
    };
  }, [previewObjectUrl, unsupported]);
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
    if (!mediaPreviewUrl) {
      setPreviewState("error");
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
  }, [artifact.id, mediaPreviewUrl, originalPreviewUrl, retry, sourceMode]);

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
    const controller = new AbortController();
    downloadRequest.current = controller;
    try {
      const blob = await fetchTaskArtifactBlob(apiFetchRef.current, artifact.download_url, controller.signal);
      if (!controller.signal.aborted) saveTaskArtifactBlob(blob, artifact.filename);
    } catch {
      if (!controller.signal.aborted) setDownloadError(t("视频下载失败，请重试。", "Could not download the video. Try again."));
    } finally {
      if (!controller.signal.aborted) setDownloadLoading(false);
      if (downloadRequest.current === controller) downloadRequest.current = null;
    }
  };

  const requestPreview = () => {
    setPreviewState("loading");
    fallbackTried.current = false;
    setUnsupported(false);
    setRetry((value) => value + 1);
  };

  return (
    <div className="relative flex h-full min-h-0 w-full flex-col bg-black/20">
      {previewObjectUrl && !unsupported ? (
        <video
          ref={videoRef}
          controls
          playsInline
          preload="metadata"
          src={previewObjectUrl}
          poster={posterObjectUrl ?? undefined}
          className="min-h-0 w-full flex-1 object-contain"
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
      ) : (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-3 py-4 text-center text-sm text-[var(--theme-text-body)]">
          {previewState === "error" || unsupported ? (
            <>
              {posterObjectUrl ? <img src={posterObjectUrl} alt={artifact.filename} className="min-h-0 w-full flex-1 object-contain" /> : null}
              <p role="alert">{t("视频暂时无法播放，可重试或下载原视频。", "The video cannot play right now. Retry or download the original.")}</p>
              <button type="button" className="theme-secondary-btn px-2.5 py-1.5 text-xs" onClick={requestPreview}>
                <RefreshCw className="h-3.5 w-3.5" />{t("重试", "Retry")}
              </button>
            </>
          ) : (
            <span role="status" className="inline-flex items-center gap-2">
              <LoaderCircle className="h-4 w-4 animate-spin" />
              {t("正在准备可播放预览…", "Preparing a playable preview…")}
            </span>
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
