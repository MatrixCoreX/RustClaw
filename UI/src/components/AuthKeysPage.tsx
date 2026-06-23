import { Fragment } from "react";
import { Check, Copy, Loader2, RefreshCw } from "lucide-react";

import { formatDateOnlyHuman, formatDateTimeHuman } from "../lib/date-format";
import type { AuthKeyListItem } from "../types/api";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;
type TranslateSlash = (text: string) => string;
type AuthKeyCopyTarget = number | "new";

export interface AuthKeysPageProps {
  lang: UiLanguage;
  t: Translate;
  tSlash: TranslateSlash;
  isAdminIdentity: boolean;
  authKeysList: AuthKeyListItem[];
  sortedAuthKeysList: AuthKeyListItem[];
  authKeysLoading: boolean;
  authKeysError: string | null;
  authKeyCreateLoading: boolean;
  authKeyCreateError: string | null;
  authKeyActionLoading: number | null;
  authKeyActionError: string | null;
  authKeyCopyingTarget: AuthKeyCopyTarget | null;
  authKeyCopiedTarget: AuthKeyCopyTarget | null;
  newlyCreatedKey: string | null;
  webdLoginEditorKeyId: number | null;
  webdLoginUsernameDraft: string;
  webdLoginPasswordDraft: string;
  onFetchAuthKeys: () => unknown | Promise<unknown>;
  onCreateAuthKey: (role?: string) => unknown | Promise<unknown>;
  onPromptCreateCustomAuthKey: () => unknown | Promise<unknown>;
  onCopyAuthKey: (options: { target: AuthKeyCopyTarget; keyId?: number; plaintextKey?: string | null }) => unknown | Promise<unknown>;
  onDismissNewlyCreatedKey: () => void;
  onUpdateAuthKey: (keyId: number, patch: { enabled?: boolean }) => unknown | Promise<unknown>;
  onPromptUpdateAuthKeyRole: (row: AuthKeyListItem) => void;
  onOpenWebdLoginEditor: (row: AuthKeyListItem) => void;
  onCloseWebdLoginEditor: () => void;
  onDeleteAuthKey: (row: AuthKeyListItem) => unknown | Promise<unknown>;
  onWebdLoginUsernameDraftChange: (value: string) => void;
  onWebdLoginPasswordDraftChange: (value: string) => void;
  onSaveWebdLoginEditor: (row: AuthKeyListItem) => unknown | Promise<unknown>;
}

function copyButtonText(
  t: Translate,
  target: AuthKeyCopyTarget,
  copyingTarget: AuthKeyCopyTarget | null,
  copiedTarget: AuthKeyCopyTarget | null,
): string {
  if (copyingTarget === target) return t("复制中...", "Copying...");
  if (copiedTarget === target) return t("已复制", "Copied");
  return t("复制 Key", "Copy key");
}

