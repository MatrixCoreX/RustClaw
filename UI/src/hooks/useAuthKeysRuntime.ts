import { useMemo, useState } from "react";

import { copyAuthKeyValue, writeTextToClipboard } from "../lib/auth-keys";
import type { ApiResponse, AuthKeyListItem } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type AuthKeyCopyTarget = number | "new";

export interface UseAuthKeysRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
}

export function useAuthKeysRuntime({ apiFetch, t }: UseAuthKeysRuntimeParams) {
  const [authKeysList, setAuthKeysList] = useState<AuthKeyListItem[]>([]);
  const [authKeysLoading, setAuthKeysLoading] = useState(false);
  const [authKeysError, setAuthKeysError] = useState<string | null>(null);
  const [authKeyCreateLoading, setAuthKeyCreateLoading] = useState(false);
  const [authKeyCreateError, setAuthKeyCreateError] = useState<string | null>(null);
  const [authKeyActionLoading, setAuthKeyActionLoading] = useState<number | null>(null);
  const [authKeyCopyingTarget, setAuthKeyCopyingTarget] = useState<AuthKeyCopyTarget | null>(null);
  const [authKeyCopiedTarget, setAuthKeyCopiedTarget] = useState<AuthKeyCopyTarget | null>(null);
  const [authKeyActionError, setAuthKeyActionError] = useState<string | null>(null);
  const [newlyCreatedKey, setNewlyCreatedKey] = useState<string | null>(null);
  const [webdLoginEditorKeyId, setWebdLoginEditorKeyId] = useState<number | null>(null);
  const [webdLoginUsernameDraft, setWebdLoginUsernameDraft] = useState("");
  const [webdLoginPasswordDraft, setWebdLoginPasswordDraft] = useState("");

  const sortedAuthKeysList = useMemo(
    () =>
      [...authKeysList].sort((a, b) => {
        const aPriority = a.role === "admin" ? 0 : 1;
        const bPriority = b.role === "admin" ? 0 : 1;
        if (aPriority !== bPriority) return aPriority - bPriority;
        return b.created_at.localeCompare(a.created_at);
      }),
    [authKeysList],
  );

  const fetchAuthKeys = async () => {
    setAuthKeysLoading(true);
    setAuthKeysError(null);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch("/v1/admin/auth-keys");
      const body = (await res.json()) as ApiResponse<{ keys: AuthKeyListItem[] }>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `auth key list fetch failed (${res.status})`);
      }
      setAuthKeysList(body.data.keys);
    } catch (err) {
      setAuthKeysError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setAuthKeysLoading(false);
    }
  };

  const createAuthKey = async (role = "user") => {
    setAuthKeyCreateLoading(true);
    setAuthKeyCreateError(null);
    setNewlyCreatedKey(null);
    setAuthKeyCopiedTarget(null);
    try {
      const res = await apiFetch("/v1/admin/auth-keys", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ role }),
      });
      const body = (await res.json()) as ApiResponse<{ user_key: string }>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `auth key create failed (${res.status})`);
      }
      setNewlyCreatedKey(body.data.user_key);
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyCreateError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setAuthKeyCreateLoading(false);
    }
  };

  const fetchFullAuthKey = async (keyId: number) => {
    const res = await apiFetch(`/v1/admin/auth-keys/${keyId}/full`);
    const body = (await res.json()) as ApiResponse<{ user_key: string }>;
    if (!res.ok || !body.ok || !body.data?.user_key) {
      throw new Error(body.error || `full auth key fetch failed (${res.status})`);
    }
    return body.data.user_key;
  };

  const copyAuthKey = async (options: { target: AuthKeyCopyTarget; keyId?: number; plaintextKey?: string | null }) => {
    setAuthKeyActionError(null);
    setAuthKeyCopyingTarget(options.target);
    try {
      await copyAuthKeyValue({
        keyId: options.keyId,
        plaintextKey: options.plaintextKey,
        fetchFullAuthKey,
        writeClipboard: async (value) => {
          await writeTextToClipboard(value);
        },
      });
      setAuthKeyCopiedTarget(options.target);
    } catch (err) {
      setAuthKeyActionError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setAuthKeyCopyingTarget(null);
    }
  };

  const updateAuthKey = async (keyId: number, patch: { role?: string; enabled?: boolean }) => {
    setAuthKeyActionLoading(keyId);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch(`/v1/admin/auth-keys/${keyId}`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(patch),
      });
      const body = (await res.json()) as ApiResponse<{ updated: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `auth key update failed (${res.status})`);
      }
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyActionError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setAuthKeyActionLoading(null);
    }
  };

  const openWebdLoginEditor = (row: AuthKeyListItem) => {
    setAuthKeyActionError(null);
    setWebdLoginEditorKeyId(row.key_id);
    setWebdLoginUsernameDraft(row.webd_username ?? "");
    setWebdLoginPasswordDraft("");
  };

  const closeWebdLoginEditor = () => {
    setWebdLoginEditorKeyId(null);
    setWebdLoginUsernameDraft("");
    setWebdLoginPasswordDraft("");
  };

  const saveWebdLoginEditor = async (row: AuthKeyListItem) => {
    const normalizedUsername = webdLoginUsernameDraft.trim();
    const normalizedPassword = webdLoginPasswordDraft.trim();
    if (!normalizedUsername) {
      setAuthKeyActionError(t("用户名不能为空", "Username is required"));
      return;
    }
    if (!normalizedPassword) {
      setAuthKeyActionError(t("密码不能为空", "Password is required"));
      return;
    }

    setAuthKeyActionLoading(row.key_id);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch("/v1/admin/webd-accounts", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          username: normalizedUsername,
          password: normalizedPassword,
          key_id: row.key_id,
        }),
      });
      const body = (await res.json()) as ApiResponse<{ updated: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `web login save failed (${res.status})`);
      }
      await fetchAuthKeys();
      closeWebdLoginEditor();
    } catch (err) {
      setAuthKeyActionError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setAuthKeyActionLoading(null);
    }
  };

  const deleteAuthKey = async (row: AuthKeyListItem) => {
    const ok = window.confirm(
      t(
        `确认删除 ${row.user_key}？删除后将移除该 Key、关联绑定，以及它对应的用户名密码登录。`,
        `Delete ${row.user_key}? This will remove the key, related bindings, and its username/password login.`,
      ),
    );
    if (!ok) return;
    setAuthKeyActionLoading(row.key_id);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch(`/v1/admin/auth-keys/${row.key_id}`, { method: "DELETE" });
      const body = (await res.json()) as ApiResponse<{ deleted: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `auth key delete failed (${res.status})`);
      }
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyActionError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setAuthKeyActionLoading(null);
    }
  };

  const promptCreateCustomAuthKey = async () => {
    const role = window.prompt(
      t("请输入自定义角色名称，例如 operator / reviewer / finance", "Enter a custom role, such as operator / reviewer / finance"),
      "",
    );
    const normalized = role?.trim();
    if (!normalized) return;
    await createAuthKey(normalized);
  };

  const promptUpdateAuthKeyRole = async (row: AuthKeyListItem) => {
    const role = window.prompt(
      t("请输入新的角色名称。内置推荐：admin / user / guest，也支持自定义。", "Enter a new role. Suggested built-ins: admin / user / guest, but custom values are also allowed."),
      row.role,
    );
    const normalized = role?.trim();
    if (!normalized || normalized === row.role) return;
    await updateAuthKey(row.key_id, { role: normalized });
  };

  const dismissNewlyCreatedKey = () => setNewlyCreatedKey(null);
  const clearAuthKeysList = () => {
    setAuthKeysList([]);
    setNewlyCreatedKey(null);
    setAuthKeyCopiedTarget(null);
    setAuthKeyActionError(null);
    closeWebdLoginEditor();
  };

  return {
    authKeysList,
    sortedAuthKeysList,
    authKeysLoading,
    authKeysError,
    authKeyCreateLoading,
    authKeyCreateError,
    authKeyActionLoading,
    authKeyActionError,
    authKeyCopyingTarget,
    authKeyCopiedTarget,
    newlyCreatedKey,
    webdLoginEditorKeyId,
    webdLoginUsernameDraft,
    webdLoginPasswordDraft,
    setWebdLoginUsernameDraft,
    setWebdLoginPasswordDraft,
    fetchAuthKeys,
    createAuthKey,
    promptCreateCustomAuthKey,
    copyAuthKey,
    dismissNewlyCreatedKey,
    updateAuthKey,
    promptUpdateAuthKeyRole,
    openWebdLoginEditor,
    closeWebdLoginEditor,
    deleteAuthKey,
    saveWebdLoginEditor,
    clearAuthKeysList,
  };
}
