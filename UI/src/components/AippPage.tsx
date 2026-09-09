import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft,
  Bot,
  Bookmark,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Download,
  Eye,
  ExternalLink,
  FileText,
  GalleryVerticalEnd,
  Heart,
  Image as ImageIcon,
  LoaderCircle,
  MessageCircle,
  Music,
  PanelsTopLeft,
  PackagePlus,
  RefreshCw,
  Search,
  Share2,
  Trash2,
  Video,
  ZoomIn,
} from "lucide-react";

import { formatUiError } from "../lib/ui-error";
import { collectionGroupKey, completeCollectionPage, groupCollectionItems } from "../lib/aipp-collection";
import { appStorageKey } from "../lib/product-identity";
import { useUiDialog } from "./UiDialogProvider";
import { AippImageViewer, type AippViewerImage } from "./AippImageViewer";
import type {
  AippCatalogItem,
  AippCatalogResponse,
  AippMediaItem,
  AippMediaPageResponse,
  AippTaskActivityArtifact,
  AippTaskActivityItem,
  AippTaskActivityPageResponse,
  ApiResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

const SELECTED_AIPP_STORAGE_KEY = appStorageKey("monitor.aipp.selectedSkill");
const AIPP_CATALOG_CACHE_KEY = appStorageKey("monitor.aipp.catalog.v1");
const AIPP_CATALOG_CACHE_TTL_MS = 5 * 60_000;
const AIPP_CATALOG_CACHE_MAX_BYTES = 256 * 1024;
const AIPP_AUTO_REFRESH_INTERVAL_MS = 10_000;
const AIPP_BRIDGE_MAX_IN_FLIGHT = 4;
const AIPP_BRIDGE_MAX_ARGS_BYTES = 64 * 1024;

export function readSelectedAipp(storage: Pick<Storage, "getItem"> | undefined): string {
  return storage?.getItem(SELECTED_AIPP_STORAGE_KEY)?.trim() || "";
}

type AippCatalogStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

function isCachedAippCatalogItem(value: unknown): value is AippCatalogItem {
  if (!value || typeof value !== "object") return false;
  const item = value as Partial<AippCatalogItem>;
  const localizedCopyIsValid = (copy: unknown) => (
    Boolean(copy)
    && typeof copy === "object"
    && Object.entries(copy as Record<string, unknown>).every(([locale, text]) => (
      locale.length > 0
      && locale.length <= 32
      && typeof text === "string"
      && text.length <= 4_096
    ))
  );
  return (
    typeof item.skill_name === "string"
    && item.skill_name.length > 0
    && item.skill_name.length <= 128
    && typeof item.package_version === "string"
    && item.package_version.length <= 128
    && typeof item.renderer === "string"
    && item.renderer.length <= 128
    && typeof item.data_contract === "string"
    && item.data_contract.length <= 128
    && typeof item.icon === "string"
    && item.icon.length <= 128
    && typeof item.default_locale === "string"
    && item.default_locale.length <= 32
    && localizedCopyIsValid(item.titles)
    && localizedCopyIsValid(item.descriptions)
    && typeof item.installed === "boolean"
    && (item.entrypoint == null || (typeof item.entrypoint === "string" && item.entrypoint.length <= 1_024))
    && Array.isArray(item.bridge_capabilities)
    && item.bridge_capabilities.length <= 128
    && item.bridge_capabilities.every((capability) => typeof capability === "string" && capability.length <= 256)
    && (item.task_channel_scope == null || (typeof item.task_channel_scope === "string" && item.task_channel_scope.length <= 64))
  );
}

export function readCachedAippCatalog(
  storage: AippCatalogStorage | undefined,
  nowMs = Date.now(),
): AippCatalogItem[] {
  if (!storage) return [];
  try {
    const raw = storage.getItem(AIPP_CATALOG_CACHE_KEY);
    if (!raw || raw.length > AIPP_CATALOG_CACHE_MAX_BYTES) return [];
    const cached = JSON.parse(raw) as {
      schema_version?: unknown;
      cached_at_ms?: unknown;
      apps?: unknown;
    };
    if (
      cached.schema_version !== 1
      || typeof cached.cached_at_ms !== "number"
      || cached.cached_at_ms > nowMs
      || nowMs - cached.cached_at_ms > AIPP_CATALOG_CACHE_TTL_MS
      || !Array.isArray(cached.apps)
      || cached.apps.length > 256
      || !cached.apps.every(isCachedAippCatalogItem)
    ) {
      storage.removeItem(AIPP_CATALOG_CACHE_KEY);
      return [];
    }
    return cached.apps;
  } catch {
    storage.removeItem(AIPP_CATALOG_CACHE_KEY);
    return [];
  }
}

export function writeCachedAippCatalog(
  storage: AippCatalogStorage | undefined,
  apps: AippCatalogItem[],
  nowMs = Date.now(),
): void {
  if (!storage || apps.length > 256 || !apps.every(isCachedAippCatalogItem)) return;
  try {
    storage.setItem(AIPP_CATALOG_CACHE_KEY, JSON.stringify({
      schema_version: 1,
      cached_at_ms: nowMs,
      apps,
    }));
  } catch {
    // Storage may be unavailable or full; the live catalog remains authoritative.
  }
}

export interface AippPageProps {
  lang: "zh" | "en";
  t: Translate;
  apiFetch: ApiFetch;
  onOpenAgent: () => void;
  onOpenSkillStore: () => void;
}

export function localizedAippCopy(
  values: Record<string, string>,
  lang: "zh" | "en",
  fallbackLocale: string,
): string {
  return values[lang] || values[fallbackLocale] || Object.values(values)[0] || "";
}

function AippIcon({ icon, className }: { icon: string; className?: string }) {
  if (icon === "gallery_vertical_end") {
    return <GalleryVerticalEnd className={className} />;
  }
  if (icon === "download") {
    return <Download className={className} />;
  }
  return <PanelsTopLeft className={className} />;
}

export function AippCatalogCard({
  app,
  lang,
  onOpen,
  onInstall,
}: {
  app: AippCatalogItem;
  lang: "zh" | "en";
  onOpen: () => void;
  onInstall: () => void;
}) {
  return (
    <article className="theme-panel flex min-h-40 w-full flex-col p-4">
      <button
        type="button"
        className="group flex min-w-0 flex-1 flex-col items-start text-left"
        onClick={app.installed ? onOpen : onInstall}
      >
        <span className="flex h-12 w-12 items-center justify-center rounded-lg border border-white/10 bg-white/6 text-white/80">
          <AippIcon icon={app.icon} className="h-6 w-6" />
        </span>
        <span className="mt-3 flex w-full min-w-0 items-center gap-2">
          <span className="min-w-0 flex-1 break-words text-base font-semibold text-white/90">
            {localizedAippCopy(app.titles, lang, app.default_locale)}
          </span>
          {app.installed ? <ChevronRight className="h-4 w-4 shrink-0 text-white/35 transition group-hover:translate-x-0.5 group-hover:text-white/65" /> : null}
        </span>
        <span className="mt-2 line-clamp-3 break-words text-sm leading-5 text-white/55">
          {localizedAippCopy(app.descriptions, lang, app.default_locale)}
        </span>
      </button>
      {!app.installed ? (
        <button type="button" className="theme-secondary-btn mt-3 w-full px-3 py-2 text-sm" onClick={onInstall}>
          <PackagePlus className="h-4 w-4" />
          {lang === "zh" ? "安装 Ai APP" : "Install Ai APP"}
        </button>
      ) : null}
    </article>
  );
}

interface AippBridgeRequest {
  schema_version: 1;
  type: "aipp.capability.invoke";
  request_id: string;
  capability: string;
  args?: Record<string, unknown>;
}

async function waitForAippTask(apiFetch: ApiFetch, taskId: string): Promise<unknown> {
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline) {
    const response = await apiFetch(`/v1/tasks/${encodeURIComponent(taskId)}`);
    const body = (await response.json()) as ApiResponse<import("../types/api").TaskQueryResponse>;
    if (!response.ok || !body.ok || !body.data) throw new Error(body.error || `aipp_task_http_${response.status}`);
    if (["succeeded", "failed", "canceled", "timeout"].includes(body.data.status)) {
      return {
        task_id: body.data.task_id,
        status: body.data.status,
        result_json: body.data.result_json ?? null,
        error_text: body.data.error_text ?? null,
      };
    }
    await new Promise((resolve) => window.setTimeout(resolve, 500));
  }
  throw new Error("aipp_task_wait_timeout");
}

