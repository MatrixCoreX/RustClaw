import { Brain, Database, Loader2, RefreshCw, Trash2 } from "lucide-react";

import { formatUnixDateTime } from "../lib/date-format";
import {
  memoryFactStatusLabel,
  memorySafetyLabel,
  shouldHideMemoryRecentContent,
} from "../lib/memory-display";
import type {
  MemoryFactItem,
  MemoryOverviewResponse,
  MemoryPreferenceItem,
  MemoryRecentItem,
} from "../types/api";

type UiLanguage = "zh" | "en";
type ClearScope = "recent" | "preferences" | "facts" | "all";
type Translate = (zh: string, en: string) => string;

export interface MemoryPageProps {
  lang: UiLanguage;
  t: Translate;
  memoryLoading: boolean;
  memoryError: string | null;
  memoryMessage: string | null;
  memoryOverview: MemoryOverviewResponse | null;
  memoryPreferences: MemoryPreferenceItem[];
  memoryFacts: MemoryFactItem[];
  memoryRecent: MemoryRecentItem[];
  memoryActionLoading: string | null;
  memorySettingsSaving: boolean;
  memoryClearScope: ClearScope;
  onMemoryClearScopeChange: (scope: ClearScope) => void;
  onFetchMemoryData: () => void | Promise<void>;
  onDeleteMemoryItem: (id: string) => void | Promise<void>;
  onExpireMemoryItem: (id: string) => void | Promise<void>;
  onClearMemoryScope: () => void | Promise<void>;
  onUpdateMemoryLongTermEnabled: (enabled: boolean) => void | Promise<void>;
}