export function AuthKeysPage({
  lang,
  t,
  tSlash,
  isAdminIdentity,
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
  onFetchAuthKeys,
  onCreateAuthKey,
  onPromptCreateCustomAuthKey,
  onCopyAuthKey,
  onDismissNewlyCreatedKey,
  onUpdateAuthKey,
  onPromptUpdateAuthKeyRole,
  onOpenWebdLoginEditor,
  onCloseWebdLoginEditor,
  onDeleteAuthKey,
  onWebdLoginUsernameDraftChange,
  onWebdLoginPasswordDraftChange,
  onSaveWebdLoginEditor,
}: AuthKeysPageProps) {
  const locale = lang === "zh" ? "zh-CN" : "en-US";

  return (
    <div className="space-y-4">
      <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold">{t("账号绑定与 Key 管理", "Account binding and key management")}</h3>
            <p className="mt-2 text-sm text-white/65">
              {t("微信、Telegram 和飞书的快捷接入已经移到通信接入页。这里现在只保留账号绑定、访问 Key 生成与管理。", "Quick WeChat, Telegram, and Feishu setup moved to Communication Setup. This page now keeps account bindings plus access key generation and management.")}
            </p>
          </div>
        </div>
        <div className="mt-4 flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => void onFetchAuthKeys()}
            disabled={authKeysLoading}
            className="theme-topbar-btn px-3 py-2 text-sm"
          >
            {authKeysLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
            {t("刷新列表", "Refresh list")}
          </button>
          {isAdminIdentity ? (
            <>
              <button
                type="button"
                onClick={() => void onCreateAuthKey("user")}
                disabled={authKeyCreateLoading}
                className="theme-accent-btn px-3 py-2 text-sm"
              >
                {authKeyCreateLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {t("生成新 Key（user）", "Generate new key (user)")}
              </button>
              <button
                type="button"
                onClick={() => void onCreateAuthKey("guest")}
                disabled={authKeyCreateLoading}
                className="theme-secondary-btn px-3 py-2 text-sm"
              >
                {authKeyCreateLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {t("生成新 Key（guest）", "Generate new key (guest)")}
              </button>
              <button
                type="button"
                onClick={() => void onPromptCreateCustomAuthKey()}
                disabled={authKeyCreateLoading}
                className="theme-topbar-btn theme-key-create-btn px-3 py-2 text-sm"
              >
                {authKeyCreateLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {t("生成新 Key（自定义角色）", "Generate new key (custom role)")}
              </button>
            </>
          ) : null}
        </div>
        {isAdminIdentity ? (
          <p className="mt-3 rounded-lg border border-sky-400/25 bg-sky-500/10 px-3 py-2 text-sm text-sky-100">
            {t("系统现在只允许 1 个 admin key。为保护记忆和绑定关系，key 一旦生成后不能修改；非 admin 登录后只会看到自己的 key。", "The system now allows only one admin key. To preserve memories and bindings, keys cannot be modified after creation; non-admin users only see their own key.")}
          </p>
        ) : null}
        {!isAdminIdentity ? (
          <p className="mt-3 rounded-lg border border-amber-400/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
            {t("当前不是 admin：这里只显示你自己的 key；你不能新增、禁用、删除，也不能修改当前 key。", "Current key is not admin: only your own key is shown here; you cannot create, disable, delete, or modify the current key.")}
          </p>
        ) : null}
        {authKeysError ? (
          <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{authKeysError}</p>
        ) : null}
        {authKeyCreateError ? (
          <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{authKeyCreateError}</p>
        ) : null}
        {authKeyActionError ? (
          <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{authKeyActionError}</p>
        ) : null}
        {newlyCreatedKey ? (
          <div className="mt-4 rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-4">
            <p className="text-sm font-medium text-emerald-200">{t("新 Key 已生成，请复制保存", "New key generated. Copy and save it.")}</p>
            <p className="mt-2 break-all font-mono text-sm text-white/90">{newlyCreatedKey}</p>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() => void onCopyAuthKey({ target: "new", plaintextKey: newlyCreatedKey })}
                disabled={authKeyCopyingTarget === "new"}
                className="theme-secondary-btn px-3 py-2 text-xs"
              >
                {authKeyCopiedTarget === "new" ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                {copyButtonText(t, "new", authKeyCopyingTarget, authKeyCopiedTarget)}
              </button>
              <button
                type="button"
                onClick={onDismissNewlyCreatedKey}
                className="text-xs text-white/70 underline"
              >
                {t("关闭", "Dismiss")}
              </button>
            </div>
          </div>
        ) : null}
        <div className="mt-4 overflow-hidden rounded-xl border border-white/10 bg-black/20">
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b border-white/10 bg-white/5">
                <th className="px-4 py-3 font-medium text-white/80">{t("Key", "Key")}</th>
                <th className="px-4 py-3 font-medium text-white/80">role</th>
                <th className="px-4 py-3 font-medium text-white/80">{t("网页登录", "Web login")}</th>
                <th className="px-4 py-3 font-medium text-white/80">{t("启用", "Enabled")}</th>
                <th className="px-4 py-3 font-medium text-white/80">{t("创建时间", "Created")}</th>
                <th className="px-4 py-3 font-medium text-white/80">{t("最后使用", "Last used")}</th>
                <th className="px-4 py-3 font-medium text-white/80">{t("操作", "Actions")}</th>
              </tr>
            </thead>
            <tbody>
              {authKeysList.length === 0 && !authKeysLoading ? (
                <tr>
                  <td colSpan={7} className="px-4 py-6 text-center text-white/50">
                    {isAdminIdentity
                      ? t("暂无数据，点击「刷新列表」或「生成新 Key」", "No keys yet. Click Refresh list or Generate new key.")
                      : t("暂无可显示的 key，请点击「刷新列表」", "No visible key yet. Click Refresh list.")}
                  </td>
                </tr>
              ) : (
                sortedAuthKeysList.map((row) => {
                  const editingWebdLogin = webdLoginEditorKeyId === row.key_id;
                  return (
                    <Fragment key={row.key_id}>
                      <tr className="border-b border-white/5">
                        <td className="px-4 py-2 font-mono text-white/85">{row.user_key}</td>
                        <td className="px-4 py-2 text-white/75">{row.role}</td>
                        <td className="px-4 py-2 text-white/75">{row.webd_username || "--"}</td>
                        <td className="px-4 py-2">{row.enabled ? t("是", "Yes") : t("否", "No")}</td>
                        <td className="px-4 py-2 text-white/65">{formatDateOnlyHuman(row.created_at, locale)}</td>
                        <td className="px-4 py-2 text-white/65">{formatDateTimeHuman(row.last_used_at, locale)}</td>
                        <td className="px-4 py-2">
                          {isAdminIdentity ? (
                            <div className="flex flex-wrap items-center gap-2">
                              <button
                                type="button"
                                disabled={authKeyCopyingTarget === row.key_id}
                                className="theme-secondary-btn px-2 py-1 text-xs"
                                onClick={() => void onCopyAuthKey({ target: row.key_id, keyId: row.key_id })}
                              >
                                {authKeyCopiedTarget === row.key_id ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                                {copyButtonText(t, row.key_id, authKeyCopyingTarget, authKeyCopiedTarget)}
                              </button>
                              {row.current_key ? (
                                <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/55">
                                  {t("当前 key 不可修改", "Current key cannot be modified")}
                                </span>
                              ) : (
                                <button
                                  type="button"
                                  disabled={authKeyActionLoading === row.key_id}
                                  className="theme-topbar-btn px-2 py-1 text-xs"
                                  onClick={() => void onUpdateAuthKey(row.key_id, { enabled: !row.enabled })}
                                >
                                  {row.enabled ? t("禁用", "Disable") : t("启用", "Enable")}
                                </button>
                              )}
                              <button
                                type="button"
                                disabled={authKeyActionLoading === row.key_id || row.role === "admin"}
                                className="theme-secondary-btn px-2 py-1 text-xs"
                                onClick={() => onPromptUpdateAuthKeyRole(row)}
                              >
                                {t("修改角色", "Change role")}
                              </button>
                              <button
                                type="button"
                                disabled={authKeyActionLoading === row.key_id}
                                className="theme-secondary-btn px-2 py-1 text-xs"
                                onClick={() => (editingWebdLogin ? onCloseWebdLoginEditor() : onOpenWebdLoginEditor(row))}
                              >
                                {row.webd_username
                                  ? t("修改登录名/密码", "Update username/password")
                                  : t("设置登录名/密码", "Set username/password")}
                              </button>
                              <button
                                type="button"
                                disabled={authKeyActionLoading === row.key_id || row.role === "admin"}
                                className="rounded-md border border-red-500/30 bg-red-500/10 px-2 py-1 text-xs text-red-200 transition hover:bg-red-500/20 disabled:opacity-50"
                                onClick={() => void onDeleteAuthKey(row)}
                              >
                                {t("删除", "Delete")}
                              </button>
                            </div>
                          ) : row.current_key ? (
                            <span className="text-xs text-white/45">{t("当前 key 不可修改", "Current key cannot be modified")}</span>
                          ) : (
                            <span className="text-xs text-white/45">--</span>
                          )}
                        </td>
                      </tr>
                      {isAdminIdentity && editingWebdLogin ? (
                        <tr className="border-b border-white/5 bg-white/[0.03]">
                          <td colSpan={7} className="px-4 py-4">
                            <div className="rounded-xl border border-white/10 bg-black/15 p-4">
                              <div className="flex flex-wrap items-center justify-between gap-3">
                                <div>
                                  <p className="text-sm font-medium text-white/90">
                                    {t("修改登录名/密码", "Update username/password")}
                                  </p>
                                  <p className="mt-1 text-xs text-white/55">
                                    {t(
                                      "为这个 Key 设置网页登录用户名和新密码。用户名会自动转成小写。",
                                      "Set the web login username and a new password for this key. The username will be normalized to lowercase.",
                                    )}
                                  </p>
                                </div>
                                <p className="font-mono text-xs text-white/45">{row.user_key}</p>
                              </div>
                              <div className="mt-4 grid gap-3 md:grid-cols-2">
                                <label className="space-y-2">
                                  <span className="text-xs uppercase tracking-widest text-white/50">{t("登录名", "Username")}</span>
                                  <input
                                    value={webdLoginUsernameDraft}
                                    onChange={(event) => onWebdLoginUsernameDraftChange(event.target.value)}
                                    className="theme-input"
                                    placeholder={t("例如 rustclaw_admin", "For example rustclaw_admin")}
                                  />
                                </label>
                                <label className="space-y-2">
                                  <span className="text-xs uppercase tracking-widest text-white/50">{t("新密码", "New password")}</span>
                                  <input
                                    type="password"
                                    value={webdLoginPasswordDraft}
                                    onChange={(event) => onWebdLoginPasswordDraftChange(event.target.value)}
                                    className="theme-input"
                                    placeholder={t("输入新的登录密码", "Enter a new login password")}
                                  />
                                </label>
                              </div>
                              <div className="mt-4 flex flex-wrap items-center gap-2">
                                <button
                                  type="button"
                                  disabled={authKeyActionLoading === row.key_id}
                                  className="theme-accent-btn px-3 py-2 text-sm"
                                  onClick={() => void onSaveWebdLoginEditor(row)}
                                >
                                  {authKeyActionLoading === row.key_id ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                                  {t("保存登录名/密码", "Save username/password")}
                                </button>
                                <button
                                  type="button"
                                  disabled={authKeyActionLoading === row.key_id}
                                  className="theme-topbar-btn px-3 py-2 text-sm"
                                  onClick={onCloseWebdLoginEditor}
                                >
                                  {t("取消", "Cancel")}
                                </button>
                              </div>
                            </div>
                          </td>
                        </tr>
                      ) : null}
                    </Fragment>
                  );
                })
              )}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
