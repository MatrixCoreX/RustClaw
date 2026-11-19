import { useEffect, useMemo, useState } from "react";
import { readJsonApiResponse } from "../lib/api-response";
import { useRequestOwner } from "../hooks/useRequestOwner";
import { useUiDialog } from "./UiDialogProvider";
import type { ApiResponse } from "../types/api";

type T = (zh: string, en: string) => string;
type Job = { job_id: string; job_kind: string; attempt: number; error_code: string | null; archived: boolean; review_token: string };
type Report = { jobs: Job[]; total_failed: number; limit: number };
function reason(code: string | null, t: T) {
  if (code === "memory_job_source_binding_unverified") return t("原始对话证据不足，暂时无法恢复。", "Original conversation evidence is insufficient for recovery.");
  if (code === "memory_job_source_binding_mismatch" || code === "memory_job_source_digest_mismatch") return t("原始记录与任务不一致，需要保留核查。", "Original records do not match this job and need review.");
  if (code === "memory_job_generation_revoked") return t("记忆生成授权已关闭。", "Memory generation consent was withdrawn.");
  return t("任务未完成。可核验恢复条件，或归档保留记录。", "The job did not complete. Check recovery eligibility or archive its record.");
}
export function MemoryJobReviewPanel({ t, apiFetch, requestScope }: { t: T; apiFetch: (path: string, init?: RequestInit) => Promise<Response>; requestScope: string }) {
  const owns = useRequestOwner(requestScope);
  const flight = useMemo(() => ({ busy: false }), [owns]);
  const empty = useMemo(() => ({ owner: owns, report: null as Report | null, busy: false, error: false, message: "" }), [owns]);
  const [stored, setStored] = useState(empty);
  const view = stored.owner === owns ? stored : empty;
  const { confirm } = useUiDialog();
  const [showArchived, setShowArchived] = useState(false);
  function update(value: Partial<typeof empty>) { if (owns()) setStored(old => ({ ...(old.owner === owns ? old : empty), ...value })); }
  async function read() {
    const response = await apiFetch("/v1/memory/jobs/review");
    const body = await readJsonApiResponse<ApiResponse<Report>>(response);
    if (!response.ok || !body.ok || !body.data) throw new Error("memory_job_review_unavailable");
    return body.data;
  }
  async function refresh() {
    if (!owns() || flight.busy) return;
    flight.busy = true; update({ busy: true, error: false });
    try { const report = await read(); update({ report }); }
    catch { update({ report: null, error: true }); }
    finally { flight.busy = false; update({ busy: false }); }
  }
  useEffect(() => { void refresh(); /* An owner change starts a distinct request epoch. */
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owns]);
  async function act(job: Job, action: "archive" | "unarchive" | "retry") {
    if (!owns() || flight.busy) return;
    const accepted = await confirm({ title: t("处理历史记忆任务", "Review a historical memory job"),
      message: action === "retry" ? t("只有来源、授权和模型配置都通过核验才会重新排队。无法核验的任务会继续保留。", "The job is queued only after source, consent and model checks pass. Unverified jobs remain blocked.") : t("归档只整理失败记录，不会删除记忆或原始证据；之后可以恢复显示。", "Archiving organizes failed records without deleting memories or evidence. You can show them again later."),
      confirmLabel: t("确认", "Confirm"), cancelLabel: t("取消", "Cancel") });
    if (!accepted || !owns() || flight.busy) return;
    flight.busy = true; update({ busy: true, error: false, message: "" });
    try {
      const response = await apiFetch(`/v1/memory/jobs/${encodeURIComponent(job.job_id)}/disposition`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ action, review_token: job.review_token }) });
      const body = await readJsonApiResponse<ApiResponse<{ requeued_jobs?: number }>>(response);
      if (!owns()) return;
      if (!response.ok || !body.ok) throw new Error("memory_job_review_conflict");
      update({ message: action === "retry" ? body.data?.requeued_jobs ? t("核验通过，任务已重新排队。", "Checks passed; the job was queued again.") : t("尚未满足恢复条件，任务保持阻止状态。", "Recovery requirements are not met; the job remains blocked.") : t("记录已更新。", "Record updated.") });
      const report = await read(); update({ report });
    } catch { update({ report: null, error: true }); }
    finally { flight.busy = false; update({ busy: false }); }
  }
  return <section className="theme-panel-soft p-4 sm:p-5">
    <div className="flex flex-wrap items-center justify-between gap-3"><h3 className="font-semibold">{t("记忆后台任务", "Memory background jobs")}</h3><button className="theme-secondary-btn px-3 py-2 text-sm" disabled={view.busy || !requestScope} onClick={() => void refresh()}>{t("刷新", "Refresh")}</button></div>
    <p className="mt-2 text-sm opacity-65">{t("查看未完成的记忆生成任务。归档会保留所有证据，恢复前会重新核验来源和授权。", "Review unsuccessful memory generation jobs. Archives retain evidence; recovery rechecks sources and consent.")}</p>
    {view.error && <p role="alert" className="mt-3 text-sm">{t("记录暂不可用或已发生变化，请刷新后重试。", "Records are unavailable or have changed. Refresh and try again.")}</p>}
    {view.report && <><label className="mt-3 flex items-center gap-2 text-sm"><input type="checkbox" checked={showArchived} onChange={event => setShowArchived(event.target.checked)} />{t("显示已归档记录", "Show archived records")}</label>
      <p className="mt-2 text-xs opacity-60">{t(`共 ${view.report.total_failed} 条失败记录，展示最近 ${view.report.limit} 条以内。`, `${view.report.total_failed} failed records; showing up to the latest ${view.report.limit}.`)}</p>
      {view.report.jobs.filter(job => showArchived || !job.archived).map(job => <div key={job.job_id} className="mt-3 border-t border-current/10 pt-3 text-sm">
        <div className="font-medium">{job.job_kind === "consolidate" ? t("记忆整合", "Memory consolidation") : t("记忆提取", "Memory extraction")}{job.archived ? t(" · 已归档", " · Archived") : ""}</div>
        <p className="mt-1 opacity-65">{reason(job.error_code, t)}</p>
        <div className="mt-2 flex flex-wrap gap-2"><button disabled={view.busy} className="theme-secondary-btn px-3 py-2 text-xs" onClick={() => void act(job, job.archived ? "unarchive" : "archive")}>{job.archived ? t("恢复显示", "Show again") : t("归档记录", "Archive record")}</button>
          {!job.archived && <button disabled={view.busy} className="theme-secondary-btn px-3 py-2 text-xs" onClick={() => void act(job, "retry")}>{t("核验并恢复", "Check and recover")}</button>}</div>
        <details className="mt-2 text-xs opacity-60"><summary>{t("诊断信息", "Diagnostics")}</summary><p>{job.error_code ?? "unknown"} · {job.attempt}</p></details>
      </div>)}
      {view.report.total_failed === 0 && <p className="mt-3 text-sm">{t("目前没有失败的后台记忆任务。", "There are no failed memory background jobs.")}</p>}
    </>}
    {view.message && <p role="status" className="mt-3 text-sm">{view.message}</p>}
  </section>;
}
