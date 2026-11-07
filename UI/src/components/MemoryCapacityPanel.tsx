import { RefreshCw } from "lucide-react";
import { useMemoryCapacity } from "../hooks/useMemoryCapacity";
import { capacityLabel, manualGrowthLabel, measurementLabel } from "../lib/memory-capacity-display";
import { formatUnixDateTime } from "../lib/date-format";
import type { MemorySettingsResult } from "../types/api";

interface Props {
  t: (zh: string, en: string) => string;
  lang: "zh" | "en";
  requestScope: string;
  settings: MemorySettingsResult | null;
  scope: "principal" | "conversation";
  conversationId: string | null;
  apiFetch: (path: string, init?: RequestInit) => Promise<Response>;
}

export function MemoryCapacityPanel({ t, lang, ...options }: Props) {
  const { data, loading, failed, selectable, refresh } = useMemoryCapacity(options);
  const locale = lang === "zh" ? "zh-CN" : "en-US";
  const count = (value: string) => BigInt(value).toLocaleString(locale);
  const bytes = (value: Parameters<typeof measurementLabel>[0]) => measurementLabel(value, locale, t);
  const quota = data?.quota.counters;
  const status = loading ? t("正在读取容量状态…", "Reading capacity…")
    : failed ? t("容量状态暂不可用，请刷新；其他记忆功能不受本次读取失败影响。", "Capacity is unavailable. Refresh to retry; this read failure does not change other memory features.")
      : !data ? t("请先加载当前账号或会话的记忆设置。", "Load the current account or conversation settings first.")
        : data.capacity_decision === "available" && !data.pressure.observed_at_ts
          ? t("存储状态尚未取得有效采样，不能据此确认就绪。", "Storage has no dated observation; readiness is not confirmed.")
          : capacityLabel(data.capacity_decision, t);
  return <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h4 className="text-sm font-semibold text-white">{t("容量与状态", "Capacity and status")}</h4>
        <p className="mt-2 text-sm text-white/70" role="status">{status}</p>
        <p className="mt-1 text-sm text-white/70" data-memory-growth-status>{loading
          ? t("正在读取新增内容状态…", "Reading new-content status…")
          : manualGrowthLabel(failed ? undefined : data?.manual_growth?.decision, t)}</p>
        <p className="mt-1 text-xs leading-5 text-white/55">{t(
          "自动生成、新增内容和账号配额分别检查；这些状态不授予权限，也不代表模型或网络就绪。本页不禁用删除、导出；刷新只读，不修复配置。",
          "Automatic generation, new-content storage and account quotas are separate. These observations grant no permission and do not confirm model or network readiness. This panel does not disable deletion or export; refresh is read-only and does not repair configuration.",
        )}</p>
      </div>
      <button type="button" onClick={() => void refresh()} disabled={loading || !selectable}
        className="theme-secondary-btn px-3 py-2 text-xs disabled:opacity-50">
        <RefreshCw className={`h-3.5 w-3.5${loading ? " animate-spin" : ""}`} />{t("刷新容量", "Refresh capacity")}
      </button>
    </div>
    {data ? <>
      <p className="mt-3 text-xs text-white/65">{t("当前策略：使用记忆", "Current policy: use memory")} {data.settings.use_memory ? t("允许", "allowed") : t("关闭", "off")}
        {" · "}{t("生成记忆", "generate memory")} {data.settings.generate_memory ? t("允许", "allowed") : t("关闭", "off")}</p>
      <details className="mt-3 text-xs text-white/60">
        <summary className="cursor-pointer text-white/75">{t("查看用量和统计范围", "View usage and measurement scopes")}</summary>
        <dl className="mt-3 grid gap-x-6 gap-y-2 sm:grid-cols-2">
          <div><dt>{t("原文配额记录：已用 / 上限", "Recorded source quota: used / limit")}</dt>
            <dd className="mt-1">{quota ? `${count(quota.used_rows)} / ${count(quota.max_rows)}` : t("暂不可用", "Unavailable")}</dd></div>
          <div><dt>{t("原文字节配额记录：已用 / 上限", "Recorded source bytes: used / limit")}</dt>
            <dd className="mt-1">{quota ? `${count(quota.used_bytes)} / ${count(quota.max_bytes)} B` : t("暂不可用", "Unavailable")}</dd></div>
          <div><dt>{t("当前账号原文实际字节", "Current account source payload bytes")}</dt>
            <dd className="mt-1">{bytes(data.measurements.own_source_transcript_payload_bytes)}</dd></div>
          <div><dt>{t("后台逻辑工作量：已用和待结算 / 上限", "Logical background work: spent and pending / limit")}</dt>
            <dd className="mt-1">{quota ? `${count(quota.spent_and_pending_logical_microunits)} / ${count(quota.max_logical_microunits)}` : t("暂不可用", "Unavailable")}</dd></div>
          {([
            [t("整个主数据库已分配页", "All allocated main database pages"), data.measurements.main_sqlite_allocated_page_bytes],
            [t("记忆域表和索引占用", "Memory-domain tables and indexes"), data.measurements.memory_domain_assigned_btree_bytes],
            [t("数据库内部空闲页", "Free pages inside SQLite"), data.measurements.sqlite_freelist_page_bytes],
            [t("数据库所在文件系统可用空间", "Available space on the database filesystem"), data.measurements.database_path_filesystem_available_bytes],
            [t("后台上次观察的数据库大小", "Database size at the last pressure observation"), data.pressure.observed_bytes],
            [t("数据库软阈值（不是用户配额）", "Database soft threshold, not an account quota"), data.pressure.configured_soft_limit_bytes],
          ] as const).filter(([, metric]) => metric.status !== "admin_only").map(([label, metric]) =>
            <div key={label}><dt>{label}</dt><dd className="mt-1">{bytes(metric)}</dd></div>)}
          {data.manual_growth && data.manual_growth.configured_limit_bytes.status !== "admin_only" ? <div>
            <dt>{t("新增内容的实际存储预算", "Effective storage budget for new content")}</dt>
            <dd className="mt-1">{bytes(data.manual_growth.configured_limit_bytes)}</dd>
          </div> : null}
        </dl>
        <p className="mt-3 leading-5">{t(
          "原文配额不包含事实、偏好和向量。后台逻辑工作量不是 token 或费用。全库大小不等于记忆占用，数据库空闲页也不等于磁盘可用空间；记忆域页不含 WAL、SHM 和其他文件。",
          "Source quotas exclude facts, preferences and vectors. Logical work is not tokens or billing. Whole-database size is not memory usage; free SQLite pages are not free disk space. Memory-domain pages exclude WAL, SHM and other files.",
        )}</p>
        <p className="mt-2">{t("存储观察时间", "Pressure observation time")}: {data.pressure.observed_at_ts
          ? formatUnixDateTime(data.pressure.observed_at_ts, locale) : t("尚无有效采样", "No dated observation")}</p>
      </details>
    </> : null}
  </div>;
}
