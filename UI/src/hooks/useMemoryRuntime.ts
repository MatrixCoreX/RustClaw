import { useState } from "react";

import { useUiDialog } from "../components/UiDialogProvider";
import { formatUiError } from "../lib/ui-error";
import type {
  ApiResponse,
  MemoryClearPreview,
  MemoryFactItem,
  MemoryExportResult,
  MemoryImportPreviewResult,
  MemoryImportResult,
  MemoryMarkdownExportResult,
  MemoryMutationResult,
  MemoryPageResult,
  MemoryOverviewResponse,
  MemoryPreferenceItem,
  MemoryRecentItem,
  MemorySettingsResult,
  MemoryVectorMutationResult,
  MemoryVectorStatus,
  RemoteMemoryDisclosure,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type MemoryClearScope = "recent" | "all";
type MemorySettingScope = "principal" | "conversation";
export interface MemoryListFilters {
  search: string;
  scope: string;
  origin: string;
  kind: string;
  status: string;
  freshness: string;
}

export interface UseMemoryRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
}

export function useMemoryRuntime({ apiFetch, t }: UseMemoryRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const [memoryOverview, setMemoryOverview] = useState<MemoryOverviewResponse | null>(null);
  const [memorySettings, setMemorySettings] = useState<MemorySettingsResult | null>(null);
  const [memoryPreferences, setMemoryPreferences] = useState<MemoryPreferenceItem[]>([]);
  const [memoryFacts, setMemoryFacts] = useState<MemoryFactItem[]>([]);
  const [memoryRecent, setMemoryRecent] = useState<MemoryRecentItem[]>([]);
  const [memoryLoading, setMemoryLoading] = useState(false);
  const [memoryError, setMemoryError] = useState<string | null>(null);
  const [memoryMessage, setMemoryMessage] = useState<string | null>(null);
  const [memoryActionLoading, setMemoryActionLoading] = useState<string | null>(null);
  const [memorySettingsSaving, setMemorySettingsSaving] = useState(false);
  const [memorySettingScope, setMemorySettingScope] = useState<MemorySettingScope>("principal");
  const [memoryClearScope, setMemoryClearScope] = useState<MemoryClearScope>("recent");
  const [memoryPage, setMemoryPage] = useState<MemoryPageResult | null>(null);
  const [memoryRemoteDisclosure, setMemoryRemoteDisclosure] = useState<RemoteMemoryDisclosure | null>(null);
  const [memoryVectorStatus, setMemoryVectorStatus] = useState<MemoryVectorStatus | null>(null);
  const [memoryUndoRevisionId, setMemoryUndoRevisionId] = useState<string | null>(null);
  const [memoryFilters, setMemoryFilters] = useState<MemoryListFilters>({
    search: "",
    scope: "",
    origin: "",
    kind: "",
    status: "",
    freshness: "",
  });

  const readApiBody = async <T,>(res: Response, label: string): Promise<T> => {
    const body = (await res.json()) as ApiResponse<T>;
    if (!res.ok || !body.ok || body.data === undefined) {
      throw new Error(body.error || `${label.replace(/\s+/g, "_")}_http_${res.status}`);
    }
    return body.data;
  };

  const fetchMemoryData = async (silent = false) => {
    if (!silent) {
      setMemoryLoading(true);
      setMemoryError(null);
    }
    try {
      const pageQuery = new URLSearchParams({ page: "1", page_size: "20" });
      Object.entries(memoryFilters).forEach(([key, value]) => {
        if (value.trim()) pageQuery.set(key, value.trim());
      });
      const vectorStatusPromise = apiFetch("/v1/memory/vector/status")
        .then((response) => readApiBody<MemoryVectorStatus>(response, "memory search index"))
        .catch(() => null);
      const [overviewRes, settingsRes, preferencesRes, factsRes, recentRes, pageRes, disclosureRes] = await Promise.all([
        apiFetch("/v1/memory"),
        apiFetch("/v1/memory/settings"),
        apiFetch("/v1/memory/preferences"),
        apiFetch("/v1/memory/facts"),
        apiFetch("/v1/memory/recent"),
        apiFetch(`/v1/memory/items?${pageQuery.toString()}`),
        apiFetch("/v1/memory/remote-disclosure"),
      ]);
      const [overview, settings, preferences, facts, recent, page, disclosure] = await Promise.all([
        readApiBody<MemoryOverviewResponse>(overviewRes, "memory overview"),
        readApiBody<MemorySettingsResult>(settingsRes, "memory settings"),
        readApiBody<MemoryPreferenceItem[]>(preferencesRes, "memory preferences"),
        readApiBody<MemoryFactItem[]>(factsRes, "memory facts"),
        readApiBody<MemoryRecentItem[]>(recentRes, "memory recent"),
        readApiBody<MemoryPageResult>(pageRes, "memory items"),
        readApiBody<RemoteMemoryDisclosure>(disclosureRes, "memory remote disclosure"),
      ]);
      setMemoryOverview(overview);
      setMemorySettings(settings);
      setMemoryPreferences(preferences);
      setMemoryFacts(facts);
      setMemoryRecent(recent);
      setMemoryPage(page);
      setMemoryRemoteDisclosure(disclosure);
      setMemoryVectorStatus(await vectorStatusPromise);
      setMemoryError(null);
    } catch (err) {
      const message = formatUiError(err, t, "记忆数据暂时无法读取。", "Memory data is temporarily unavailable.");
      setMemoryError(message);
    } finally {
      if (!silent) {
        setMemoryLoading(false);
      }
    }
  };

  const fetchMemoryPage = async (
    filters: MemoryListFilters = memoryFilters,
    page = 1,
  ) => {
    setMemoryLoading(true);
    setMemoryError(null);
    try {
      const query = new URLSearchParams({ page: String(page), page_size: "20" });
      Object.entries(filters).forEach(([key, value]) => {
        if (value.trim()) query.set(key, value.trim());
      });
      const response = await apiFetch(`/v1/memory/items?${query.toString()}`);
      setMemoryPage(await readApiBody<MemoryPageResult>(response, "memory items"));
      setMemoryFilters(filters);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "记忆数据暂时无法读取。", "Memory data is temporarily unavailable."));
    } finally {
      setMemoryLoading(false);
    }
  };

  const correctMemoryItem = async (id: string, revision: number, content: string) => {
    setMemoryActionLoading(`correct:${id}`);
    setMemoryError(null);
    try {
      const response = await apiFetch(`/v1/memory/${encodeURIComponent(id)}/correct`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ expected_revision: revision, content }),
      });
      const result = await readApiBody<MemoryMutationResult>(response, "correct memory");
      setMemoryUndoRevisionId(result.revision_id ?? null);
      setMemoryMessage(t("已保存纠正，旧内容已停止使用。", "Correction saved; the old item is no longer used."));
      await fetchMemoryPage(memoryFilters, memoryPage?.page ?? 1);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "记忆设置保存失败，请稍后重试。", "Memory settings could not be saved. Try again shortly."));
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const sendMemoryFeedback = async (
    id: string,
    revision: number,
    feedbackKind: "irrelevant" | "do_not_use",
  ) => {
    setMemoryActionLoading(`feedback:${id}`);
    setMemoryError(null);
    try {
      const response = await apiFetch(`/v1/memory/${encodeURIComponent(id)}/feedback`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ expected_revision: revision, feedback_kind: feedbackKind }),
      });
      await readApiBody<MemoryMutationResult>(response, "memory feedback");
      setMemoryMessage(
        feedbackKind === "irrelevant"
          ? t("已记录“与本次无关”，记忆本身没有被删除。", "Marked irrelevant for this retrieval; the memory was not deleted.")
          : t("这条记忆已停止用于后续回复。", "This memory will no longer be used in future replies."),
      );
      await fetchMemoryPage(memoryFilters, memoryPage?.page ?? 1);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "记忆偏好保存失败，请稍后重试。", "The memory preference could not be saved. Try again shortly."));
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const deleteMemoryItemWithRevision = async (id: string, revision: number) => {
    const confirmed = await showConfirm({
      title: t("删除记忆", "Delete memory"),
      message: t("删除会同时停止相关后台作业，并从召回索引移除。确定继续吗？", "This also stops related background jobs and removes recall indexes. Continue?"),
      confirmLabel: t("删除", "Delete"),
      tone: "danger",
    });
    if (!confirmed) return;
    setMemoryActionLoading(`delete:${id}`);
    try {
      const response = await apiFetch(`/v1/memory/${encodeURIComponent(id)}/delete`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ expected_revision: revision }),
      });
      const result = await readApiBody<MemoryMutationResult>(response, "delete memory");
      setMemoryUndoRevisionId(result.revision_id ?? null);
      setMemoryMessage(t("记忆已删除，短暂撤销期后会清除恢复副本。", "Memory deleted; its recovery copy is scrubbed after the short undo window."));
      await fetchMemoryPage(memoryFilters, memoryPage?.page ?? 1);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "记忆条目更新失败，请稍后重试。", "The memory entry could not be updated. Try again shortly."));
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const undoMemoryMutation = async () => {
    if (!memoryUndoRevisionId) return;
    setMemoryActionLoading("undo");
    setMemoryError(null);
    try {
      const response = await apiFetch("/v1/memory/undo", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ revision_id: memoryUndoRevisionId }),
      });
      await readApiBody<MemoryMutationResult>(response, "undo memory change");
      setMemoryUndoRevisionId(null);
      setMemoryMessage(t("已撤销刚才的记忆修改。", "The last memory change was undone."));
      await fetchMemoryData(true);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "记忆导出失败，请稍后重试。", "Memory export failed. Try again shortly."));
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const exportMemory = async (format: "json" | "markdown" = "json") => {
    const confirmed = await showConfirm({
      title: t("导出记忆", "Export memory"),
      message: t(
        "导出文件可能包含个人偏好和对话内容。文件不会包含登录密钥或隐藏的安全内容。确定继续吗？",
        "The export may contain preferences and conversation text. It excludes login credentials and hidden safety content. Continue?",
      ),
      confirmLabel: t("导出", "Export"),
    });
    if (!confirmed) return;
    setMemoryActionLoading("export");
    try {
      const response = await apiFetch(format === "markdown" ? "/v1/memory/export/markdown" : "/v1/memory/export");
      const data = format === "markdown"
        ? await readApiBody<MemoryMarkdownExportResult>(response, "memory markdown export")
        : await readApiBody<MemoryExportResult>(response, "memory export");
      const content = format === "markdown"
        ? (data as MemoryMarkdownExportResult).content
        : JSON.stringify(data, null, 2);
      const blob = new Blob([content], { type: format === "markdown" ? "text/markdown" : "application/json" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `memory-export-${data.exported_at_ts}.${format === "markdown" ? "md" : "json"}`;
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "记忆导出失败，请稍后重试。", "Memory export failed. Try again shortly."));
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const importMemory = async (file: File) => {
    setMemoryActionLoading("import");
    setMemoryError(null);
    try {
      const parsed = JSON.parse(await file.text()) as MemoryExportResult;
      const previewResponse = await apiFetch("/v1/memory/import/preview", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ export: parsed }),
      });
      const preview = await readApiBody<MemoryImportPreviewResult>(previewResponse, "memory import preview");
      const confirmed = await showConfirm({
        title: t("确认导入记忆", "Confirm memory import"),
        message: t(
          `可导入 ${preview.accepted_items} 条，跳过 ${preview.skipped_items} 条，重复 ${preview.duplicate_items} 条。所有内容都会降级为“旧数据导入”，并限制在当前账号范围。`,
          `${preview.accepted_items} items can be imported; ${preview.skipped_items} skipped and ${preview.duplicate_items} duplicate. All items are downgraded to imported legacy trust and limited to this account.`,
        ),
        confirmLabel: t("确认导入", "Import"),
      });
      if (!confirmed) return;
      const confirmResponse = await apiFetch("/v1/memory/import/confirm", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          import_id: preview.import_id,
          expected_payload_digest: preview.payload_digest,
        }),
      });
      const result = await readApiBody<MemoryImportResult>(confirmResponse, "memory import");
      setMemoryMessage(t(
        `已导入 ${result.imported_items} 条，已有 ${result.existing_items} 条。`,
        `Imported ${result.imported_items} items; ${result.existing_items} already existed.`,
      ));
      await fetchMemoryData(true);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "导入文件无效或无法读取。", "The import file is invalid or unreadable."));
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const selectMemorySettingScope = async (
    scope: MemorySettingScope,
    conversationId?: string | null,
  ) => {
    if (scope === "conversation" && !conversationId?.trim()) return;
    setMemorySettingsSaving(true);
    setMemoryError(null);
    try {
      const query = new URLSearchParams({ scope });
      if (scope === "conversation" && conversationId) {
        query.set("conversation_id", conversationId);
      }
      const res = await apiFetch(`/v1/memory/settings?${query.toString()}`);
      const data = await readApiBody<MemorySettingsResult>(res, "memory settings");
      setMemorySettingScope(scope);
      setMemorySettings(data);
    } catch (err) {
      const message = formatUiError(err, t, "记忆设置保存失败，请稍后重试。", "Memory settings could not be saved. Try again shortly.");
      setMemoryError(message);
    } finally {
      setMemorySettingsSaving(false);
    }
  };

  const clearMemoryScope = async () => {
    const labelMap: Record<MemoryClearScope, string> = {
      recent: t("近期记录", "recent memories"),
      all: t("全部记忆", "all memory data"),
    };
    const mode = memoryClearScope === "recent" ? "transcript" : "transcript_and_derived";
    let scopedPreview: MemoryClearPreview;
    try {
      const response = await apiFetch(`/v1/memory/clear/preview?mode=${mode}`);
      scopedPreview = await readApiBody<MemoryClearPreview>(response, "memory clear preview");
    } catch (err) {
      setMemoryError(formatUiError(err, t, "记忆导入失败，请稍后重试。", "Memory import failed. Try again shortly."));
      return;
    }
    const confirmed = await showConfirm({
      title: t("清空记忆", "Clear memory"),
      message: t(
        `将删除 ${scopedPreview.transcript_rows} 条${labelMap[memoryClearScope]}、${scopedPreview.derived_rows} 条派生记忆，并停止 ${scopedPreview.pending_jobs} 个后台作业。确定继续吗？`,
        `This removes ${scopedPreview.transcript_rows} ${labelMap[memoryClearScope]}, ${scopedPreview.derived_rows} derived memories, and stops ${scopedPreview.pending_jobs} background jobs. Continue?`,
      ),
      confirmLabel: t("清空", "Clear"),
      tone: "danger",
    });
    if (!confirmed) return;
    setMemoryActionLoading(`clear:${memoryClearScope}`);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const response = await apiFetch("/v1/memory/clear/scoped", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          mode: scopedPreview.mode,
          expected_transcript_rows: scopedPreview.transcript_rows,
          expected_derived_rows: scopedPreview.derived_rows,
        }),
      });
      await readApiBody<MemoryClearPreview>(response, "memory clear");
      setMemoryMessage(t("清理完成。", "Memory cleared."));
      await fetchMemoryData(true);
    } catch (err) {
      const message = formatUiError(err, t, "记忆清理失败，请稍后重试。", "Memory cleanup failed. Try again shortly.");
      setMemoryError(message);
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const updateMemoryExternalPolicy = async (policy: "exclude" | "evidence_only" | "allow") => {
    if (!memorySettings) return;
    setMemorySettingsSaving(true);
    setMemoryError(null);
    try {
      const response = await apiFetch("/v1/memory/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          scope: memorySettingScope,
          conversation_id: memorySettingScope === "conversation" ? memorySettings.conversation_id : undefined,
          expected_revision: memorySettings.revision,
          external_context_policy: policy,
        }),
      });
      const settings = await readApiBody<MemorySettingsResult>(response, "memory settings");
      setMemorySettings(settings);
      setMemoryMessage(t("远程处理范围已更新。", "Remote processing scope updated."));
      await fetchMemoryData(true);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "记忆清理失败，请稍后重试。", "Memory cleanup failed. Try again shortly."));
    } finally {
      setMemorySettingsSaving(false);
    }
  };

  const updateMemorySetting = async (kind: "use" | "generate", enabled: boolean) => {
    if (!memorySettings) return;
    setMemorySettingsSaving(true);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch("/v1/memory/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          scope: memorySettingScope,
          conversation_id:
            memorySettingScope === "conversation" ? memorySettings.conversation_id : undefined,
          expected_revision: memorySettings.revision,
          [kind === "use" ? "use_mode" : "generate_mode"]: enabled ? "enabled" : "disabled",
        }),
      });
      const data = await readApiBody<MemorySettingsResult>(res, "memory settings");
      setMemorySettings(data);
      setMemoryOverview((prev) =>
        prev ? { ...prev, long_term_enabled: data.use_memory && data.generate_memory } : prev,
      );
      setMemoryMessage(t("记忆设置已立即生效。", "Memory setting is now active."));
    } catch (err) {
      const message = formatUiError(err, t, "记忆设置保存失败，请稍后重试。", "Memory settings could not be saved. Try again shortly.");
      setMemoryError(message);
    } finally {
      setMemorySettingsSaving(false);
    }
  };

  const controlMemoryVector = async (
    action: "reindex" | "pause" | "resume" | "cancel",
  ) => {
    if (!memorySettings) return;
    setMemoryActionLoading(`vector:${action}`);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const response = await apiFetch(`/v1/memory/vector/${action}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ expected_policy_digest: memorySettings.policy_digest }),
      });
      const result = await readApiBody<MemoryVectorMutationResult>(response, "memory search index");
      setMemoryMessage(action === "reindex"
        ? t(`已安排重建 ${result.queued_rows} 条搜索索引。`, `Queued ${result.queued_rows} items for search-index rebuild.`)
        : t("搜索索引状态已更新。", "Search-index state updated."));
      await fetchMemoryData(true);
    } catch (err) {
      setMemoryError(formatUiError(err, t, "搜索索引操作失败，请稍后重试。", "The search-index action failed. Try again shortly."));
    } finally {
      setMemoryActionLoading(null);
    }
  };

  return {
    memoryOverview,
    memorySettings,
    memoryPreferences,
    memoryFacts,
    memoryRecent,
    memoryLoading,
    memoryError,
    memoryMessage,
    memoryActionLoading,
    memorySettingsSaving,
    memorySettingScope,
    memoryClearScope,
    memoryPage,
    memoryFilters,
    memoryRemoteDisclosure,
    memoryVectorStatus,
    memoryUndoRevisionId,
    setMemoryClearScope,
    fetchMemoryData,
    fetchMemoryPage,
    correctMemoryItem,
    sendMemoryFeedback,
    deleteMemoryItemWithRevision,
    undoMemoryMutation,
    exportMemory,
    importMemory,
    clearMemoryScope,
    updateMemorySetting,
    updateMemoryExternalPolicy,
    selectMemorySettingScope,
    controlMemoryVector,
  };
}