export function SandboxedAipp({
  app,
  lang,
  apiFetch,
}: {
  app: AippCatalogItem;
  lang: "zh" | "en";
  apiFetch: ApiFetch;
}) {
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const pendingRequests = useRef(new Set<string>());
  const allowedCapabilities = useMemo(() => new Set(app.bridge_capabilities), [app.bridge_capabilities]);
  const post = useCallback((payload: unknown) => frameRef.current?.contentWindow?.postMessage(payload, "*"), []);
  const postContext = useCallback(() => post({
    schema_version: 1,
    type: "aipp.host.context",
    locale: lang,
    skill_name: app.skill_name,
    package_version: app.package_version,
    capabilities: [...allowedCapabilities],
  }), [allowedCapabilities, app.package_version, app.skill_name, lang, post]);

  useEffect(() => {
    const receive = (event: MessageEvent) => {
      if (event.source !== frameRef.current?.contentWindow || !event.data || typeof event.data !== "object") return;
      const request = event.data as Partial<AippBridgeRequest>;
      if (request.schema_version === 1 && (request as { type?: string }).type === "aipp.ready") {
        postContext();
        return;
      }
      if (
        request.schema_version !== 1
        || request.type !== "aipp.capability.invoke"
        || typeof request.request_id !== "string"
        || request.request_id.length === 0
        || request.request_id.length > 128
        || typeof request.capability !== "string"
      ) return;
      const respond = (ok: boolean, data?: unknown, error_code?: string) => post({
        schema_version: 1,
        type: "aipp.capability.result",
        request_id: request.request_id,
        ok,
        data,
        error_code,
      });
      if (!allowedCapabilities.has(request.capability)) {
        respond(false, undefined, "aipp_bridge_capability_denied");
        return;
      }
      const args = request.args && typeof request.args === "object" && !Array.isArray(request.args) ? request.args : {};
      let argsBytes = AIPP_BRIDGE_MAX_ARGS_BYTES + 1;
      try {
        argsBytes = new TextEncoder().encode(JSON.stringify(args)).byteLength;
      } catch {
        respond(false, undefined, "aipp_bridge_args_invalid");
        return;
      }
      if (
        pendingRequests.current.has(request.request_id)
        || pendingRequests.current.size >= AIPP_BRIDGE_MAX_IN_FLIGHT
      ) {
        respond(false, undefined, "aipp_bridge_busy");
        return;
      }
      if (argsBytes > AIPP_BRIDGE_MAX_ARGS_BYTES) {
        respond(false, undefined, "aipp_bridge_args_too_large");
        return;
      }
      pendingRequests.current.add(request.request_id);
      void (async () => {
        try {
          const response = await apiFetch("/v1/tasks", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              channel: "ui",
              kind: "ask",
              idempotency_key: `aipp-${crypto.randomUUID()}`,
              payload: {
                entrypoint: "run_capability",
                capability: request.capability,
                args,
              },
            }),
          });
          const body = (await response.json()) as ApiResponse<{ task_id: string }>;
          if (!response.ok || !body.ok || !body.data?.task_id) throw new Error(body.error || `aipp_submit_http_${response.status}`);
          respond(true, await waitForAippTask(apiFetch, body.data.task_id));
        } catch (error) {
          respond(false, undefined, error instanceof Error ? error.message : "aipp_bridge_request_failed");
        } finally {
          pendingRequests.current.delete(request.request_id as string);
        }
      })();
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, [allowedCapabilities, apiFetch, post, postContext]);

  const entrypoint = app.entrypoint || "";
  return (
    <iframe
      ref={frameRef}
      className="min-h-[65vh] w-full rounded-md border border-white/10 bg-transparent"
      src={`/v1/aipps/${encodeURIComponent(app.skill_name)}/assets/${entrypoint.split("/").map(encodeURIComponent).join("/")}`}
      title={localizedAippCopy(app.titles, lang, app.default_locale)}
      sandbox="allow-scripts allow-downloads"
      referrerPolicy="no-referrer"
      onLoad={postContext}
    />
  );
}

function formatCollectedAt(value: string | null, lang: "zh" | "en"): string {
  if (!value) return "-";
  const numeric = /^\d+$/.test(value) ? Number(value) : Number.NaN;
  const timestamp = Number.isFinite(numeric)
    ? numeric * (numeric < 10_000_000_000 ? 1_000 : 1)
    : Date.parse(value);
  if (!Number.isFinite(timestamp)) return value;
  return new Intl.DateTimeFormat(lang === "zh" ? "zh-CN" : "en", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(timestamp));
}

export function formatPublishedAt(value: string | null | undefined, lang: "zh" | "en"): string | null {
  if (!value) return null;
  // Preserve date-only precision and avoid shifting it across browser timezones.
  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    const date = new Date(`${value}T00:00:00Z`);
    return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value ? value : null;
  }
  if (!/^\d{4}-\d{2}-\d{2}T.+(?:Z|[+-]\d{2}:\d{2})$/.test(value) || !Number.isFinite(Date.parse(value))) return null;
  return formatCollectedAt(value, lang);
}

