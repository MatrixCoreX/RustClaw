import { useEffect, useMemo, useRef, useState } from "react";

import { useUiDialog, type UiConfirmOptions } from "../components/UiDialogProvider";
import { formatUiError } from "../lib/ui-error";
import type {
  ApiResponse, MemoryClearPreview, MemoryExportResult, MemoryFactItem, MemoryImportPreviewResult,
  MemoryImportResult, MemoryMarkdownExportResult, MemoryMutationResult, MemoryOverviewResponse,
  MemoryPageResult, MemoryPreferenceItem, MemoryRecentItem, MemorySettingsResult,
  MemoryVectorMutationResult, MemoryVectorStatus, RemoteMemoryDisclosure,
} from "../types/api";
import { useRequestOwner } from "./useRequestOwner";
import {
  emptyMemoryView, memoryRequests, settingsDomain, type MemoryClearScope, type MemoryListFilters,
  type MemoryOwner, type MemorySettingScope, type MemoryView,
} from "./memory-runtime-state";

export type { MemoryListFilters } from "./memory-runtime-state";
type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type Change = Partial<Omit<MemoryView, "owner">> | ((view: MemoryView) => Partial<Omit<MemoryView, "owner">>);
type Failure = [string, string];
const STALE = Symbol("obsolete memory request");
const SETTINGS_FAILURE: Failure = ["记忆设置保存失败，请稍后重试。", "Memory settings could not be saved. Try again shortly."];

export interface UseMemoryRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  requestScope: string;
}

