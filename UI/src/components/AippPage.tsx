import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Bot,
  ExternalLink,
  GalleryVerticalEnd,
  Image as ImageIcon,
  LoaderCircle,
  RefreshCw,
  Search,
  Video,
} from "lucide-react";

import { formatUiError } from "../lib/ui-error";
import type {
  AippCatalogItem,
  AippCatalogResponse,
  AippMediaItem,
  AippMediaPageResponse,
  ApiResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

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

function formatCollectedAt(value: string | null, lang: "zh" | "en"): string {
  if (!value) return "-";
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return value;
  return new Intl.DateTimeFormat(lang === "zh" ? "zh-CN" : "en", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(timestamp));
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
  const apiFetchRef = useRef(apiFetch);
  apiFetchRef.current = apiFetch;

  useEffect(() => {
    if (item.kind !== "video" || !item.preview_available) return;
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
  }, [item.global_sequence, item.kind, item.preview_available, skillName]);

  const source = item.kind === "image" ? item.image_url : previewUrl;
  if (source) {
    return (
      <img
        src={source}
        alt=""
        loading="lazy"
        referrerPolicy="no-referrer"
        className="h-full w-full object-contain"
      />
    );
  }
  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-2 text-white/45">
      {item.kind === "video" ? <Video className="h-8 w-8" /> : <ImageIcon className="h-8 w-8" />}
      <span className="text-xs">{t("暂无预览", "Preview unavailable")}</span>
    </div>
  );
}

export function AippMediaItemCard({
  item,
  skillName,
  apiFetch,
  t,
  lang,
}: {
  item: AippMediaItem;
  skillName: string;
  apiFetch: ApiFetch;
  t: Translate;
  lang: "zh" | "en";
}) {
  return (
    <article className="theme-panel overflow-hidden">
      <div className="grid min-w-0 md:grid-cols-[minmax(240px,36%)_1fr]">
        <div className="aspect-video min-h-44 bg-black/20 md:aspect-auto md:min-h-64">
          <MediaPreview item={item} skillName={skillName} apiFetch={apiFetch} t={t} />
        </div>
        <div className="min-w-0 p-4 sm:p-5">
          <div className="flex flex-wrap items-center gap-2 text-xs text-white/45">
            <span className="inline-flex items-center gap-1">{item.kind === "video" ? <Video className="h-3.5 w-3.5" /> : <ImageIcon className="h-3.5 w-3.5" />}{item.kind === "video" ? t("视频", "Video") : t("图片", "Image")}</span>
            <span>{item.platform}</span>
            <span>#{item.global_sequence}</span>
            <span>{formatCollectedAt(item.discovered_at, lang)}</span>
          </div>
          <h2 className="mt-2 text-base font-semibold leading-6 text-white/90">{item.title || t("未提供标题", "Untitled")}</h2>
          {item.recognized_text ? <p className="mt-3 whitespace-pre-wrap text-sm leading-6 text-white/72">{item.recognized_text}</p> : null}
          {!item.recognized_text && item.platform_text ? <p className="mt-3 whitespace-pre-wrap text-sm leading-6 text-white/65">{item.platform_text}</p> : null}
          {item.source_url ? (
            <a className="theme-secondary-btn mt-4 w-fit px-3 py-2 text-xs" href={item.source_url} target="_blank" rel="noreferrer noopener">
              {t("查看来源", "Open source")}<ExternalLink className="h-3.5 w-3.5" />
            </a>
          ) : null}
        </div>
      </div>
    </article>
  );
}