function formatArtifactSize(value: number | null): string {
  if (value == null || !Number.isFinite(value) || value < 0) return "-";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function activityChannelLabel(channel: string, t: Translate): string {
  const labels: Record<string, [string, string]> = {
    ui: ["网页", "Web"],
    wechat: ["微信", "WeChat"],
    telegram: ["Telegram", "Telegram"],
    whatsapp: ["WhatsApp", "WhatsApp"],
    feishu: ["飞书", "Feishu"],
    lark: ["Lark", "Lark"],
  };
  const label = labels[channel];
  return label ? t(label[0], label[1]) : channel;
}

function activityStatusLabel(status: string, t: Translate): string {
  const labels: Record<string, [string, string]> = {
    queued: ["等待中", "Queued"],
    running: ["处理中", "Running"],
    succeeded: ["已完成", "Completed"],
    failed: ["失败", "Failed"],
    canceled: ["已取消", "Canceled"],
    timeout: ["已超时", "Timed out"],
  };
  const label = labels[status];
  return label ? t(label[0], label[1]) : status;
}

function ActivityArtifactIcon({ artifact }: { artifact: AippTaskActivityArtifact }) {
  if (artifact.kind === "image") return <ImageIcon className="h-4 w-4" />;
  if (artifact.kind === "video") return <Video className="h-4 w-4" />;
  if (artifact.kind === "audio") return <Music className="h-4 w-4" />;
  return <FileText className="h-4 w-4" />;
}

function ActivityImagePreview({ artifact, apiFetch, t, onOpen }: {
  artifact: AippTaskActivityArtifact;
  apiFetch: ApiFetch;
  t: Translate;
  onOpen: (source: string | null) => void;
}) {
  const [source, setSource] = useState<string | null>(null);
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;
  useEffect(() => {
    setSource(null);
    const endpoint = artifact.preview_url;
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
  }, [artifact.preview_url]);
  return (
    <button type="button" className="group relative block h-full w-full cursor-zoom-in" onClick={() => onOpen(source)} title={t("放大图片", "Enlarge image")} aria-label={t("放大图片", "Enlarge image")}>
      {source ? <img src={source} alt={artifact.filename} className="h-full w-full object-contain" /> : <span className="flex h-full w-full items-center justify-center text-white/35"><ImageIcon className="h-8 w-8" /></span>}
      <span className="absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-md border border-white/20 bg-black/60 text-white/85"><ZoomIn className="h-4 w-4" /></span>
    </button>
  );
}

export function AippTaskActivityCard({
  item,
  apiFetch,
  t,
  lang,
}: {
  item: AippTaskActivityItem;
  apiFetch: ApiFetch;
  t: Translate;
  lang: "zh" | "en";
}) {
  const [expanded, setExpanded] = useState(false);
  const [artifactAction, setArtifactAction] = useState<string | null>(null);
  const [artifactError, setArtifactError] = useState<string | null>(null);
  const [viewerImage, setViewerImage] = useState<AippViewerImage | null>(null);
  const openImage = (artifact: AippTaskActivityArtifact, initialSource?: string | null) => {
    setViewerImage({
      title: artifact.filename,
      filename: artifact.filename,
      previewUrl: artifact.preview_url || artifact.download_url,
      downloadUrl: artifact.download_url,
      initialSource,
    });
  };
  const preview = item.artifacts.find((artifact) => artifact.kind === "image" && artifact.preview_url);
  const longContent = item.input_text.length > 320 || item.result_text.length > 720;
  const fetchArtifact = async (artifact: AippTaskActivityArtifact, open: boolean) => {
    if (open && artifact.kind === "image") {
      openImage(artifact);
      return;
    }
    if (artifactAction) return;
    setArtifactAction(`${open ? "open" : "download"}:${artifact.id}`);
    setArtifactError(null);
    try {
      const endpoint = open && artifact.preview_url ? artifact.preview_url : artifact.download_url;
      const response = await apiFetch(endpoint);
      if (!response.ok) throw new Error(`aipp_activity_artifact_http_${response.status}`);
      const objectUrl = URL.createObjectURL(await response.blob());
      const anchor = document.createElement("a");
      anchor.href = objectUrl;
      if (open) {
        anchor.target = "_blank";
        anchor.rel = "noreferrer noopener";
      } else {
        anchor.download = artifact.filename;
      }
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      window.setTimeout(() => URL.revokeObjectURL(objectUrl), 60_000);
    } catch {
      setArtifactError(t("文件读取失败，请重试。", "Could not read the file. Try again."));
    } finally {
      setArtifactAction(null);
    }
  };
  return (
    <article className="theme-panel min-w-0 overflow-hidden">
      <div className={preview ? "grid min-w-0 sm:grid-cols-[minmax(120px,22%)_minmax(0,1fr)]" : "min-w-0"}>
        {preview ? (
          <div className="aspect-video max-h-44 min-h-28 overflow-hidden bg-black/20 sm:aspect-auto">
            <ActivityImagePreview artifact={preview} apiFetch={apiFetch} t={t} onOpen={(source) => openImage(preview, source)} />
          </div>
        ) : null}
        <div className="min-w-0 p-3 sm:p-4">
          <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs text-white/48">
            <span>{activityChannelLabel(item.channel, t)}</span>
            <span className={item.status === "failed" || item.status === "timeout" ? "text-red-200" : "text-white/70"}>
              {activityStatusLabel(item.status, t)}
            </span>
            <span>{formatCollectedAt(item.created_at, lang)}</span>
            <span title={item.task_id}>#{item.task_id.slice(0, 8)}</span>
          </div>
          {item.actions.length > 0 ? (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {item.actions.map((action) => (
                <span key={action} className="rounded border border-white/10 px-2 py-0.5 text-xs text-white/55">
                  {action.split(".").pop() || action}
                </span>
              ))}
            </div>
          ) : null}
          <section className="mt-3 min-w-0">
            <p className="text-xs font-medium text-white/45">{t("原始请求", "Original request")}</p>
            <p className={`mt-1 whitespace-pre-wrap break-words text-sm leading-5 text-white/72 [overflow-wrap:anywhere] ${longContent && !expanded ? "line-clamp-3" : ""}`}>
              {item.input_text || t("没有可展示的文本输入", "No text input is available")}
            </p>
          </section>
          {item.source_urls.length > 0 ? (
            <div className="mt-2 flex min-w-0 flex-wrap gap-2">
              {item.source_urls.map((url, index) => (
                <a key={url} className="theme-secondary-btn max-w-full px-2.5 py-1 text-xs" href={url} target="_blank" rel="noreferrer noopener" title={url}>
                  <span className="max-w-64 truncate">{t("媒体链接", "Media link")} {index + 1}</span>
                  <ExternalLink className="h-3.5 w-3.5 shrink-0" />
                </a>
              ))}
            </div>
          ) : null}
          {item.result_text ? (
            <section className="mt-3 min-w-0 border-t border-white/8 pt-3">
              <p className="text-xs font-medium text-white/45">{t("处理结果", "Processed result")}</p>
              <p className={`mt-1 whitespace-pre-wrap break-words text-sm leading-6 text-white/80 [overflow-wrap:anywhere] ${longContent && !expanded ? "line-clamp-6" : ""}`}>
                {item.result_text}
              </p>
            </section>
          ) : null}
          {item.error_text ? <p className="mt-3 break-words text-sm text-red-200">{item.error_text}</p> : null}
          {longContent ? (
            <button type="button" className="theme-secondary-btn mt-3 px-3 py-1.5 text-xs" onClick={() => setExpanded((current) => !current)}>
              {expanded ? t("收起", "Collapse") : t("展开全文", "Show all")}
              {expanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
            </button>
          ) : null}
        </div>
      </div>
      {item.artifacts.length > 0 ? (
        <div className="border-t border-white/8 px-3 py-2 sm:px-4">
          <p className="mb-1 text-xs font-medium text-white/45">{t("生成文件", "Output files")}</p>
          {item.artifacts.map((artifact) => (
            <div key={artifact.id} className="flex min-w-0 items-center gap-2 border-t border-white/6 py-2 first:border-t-0">
              <span className="shrink-0 text-white/50"><ActivityArtifactIcon artifact={artifact} /></span>
              <span className="min-w-0 flex-1 truncate text-sm text-white/75" title={artifact.filename}>{artifact.filename}</span>
              <span className="shrink-0 text-xs text-white/40">{formatArtifactSize(artifact.size_bytes)}</span>
              {artifact.preview_url ? (
                <button type="button" className="theme-icon-btn h-8 w-8 shrink-0" onClick={() => void fetchArtifact(artifact, true)} title={t("预览", "Preview")}>
                  {artifactAction === `open:${artifact.id}` ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Eye className="h-4 w-4" />}
                </button>
              ) : null}
              <button type="button" className="theme-icon-btn h-8 w-8 shrink-0" onClick={() => void fetchArtifact(artifact, false)} title={t("下载", "Download")}>
                {artifactAction === `download:${artifact.id}` ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
              </button>
            </div>
          ))}
          {artifactError ? <p className="pb-2 text-xs text-red-200">{artifactError}</p> : null}
        </div>
      ) : null}
      {viewerImage ? <AippImageViewer image={viewerImage} apiFetch={apiFetch} t={t} onClose={() => setViewerImage(null)} /> : null}
    </article>
  );
}

function MediaPreview({
  item,
  skillName,
  apiFetch,
  t,
}: {
  item: AippMediaItem;
  skillName: string;
  apiFetch: ApiFetch;
  t: Translate;
}) {
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const mayHaveLocalPreview = item.kind === "image" || item.preview_available;
  const [shouldLoad, setShouldLoad] = useState(!mayHaveLocalPreview);
  const [viewerOpen, setViewerOpen] = useState(false);
  const visibilityRef = useRef<HTMLDivElement | null>(null);
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;

  useEffect(() => {
    if (!mayHaveLocalPreview || shouldLoad) return;
    if (typeof IntersectionObserver === "undefined") {
      setShouldLoad(true);
      return;
    }
    const target = visibilityRef.current;
    if (!target) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) setShouldLoad(true);
    }, { rootMargin: "240px" });
    observer.observe(target);
    return () => observer.disconnect();
  }, [mayHaveLocalPreview, shouldLoad]);

  useEffect(() => {
    if (!mayHaveLocalPreview || !shouldLoad) return;
    let disposed = false;
    let objectUrl: string | null = null;
    void apiFetchRef.current(
      `/v1/aipps/${encodeURIComponent(skillName)}/items/${item.global_sequence}/preview`,
    )
      .then(async (response) => {
        if (!response.ok) throw new Error(`aipp_preview_http_${response.status}`);
        return response.blob();
      })
      .then((blob) => {
        if (disposed) return;
        objectUrl = URL.createObjectURL(blob);
        setPreviewUrl(objectUrl);
      })
      .catch(() => {
        if (!disposed) setPreviewUrl(null);
      });
    return () => {
      disposed = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [item.global_sequence, mayHaveLocalPreview, shouldLoad, skillName]);

  const source = previewUrl || (item.kind === "image" ? item.image_url : null);
  if (source) {
    const image = (
      <img
        src={source}
        alt={item.title}
        loading="lazy"
        referrerPolicy="no-referrer"
        className="h-full w-full object-contain"
      />
    );
    return (
      <>
        <button
          type="button"
          className="group relative block h-full w-full cursor-zoom-in overflow-hidden"
          onClick={() => setViewerOpen(true)}
          title={t("放大图片", "Enlarge image")}
          aria-label={t("放大图片", "Enlarge image")}
        >
          {image}
          <span className="absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-md border border-white/20 bg-black/60 text-white/85 opacity-80 transition group-hover:opacity-100">
            <ZoomIn className="h-4 w-4" />
          </span>
        </button>
        {viewerOpen ? <AippImageViewer image={{
          title: item.title,
          filename: `media-${String(item.global_sequence).padStart(12, "0")}.png`,
          previewUrl: `/v1/aipps/${encodeURIComponent(skillName)}/items/${item.global_sequence}/preview`,
          downloadUrl: `/v1/aipps/${encodeURIComponent(skillName)}/items/${item.global_sequence}/preview`,
          initialSource: source,
        }} apiFetch={apiFetch} t={t} onClose={() => setViewerOpen(false)} /> : null}
      </>
    );
  }
  return (
    <div ref={visibilityRef} className="flex h-full w-full flex-col items-center justify-center gap-2 text-white/45">
      {item.kind === "video" ? <Video className="h-8 w-8" /> : <ImageIcon className="h-8 w-8" />}
      <span className="text-xs">{t("暂无预览", "Preview unavailable")}</span>
    </div>
  );
}

export function AippMediaItemCard({
  item,
  images,
  skillName,
  apiFetch,
  t,
  lang,
}: {
  item: AippMediaItem;
  images?: AippMediaItem[];
  skillName: string;
  apiFetch: ApiFetch;
  t: Translate;
  lang: "zh" | "en";
}) {
  const [expanded, setExpanded] = useState(false);
  const [selectedImageId, setSelectedImageId] = useState(item.global_sequence);
  const gallery = images?.length ? images : [item];
  const selectedIndex = Math.max(0, gallery.findIndex(image => image.global_sequence === selectedImageId));
  const selectedImage = gallery[selectedIndex];
  const captionText = item.platform_text.trim();
  const publishedAt = formatPublishedAt(item.published_at, lang);
  const captionRef = useRef<HTMLParagraphElement>(null);
  const [canCollapse, setCanCollapse] = useState(false);
  useEffect(() => {
    if (expanded) return;
    const node = captionRef.current;
    if (!node) { setCanCollapse(false); return; }
    const measure = () => setCanCollapse(node.scrollHeight > node.clientHeight + 1);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [captionText, expanded]);
  const textSections = [
    { key: "caption", label: t("帖子文案", "Post caption"), text: captionText },
  ].filter((section) => section.text);
  const hasPreview = gallery.length > 1 || selectedImage.preview_available || (selectedImage.kind === "image" && Boolean(selectedImage.image_url));
  const metricPresentation = [
    { key: "views" as const, icon: Eye, label: t("播放", "Views") },
    { key: "likes" as const, icon: Heart, label: t("点赞", "Likes") },
    { key: "comments" as const, icon: MessageCircle, label: t("评论", "Comments") },
    { key: "favorites" as const, icon: Bookmark, label: t("收藏", "Favorites") },
    { key: "shares" as const, icon: Share2, label: t("分享", "Shares") },
  ].flatMap(({ key, icon: Icon, label }) => {
    const metric = item.engagement?.metrics[key];
    return metric ? [{ key, Icon, label, display: metric.display }] : [];
  });
  return (
    <article className="theme-panel min-w-0 overflow-hidden">
      <div className={hasPreview ? "grid min-w-0 sm:grid-cols-[minmax(104px,24%)_minmax(0,1fr)]" : "min-w-0"}>
        {hasPreview ? (
          <div className="relative aspect-video max-h-36 min-h-24 overflow-hidden bg-black/20 sm:aspect-auto sm:min-h-28 sm:max-h-36">
            <MediaPreview key={selectedImage.global_sequence} item={selectedImage} skillName={skillName} apiFetch={apiFetch} t={t} />
            {gallery.length > 1 ? <div className="absolute inset-x-0 bottom-0 flex h-8 items-center justify-between border-t border-[var(--theme-border)] bg-[var(--theme-dialog-bg)] px-1 text-xs text-[var(--theme-text-strong)]">
              <button type="button" className="flex h-7 w-7 items-center justify-center disabled:opacity-30" title={t("上一张图片", "Previous image")} disabled={selectedIndex === 0} onClick={() => setSelectedImageId(gallery[selectedIndex - 1].global_sequence)}><ChevronLeft className="h-4 w-4" /></button>
              <span aria-live="polite">{selectedIndex + 1} / {gallery.length}</span>
              <button type="button" className="flex h-7 w-7 items-center justify-center disabled:opacity-30" title={t("下一张图片", "Next image")} disabled={selectedIndex === gallery.length - 1} onClick={() => setSelectedImageId(gallery[selectedIndex + 1].global_sequence)}><ChevronRight className="h-4 w-4" /></button>
            </div> : null}
          </div>
        ) : null}
        <div className="min-w-0 p-3 sm:p-4">
          <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs text-white/45">
            <span className="inline-flex items-center gap-1">{item.kind === "video" ? <Video className="h-3.5 w-3.5" /> : <ImageIcon className="h-3.5 w-3.5" />}{item.kind === "video" ? t("视频", "Video") : t("图片", "Image")}</span>
            <span className="break-all">{item.platform}</span>
            <span>#{item.global_sequence}</span>
            {gallery.length > 1 ? <span>{gallery.length} {t("张图片", "images")}</span> : null}
          </div>
          <h2 className="mt-2 line-clamp-2 break-words text-sm font-semibold leading-5 text-white/90 [overflow-wrap:anywhere]">{item.title || t("未提供标题", "Untitled")}</h2>
          <div className="mt-1.5 space-y-1 break-words text-xs leading-4 text-white/45 [overflow-wrap:anywhere]">
            <p>{publishedAt ? t("发布：", "Published: ") : item.publication_text ? t("发布（采集时显示）：", "Published (as captured): ") : t("发布：", "Published: ")}{publishedAt || item.publication_text || t("未提供", "Unavailable")}</p>
            <p>{t("采集：", "Collected: ")}{formatCollectedAt(item.discovered_at, lang)}</p>
          </div>
          {metricPresentation.length > 0 ? (
            <div className="mt-2 grid grid-cols-2 gap-x-3 gap-y-1.5 text-xs text-white/58 sm:grid-cols-3" aria-label={t("采集时互动数据", "Engagement at capture")}>
              {metricPresentation.map(({ key, Icon, label, display }) => (
                <span key={key} className="inline-flex min-w-0 items-center gap-1" title={`${label}: ${display} · ${t("采集：", "Captured: ")}${formatCollectedAt(item.engagement?.captured_at || item.discovered_at, lang)}`}>
                  <Icon className="h-3.5 w-3.5 shrink-0" />
                  <span className="truncate">{display}</span>
                </span>
              ))}
            </div>
          ) : null}
          {textSections.length > 0 ? (
            <div className="mt-2 space-y-2">
              {textSections.map((section) => (
                <section key={section.key} className="min-w-0">
                  <p className="text-xs font-medium text-white/45">{section.label}</p>
                  <p ref={captionRef} className={`mt-0.5 whitespace-pre-wrap break-words text-sm leading-5 text-white/72 [overflow-wrap:anywhere] ${!expanded ? "line-clamp-2" : ""}`}>
                    {section.text}
                  </p>
                </section>
              ))}
            </div>
          ) : null}
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {canCollapse ? (
              <button type="button" className="theme-secondary-btn px-3 py-1.5 text-xs" onClick={() => setExpanded((current) => !current)}>
                {expanded ? t("收起", "Collapse") : t("展开全文", "Show all")}
                {expanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
              </button>
            ) : null}
            {item.source_url ? (
              <a className="theme-secondary-btn px-3 py-1.5 text-xs" href={item.source_url} target="_blank" rel="noreferrer noopener">
              {t("查看来源", "Open source")}<ExternalLink className="h-3.5 w-3.5" />
              </a>
            ) : null}
          </div>
        </div>
      </div>
    </article>
  );
}

export function AippPage({ lang, t, apiFetch, onOpenAgent, onOpenSkillStore }: AippPageProps) {
  const { confirm } = useUiDialog();
  const initialCatalog = useRef(
    readCachedAippCatalog(typeof window === "undefined" ? undefined : window.sessionStorage),
  );
  const [catalog, setCatalog] = useState<AippCatalogItem[]>(initialCatalog.current);
  const [selectedSkill, setSelectedSkill] = useState(() =>
    readSelectedAipp(typeof window === "undefined" ? undefined : window.localStorage),
  );
  const [catalogLoading, setCatalogLoading] = useState(initialCatalog.current.length === 0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState<AippMediaPageResponse | null>(null);
  const [activityPage, setActivityPage] = useState<AippTaskActivityPageResponse | null>(null);
  const [kind, setKind] = useState<"all" | "video" | "image">("all");
  const [platform, setPlatform] = useState("all");
  const [activityChannel, setActivityChannel] = useState("all");
  const [activityStatus, setActivityStatus] = useState("all");
  const [searchDraft, setSearchDraft] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [sortOrder, setSortOrder] = useState<"newest" | "oldest">("newest");
  const [cursor, setCursor] = useState<number | null>(null);
  const [cursorHistory, setCursorHistory] = useState<Array<number | null>>([]);
  const [installActionSkill, setInstallActionSkill] = useState<string | null>(null);
  const requestSequence = useRef(0);
  const autoRefreshInFlight = useRef(false);
  const apiFetchRef = useRef(apiFetch);
  const translateRef = useRef(t);
  apiFetchRef.current = apiFetch;
  translateRef.current = t;
  const selectedApp = useMemo(
    () => catalog.find((app) => app.skill_name === selectedSkill) || null,
    [catalog, selectedSkill],
  );
  const activityChannels = selectedApp?.task_channel_scope === "communication"
    ? (["wechat", "telegram", "whatsapp", "feishu", "lark"] as const)
    : (["ui", "wechat", "telegram", "whatsapp", "feishu", "lark"] as const);

  const fetchCatalog = useCallback(async (silent = false) => {
    if (!silent) {
      setCatalogLoading(true);
      setError(null);
    }
    try {
      const response = await apiFetchRef.current("/v1/aipps");
      const body = (await response.json()) as ApiResponse<AippCatalogResponse>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `aipp_catalog_http_${response.status}`);
      }
      setCatalog(body.data.apps);
      writeCachedAippCatalog(
        typeof window === "undefined" ? undefined : window.sessionStorage,
        body.data.apps,
      );
      setSelectedSkill((current) =>
        body.data?.apps.some((app) => app.skill_name === current && app.installed)
          ? current
          : "",
      );
    } catch (cause) {
      if (!silent) {
        setError(formatUiError(cause, translateRef.current, "AiAPP 列表读取失败。", "Could not load the AiAPP catalog."));
      }
    } finally {
      if (!silent) setCatalogLoading(false);
    }
  }, []);

  const updateAippInstallState = useCallback(async (app: AippCatalogItem, installed: boolean) => {
    if (!installed) {
      const accepted = await confirm({
        title: t("卸载 Ai APP", "Uninstall Ai APP"),
        message: t(
          "只移除这个可视化应用。对应技能、配置和采集数据都会保留，可以随时重新安装。",
          "Only the visual app will be removed. Its skill, configuration, and collected data remain available for later reinstallation.",
        ),
        confirmLabel: t("卸载", "Uninstall"),
        cancelLabel: t("取消", "Cancel"),
        tone: "danger",
      });
      if (!accepted) return;
    }
    setInstallActionSkill(app.skill_name);
    setError(null);
    try {
      const response = await apiFetchRef.current(`/v1/aipps/${encodeURIComponent(app.skill_name)}`, {
        method: installed ? "POST" : "DELETE",
      });
      const body = (await response.json()) as ApiResponse<{ installed: boolean }>;
      if (!response.ok || !body.ok) throw new Error(body.error || `aipp_install_state_http_${response.status}`);
      if (installed) setSelectedSkill(app.skill_name);
      else if (selectedSkill === app.skill_name) setSelectedSkill("");
      await fetchCatalog();
    } catch (cause) {
      setError(formatUiError(cause, t, "Ai APP 状态更新失败。", "Could not update the Ai APP."));
    } finally {
      setInstallActionSkill(null);
    }
  }, [confirm, fetchCatalog, selectedSkill, t]);

  const fetchPage = useCallback(async (silent = false) => {
    const currentRequest = ++requestSequence.current;
    if (
      !selectedSkill
      || !selectedApp?.installed
      || !["collection_feed_v1", "task_activity_v1"].includes(selectedApp.renderer)
    ) {
      setPage(null);
      setActivityPage(null);
      setLoading(false);
      return;
    }
    if (!silent) {
      setLoading(true);
      setError(null);
    }
    const params = new URLSearchParams({ limit: "20", sort_order: sortOrder });
    if (cursor != null) params.set("cursor_sequence", String(cursor));
    if (selectedApp.renderer === "collection_feed_v1") {
      if (kind !== "all") params.set("kind", kind);
      if (platform !== "all") params.set("platform", platform);
    } else {
      if (activityChannel !== "all") params.set("channel", activityChannel);
      if (activityStatus !== "all") params.set("status", activityStatus);
    }
    if (searchQuery) params.set("query", searchQuery);
    try {
      const response = await apiFetchRef.current(
        `/v1/aipps/${encodeURIComponent(selectedSkill)}/items?${params.toString()}`,
      );
      const body = (await response.json()) as ApiResponse<AippMediaPageResponse | AippTaskActivityPageResponse>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `aipp_items_http_${response.status}`);
      }
      if (currentRequest === requestSequence.current) {
        if (selectedApp.renderer === "collection_feed_v1") {
          const complete = await completeCollectionPage(body.data as AippMediaPageResponse, async nextCursor => {
            const nextParams = new URLSearchParams(params);
            nextParams.set("cursor_sequence", String(nextCursor));
            const nextResponse = await apiFetchRef.current(`/v1/aipps/${encodeURIComponent(selectedSkill)}/items?${nextParams}`);
            const nextBody = await nextResponse.json() as ApiResponse<AippMediaPageResponse>;
            if (!nextResponse.ok || !nextBody.ok || !nextBody.data) throw new Error(nextBody.error || `aipp_items_http_${nextResponse.status}`);
            return nextBody.data;
          }, () => currentRequest === requestSequence.current);
          if (currentRequest !== requestSequence.current) return;
          setPage(complete);
          setActivityPage(null);
        } else {
          setActivityPage(body.data as AippTaskActivityPageResponse);
          setPage(null);
        }
      }
    } catch (cause) {
      if (!silent && currentRequest === requestSequence.current) {
        setError(formatUiError(cause, translateRef.current, "Ai APP 内容读取失败。", "Could not load Ai APP content."));
      }
    } finally {
      if (!silent && currentRequest === requestSequence.current) setLoading(false);
    }
  }, [activityChannel, activityStatus, cursor, kind, platform, searchQuery, selectedApp?.installed, selectedApp?.renderer, selectedSkill, sortOrder]);

  useEffect(() => {
    void fetchCatalog(initialCatalog.current.length > 0);
  }, [fetchCatalog]);

  useEffect(() => {
    if (selectedSkill) window.localStorage.setItem(SELECTED_AIPP_STORAGE_KEY, selectedSkill);
    else window.localStorage.removeItem(SELECTED_AIPP_STORAGE_KEY);
  }, [selectedSkill]);

  useEffect(() => {
    const timer = window.setTimeout(() => setSearchQuery(searchDraft.trim()), 300);
    return () => window.clearTimeout(timer);
  }, [searchDraft]);

  useEffect(() => {
    setCursor(null);
    setCursorHistory([]);
  }, [selectedSkill, kind, platform, activityChannel, activityStatus, searchQuery, sortOrder]);

  useEffect(() => {
    if (selectedApp?.task_channel_scope === "communication" && activityChannel === "ui") {
      setActivityChannel("all");
    }
  }, [activityChannel, selectedApp?.task_channel_scope]);

  useEffect(() => {
    void fetchPage();
  }, [fetchPage]);

  useEffect(() => {
    if (!selectedSkill || !["collection_feed_v1", "task_activity_v1"].includes(selectedApp?.renderer || "")) return;
    const refreshVisiblePage = async () => {
      if (document.visibilityState !== "visible" || autoRefreshInFlight.current) return;
      autoRefreshInFlight.current = true;
      try {
        await fetchPage(true);
      } finally {
        autoRefreshInFlight.current = false;
      }
    };
    const timer = window.setInterval(() => void refreshVisiblePage(), AIPP_AUTO_REFRESH_INTERVAL_MS);
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") void refreshVisiblePage();
    };
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [fetchPage, selectedApp?.renderer, selectedSkill]);
  const platforms = useMemo(
    () => Object.keys(page?.platform_states || {}).sort(),
    [page?.platform_states],
  );

  const openNext = () => {
    const next = selectedApp?.renderer === "task_activity_v1"
      ? activityPage?.next_cursor_sequence
      : page?.next_cursor_sequence;
    if (next == null) return;
    setCursorHistory((current) => [...current, cursor]);
    setCursor(next);
  };
  const openPrevious = () => {
    setCursorHistory((current) => {
      if (current.length === 0) return current;
      const previous = current[current.length - 1];
      setCursor(previous);
      return current.slice(0, -1);
    });
  };

  if (catalogLoading && catalog.length === 0) {
    return <div className="flex min-h-64 items-center justify-center"><LoaderCircle className="h-6 w-6 animate-spin text-white/55" /></div>;
  }

  if (catalog.length === 0) {
    return (
      <section className="theme-panel p-5 sm:p-6">
        <div className="flex items-start gap-3">
          <GalleryVerticalEnd className="mt-0.5 h-6 w-6 text-white/65" />
          <div className="min-w-0">
            <h1 className="text-lg font-semibold text-white">AiAPP</h1>
            <p className={`mt-2 text-sm leading-6 ${error ? "text-red-200" : "text-white/60"}`}>
              {error || t("当前没有已启用技能提供 AiAPP。", "No enabled skill currently provides an AiAPP.")}
            </p>
            <button type="button" className="theme-secondary-btn mt-4 px-3 py-2 text-sm" onClick={onOpenSkillStore}>
              {t("打开 Skill Store", "Open Skill Store")}
            </button>
          </div>
        </div>
      </section>
    );
  }

  if (!selectedApp) {
    return (
      <section className="space-y-4">
        <header>
          <p className="text-xs font-medium text-white/45">AiAPP</p>
          <h1 className="mt-1 text-xl font-semibold text-white">{t("应用", "Apps")}</h1>
        </header>
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
          {catalog.map((app) => (
            <AippCatalogCard
              key={app.skill_name}
              app={app}
              lang={lang}
              onOpen={() => setSelectedSkill(app.skill_name)}
              onInstall={() => void updateAippInstallState(app, true)}
            />
          ))}
        </div>
      </section>
    );
  }

  return (
    <section className="space-y-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div className="flex min-w-0 items-start gap-3">
          <button
            type="button"
            className="theme-icon-btn mt-0.5 h-9 w-9 shrink-0"
            onClick={() => setSelectedSkill("")}
            title={t("返回应用列表", "Back to apps")}
          >
            <ArrowLeft className="h-4 w-4" />
          </button>
          <div className="min-w-0">
            <p className="text-xs font-medium text-white/45">AiAPP</p>
            <h1 className="mt-1 text-xl font-semibold text-white">
              {localizedAippCopy(selectedApp.titles, lang, selectedApp.default_locale)}
            </h1>
            <p className="mt-1 max-w-3xl text-sm leading-6 text-white/58">
              {localizedAippCopy(selectedApp.descriptions, lang, selectedApp.default_locale)}
            </p>
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" onClick={onOpenAgent}>
            <Bot className="h-4 w-4" />
            Agent
          </button>
          {["collection_feed_v1", "task_activity_v1"].includes(selectedApp.renderer) ? (
            <button type="button" className="theme-icon-btn h-9 w-9" onClick={() => void fetchPage()} title={t("刷新内容", "Refresh content")}>
              <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
            </button>
          ) : null}
          <button
            type="button"
            className="theme-icon-btn h-9 w-9 text-red-200"
            onClick={() => void updateAippInstallState(selectedApp, false)}
            disabled={installActionSkill === selectedApp.skill_name}
            title={t("卸载 Ai APP", "Uninstall Ai APP")}
          >
            {installActionSkill === selectedApp.skill_name ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Trash2 className="h-4 w-4" />}
          </button>
        </div>
      </header>

      {selectedApp.renderer === "sandbox_bundle_v1" ? (
        <SandboxedAipp app={selectedApp} lang={lang} apiFetch={apiFetch} />
      ) : null}

      {selectedApp.renderer === "task_activity_v1" ? <>
        <section className="theme-panel-soft p-3 sm:p-4">
          <div className="grid gap-2 sm:grid-cols-2 sm:gap-3">
            <div>
              <p className="text-xs text-white/45">{t("当前页记录", "Records on this page")}</p>
              <p className="mt-1 text-sm font-medium text-white/85">{activityPage?.page_item_count ?? 0}</p>
            </div>
            <div className="min-w-0">
              <p className="text-xs text-white/45">{t("最近更新", "Latest update")}</p>
              <p className="mt-1 truncate text-xs font-medium text-white/85">
                {activityPage?.updated_at_ms
                  ? formatCollectedAt(String(activityPage.updated_at_ms), lang)
                  : "-"}
              </p>
            </div>
          </div>
        </section>

        <div className="grid min-w-0 gap-2 lg:grid-cols-[minmax(240px,1fr)_auto_auto_auto] lg:items-center">
          <label className="relative min-w-0 flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-white/40" />
            <input
              className="theme-input w-full py-2 pl-9 pr-3 text-sm"
              value={searchDraft}
              onChange={(event) => setSearchDraft(event.target.value)}
              placeholder={t("搜索原始请求或处理结果", "Search requests or processed results")}
              maxLength={200}
            />
          </label>
          <select className="theme-input w-full min-w-0 py-2 text-sm lg:w-auto lg:min-w-32" value={activityChannel} onChange={(event) => setActivityChannel(event.target.value)}>
            <option value="all">{t("全部通信端", "All channels")}</option>
            {activityChannels.map((channel) => (
              <option key={channel} value={channel}>{activityChannelLabel(channel, t)}</option>
            ))}
          </select>
          <select className="theme-input w-full min-w-0 py-2 text-sm lg:w-auto lg:min-w-28" value={activityStatus} onChange={(event) => setActivityStatus(event.target.value)}>
            <option value="all">{t("全部状态", "All statuses")}</option>
            {(["succeeded", "running", "failed", "canceled", "timeout"] as const).map((status) => (
              <option key={status} value={status}>{activityStatusLabel(status, t)}</option>
            ))}
          </select>
          <select
            className="theme-input w-full min-w-0 py-2 text-sm lg:w-auto lg:min-w-36"
            value={sortOrder}
            onChange={(event) => setSortOrder(event.target.value as "newest" | "oldest")}
            aria-label={t("按处理时间排序", "Sort by processing time")}
          >
            <option value="newest">{t("最新优先", "Newest first")}</option>
            <option value="oldest">{t("最早优先", "Oldest first")}</option>
          </select>
        </div>

        {error ? <div className="rounded-lg border border-red-400/20 bg-red-500/8 px-4 py-3 text-sm text-red-100">{error}</div> : null}

        <div className="grid min-w-0 gap-3" aria-busy={loading}>
          {(activityPage?.items || []).map((item) => (
            <AippTaskActivityCard key={item.task_id} item={item} apiFetch={apiFetch} t={t} lang={lang} />
          ))}
          {!loading && activityPage?.items.length === 0 ? (
            <div className="theme-panel-soft flex min-h-40 flex-col items-center justify-center gap-2 p-5 text-center text-sm text-white/55">
              <Download className="h-7 w-7" />
              <span>{t("还没有符合条件的媒体处理记录。", "No media processing records match these filters.")}</span>
            </div>
          ) : null}
        </div>

        <div className="flex items-center justify-between gap-3">
          <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={cursorHistory.length === 0 || loading} onClick={openPrevious}>{t("上一页", "Previous")}</button>
          {loading ? <LoaderCircle className="h-5 w-5 animate-spin text-white/45" /> : <span className="text-xs text-white/40">{activityPage?.items.length || 0}</span>}
          <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={activityPage?.next_cursor_sequence == null || loading} onClick={openNext}>{t("下一页", "Next")}</button>
        </div>
      </> : null}

      {selectedApp.renderer !== "collection_feed_v1" ? null : <>

      <section className="theme-panel-soft p-3 sm:p-4">
        <div className="grid gap-2 sm:grid-cols-3 sm:gap-3">
          <div><p className="text-xs text-white/45">{t("当前状态", "Current status")}</p><p className="mt-1 text-sm font-medium text-white/85">{page?.active_run ? t("正在采集", "Collecting") : t("空闲", "Idle")}</p></div>
          <div><p className="text-xs text-white/45">{t("当前结果", "Current results")}</p><p className="mt-1 text-sm font-medium text-white/85">{page?.matching_total ?? 0}</p></div>
          <div className="min-w-0"><p className="text-xs text-white/45">{t("数据更新时间", "Data updated")}</p><p className="mt-1 truncate text-xs font-medium text-white/85" title={formatCollectedAt(page?.updated_at || null, lang)}>{formatCollectedAt(page?.updated_at || null, lang)}</p></div>
        </div>
      </section>

      <div className="grid min-w-0 gap-2 lg:grid-cols-[minmax(240px,1fr)_auto_auto_auto] lg:items-center">
        <label className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-white/40" />
          <input
            className="theme-input w-full py-2 pl-9 pr-3 text-sm"
            value={searchDraft}
            onChange={(event) => setSearchDraft(event.target.value)}
            placeholder={t("搜索标题或帖子文案", "Search titles or post captions")}
            maxLength={200}
          />
        </label>
        <div className="flex min-w-0 gap-2 overflow-x-auto pb-0.5 lg:pb-0">
          {(["all", "video", "image"] as const).map((value) => (
            <button key={value} type="button" className={kind === value ? "theme-accent-btn shrink-0 px-3 py-2 text-xs" : "theme-secondary-btn shrink-0 px-3 py-2 text-xs"} onClick={() => setKind(value)}>
              {value === "all" ? t("全部", "All") : value === "video" ? t("视频", "Videos") : t("图片", "Images")}
            </button>
          ))}
        </div>
        {platforms.length > 1 ? (
          <select aria-label={t("按平台筛选", "Filter by platform")} className="theme-input w-full min-w-0 py-2 text-sm lg:w-auto lg:min-w-36" value={platform} onChange={(event) => setPlatform(event.target.value)}>
            <option value="all">{t("全部平台", "All platforms")}</option>
            {platforms.map((name) => <option key={name} value={name}>{name}</option>)}
          </select>
        ) : null}
        <select
          className="theme-input w-full min-w-0 py-2 text-sm lg:w-auto lg:min-w-44"
          value={sortOrder}
          onChange={(event) => setSortOrder(event.target.value as "newest" | "oldest")}
          aria-label={t("按采集时间排序", "Sort by collection time")}
        >
          <option value="newest">{t("采集时间：最新优先", "Collected: newest first")}</option>
          <option value="oldest">{t("采集时间：最早优先", "Collected: oldest first")}</option>
        </select>
      </div>

      {error ? <div className="rounded-lg border border-red-400/20 bg-red-500/8 px-4 py-3 text-sm text-red-100">{error}</div> : null}

      <div className="grid min-w-0 gap-2 md:grid-cols-2 xl:grid-cols-3" aria-busy={loading}>
        {groupCollectionItems(page?.items || []).map((images) => (
          <AippMediaItemCard key={collectionGroupKey(images[0])} item={images[0]} images={images} skillName={selectedSkill} apiFetch={apiFetch} t={t} lang={lang} />
        ))}
        {!loading && page?.items.length === 0 ? (
          <div className="theme-panel-soft col-span-full flex min-h-40 flex-col items-center justify-center gap-2 p-5 text-center text-sm text-white/55">
            <GalleryVerticalEnd className="h-7 w-7" />
            <span>{t("还没有符合条件的采集内容。", "No collected content matches these filters.")}</span>
          </div>
        ) : null}
      </div>

      <div className="flex items-center justify-between gap-3">
        <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={cursorHistory.length === 0 || loading} onClick={openPrevious}>{t("上一页", "Previous")}</button>
        {loading ? <LoaderCircle className="h-5 w-5 animate-spin text-white/45" /> : <span className="text-xs text-white/40">{t(`第 ${cursorHistory.length + 1} 页 · ${groupCollectionItems(page?.items || []).length} 篇`, `Page ${cursorHistory.length + 1} · ${groupCollectionItems(page?.items || []).length} posts`)}</span>}
        <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={page?.next_cursor_sequence == null || loading} onClick={openNext}>{t("下一页", "Next")}</button>
      </div>
      </>}
    </section>
  );
}
