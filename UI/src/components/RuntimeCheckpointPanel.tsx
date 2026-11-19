import { useEffect, useMemo, useState } from "react";
import { useRequestOwner } from "../hooks/useRequestOwner";
import { readJsonApiResponse } from "../lib/api-response";
import { formatBytes } from "../lib/display-format";
import { formatUnixDateTime } from "../lib/date-format";
import type { ApiResponse } from "../types/api";
type Report = { status: string; created_at_ts?: number; bytes?: number; database_count?: number };
export function RuntimeCheckpointPanel({ t, apiFetch, requestScope }: { t: (zh: string, en: string) => string; apiFetch: (path: string, init?: RequestInit) => Promise<Response>; requestScope: string }) {
  const owns = useRequestOwner(requestScope);
  const empty = useMemo(() => ({ owner: owns, report: null as Report | null, busy: false }), [owns]);
  const flight = useMemo(() => ({ busy: false }), [owns]);
  const [stored, setStored] = useState(empty);
  const view = stored.owner === owns ? stored : empty;
  async function refresh() {
    if (!owns() || flight.busy) return;
    flight.busy = true;
    setStored({ ...empty, busy: true });
    try {
      const response = await apiFetch("/v1/system/checkpoint-status");
      const body = await readJsonApiResponse<ApiResponse<Report>>(response);
      if (owns()) setStored({ ...empty, report: response.ok && body.ok ? body.data ?? null : null });
    } catch { if (owns()) setStored(empty); }
    finally { flight.busy = false; }
  }
  useEffect(() => { void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owns]);
  const ready = view.report?.status === "verified";
  return <section className="theme-panel-soft p-4 sm:p-5">
    <div className="flex items-center justify-between gap-3"><h3 className="font-semibold">{t("自动备份", "Automatic backups")}</h3><button className="theme-secondary-btn px-3 py-2 text-sm" disabled={view.busy || !requestScope} onClick={() => void refresh()}>{t("刷新状态", "Refresh status")}</button></div>
    <p className="mt-2 text-sm">{ready ? t("最近一次备份已完成并通过校验。", "The latest checkpoint completed and passed verification.") : view.report?.status === "failed" ? t("最近一次备份未完成，请检查目标磁盘空间和服务权限。原备份会保留。", "The latest checkpoint failed. Check destination space and service permissions. The previous checkpoint is retained.") : t("暂未取得已验证的备份，请检查备份计划。", "No verified checkpoint is available. Check the backup schedule.")}</p>
    {ready && <p className="mt-2 text-xs opacity-65">{formatUnixDateTime(view.report?.created_at_ts, t("zh-CN", "en-US"))} · {formatBytes(view.report?.bytes ?? 0)}</p>}
    <p className="mt-2 text-xs opacity-60">{t("备份任务数据库、技能数据库、配置、运行程序与技能授权状态。任务附件、模型和技能包文件需要独立保留；恢复先写入新目录并验证。", "Checkpoints include runtime and skill databases, configuration, programs and skill authorization. Keep attachments, models and skill package files separately. Restore writes to a new directory and verifies it first.")}</p>
  </section>;
}