export function AippPage({ lang, t, apiFetch, onOpenAgent, onOpenSkillStore }: AippPageProps) {
  const [catalog, setCatalog] = useState<AippCatalogItem[]>([]);
  const [selectedSkill, setSelectedSkill] = useState("");
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState<AippMediaPageResponse | null>(null);
  const [kind, setKind] = useState<"all" | "video" | "image">("all");
  const [platform, setPlatform] = useState("all");
  const [searchDraft, setSearchDraft] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [cursor, setCursor] = useState<number | null>(null);
  const [cursorHistory, setCursorHistory] = useState<Array<number | null>>([]);
  const requestSequence = useRef(0);
  const apiFetchRef = useRef(apiFetch);
  const translateRef = useRef(t);
  apiFetchRef.current = apiFetch;
  translateRef.current = t;

  const fetchCatalog = useCallback(async () => {
    setCatalogLoading(true);
    setError(null);
    try {
      const response = await apiFetchRef.current("/v1/aipps");
      const body = (await response.json()) as ApiResponse<AippCatalogResponse>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `aipp_catalog_http_${response.status}`);
      }
      setCatalog(body.data.apps);
      setSelectedSkill((current) =>
        body.data?.apps.some((app) => app.skill_name === current)
          ? current
          : body.data?.apps[0]?.skill_name || "",
      );
    } catch (cause) {
      setError(formatUiError(cause, translateRef.current, "AiPP 列表读取失败。", "Could not load the AiPP catalog."));
    } finally {
      setCatalogLoading(false);
    }
  }, []);

  const fetchPage = useCallback(async () => {
    if (!selectedSkill) {
      setPage(null);
      return;
    }
    const currentRequest = ++requestSequence.current;
    setLoading(true);
    setError(null);
    const params = new URLSearchParams({ limit: "20" });
    if (cursor != null) params.set("before_sequence", String(cursor));
    if (kind !== "all") params.set("kind", kind);
    if (platform !== "all") params.set("platform", platform);
    if (searchQuery) params.set("query", searchQuery);
    try {
      const response = await apiFetchRef.current(
        `/v1/aipps/${encodeURIComponent(selectedSkill)}/items?${params.toString()}`,
      );
      const body = (await response.json()) as ApiResponse<AippMediaPageResponse>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `aipp_items_http_${response.status}`);
      }
      if (currentRequest === requestSequence.current) setPage(body.data);
    } catch (cause) {
      if (currentRequest === requestSequence.current) {
        setError(formatUiError(cause, translateRef.current, "采集内容读取失败。", "Could not load collected content."));
      }
    } finally {
      if (currentRequest === requestSequence.current) setLoading(false);
    }
  }, [cursor, kind, platform, searchQuery, selectedSkill]);

  useEffect(() => {
    void fetchCatalog();
  }, [fetchCatalog]);

  useEffect(() => {
    const timer = window.setTimeout(() => setSearchQuery(searchDraft.trim()), 300);
    return () => window.clearTimeout(timer);
  }, [searchDraft]);

  useEffect(() => {
    setCursor(null);
    setCursorHistory([]);
  }, [selectedSkill, kind, platform, searchQuery]);

  useEffect(() => {
    void fetchPage();
  }, [fetchPage]);

  const selectedApp = catalog.find((app) => app.skill_name === selectedSkill) || null;
  const platforms = useMemo(
    () => Object.keys(page?.platform_states || {}).sort(),
    [page?.platform_states],
  );

  const openNext = () => {
    const next = page?.next_before_sequence;
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
            <h1 className="text-lg font-semibold text-white">AiPP</h1>
            <p className={`mt-2 text-sm leading-6 ${error ? "text-red-200" : "text-white/60"}`}>
              {error || t("当前没有已启用技能提供 AiPP。", "No enabled skill currently provides an AiPP.")}
            </p>
            <button type="button" className="theme-secondary-btn mt-4 px-3 py-2 text-sm" onClick={onOpenSkillStore}>
              {t("打开 Skill Store", "Open Skill Store")}
            </button>
          </div>
        </div>
      </section>
    );
  }

  return (
    <section className="space-y-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div className="min-w-0">
          <p className="text-xs font-medium text-white/45">AiPP</p>
          <h1 className="mt-1 text-xl font-semibold text-white">
            {selectedApp ? localizedAippCopy(selectedApp.titles, lang, selectedApp.default_locale) : "-"}
          </h1>
          {selectedApp ? (
            <p className="mt-1 max-w-3xl text-sm leading-6 text-white/58">
              {localizedAippCopy(selectedApp.descriptions, lang, selectedApp.default_locale)}
            </p>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" onClick={onOpenAgent}>
            <Bot className="h-4 w-4" />
            Agent
          </button>
          <button type="button" className="theme-icon-btn h-9 w-9" onClick={() => void fetchPage()} title={t("刷新内容", "Refresh content")}>
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          </button>
        </div>
      </header>

      {catalog.length > 1 ? (
        <div className="flex gap-2 overflow-x-auto pb-1" role="tablist" aria-label="AiPP">
          {catalog.map((app) => (
            <button
              key={app.skill_name}
              type="button"
              role="tab"
              aria-selected={selectedSkill === app.skill_name}
              className={selectedSkill === app.skill_name ? "theme-accent-btn shrink-0 px-3 py-2 text-sm" : "theme-secondary-btn shrink-0 px-3 py-2 text-sm"}
              onClick={() => setSelectedSkill(app.skill_name)}
            >
              {localizedAippCopy(app.titles, lang, app.default_locale)}
            </button>
          ))}
        </div>
      ) : null}

      <section className="theme-panel-soft p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-3">
          <div><p className="text-xs text-white/45">{t("当前状态", "Current status")}</p><p className="mt-1 text-sm font-medium text-white/85">{page?.active_run ? t("正在采集", "Collecting") : t("空闲", "Idle")}</p></div>
          <div><p className="text-xs text-white/45">{t("当前结果", "Current results")}</p><p className="mt-1 text-sm font-medium text-white/85">{page?.matching_total ?? 0}</p></div>
          <div><p className="text-xs text-white/45">{t("数据更新时间", "Data updated")}</p><p className="mt-1 text-sm font-medium text-white/85">{formatCollectedAt(page?.updated_at || null, lang)}</p></div>
        </div>
      </section>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <label className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-white/40" />
          <input
            className="theme-input w-full py-2 pl-9 pr-3 text-sm"
            value={searchDraft}
            onChange={(event) => setSearchDraft(event.target.value)}
            placeholder={t("搜索标题或识别文字", "Search titles or recognized text")}
            maxLength={200}
          />
        </label>
        <div className="flex gap-2 overflow-x-auto">
          {(["all", "video", "image"] as const).map((value) => (
            <button key={value} type="button" className={kind === value ? "theme-accent-btn shrink-0 px-3 py-2 text-xs" : "theme-secondary-btn shrink-0 px-3 py-2 text-xs"} onClick={() => setKind(value)}>
              {value === "all" ? t("全部", "All") : value === "video" ? t("视频", "Videos") : t("图片", "Images")}
            </button>
          ))}
        </div>
        {platforms.length > 1 ? (
          <select className="theme-input min-w-36 py-2 text-sm" value={platform} onChange={(event) => setPlatform(event.target.value)}>
            <option value="all">{t("全部平台", "All platforms")}</option>
            {platforms.map((name) => <option key={name} value={name}>{name}</option>)}
          </select>
        ) : null}
      </div>

      {error ? <div className="rounded-lg border border-red-400/20 bg-red-500/8 px-4 py-3 text-sm text-red-100">{error}</div> : null}

      <div className="space-y-3" aria-busy={loading}>
        {(page?.items || []).map((item) => (
          <AippMediaItemCard key={item.global_sequence} item={item} skillName={selectedSkill} apiFetch={apiFetch} t={t} lang={lang} />
        ))}
        {!loading && page?.items.length === 0 ? (
          <div className="theme-panel-soft flex min-h-40 flex-col items-center justify-center gap-2 p-5 text-center text-sm text-white/55">
            <GalleryVerticalEnd className="h-7 w-7" />
            <span>{t("还没有符合条件的采集内容。", "No collected content matches these filters.")}</span>
          </div>
        ) : null}
      </div>

      <div className="flex items-center justify-between gap-3">
        <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={cursorHistory.length === 0 || loading} onClick={openPrevious}>{t("上一页", "Previous")}</button>
        {loading ? <LoaderCircle className="h-5 w-5 animate-spin text-white/45" /> : <span className="text-xs text-white/40">{page?.items.length || 0}</span>}
        <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={page?.next_before_sequence == null || loading} onClick={openNext}>{t("下一页", "Next")}</button>
      </div>
    </section>
  );
}