export function useMemoryRuntime({ apiFetch, t, requestScope }: UseMemoryRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const owns = useRequestOwner(requestScope);
  const requests = useMemo(() => memoryRequests(owns), [owns]);
  const empty = useMemo(() => emptyMemoryView(owns), [owns]);
  const [stored, setStored] = useState(empty);
  // The render projection itself is owner-bound; an effect is not the privacy boundary.
  const view = stored.owner === owns ? stored : empty;
  const latest = useRef(view);
  const activeOwner = useRef(owns);
  latest.current = view;
  activeOwner.current = owns;
  useEffect(() => {
    if (activeOwner.current === owns) { latest.current = empty; setStored(empty); }
  }, [empty, owns]);

  const update = (change: Change, current: MemoryOwner = owns) => {
    if (!owns() || !current()) return;
    const previous = latest.current.owner === owns ? latest.current : empty;
    const next = { ...previous, ...(typeof change === "function" ? change(previous) : change), owner: owns };
    latest.current = next;
    setStored(previous => owns() && current() ? next : previous);
  };
  const failure = (error: unknown, fallback: Failure, current: MemoryOwner) => {
    if (error !== STALE && owns() && current()) update({ memoryError: formatUiError(error, t, ...fallback) }, current);
  };
  const read = async <T,>(path: string, init?: RequestInit): Promise<T> => {
    if (!owns()) throw STALE;
    const response = await apiFetch(path, init);
    if (!owns()) throw STALE;
    const body = (await response.json()) as ApiResponse<T>;
    if (!owns()) throw STALE;
    if (!response.ok || !body.ok || body.data === undefined) throw new Error(body.error || `memory_http_${response.status}`);
    return body.data;
  };
  const post = <T,>(path: string, body: unknown) => read<T>(path, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
  });
  const confirm = async (options: UiConfirmOptions) => owns() && await showConfirm(options) && owns();
  const settingsPath = (state: MemoryView, explicit = false) => {
    if (!explicit && state.memorySettingScope === "principal") return "/v1/memory/settings";
    const query = new URLSearchParams({ scope: state.memorySettingScope });
    if (state.settingsConversationId) query.set("conversation_id", state.settingsConversationId);
    return `/v1/memory/settings?${query}`;
  };
  const pagePath = (filters: MemoryListFilters, page: number) => {
    const query = new URLSearchParams({ page: String(page), page_size: "20" });
    Object.entries(filters).forEach(([key, value]) => { if (value.trim()) query.set(key, value.trim()); });
    return `/v1/memory/items?${query}`;
  };

  const fetchMemoryData = async (silent = false) => {
    if (!owns()) return;
    const state = latest.current;
    const domain = settingsDomain(state);
    const items = requests.token("bundle", ["items"]);
    const overview = requests.token("overview", ["items", domain]);
    const settings = requests.token("settings", [domain]);
    const page = requests.token("page", ["items"]);
    const disclosure = requests.token("disclosure", [domain]);
    const vector = requests.token("vector", ["vector", domain]);
    const notice = requests.token("notice");
    const busy = silent ? null : requests.token("readBusy");
    if (busy) update({ memoryLoading: true, memoryError: null }, busy);
    try {
      const [overviewData, settingsData, preferences, facts, recent, pageData, disclosureData, vectorData] = await Promise.all([
        read<MemoryOverviewResponse>("/v1/memory"), read<MemorySettingsResult>(settingsPath(state)),
        read<MemoryPreferenceItem[]>("/v1/memory/preferences"), read<MemoryFactItem[]>("/v1/memory/facts"),
        read<MemoryRecentItem[]>("/v1/memory/recent"), read<MemoryPageResult>(pagePath(state.memoryFilters, state.requestedPage)),
        read<RemoteMemoryDisclosure>("/v1/memory/remote-disclosure"),
        read<MemoryVectorStatus>("/v1/memory/vector/status").catch(() => null),
      ]);
      update({ memoryPreferences: preferences, memoryFacts: facts, memoryRecent: recent }, items);
      update({ memoryOverview: overviewData }, overview);
      if (settingsDomain(latest.current) === domain) update({ memorySettings: settingsData }, settings);
      update({ memoryPage: pageData }, page);
      update({ memoryRemoteDisclosure: disclosureData }, disclosure);
      update({ memoryVectorStatus: vectorData }, vector);
      update({ memoryError: null }, notice);
    } catch (error) {
      failure(error, ["记忆数据暂时无法读取。", "Memory data is temporarily unavailable."], notice);
    } finally {
      if (busy) update({ memoryLoading: false }, busy);
    }
  };

  const fetchMemoryPage = async (filters = latest.current.memoryFilters, page = 1) => {
    if (!owns()) return;
    const current = requests.token("page", ["items"]);
    const notice = requests.token("notice");
    const busy = requests.token("readBusy");
    update({ memoryFilters: filters, requestedPage: page, memoryLoading: true, memoryError: null });
    try {
      update({ memoryPage: await read<MemoryPageResult>(pagePath(filters, page)) }, current);
    } catch (error) {
      if (current()) failure(error, ["记忆数据暂时无法读取。", "Memory data is temporarily unavailable."], notice);
    } finally { update({ memoryLoading: false }, busy); }
  };

  const change = async <T,>(kind: "action" | "settings", label: string, domains: string[],
    execute: () => Promise<T>, apply: (result: T, present: MemoryOwner) => void | Promise<void>, fallback: Failure) => {
    if (!owns()) return;
    const present = requests.token(kind === "settings" ? "settingsBusy" : "actionBusy");
    const notice = requests.token("notice");
    const finish = requests.write(...domains);
    update({ memoryError: null, memoryMessage: null,
      ...(kind === "settings" ? { memorySettingsSaving: true } : { memoryActionLoading: label }) });
    try {
      const result = await execute();
      finish();
      if (owns()) await apply(result, present);
    } catch (error) { failure(error, fallback, notice); }
    finally {
      finish();
      update(kind === "settings" ? { memorySettingsSaving: false } : { memoryActionLoading: null }, present);
    }
  };
  const refreshPage = () => fetchMemoryPage(latest.current.memoryFilters, latest.current.requestedPage);
  const refreshVectorStatus = async () => {
    const current = requests.token("vector", ["vector"]);
    const status = await read<MemoryVectorStatus>("/v1/memory/vector/status").catch(() => null);
    update({ memoryVectorStatus: status }, current);
  };

  const correctMemoryItem = (id: string, revision: number, content: string) => change("action", `correct:${id}`, ["items", "vector"],
    () => post<MemoryMutationResult>(`/v1/memory/${encodeURIComponent(id)}/correct`, { expected_revision: revision, content }),
    async (result, present) => {
      update({ memoryUndoRevisionId: result.revision_id ?? null,
        memoryMessage: t("已保存纠正，旧内容已停止使用。", "Correction saved; the old item is no longer used.") }, present);
      await refreshPage();
    }, ["记忆设置保存失败，请稍后重试。", "Memory settings could not be saved. Try again shortly."]);

  const sendMemoryFeedback = (id: string, revision: number, feedbackKind: "irrelevant" | "do_not_use") => change("action", `feedback:${id}`, ["items", "vector"],
    () => post<MemoryMutationResult>(`/v1/memory/${encodeURIComponent(id)}/feedback`, { expected_revision: revision, feedback_kind: feedbackKind }),
    async (_, present) => {
      update({ memoryMessage: feedbackKind === "irrelevant"
        ? t("已记录“与本次无关”，记忆本身没有被删除。", "Marked irrelevant for this retrieval; the memory was not deleted.")
        : t("这条记忆已停止用于后续回复。", "This memory will no longer be used in future replies.") }, present);
      await refreshPage();
    }, ["记忆偏好保存失败，请稍后重试。", "The memory preference could not be saved. Try again shortly."]);

  const deleteMemoryItemWithRevision = async (id: string, revision: number) => {
    if (!await confirm({ title: t("删除记忆", "Delete memory"),
      message: t("删除会同时停止相关后台作业，并从召回索引移除。确定继续吗？", "This also stops related background jobs and removes recall indexes. Continue?"),
      confirmLabel: t("删除", "Delete"), tone: "danger" })) return;
    await change("action", `delete:${id}`, ["items", "vector"],
      () => post<MemoryMutationResult>(`/v1/memory/${encodeURIComponent(id)}/delete`, { expected_revision: revision }),
      async (result, present) => {
        update({ memoryUndoRevisionId: result.revision_id ?? null,
          memoryMessage: t("记忆已删除，短暂撤销期后会清除恢复副本。", "Memory deleted; its recovery copy is scrubbed after the short undo window.") }, present);
        await refreshPage();
      }, ["记忆条目更新失败，请稍后重试。", "The memory entry could not be updated. Try again shortly."]);
  };

  const undoMemoryMutation = async () => {
    if (!owns() || !latest.current.memoryUndoRevisionId) return;
    const revisionId = latest.current.memoryUndoRevisionId;
    await change("action", "undo", ["items", "vector"],
      () => post<MemoryMutationResult>("/v1/memory/undo", { revision_id: revisionId }), async (_, present) => {
        update({ memoryUndoRevisionId: null, memoryMessage: t("已撤销刚才的记忆修改。", "The last memory change was undone.") }, present);
        await fetchMemoryData(true);
      }, ["记忆修改暂时无法撤销，请稍后重试。", "The memory change could not be undone. Try again shortly."]);
  };

  const exportMemory = async (format: "json" | "markdown" = "json") => {
    if (!await confirm({ title: t("导出记忆", "Export memory"), message: t(
      "导出文件可能包含个人偏好和对话内容。文件不会包含登录密钥或隐藏的安全内容。确定继续吗？",
      "The export may contain preferences and conversation text. It excludes login credentials and hidden safety content. Continue?"),
      confirmLabel: t("导出", "Export") })) return;
    const present = requests.token("actionBusy");
    const notice = requests.token("notice");
    update({ memoryActionLoading: "export" });
    try {
      const data = format === "markdown" ? await read<MemoryMarkdownExportResult>("/v1/memory/export/markdown")
        : await read<MemoryExportResult>("/v1/memory/export");
      if (!present()) return;
      const content = format === "markdown" ? (data as MemoryMarkdownExportResult).content : JSON.stringify(data, null, 2);
      const url = URL.createObjectURL(new Blob([content], { type: format === "markdown" ? "text/markdown" : "application/json" }));
      try {
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = `memory-export-${data.exported_at_ts}.${format === "markdown" ? "md" : "json"}`;
        anchor.click();
      } finally { URL.revokeObjectURL(url); }
    } catch (error) { failure(error, ["记忆导出失败，请稍后重试。", "Memory export failed. Try again shortly."], notice); }
    finally { update({ memoryActionLoading: null }, present); }
  };

  const importMemory = async (file: File) => {
    if (!owns()) return;
    let present = requests.token("actionBusy");
    let notice = requests.token("notice");
    update({ memoryActionLoading: "import", memoryError: null });
    try {
      const text = await file.text();
      if (!owns()) return;
      const preview = await post<MemoryImportPreviewResult>("/v1/memory/import/preview", { export: JSON.parse(text) });
      if (!await confirm({ title: t("确认导入记忆", "Confirm memory import"), message: t(
        `可导入 ${preview.accepted_items} 条，跳过 ${preview.skipped_items} 条，重复 ${preview.duplicate_items} 条。所有内容都会降级为“旧数据导入”，并限制在当前账号范围。`,
        `${preview.accepted_items} items can be imported; ${preview.skipped_items} skipped and ${preview.duplicate_items} duplicate. All items are downgraded to imported legacy trust and limited to this account.`),
        confirmLabel: t("确认导入", "Import") })) return;
      present = requests.token("actionBusy");
      notice = requests.token("notice");
      update({ memoryActionLoading: "import", memoryError: null });
      const finish = requests.write("items", "vector");
      try {
        const result = await post<MemoryImportResult>("/v1/memory/import/confirm", {
          import_id: preview.import_id, expected_payload_digest: preview.payload_digest,
        });
        finish();
        update({ memoryMessage: t(`已导入 ${result.imported_items} 条，已有 ${result.existing_items} 条。`,
          `Imported ${result.imported_items} items; ${result.existing_items} already existed.`) }, present);
        await fetchMemoryData(true);
      } finally { finish(); }
    } catch (error) { failure(error, ["导入文件无效或无法读取。", "The import file is invalid or unreadable."], notice); }
    finally { update({ memoryActionLoading: null }, present); }
  };

  const selectMemorySettingScope = async (scope: MemorySettingScope, conversationId?: string | null) => {
    if (!owns() || (scope === "conversation" && !conversationId?.trim())) return;
    const selected = { ...latest.current, memorySettingScope: scope, settingsConversationId: scope === "conversation" ? conversationId! : null };
    const domain = settingsDomain(selected);
    requests.selectSettingsDomain(domain);
    const current = requests.token("settings", [domain]);
    const busy = requests.token("settingsBusy");
    const notice = requests.token("notice");
    update({ memorySettingScope: scope, settingsConversationId: selected.settingsConversationId,
      memorySettings: null, memorySettingsSaving: true, memoryError: null });
    try { update({ memorySettings: await read<MemorySettingsResult>(settingsPath(selected, true)) }, current); }
    catch (error) { if (current()) failure(error, SETTINGS_FAILURE, notice); }
    finally { update({ memorySettingsSaving: false }, busy); }
  };

  const clearMemoryScope = async () => {
    if (!owns()) return;
    const scope = latest.current.memoryClearScope;
    const mode = scope === "recent" ? "transcript" : "transcript_and_derived";
    const notice = requests.token("notice");
    try {
      const preview = await read<MemoryClearPreview>(`/v1/memory/clear/preview?mode=${mode}`);
      const warning = t("清空后无法撤销，相关待确认导入也会失效。需要保留的内容请先导出。",
        "Clearing cannot be undone, and affected pending imports will be invalidated. Export anything you need to keep first.");
      if (!await confirm({ title: t("清空记忆", "Clear memory"), message: t(
        `将删除 ${preview.transcript_rows} 条近期记录、${preview.derived_rows} 条派生记忆，并停止 ${preview.pending_jobs} 个后台作业。${warning}确定继续吗？`,
        `This removes ${preview.transcript_rows} recent records, ${preview.derived_rows} derived memories, and stops ${preview.pending_jobs} background jobs. ${warning} Continue?`),
        confirmLabel: t("清空", "Clear"), tone: "danger" })) return;
      await change("action", `clear:${scope}`, ["items", "vector"], () => post<MemoryClearPreview>("/v1/memory/clear/scoped", {
        mode: preview.mode, expected_transcript_rows: preview.transcript_rows, expected_derived_rows: preview.derived_rows,
      }), async (_, present) => {
        update({ memoryMessage: t("清理完成。", "Memory cleared.") }, present);
        await fetchMemoryData(true);
      }, ["记忆清理失败，请稍后重试。", "Memory cleanup failed. Try again shortly."]);
    } catch (error) { failure(error, ["记忆清理预览暂时无法读取。", "The memory cleanup preview could not be loaded."], notice); }
  };

  const saveSettings = async (body: Record<string, unknown>, external: boolean) => {
    if (!owns() || !latest.current.memorySettings) return;
    const state = latest.current;
    const domain = settingsDomain(state);
    const currentWrite = requests.token(`settingsWrite:${domain}`);
    const domains = state.memorySettingScope === "principal" ? [domain, "vector"] : [domain];
    await change("settings", "settings", domains, () => post<MemorySettingsResult>("/v1/memory/settings", {
      scope: state.memorySettingScope,
      conversation_id: state.memorySettingScope === "conversation" ? state.settingsConversationId : undefined,
      expected_revision: state.memorySettings!.revision, ...body,
    }), async (settings) => {
      if (!currentWrite()) return;
      if (state.memorySettingScope === "principal") {
        update(current => ({ memoryVectorStatus: current.memoryVectorStatus
          ? { ...current.memoryVectorStatus, policy_digest: undefined } : null }), currentWrite);
      }
      if (settingsDomain(latest.current) === domain
        && (!latest.current.memorySettings || latest.current.memorySettings.revision <= settings.revision)) {
        update(current => ({ memorySettings: settings,
          memoryOverview: current.memoryOverview ? { ...current.memoryOverview, long_term_enabled: settings.use_memory && settings.generate_memory } : null,
          memoryMessage: external ? t("远程处理范围已更新。", "Remote processing scope updated.")
            : t("记忆设置已立即生效。", "Memory setting is now active.") }), currentWrite);
      }
      if (state.memorySettingScope === "principal") await refreshVectorStatus();
      if (external && currentWrite() && settingsDomain(latest.current) === domain) await fetchMemoryData(true);
    }, SETTINGS_FAILURE);
  };
  const updateMemorySetting = (kind: "use" | "generate", enabled: boolean) => saveSettings({
    [kind === "use" ? "use_mode" : "generate_mode"]: enabled ? "enabled" : "disabled",
  }, false);
  const updateMemoryExternalPolicy = (policy: "exclude" | "evidence_only" | "allow") => saveSettings({ external_context_policy: policy }, true);

  const controlMemoryVector = async (action: "reindex" | "pause" | "resume" | "cancel") => {
    if (!owns() || !latest.current.memoryVectorStatus) return;
    const snapshot = latest.current.memoryVectorStatus;
    await change("action", `vector:${action}`, ["vector"],
      async () => {
        // A device may omit this field, or an account edit may invalidate the
        // cached policy. Read account policy without changing the selected
        // view; never replay a rejected mutation with a refreshed digest.
        const digest = snapshot.policy_digest ?? (await read<MemorySettingsResult>("/v1/memory/settings")).policy_digest;
        return post<MemoryVectorMutationResult>(`/v1/memory/vector/${action}`, { expected_policy_digest: digest });
      }, async (result, present) => {
        update({ memoryMessage: action === "reindex"
          ? t(`已安排重建 ${result.queued_rows} 条搜索索引。`, `Queued ${result.queued_rows} items for search-index rebuild.`)
          : t("搜索索引状态已更新。", "Search-index state updated.") }, present);
        await fetchMemoryData(true);
      }, ["搜索索引操作失败，请稍后重试。", "The search-index action failed. Try again shortly."]);
  };

  const { owner: _owner, settingsConversationId: _conversation, requestedPage: _page, ...publicView } = view;
  return { ...publicView,
    setMemoryClearScope: (scope: MemoryClearScope) => update({ memoryClearScope: scope }),
    fetchMemoryData, fetchMemoryPage, correctMemoryItem, sendMemoryFeedback, deleteMemoryItemWithRevision,
    undoMemoryMutation, exportMemory, importMemory, clearMemoryScope, updateMemorySetting,
    updateMemoryExternalPolicy, selectMemorySettingScope, controlMemoryVector,
  };
}
