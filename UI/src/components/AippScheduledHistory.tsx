import { useEffect, useState } from "react";

type Translate = (zh: string, en: string) => string;
interface Run {
  run_id: string;
  job_id: string;
  task_status: string;
  started_at: string;
  result_summary: { error_code?: string | null; notification?: { delivery_status?: string } };
}
interface Page { items: Run[]; next_cursor: number | null }

const statusNames: Record<string, [string, string]> = {
  queued: ["排队中", "Queued"], running: ["执行中", "Running"], succeeded: ["成功", "Succeeded"],
  failed: ["失败", "Failed"], timeout: ["超时", "Timed out"], canceled: ["已取消", "Cancelled"],
  cancelled: ["已取消", "Cancelled"], waiting: ["等待中", "Waiting"], background: ["后台运行", "In background"],
  needs_user: ["等待用户", "Needs input"], delivered: ["已送达", "Delivered"], read: ["已读", "Read"],
  accepted: ["已接收", "Accepted"], query_required: ["待核对", "Needs confirmation"],
};

export function AippScheduledHistory({ skill, apiFetch, t }: {
  skill: string; apiFetch: (path: string, init?: RequestInit) => Promise<Response>; t: Translate;
}) {
  const [cursors, setCursors] = useState<number[]>([]);
  const [status, setStatus] = useState("");
  const [page, setPage] = useState<Page | null>(null);
  const [error, setError] = useState(false);
  const [revision, setRevision] = useState(0);
  const cursor = cursors.at(-1);
  useEffect(() => {
    const controller = new AbortController();
    setPage(null);
    setError(false);
    const params = new URLSearchParams({ limit: "30" });
    if (cursor !== undefined) params.set("cursor_sequence", String(cursor));
    if (status) params.set("status", status);
    void (async () => {
      const response = await apiFetch(`/v1/aipps/${encodeURIComponent(skill)}/items?${params}`, { signal: controller.signal });
      const result = await response.json();
      if (!response.ok || !result.ok || !Array.isArray(result.data?.items)) throw new Error("history_read_failed");
      if (!controller.signal.aborted) setPage(result.data);
    })().catch(() => { if (!controller.signal.aborted) setError(true); });
    return () => controller.abort();
  }, [skill, cursor, status, revision, apiFetch]);
  const label = (value: string) => statusNames[value] ? t(...statusNames[value]) : value;
  return <section className="theme-panel space-y-3 p-4">
    <div className="flex flex-wrap items-center gap-3">
      <label>{t("执行状态", "Run status")} <select value={status} onChange={e => { setCursors([]); setStatus(e.target.value); }}>
        <option value="">{t("全部", "All")}</option>
        {["succeeded","failed","timeout","canceled","running","queued"].map(value => <option key={value} value={value}>{label(value)}</option>)}
      </select></label>
      <button className="theme-secondary-btn px-3 py-1" onClick={() => setRevision(x => x + 1)}>{t("刷新", "Refresh")}</button>
    </div>
    {error ? <p role="alert">{t("无法读取历史，请刷新重试。", "Could not load the history. Refresh to retry.")}</p>
      : !page ? <p role="status">{t("正在读取…", "Loading…")}</p>
      : page.items.length === 0 ? <p>{t("没有符合条件的执行记录。", "No matching runs.")}</p>
      : <ul className="divide-y">{page.items.map(run => {
        const reason = run.result_summary.error_code;
        const delivery = run.result_summary.notification?.delivery_status;
        return <li key={run.run_id} className="space-y-1 py-2">
          <div className="flex flex-wrap justify-between gap-2"><time>{new Date(Number(run.started_at) * 1000).toLocaleString()}</time>
            <strong>{label(run.task_status)}</strong></div>
          <div className="break-all text-sm">{t("计划", "Schedule")} · {run.job_id}</div>
          {reason && <div className="text-sm">{t("失败原因", "Failure reason")} · <code>{reason}</code></div>}
          {delivery && <div className="text-sm">{t("消息投递", "Message delivery")} · {label(delivery)}</div>}
        </li>;
      })}</ul>}
    <div className="flex gap-2">
      <button className="theme-secondary-btn px-3 py-1" disabled={!cursors.length} onClick={() => setCursors(values => values.slice(0,-1))}>{t("上一页", "Previous")}</button>
      <button className="theme-secondary-btn px-3 py-1" disabled={page?.next_cursor == null} onClick={() => {
        if (page?.next_cursor != null) setCursors(values => [...values, page.next_cursor!]);
      }}>{t("下一页", "Next")}</button>
    </div>
  </section>;
}