export function MemoryPage({
  lang,
  t,
  memoryLoading,
  memoryError,
  memoryMessage,
  memoryOverview,
  memoryPreferences,
  memoryFacts,
  memoryRecent,
  memoryActionLoading,
  memorySettingsSaving,
  memoryClearScope,
  onMemoryClearScopeChange,
  onFetchMemoryData,
  onDeleteMemoryItem,
  onExpireMemoryItem,
  onClearMemoryScope,
  onUpdateMemoryLongTermEnabled,
}: MemoryPageProps) {
  const dateLocale = lang === "zh" ? "zh-CN" : "en-US";
  const timeLabel = (ts: number | null | undefined) => formatUnixDateTime(ts, dateLocale);

  return (
    <section className="space-y-4">
      <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <p className="text-[10px] uppercase tracking-[0.28em] text-white/45">
              {t("记忆管理", "Memory Control")}
            </p>
            <h3 className="mt-2 text-base font-semibold text-white">
              {t("查看和管理 RustClaw 会用于回复的记忆。", "Review and manage the memory RustClaw can use in replies.")}
            </h3>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-white/60">
              {t(
                "这里展示当前账号与会话下的近期记录、偏好和长期事实卡片。删除或过期后，后续回复不会再主动使用这些内容。",
                "This page shows recent records, preferences, and long-term fact cards for the current account and chat. Deleted or expired items will not be actively used in future replies.",
              )}
            </p>
          </div>
          <button
            type="button"
            onClick={() => void onFetchMemoryData()}
            disabled={memoryLoading}
            className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
          >
            {memoryLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            {t("刷新", "Refresh")}
          </button>
        </div>

        {memoryError ? (
          <p className="mt-4 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
            {memoryError}
          </p>
        ) : null}
        {memoryMessage ? (
          <p className="mt-4 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
            {memoryMessage}
          </p>
        ) : null}

        <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
          {[
            { label: t("近期记录", "Recent"), value: memoryOverview?.counts.recent ?? 0 },
            { label: t("偏好", "Preferences"), value: memoryOverview?.counts.preferences ?? 0 },
            { label: t("有效事实", "Active facts"), value: memoryOverview?.counts.facts_active ?? 0 },
            { label: t("事实总数", "Total facts"), value: memoryOverview?.counts.facts_total ?? 0 },
            { label: t("长期摘要", "Summaries"), value: memoryOverview?.counts.long_term_summaries ?? 0 },
          ].map((item) => (
            <div key={item.label} className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
              <p className="text-[10px] uppercase tracking-widest text-white/45">{item.label}</p>
              <p className="mt-2 text-2xl font-semibold text-white">{item.value}</p>
            </div>
          ))}
        </div>
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(280px,360px)]">
        <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h4 className="text-sm font-semibold text-white">{t("偏好", "Preferences")}</h4>
              <p className="mt-1 text-xs leading-5 text-white/55">
                {t("偏好用于保持长期个人化设置，例如输出风格、默认路径或常用选择。", "Preferences keep long-lived personal settings, such as output style, default paths, or common choices.")}
              </p>
            </div>
            <span className="theme-meta-pill !rounded-xl !px-2.5 !py-1 text-[11px]">
              {memoryPreferences.length}
            </span>
          </div>
          <div className="mt-4 space-y-2">
            {memoryPreferences.map((item) => (
              <div key={item.id} className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-semibold text-white">{item.key}</p>
                    <p className="mt-1 break-words text-sm leading-6 text-white/70">{item.value}</p>
                    <p className="mt-2 text-[11px] text-white/40">
                      {t("来源", "Source")}: {item.source || "--"} · {t("置信度", "Confidence")}: {Math.round(item.confidence * 100)}% · {timeLabel(item.updated_at_ts)}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => void onDeleteMemoryItem(item.id)}
                    disabled={memoryActionLoading === `delete:${item.id}`}
                    className="inline-flex items-center gap-1 rounded-lg border border-red-500/25 bg-red-500/10 px-2 py-1 text-[11px] text-red-100 hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {memoryActionLoading === `delete:${item.id}` ? <Loader2 className="h-3 w-3 animate-spin" /> : <Trash2 className="h-3 w-3" />}
                    {t("删除", "Delete")}
                  </button>
                </div>
              </div>
            ))}
            {!memoryLoading && memoryPreferences.length === 0 ? (
              <p className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3 text-sm text-white/50">
                {t("当前没有偏好记忆。", "No preference memories yet.")}
              </p>
            ) : null}
          </div>
        </div>

        <div className="space-y-4">
          <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
            <h4 className="text-sm font-semibold text-white">{t("长期记忆开关", "Long-Term Memory")}</h4>
            <p className="mt-2 text-xs leading-5 text-white/55">
              {t("关闭后不再写入和使用长期记忆；保存配置后需要重启 RustClaw 才会生效。", "When off, long-term memory is no longer written or used. Restart RustClaw after saving for the change to take effect.")}
            </p>
            <div className="mt-4 flex flex-wrap items-center gap-2">
              <span
                className={
                  memoryOverview?.long_term_enabled
                    ? "inline-flex items-center gap-1 rounded-full border border-emerald-500/35 bg-emerald-500/12 px-2 py-1 text-[11px] font-medium text-emerald-200"
                    : "inline-flex items-center gap-1 rounded-full border border-amber-500/35 bg-amber-500/12 px-2 py-1 text-[11px] font-medium text-amber-200"
                }
              >
                <span className={memoryOverview?.long_term_enabled ? "h-1.5 w-1.5 rounded-full bg-emerald-300" : "h-1.5 w-1.5 rounded-full bg-amber-300"} />
                {memoryOverview?.long_term_enabled ? t("已开启", "On") : t("已关闭", "Off")}
              </span>
              <button
                type="button"
                onClick={() => void onUpdateMemoryLongTermEnabled(!(memoryOverview?.long_term_enabled ?? false))}
                disabled={memorySettingsSaving || !memoryOverview}
                className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
              >
                {memorySettingsSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Database className="h-3.5 w-3.5" />}
                {memoryOverview?.long_term_enabled ? t("关闭长期记忆", "Turn off") : t("开启长期记忆", "Turn on")}
              </button>
            </div>
          </div>

          <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
            <h4 className="text-sm font-semibold text-white">{t("批量清理", "Bulk Clear")}</h4>
            <p className="mt-2 text-xs leading-5 text-white/55">
              {t("只在确认记忆明显错误、过期或需要重置会话时使用。", "Use this only when memories are clearly wrong, outdated, or the chat needs a reset.")}
            </p>
            <div className="mt-4 grid gap-2">
              <select
                className="theme-input"
                value={memoryClearScope}
                onChange={(event) => onMemoryClearScopeChange(event.target.value as ClearScope)}
              >
                <option value="recent">{t("只清空近期记录", "Clear recent records only")}</option>
                <option value="preferences">{t("只清空偏好", "Clear preferences only")}</option>
                <option value="facts">{t("只清空事实卡片", "Clear fact cards only")}</option>
                <option value="all">{t("清空全部记忆", "Clear all memory")}</option>
              </select>
              <button
                type="button"
                onClick={() => void onClearMemoryScope()}
                disabled={Boolean(memoryActionLoading?.startsWith("clear:"))}
                className="inline-flex items-center justify-center gap-2 rounded-xl border border-red-500/25 bg-red-500/10 px-3 py-2 text-xs font-medium text-red-100 hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
              >
                {memoryActionLoading?.startsWith("clear:") ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                {t("执行清理", "Clear")}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h4 className="text-sm font-semibold text-white">{t("事实卡片", "Fact Cards")}</h4>
            <p className="mt-1 text-xs leading-5 text-white/55">
              {t("事实卡片是结构化长期记忆，适合保存稳定信息。可以把错误事实标记为过期或删除。", "Fact cards are structured long-term memories for stable information. Incorrect facts can be expired or deleted.")}
            </p>
          </div>
          <span className="theme-meta-pill !rounded-xl !px-2.5 !py-1 text-[11px]">
            {memoryFacts.length}
          </span>
        </div>
        <div className="mt-4 grid gap-3 lg:grid-cols-2">
          {memoryFacts.map((item) => {
            const isActive = item.status.toLowerCase() === "active";
            return (
              <div key={item.id} className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span
                        className={
                          isActive
                            ? "rounded-full border border-emerald-500/35 bg-emerald-500/12 px-2 py-0.5 text-[10px] text-emerald-200"
                            : "rounded-full border border-white/15 bg-white/5 px-2 py-0.5 text-[10px] text-white/55"
                        }
                      >
                        {memoryFactStatusLabel(item.status, lang)}
                      </span>
                      <span className="rounded-full border border-white/10 bg-white/5 px-2 py-0.5 text-[10px] text-white/45">
                        {item.namespace || "default"}
                      </span>
                      <span className="text-[10px] text-white/35">{Math.round(item.confidence * 100)}%</span>
                    </div>
                    <p className="mt-2 break-words text-sm leading-6 text-white/80">{item.fact_text || item.fact_value}</p>
                    <p className="mt-2 text-[11px] text-white/40">
                      {item.fact_key} · {t("更新", "Updated")}: {timeLabel(item.updated_at_ts)}
                      {item.expires_at_ts ? ` · ${t("过期", "Expires")}: ${timeLabel(item.expires_at_ts)}` : ""}
                    </p>
                    <details className="mt-2 text-[11px] text-white/45">
                      <summary className="cursor-pointer select-none text-white/55">{t("查看依据", "Show details")}</summary>
                      <div className="mt-2 space-y-1 rounded-lg border border-white/10 bg-black/20 p-2">
                        <p>{t("来源", "Source")}: {item.source_kind || "--"} / {item.source_ref || "--"}</p>
                        <p>{t("原因", "Reason")}: {item.reason || "--"}</p>
                        {item.conflict_group ? <p>{t("冲突组", "Conflict group")}: {item.conflict_group}</p> : null}
                      </div>
                    </details>
                  </div>
                  <div className="flex shrink-0 flex-wrap gap-1.5">
                    <button
                      type="button"
                      onClick={() => void onExpireMemoryItem(item.id)}
                      disabled={!isActive || memoryActionLoading === `expire:${item.id}`}
                      className="rounded-lg border border-amber-500/25 bg-amber-500/10 px-2 py-1 text-[11px] text-amber-100 hover:bg-amber-500/15 disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      {memoryActionLoading === `expire:${item.id}` ? t("处理中", "Working") : t("过期", "Expire")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void onDeleteMemoryItem(item.id)}
                      disabled={memoryActionLoading === `delete:${item.id}`}
                      className="rounded-lg border border-red-500/25 bg-red-500/10 px-2 py-1 text-[11px] text-red-100 hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {memoryActionLoading === `delete:${item.id}` ? t("删除中", "Deleting") : t("删除", "Delete")}
                    </button>
                  </div>
                </div>
              </div>
            );
          })}
          {!memoryLoading && memoryFacts.length === 0 ? (
            <p className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3 text-sm text-white/50 lg:col-span-2">
              {t("当前没有事实卡片。", "No fact cards yet.")}
            </p>
          ) : null}
        </div>
      </div>

      <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h4 className="text-sm font-semibold text-white">{t("近期记录", "Recent Records")}</h4>
            <p className="mt-1 text-xs leading-5 text-white/55">
              {t("近期记录帮助 RustClaw 理解当前对话上下文。带安全标记的内容默认隐藏。", "Recent records help RustClaw understand the current chat context. Safety-flagged content is hidden by default.")}
            </p>
          </div>
          <span className="theme-meta-pill !rounded-xl !px-2.5 !py-1 text-[11px]">
            {memoryRecent.length}
          </span>
        </div>
        <div className="mt-4 space-y-2">
          {memoryRecent.map((item) => {
            const hidden = shouldHideMemoryRecentContent(item.safety_flag);
            return (
              <div key={item.id} className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="rounded-full border border-white/10 bg-white/5 px-2 py-0.5 text-[10px] text-white/55">{item.role}</span>
                      <span className="rounded-full border border-white/10 bg-white/5 px-2 py-0.5 text-[10px] text-white/45">{item.memory_type}</span>
                      <span className={hidden ? "rounded-full border border-amber-500/25 bg-amber-500/10 px-2 py-0.5 text-[10px] text-amber-100" : "rounded-full border border-white/10 bg-white/5 px-2 py-0.5 text-[10px] text-white/45"}>
                        {memorySafetyLabel(item.safety_flag, lang)}
                      </span>
                      <span className="text-[10px] text-white/35">{timeLabel(item.created_at_ts)}</span>
                    </div>
                    <p className="mt-2 line-clamp-3 break-words text-sm leading-6 text-white/70">
                      {hidden ? t("这条记录带有安全标记，内容已隐藏。", "This record is safety-flagged, so its content is hidden.") : item.content}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => void onDeleteMemoryItem(item.id)}
                    disabled={memoryActionLoading === `delete:${item.id}`}
                    className="inline-flex items-center gap-1 rounded-lg border border-red-500/25 bg-red-500/10 px-2 py-1 text-[11px] text-red-100 hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {memoryActionLoading === `delete:${item.id}` ? <Loader2 className="h-3 w-3 animate-spin" /> : <Trash2 className="h-3 w-3" />}
                    {t("删除", "Delete")}
                  </button>
                </div>
              </div>
            );
          })}
          {!memoryLoading && memoryRecent.length === 0 ? (
            <p className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3 text-sm text-white/50">
              {t("当前没有近期记录。", "No recent records yet.")}
            </p>
          ) : null}
        </div>
      </div>
    </section>
  );
}
