import { useMemo, useState } from "react";

import { useUiDialog } from "../components/UiDialogProvider";
import { copyAuthKeyValue, writeTextToClipboard } from "../lib/auth-keys";
import { formatUiError } from "../lib/ui-error";
import type { ApiResponse, AuthKeyListItem, WebdSessionListItem } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type AuthKeyCopyTarget = number | "new";

export interface UseAuthKeysRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
}

export function useAuthKeysRuntime({ apiFetch, t }: UseAuthKeysRuntimeParams) {
  const { confirm: showConfirm, prompt: showPrompt } = useUiDialog();
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
  const [webdSessions, setWebdSessions] = useState<WebdSessionListItem[]>([]);
  const [webdSessionsLoading, setWebdSessionsLoading] = useState(false);
  const [webdSessionsError, setWebdSessionsError] = useState<string | null>(null);
  const [webdSessionRevoking, setWebdSessionRevoking] = useState<string | null>(null);

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
        throw new Error(body.error || `auth_key_list_fetch_http_${res.status}`);
      }
      setAuthKeysList(body.data.keys);
    } catch (err) {
      setAuthKeysError(formatUiError(err, t, "访问 Key 暂时无法读取。", "Access keys are temporarily unavailable."));
    } finally {
      setAuthKeysLoading(false);
    }
  };

  const fetchWebdSessions = async () => {
    setWebdSessionsLoading(true);
    setWebdSessionsError(null);
    try {
      const res = await apiFetch("/webd/sessions");
      const body = (await res.json()) as ApiResponse<{ sessions: WebdSessionListItem[] }>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `web_session_list_fetch_http_${res.status}`);
      }
      setWebdSessions(body.data.sessions);
    } catch (err) {
      setWebdSessionsError(formatUiError(err, t, "Web 会话暂时无法读取。", "Web sessions are temporarily unavailable."));
    } finally {
      setWebdSessionsLoading(false);
    }
  };

  const revokeWebdSession = async (session: WebdSessionListItem) => {
    const ok = await showConfirm({
      title: t("撤销 Web 会话", "Revoke web session"),
      message: session.current
        ? t("撤销当前会话后需要重新登录。", "Revoking the current session requires signing in again.")
        : t(`撤销 ${session.username} 的这个 Web 会话？`, `Revoke this web session for ${session.username}?`),
      confirmLabel: t("撤销", "Revoke"),
      tone: "danger",
    });
    if (!ok) return;
    setWebdSessionRevoking(session.session_handle);
    setWebdSessionsError(null);
    try {
      const res = await apiFetch("/webd/sessions/revoke", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_handle: session.session_handle }),
      });
      const body = (await res.json()) as ApiResponse<{ revoked: boolean }>;
      if (!res.ok || !body.ok || !body.data?.revoked) {
        throw new Error(body.error || `web_session_revoke_http_${res.status}`);
      }
      await fetchWebdSessions();
    } catch (err) {
      setWebdSessionsError(formatUiError(err, t, "Web 会话撤销失败，请稍后重试。", "The web session could not be revoked. Try again shortly."));
    } finally {
      setWebdSessionRevoking(null);
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
        throw new Error(body.error || `auth_key_create_http_${res.status}`);
      }
      setNewlyCreatedKey(body.data.user_key);
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyCreateError(formatUiError(err, t, "访问 Key 创建失败，请稍后重试。", "The access key could not be created. Try again shortly."));
    } finally {
      setAuthKeyCreateLoading(false);
    }
  };

  const fetchFullAuthKey = async (keyId: number) => {
    const res = await apiFetch(`/v1/admin/auth-keys/${keyId}/full`);
    const body = (await res.json()) as ApiResponse<{ user_key: string }>;
    if (!res.ok || !body.ok || !body.data?.user_key) {
      throw new Error(body.error || `full_auth_key_fetch_http_${res.status}`);
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
      setAuthKeyActionError(formatUiError(err, t, "访问 Key 复制失败。", "The access key could not be copied."));
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
        throw new Error(body.error || `auth_key_update_http_${res.status}`);
      }
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyActionError(formatUiError(err, t, "访问 Key 更新失败，请稍后重试。", "The access key could not be updated. Try again shortly."));
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
        throw new Error(body.error || `web_login_save_http_${res.status}`);
      }
      await fetchAuthKeys();
      closeWebdLoginEditor();
    } catch (err) {
      setAuthKeyActionError(formatUiError(err, t, "网页登录设置保存失败，请稍后重试。", "The web sign-in settings could not be saved. Try again shortly."));
    } finally {
      setAuthKeyActionLoading(null);
    }
  };

  const deleteAuthKey = async (row: AuthKeyListItem) => {
    const ok = await showConfirm({
      title: t("删除访问 Key", "Delete access key"),
      message: t(
        `确认删除 ${row.user_key}？删除后将移除该 Key、关联绑定，以及它对应的用户名密码登录。`,
        `Delete ${row.user_key}? This will remove the key, related bindings, and its username/password login.`,
      ),
      confirmLabel: t("删除", "Delete"),
      tone: "danger",
    });
    if (!ok) return;
    setAuthKeyActionLoading(row.key_id);
    setAuthKeyActionError(null);
    try {
      const res = await apiFetch(`/v1/admin/auth-keys/${row.key_id}`, { method: "DELETE" });
      const body = (await res.json()) as ApiResponse<{ deleted: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `auth_key_delete_http_${res.status}`);
      }
      await fetchAuthKeys();
    } catch (err) {
      setAuthKeyActionError(formatUiError(err, t, "访问 Key 删除失败，请稍后重试。", "The access key could not be deleted. Try again shortly."));
    } finally {
      setAuthKeyActionLoading(null);
    }
  };

  const promptCreateCustomAuthKey = async () => {
    const role = await showPrompt({
      title: t("创建自定义角色 Key", "Create a custom-role key"),
      message: t("请输入自定义角色名称，例如 operator / reviewer / finance。", "Enter a custom role, such as operator / reviewer / finance."),
      inputLabel: t("角色名称", "Role name"),
      placeholder: "operator",
      confirmLabel: t("创建", "Create"),
    });
    const normalized = role?.trim();
    if (!normalized) return;
    await createAuthKey(normalized);
  };

  const promptUpdateAuthKeyRole = async (row: AuthKeyListItem) => {
    const role = await showPrompt({
      title: t("修改 Key 角色", "Change key role"),
      message: t("请输入新的角色名称。内置推荐：admin / user / guest，也支持自定义。", "Enter a new role. Suggested built-ins: admin / user / guest, but custom values are also allowed."),
      inputLabel: t("角色名称", "Role name"),
      initialValue: row.role,
      confirmLabel: t("保存", "Save"),
    });
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
    setWebdSessions([]);
    setWebdSessionsError(null);
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
    webdSessions,
    webdSessionsLoading,
    webdSessionsError,
    webdSessionRevoking,
    setWebdLoginUsernameDraft,
    setWebdLoginPasswordDraft,
    fetchAuthKeys,
    fetchWebdSessions,
    revokeWebdSession,
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
