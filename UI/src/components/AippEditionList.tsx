import { useEffect, useRef, useState } from "react";

type Translate = (zh: string, en: string) => string;
type Trigger = "manual" | "scheduled";
interface Source {
  title: string | null; url: string; source: string | null; date: string | null; excerpt: string | null;
}
interface Edition {
  task_id: string; created_at: string; trigger: Trigger; channel: string; status: string;
  request_title: string | null; delivery_status: string | null; error_code: string | null; text: string | null; sources: Source[];
}
interface Page { items: Edition[]; next_cursor: number | null }

const labels: Record<string, [string, string]> = {
  scheduled: ["定时", "Scheduled"], manual: ["手动", "Manual"], wechat: ["微信", "WeChat"], ui: ["控制台", "Console"],
  succeeded: ["已完成", "Completed"], failed: ["失败", "Failed"], timeout: ["超时", "Timed out"],
  canceled: ["已取消", "Cancelled"], running: ["处理中", "Running"], queued: ["排队中", "Queued"],
  accepted: ["已接收", "Accepted"], delivered: ["已送达", "Delivered"], read: ["已读", "Read"],
  query_required: ["待核对", "Needs confirmation"],
};

export function AippEditionList({ skill, apiFetch, lang, t }: {
  skill: string; apiFetch: (path: string, init?: RequestInit) => Promise<Response>;
  lang: "zh" | "en"; t: Translate;
}) {
  const fetchRef = useRef(apiFetch);
  fetchRef.current = apiFetch;
  const [filter, setFilter] = useState<Trigger | "">("");
  const [cursors, setCursors] = useState<number[]>([]);
  const [revision, setRevision] = useState(0);
  const [result, setResult] = useState<{ key: string; page?: Page; error?: boolean } | null>(null);
  const cursor = cursors.at(-1);
  const key = JSON.stringify([skill, filter, cursor, revision]);
  const page = result?.key === key ? result.page : undefined;
  const failed = result?.key === key && result.error;
  const label = (value: string) => labels[value] ? t(...labels[value]) : value;

  useEffect(() => {
    const abort = new AbortController();
    const params = new URLSearchParams({ limit: "12" });
    if (filter) params.set("trigger", filter);
    if (cursor !== undefined) params.set("cursor_sequence", String(cursor));
    void (async () => {
      const response = await fetchRef.current(`/v1/aipps/${encodeURIComponent(skill)}/items?${params}`, { signal: abort.signal, cache: "no-store" });
      const body = await response.json();
      if (!response.ok || !body.ok || !Array.isArray(body.data?.items)) throw new Error("edition_read_failed");
      if (!abort.signal.aborted) setResult({ key, page: body.data });
    })().catch(() => { if (!abort.signal.aborted) setResult({ key, error: true }); });
    return () => abort.abort();
  }, [key, skill, filter, cursor]);

  return <section className="space-y-3">
    <div className="flex flex-wrap items-center gap-2">
      {(["", "scheduled", "manual"] as const).map(value =>
        <button type="button" key={value} aria-pressed={filter === value}
          className={`${filter === value ? "theme-accent-btn" : "theme-secondary-btn"} px-3 py-1.5 text-sm`}
          onClick={() => { setFilter(value); setCursors([]); }}>
          {value ? label(value) : t("全部", "All")}
        </button>)}
      <button type="button" className="theme-secondary-btn ml-auto px-3 py-1.5 text-sm" onClick={() => setRevision(v => v + 1)}>{t("刷新", "Refresh")}</button>
    </div>
    {failed ? <p role="alert">{t("简报读取失败，请点刷新重试。", "Could not load editions. Select Refresh to retry.")}</p>
      : !page ? <p role="status">{t("正在读取…", "Loading…")}</p>
      : !page.items.length ? <p>{t("暂无符合条件的简报。", "No matching editions yet.")}</p>
      : page.items.map((item, i) => {
        const date = new Date(Number(item.created_at) * 1000);
        const validDate = Number.isFinite(date.getTime());
        return <details key={item.task_id} open={i === 0} className="theme-panel p-4">
          <summary className="cursor-pointer leading-7">
            <strong>{label(item.trigger)}</strong>{" · "}<time dateTime={validDate ? date.toISOString() : undefined}>{validDate ? date.toLocaleString(lang === "zh" ? "zh-CN" : "en") : item.created_at}</time>
            {" · "}{label(item.channel)}{" · "}{label(item.status)}
          </summary>
          {item.trigger === "manual" && item.request_title && <p className="mt-3 text-sm font-semibold break-words">{item.request_title}</p>}
          {item.text && <p className="mt-3 whitespace-pre-wrap break-words leading-7">{item.text}</p>}
          {item.error_code && <p className="mt-2 text-sm">{t("执行原因", "Execution reason")} · {item.error_code}</p>}
          {(item.delivery_status || (item.trigger === "scheduled" && item.channel !== "ui")) && <p className="mt-2 text-sm">
            {t("消息投递", "Delivery")} · {item.delivery_status ? label(item.delivery_status) : t("未记录", "Unrecorded")}
          </p>}
          {!!item.sources.length && <details className="mt-3 border-t border-[var(--theme-border)] pt-2">
            <summary className="cursor-pointer text-sm">{t("抓取来源", "Fetched sources")} ({item.sources.length})</summary>
            <ul className="ml-4 list-disc space-y-3 py-2">{item.sources.map(source =>
              <li key={source.url} className="break-words">
                <a className="underline" href={source.url} target="_blank" rel="noopener noreferrer">{source.title || source.url}</a>
                <p className="text-sm">{[source.source, source.date].filter(Boolean).join(" · ")}</p>
                {source.excerpt && <p className="mt-1 text-sm">{source.excerpt}</p>}
              </li>)}</ul>
          </details>}
        </details>;
      })}
    {page && (cursors.length > 0 || page.next_cursor !== null) && <nav className="flex items-center gap-3" aria-label={t("简报分页", "Edition pages")}>
      <button className="theme-secondary-btn px-3 py-1" disabled={!cursors.length} onClick={() => setCursors(v => v.slice(0,-1))}>{t("上一页", "Previous")}</button>
      <span>{t(`第 ${cursors.length + 1} 页`, `Page ${cursors.length + 1}`)}</span>
      <button className="theme-secondary-btn px-3 py-1" disabled={page.next_cursor == null} onClick={() => { if (page.next_cursor != null) setCursors(v => [...v, page.next_cursor!]); }}>{t("下一页", "Next")}</button>
    </nav>}
  </section>;
}
