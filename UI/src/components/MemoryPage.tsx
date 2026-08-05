import { useState } from "react";
import { Database, Download, FileUp, Loader2, RefreshCw, Trash2, Undo2 } from "lucide-react";

import { formatUnixDateTime } from "../lib/date-format";
import {
  memoryFactStatusLabel,
  memorySafetyLabel,
  shouldHideMemoryRecentContent,
} from "../lib/memory-display";
import type {
  MemoryFactItem,
  MemoryOverviewResponse,
  MemoryPageResult,
  MemorySettingsResult,
  MemoryVectorStatus,
  MemoryPreferenceItem,
  MemoryRecentItem,
  RemoteMemoryDisclosure,
} from "../types/api";
import type { MemoryListFilters } from "../hooks/useMemoryRuntime";

type UiLanguage = "zh" | "en";
type ClearScope = "recent" | "all";
type Translate = (zh: string, en: string) => string;

export interface MemoryPageProps {
  lang: UiLanguage;
  t: Translate;
  memoryLoading: boolean;
  memoryError: string | null;
  memoryMessage: string | null;
  memoryOverview: MemoryOverviewResponse | null;
  memorySettings: MemorySettingsResult | null;
  memorySettingScope: "principal" | "conversation";
  activeConversationId: string | null;
  memoryPreferences: MemoryPreferenceItem[];
  memoryFacts: MemoryFactItem[];
  memoryRecent: MemoryRecentItem[];
  memoryActionLoading: string | null;
  memorySettingsSaving: boolean;
  memoryClearScope: ClearScope;
  memoryPage: MemoryPageResult | null;
  memoryFilters: MemoryListFilters;
  memoryRemoteDisclosure: RemoteMemoryDisclosure | null;
  memoryVectorStatus: MemoryVectorStatus | null;
  memoryUndoRevisionId: string | null;
  onMemoryClearScopeChange: (scope: ClearScope) => void;
  onFetchMemoryData: () => void | Promise<void>;
  onFetchMemoryPage: (filters: MemoryListFilters, page?: number) => void | Promise<void>;
  onCorrectMemoryItem: (id: string, revision: number, content: string) => void | Promise<void>;
  onSendMemoryFeedback: (
    id: string,
    revision: number,
    kind: "irrelevant" | "do_not_use",
  ) => void | Promise<void>;
  onDeleteMemoryItemWithRevision: (id: string, revision: number) => void | Promise<void>;
  onUndoMemoryMutation: () => void | Promise<void>;
  onExportMemory: (format?: "json" | "markdown") => void | Promise<void>;
  onImportMemory: (file: File) => void | Promise<void>;
  onClearMemoryScope: () => void | Promise<void>;
  onUpdateMemorySetting: (kind: "use" | "generate", enabled: boolean) => void | Promise<void>;
  onUpdateMemoryExternalPolicy: (policy: "exclude" | "evidence_only" | "allow") => void | Promise<void>;
  onSelectMemorySettingScope: (
    scope: "principal" | "conversation",
    conversationId?: string | null,
  ) => void | Promise<void>;
  onControlMemoryVector: (
    action: "reindex" | "pause" | "resume" | "cancel",
  ) => void | Promise<void>;
}

