import { useState } from "react";

import type {
  ApiResponse,
  MemoryClearResult,
  MemoryDeleteResult,
  MemoryExpireResult,
  MemoryFactItem,
  MemoryOverviewResponse,
  MemoryPreferenceItem,
  MemoryRecentItem,
  MemorySettingsResult,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type MemoryClearScope = "recent" | "preferences" | "facts" | "all";

export interface UseMemoryRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
}

export function useMemoryRuntime({ apiFetch, t }: UseMemoryRuntimeParams) {
  const [memoryOverview, setMemoryOverview] = useState<MemoryOverviewResponse | null>(null);
  const [memoryPreferences, setMemoryPreferences] = useState<MemoryPreferenceItem[]>([]);
  const [memoryFacts, setMemoryFacts] = useState<MemoryFactItem[]>([]);
  const [memoryRecent, setMemoryRecent] = useState<MemoryRecentItem[]>([]);
  const [memoryLoading, setMemoryLoading] = useState(false);
  const [memoryError, setMemoryError] = useState<string | null>(null);
  const [memoryMessage, setMemoryMessage] = useState<string | null>(null);
  const [memoryActionLoading, setMemoryActionLoading] = useState<string | null>(null);
  const [memorySettingsSaving, setMemorySettingsSaving] = useState(false);
  const [memoryClearScope, setMemoryClearScope] = useState<MemoryClearScope>("recent");

  const readApiBody = async <T,>(res: Response, label: string): Promise<T> => {
    const body = (await res.json()) as ApiResponse<T>;
    if (!res.ok || !body.ok || body.data === undefined) {
      throw new Error(body.error || `${label} request failed (${res.status})`);
    }
    return body.data;
  };

  const fetchMemoryData = async (silent = false) => {
    if (!silent) {
      setMemoryLoading(true);
      setMemoryError(null);
    }
    try {
      const [overviewRes, preferencesRes, factsRes, recentRes] = await Promise.all([
        apiFetch("/v1/memory"),
        apiFetch("/v1/memory/preferences"),
        apiFetch("/v1/memory/facts"),
        apiFetch("/v1/memory/recent"),
      ]);
      const [overview, preferences, facts, recent] = await Promise.all([
        readApiBody<MemoryOverviewResponse>(overviewRes, "memory overview"),
        readApiBody<MemoryPreferenceItem[]>(preferencesRes, "memory preferences"),
        readApiBody<MemoryFactItem[]>(factsRes, "memory facts"),
        readApiBody<MemoryRecentItem[]>(recentRes, "memory recent"),
      ]);
      setMemoryOverview(overview);
      setMemoryPreferences(preferences);
      setMemoryFacts(facts);
      setMemoryRecent(recent);
      setMemoryError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      if (!silent) {
        setMemoryLoading(false);
      }
    }
  };

  const deleteMemoryItem = async (id: string) => {
    const confirmed = window.confirm(
      t("确定删除这条记忆吗？删除后不会再用于后续回复。", "Delete this memory item? It will no longer be used in future replies."),
    );
    if (!confirmed) return;
    setMemoryActionLoading(`delete:${id}`);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch(`/v1/memory/${encodeURIComponent(id)}`, { method: "DELETE" });
      const data = await readApiBody<MemoryDeleteResult>(res, "delete memory");
      setMemoryMessage(
        data.deleted
          ? t("已删除这条记忆。", "Memory item deleted.")
          : t("没有找到这条记忆，可能已经被删除。", "Memory item was not found. It may already be deleted."),
      );
      await fetchMemoryData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const expireMemoryItem = async (id: string) => {
    const confirmed = window.confirm(
      t("确定把这条记忆标记为过期吗？过期后不会再主动用于回复。", "Mark this memory item as expired? Expired items will not be actively used in replies."),
    );
    if (!confirmed) return;
    setMemoryActionLoading(`expire:${id}`);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch(`/v1/memory/${encodeURIComponent(id)}/expire`, { method: "POST" });
      const data = await readApiBody<MemoryExpireResult>(res, "expire memory");
      setMemoryMessage(
        data.expired
          ? t("已把这条记忆标记为过期。", "Memory item marked as expired.")
          : t("没有找到这条记忆，可能已经处理过。", "Memory item was not found. It may already have been handled."),
      );
      await fetchMemoryData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const clearMemoryScope = async () => {
    const labelMap: Record<MemoryClearScope, string> = {
      recent: t("近期记录", "recent memories"),
      preferences: t("偏好", "preferences"),
      facts: t("事实卡片", "fact cards"),
      all: t("全部记忆", "all memory data"),
    };
    const confirmed = window.confirm(
      t(
        `确定清空${labelMap[memoryClearScope]}吗？这个操作会影响后续回复使用的记忆。`,
        `Clear ${labelMap[memoryClearScope]}? This affects which memories are used in future replies.`,
      ),
    );
    if (!confirmed) return;
    setMemoryActionLoading(`clear:${memoryClearScope}`);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch("/v1/memory/clear", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ scope: memoryClearScope }),
      });
      const data = await readApiBody<MemoryClearResult>(res, "clear memory");
      setMemoryMessage(
        t(
          `清理完成：近期 ${data.recent_deleted} 条，偏好 ${data.preferences_deleted} 条，事实 ${data.facts_deleted} 条。`,
          `Cleared: ${data.recent_deleted} recent, ${data.preferences_deleted} preferences, ${data.facts_deleted} facts.`,
        ),
      );
      await fetchMemoryData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      setMemoryActionLoading(null);
    }
  };

  const updateMemoryLongTermEnabled = async (enabled: boolean) => {
    setMemorySettingsSaving(true);
    setMemoryError(null);
    setMemoryMessage(null);
    try {
      const res = await apiFetch("/v1/memory/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ long_term_enabled: enabled }),
      });
      const data = await readApiBody<MemorySettingsResult>(res, "memory settings");
      setMemoryOverview((prev) => (prev ? { ...prev, long_term_enabled: data.long_term_enabled } : prev));
      setMemoryMessage(
        data.restart_required
          ? t("记忆设置已保存。重启 RustClaw 后生效。", "Memory setting saved. Restart RustClaw for it to take effect.")
          : t("记忆设置没有变化。", "Memory setting is unchanged."),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setMemoryError(message);
    } finally {
      setMemorySettingsSaving(false);
    }
  };

  return {
    memoryOverview,
    memoryPreferences,
    memoryFacts,
    memoryRecent,
    memoryLoading,
    memoryError,
    memoryMessage,
    memoryActionLoading,
    memorySettingsSaving,
    memoryClearScope,
    setMemoryClearScope,
    fetchMemoryData,
    deleteMemoryItem,
    expireMemoryItem,
    clearMemoryScope,
    updateMemoryLongTermEnabled,
  };
}