export function MemoryPage({
  lang,
  t,
  memoryLoading,
  memoryError,
  memoryMessage,
  memoryOverview,
  memorySettings,
  memorySettingScope,
  activeConversationId,
  memoryPreferences,
  memoryFacts,
  memoryRecent,
  memoryActionLoading,
  memorySettingsSaving,
  memoryClearScope,
  memoryPage,
  memoryFilters,
  memoryRemoteDisclosure,
  memoryVectorStatus,
  memoryUndoRevisionId,
  onMemoryClearScopeChange,
  onFetchMemoryData,
  onFetchMemoryPage,
  onCorrectMemoryItem,
  onSendMemoryFeedback,
  onDeleteMemoryItemWithRevision,
  onUndoMemoryMutation,
  onExportMemory,
  onImportMemory,
  onClearMemoryScope,
  onUpdateMemorySetting,
  onUpdateMemoryExternalPolicy,
  onSelectMemorySettingScope,
  onControlMemoryVector,
}: MemoryPageProps) {
  const dateLocale = lang === "zh" ? "zh-CN" : "en-US";
  const timeLabel = (ts: number | null | undefined) => formatUnixDateTime(ts, dateLocale);
  const [filters, setFilters] = useState<MemoryListFilters>(memoryFilters);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [correction, setCorrection] = useState("");

  return (
    <section className="space-y-4">
      <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <p className="text-[10px] uppercase tracking-[0.28em] text-white/45">
              {t("记忆管理", "Memory Control")}
            </p>
            <h3 className="mt-2 text-base font-semibold text-white">
              {t("查看和管理 {product_name} 会用于回复的记忆。", "Review and manage the memory {product_name} can use in replies.")}
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
          <div className="mt-4 flex flex-wrap items-center justify-between gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200" role="status">
            <span>{memoryMessage}</span>
            {memoryUndoRevisionId ? (
              <button
                type="button"
                className="theme-secondary-btn px-3 py-1.5 text-xs"
                disabled={memoryActionLoading === "undo"}
                onClick={() => void onUndoMemoryMutation()}
              >
                {memoryActionLoading === "undo" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Undo2 className="h-3.5 w-3.5" />}
                {t("撤销", "Undo")}
              </button>
            ) : null}
          </div>
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

      <div className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h4 className="text-sm font-semibold text-white">{t("记忆条目", "Memory items")}</h4>
            <p className="mt-1 text-xs leading-5 text-white/55">
              {t(
                "按范围和状态查找记忆。纠正会保留版本记录；“与本次无关”不会删除记忆；“不要再用”会停止召回。",
                "Filter by scope and status. Corrections keep revision history; “irrelevant” does not delete; “do not use” stops recall.",
              )}
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => void onExportMemory("json")}
              disabled={memoryActionLoading === "export"}
              className="theme-secondary-btn px-3 py-2 text-xs disabled:opacity-50"
            >
              {memoryActionLoading === "export" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
              {t("导出 JSON", "Export JSON")}
            </button>
            <button type="button" onClick={() => void onExportMemory("markdown")} className="theme-secondary-btn px-3 py-2 text-xs">
              <Download className="h-3.5 w-3.5" />
              {t("导出 Markdown", "Export Markdown")}
            </button>
            <label className="theme-secondary-btn cursor-pointer px-3 py-2 text-xs">
              <FileUp className="h-3.5 w-3.5" />
              {t("导入预检", "Preview import")}
              <input
                type="file"
                accept="application/json,.json"
                className="sr-only"
                disabled={memoryActionLoading === "import"}
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  if (file) void onImportMemory(file);
                  event.currentTarget.value = "";
                }}
              />
            </label>
          </div>
        </div>
        <div className="mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-7">
          <input
            className="theme-input xl:col-span-2"
            value={filters.search}
            placeholder={t("搜索记忆内容", "Search memory")}
            onChange={(event) => setFilters((value) => ({ ...value, search: event.target.value }))}
          />
          {([
            ["scope", filters.scope, [["", t("全部范围", "All scopes")], ["principal", t("账号", "Account")], ["conversation", t("对话", "Conversation")], ["project", t("项目", "Project")]]],
            ["kind", filters.kind, [["", t("全部类型", "All kinds")], ["fact", t("事实", "Facts")], ["preference", t("偏好", "Preferences")], ["recent", t("近期记录", "Recent")]]],
            ["status", filters.status, [["", t("全部状态", "All statuses")], ["active", t("有效", "Active")], ["superseded", t("已取代", "Superseded")], ["expired", t("已过期", "Expired")]]],
            ["freshness", filters.freshness, [["", t("全部时效", "All freshness")], ["fresh", t("较新", "Fresh")], ["stale", t("需复核", "Needs review")]]],
          ] as const).map(([field, value, options]) => (
            <select
              key={field}
              className="theme-input"
              value={value}
              onChange={(event) => setFilters((current) => ({ ...current, [field]: event.target.value }))}
            >
              {options.map(([optionValue, label]) => <option key={optionValue} value={optionValue}>{label}</option>)}
            </select>
          ))}
          <button
            type="button"
            className="theme-secondary-btn px-3 py-2 text-xs"
            onClick={() => void onFetchMemoryPage(filters, 1)}
          >
            {t("筛选", "Apply")}
          </button>
        </div>
        <div className="mt-4 grid gap-3 lg:grid-cols-2">
          {memoryPage?.items.map((item) => (
            <article key={item.id} className="rounded-xl border border-white/10 bg-[#12151f] p-4">
              <div className="flex flex-wrap items-center gap-2 text-[10px] text-white/45">
                <span className="theme-meta-pill !px-2 !py-0.5">{item.kind}</span>
                <span>{item.scope_kind}</span>
                <span>{item.status}</span>
                <span>{item.freshness === "stale" ? t("需复核", "Needs review") : t("较新", "Fresh")}</span>
              </div>
              {editingId === item.id ? (
                <div className="mt-3 space-y-2">
                  <textarea
                    className="theme-input min-h-24 w-full resize-y"
                    value={correction}
                    onChange={(event) => setCorrection(event.target.value)}
                  />
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      className="theme-secondary-btn px-3 py-2 text-xs"
                      disabled={!correction.trim() || memoryActionLoading === `correct:${item.id}`}
                      onClick={async () => {
                        await onCorrectMemoryItem(item.id, item.revision, correction);
                        setEditingId(null);
                        setCorrection("");
                      }}
                    >
                      {t("保存纠正", "Save correction")}
                    </button>
                    <button type="button" className="theme-secondary-btn px-3 py-2 text-xs" onClick={() => setEditingId(null)}>
                      {t("取消", "Cancel")}
                    </button>
                  </div>
                </div>
              ) : (
                <p className="mt-3 break-words text-sm leading-6 text-white/80">{item.content}</p>
              )}
              <p className="mt-3 text-[11px] leading-5 text-white/40">
                {t("来源", "Source")}: {item.source || "--"} · {t("生成方式", "Origin")}: {item.origin || "--"}<br />
                {t("依据", "Evidence")}: {item.evidence_available ? t("可用", "Available") : t("不可用", "Unavailable")} · {t("更新", "Updated")}: {timeLabel(item.updated_at_ts)}
                {item.expires_at_ts ? ` · ${t("过期", "Expires")}: ${timeLabel(item.expires_at_ts)}` : ""}
              </p>
              <div className="mt-3 flex flex-wrap gap-2">
                {item.kind !== "recent" ? (
                  <button
                    type="button"
                    className="theme-secondary-btn px-2.5 py-1.5 text-[11px]"
                    onClick={() => { setEditingId(item.id); setCorrection(item.content); }}
                  >
                    {t("信息错误，纠正", "Incorrect — correct")}
                  </button>
                ) : null}
                <button type="button" className="theme-secondary-btn px-2.5 py-1.5 text-[11px]" onClick={() => void onSendMemoryFeedback(item.id, item.revision, "irrelevant")}>{t("与本次无关", "Irrelevant")}</button>
                <button type="button" className="theme-secondary-btn px-2.5 py-1.5 text-[11px]" onClick={() => void onSendMemoryFeedback(item.id, item.revision, "do_not_use")}>{t("不要再用", "Do not use")}</button>
                <button type="button" className="rounded-lg border border-red-500/25 bg-red-500/10 px-2.5 py-1.5 text-[11px] text-red-100" onClick={() => void onDeleteMemoryItemWithRevision(item.id, item.revision)}>{t("删除", "Delete")}</button>
              </div>
            </article>
          ))}
          {!memoryLoading && (memoryPage?.items.length ?? 0) === 0 ? (
            <p className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3 text-sm text-white/50 lg:col-span-2">{t("没有符合条件的记忆。", "No matching memories.")}</p>
          ) : null}
        </div>
        <div className="mt-4 flex items-center justify-between gap-3 text-xs text-white/55">
          <span>{t("共", "Total")} {memoryPage?.total ?? 0}</span>
          <div className="flex gap-2">
            <button type="button" className="theme-secondary-btn px-3 py-2 text-xs disabled:opacity-40" disabled={!memoryPage || memoryPage.page <= 1} onClick={() => void onFetchMemoryPage(filters, (memoryPage?.page ?? 1) - 1)}>{t("上一页", "Previous")}</button>
            <button type="button" className="theme-secondary-btn px-3 py-2 text-xs disabled:opacity-40" disabled={!memoryPage?.has_more} onClick={() => void onFetchMemoryPage(filters, (memoryPage?.page ?? 1) + 1)}>{t("下一页", "Next")}</button>
          </div>
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
                  <span className="text-[11px] text-white/35">{t("请在上方记忆条目中管理", "Manage above")}</span>
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
            <h4 className="text-sm font-semibold text-white">{t("记忆使用方式", "How memory is used")}</h4>
            <p className="mt-2 text-xs leading-5 text-white/55">
              {t("两个开关互不影响，修改后立即生效。关闭参考不会删除已有记忆；关闭形成不会影响对话记录。", "These controls are independent and apply immediately. Turning off recall keeps existing memories; turning off learning keeps conversation history.")}
            </p>
            <div className="mt-3 flex flex-wrap gap-2" role="group" aria-label={t("记忆设置范围", "Memory setting scope")}>
              <button
                type="button"
                aria-pressed={memorySettingScope === "principal"}
                className="theme-secondary-btn px-3 py-2 text-xs"
                onClick={() => void onSelectMemorySettingScope("principal")}
              >
                {t("我的默认设置", "My defaults")}
              </button>
              <button
                type="button"
                aria-pressed={memorySettingScope === "conversation"}
                className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                disabled={!activeConversationId}
                onClick={() => void onSelectMemorySettingScope("conversation", activeConversationId)}
              >
                {t("当前对话", "Current conversation")}
              </button>
            </div>
            <div className="mt-4 grid gap-3 sm:grid-cols-2">
              {([
                ["use", t("参考已有记忆", "Use saved memory"), memorySettings?.use_memory ?? false],
                ["generate", t("从对话形成未来记忆", "Learn from conversations"), memorySettings?.generate_memory ?? false],
              ] as const).map(([kind, label, enabled]) => (
                <div key={kind} className="flex items-center justify-between gap-3 rounded-xl border border-white/10 bg-black/10 p-3">
                  <div>
                    <p className="text-xs font-medium text-white/85">{label}</p>
                    <p className="mt-1 text-[11px] text-white/45">{enabled ? t("已开启", "On") : t("已关闭", "Off")}</p>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={enabled}
                    onClick={() => void onUpdateMemorySetting(kind, !enabled)}
                    disabled={memorySettingsSaving || !memorySettings || Boolean(memorySettings.managed_deny_reason)}
                    className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {memorySettingsSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Database className="h-3.5 w-3.5" />}
                    {enabled ? t("关闭", "Turn off") : t("开启", "Turn on")}
                  </button>
                </div>
              ))}
            </div>
            {memoryRemoteDisclosure ? (
              <details className="mt-4 rounded-xl border border-white/10 bg-black/10 p-3 text-xs text-white/55">
                <summary className="cursor-pointer font-medium text-white/75">
                  {t("远程处理会发送什么", "What remote processing sends")}
                </summary>
                <div className="mt-3 space-y-2 leading-5">
                  <p>
                    {t("当前授权", "Current consent")}: {memoryRemoteDisclosure.consent_state}
                  </p>
                  <label className="block">
                    <span className="mb-1 block text-white/65">{t("允许的远程内容", "Allowed remote content")}</span>
                    <select
                      className="theme-input w-full"
                      value={memorySettings?.external_context_policy === "inherit" ? "exclude" : memorySettings?.external_context_policy}
                      disabled={memorySettingsSaving || !memorySettings}
                      onChange={(event) => void onUpdateMemoryExternalPolicy(event.target.value as "exclude" | "evidence_only" | "allow")}
                    >
                      <option value="exclude">{t("不发送", "Do not send")}</option>
                      <option value="evidence_only">{t("只发送依据引用", "Evidence references only")}</option>
                      <option value="allow">{t("发送符合资格的片段", "Eligible excerpts")}</option>
                    </select>
                  </label>
                  <p>
                    {t("提取模型", "Extraction model")}: {memoryRemoteDisclosure.extraction_provider} / {memoryRemoteDisclosure.extraction_model}
                  </p>
                  <p>
                    {t("整理模型", "Consolidation model")}: {memoryRemoteDisclosure.consolidation_provider} / {memoryRemoteDisclosure.consolidation_model}
                  </p>
                  <p>
                    {t(
                      "只有符合资格并已去除敏感信息的用户片段和依据引用会用于后台整理。远程向量功能还会发送可搜索的记忆文本与已同意发送的查询文本。撤回授权会停止新请求、取消任务并清理本地远程索引。",
                      "Only eligible, sensitivity-minimized user excerpts and evidence references are used for background processing. Remote embeddings also send searchable memory text and consented query text. Withdrawing consent stops new requests, cancels jobs, and removes local remote indexes.",
                    )}
                  </p>
                </div>
              </details>
            ) : null}
            {memoryVectorStatus ? (
              <details className="mt-4 rounded-xl border border-white/10 bg-black/10 p-3 text-xs text-white/55">
                <summary className="cursor-pointer font-medium text-white/75">
                  {t("搜索索引", "Search index")}
                </summary>
                <div className="mt-3 space-y-3 leading-5">
                  <p>
                    {memoryVectorStatus.provider_location === "local"
                      ? t("在本机生成，不会发送记忆内容。", "Built locally; memory content is not sent away.")
                      : t("使用已授权的远程模型生成。", "Built with the authorized remote model.")}
                  </p>
                  <p>
                    {t("状态", "Status")}: {memoryVectorStatus.state} · {t("已索引", "Indexed")}: {memoryVectorStatus.indexed_rows}
                    {memoryVectorStatus.queued_jobs > 0 ? ` · ${t("待处理", "Queued")}: ${memoryVectorStatus.queued_jobs}` : ""}
                    {memoryVectorStatus.failed_jobs > 0 ? ` · ${t("失败", "Failed")}: ${memoryVectorStatus.failed_jobs}` : ""}
                  </p>
                  <div className="flex flex-wrap gap-2">
                    <button type="button" className="theme-secondary-btn px-3 py-2 text-xs" disabled={Boolean(memoryActionLoading?.startsWith("vector:")) || memoryVectorStatus.state === "building"} onClick={() => void onControlMemoryVector("reindex")}>
                      {memoryActionLoading === "vector:reindex" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                      {t("重建索引", "Rebuild index")}
                    </button>
                    {memoryVectorStatus.state === "paused" ? (
                      <button type="button" className="theme-secondary-btn px-3 py-2 text-xs" disabled={Boolean(memoryActionLoading?.startsWith("vector:"))} onClick={() => void onControlMemoryVector("resume")}>{t("继续", "Resume")}</button>
                    ) : memoryVectorStatus.state === "building" ? (
                      <button type="button" className="theme-secondary-btn px-3 py-2 text-xs" disabled={Boolean(memoryActionLoading?.startsWith("vector:"))} onClick={() => void onControlMemoryVector("pause")}>{t("暂停", "Pause")}</button>
                    ) : null}
                    {memoryVectorStatus.state === "building" || memoryVectorStatus.state === "paused" ? (
                      <button type="button" className="theme-secondary-btn px-3 py-2 text-xs" disabled={Boolean(memoryActionLoading?.startsWith("vector:"))} onClick={() => void onControlMemoryVector("cancel")}>{t("取消重建", "Cancel rebuild")}</button>
                    ) : null}
                  </div>
                </div>
              </details>
            ) : null}
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
                <option value="all">{t("清空近期记录和派生记忆", "Clear transcript and derived memory")}</option>
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
                  <span className="shrink-0 text-[11px] text-white/35">{t("请在上方记忆条目中管理", "Manage above")}</span>
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
              {t("近期记录帮助 {product_name} 理解当前对话上下文。带安全标记的内容默认隐藏。", "Recent records help {product_name} understand the current chat context. Safety-flagged content is hidden by default.")}
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
                  <span className="text-[11px] text-white/35">{t("请在上方记忆条目中管理", "Manage above")}</span>
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
