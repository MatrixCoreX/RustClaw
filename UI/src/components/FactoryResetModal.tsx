import { AlertCircle, Copy, Loader2, ShieldAlert, Trash2 } from "lucide-react";

import { writeTextToClipboard } from "../lib/auth-keys";
import type { FactoryResetResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface FactoryResetModalProps {
  t: Translate;
  confirmWord: string;
  countdown: number;
  confirmText: string;
  loading: boolean;
  error: string | null;
  result: FactoryResetResponse | null;
  canConfirm: boolean;
  onConfirmTextChange: (value: string) => void;
  onClose: () => void;
  onRunFactoryReset: () => unknown | Promise<unknown>;
}

export function FactoryResetModal({
  t,
  confirmWord,
  countdown,
  confirmText,
  loading,
  error,
  result,
  canConfirm,
  onConfirmTextChange,
  onClose,
  onRunFactoryReset,
}: FactoryResetModalProps) {
  const deletedTotal = result?.database
    ? Object.values(result.database).reduce((sum, value) => sum + (Number.isFinite(value) ? value : 0), 0)
    : 0;

  return (
    <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/70 px-3 py-6 backdrop-blur-sm">
      <div className="w-full max-w-2xl rounded-2xl border border-red-400/30 bg-[#151923] p-5 text-white shadow-2xl sm:p-6">
        <div className="flex items-start gap-3">
          <div className="rounded-2xl border border-red-400/35 bg-red-500/15 p-3 text-red-100">
            <ShieldAlert className="h-6 w-6" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] uppercase tracking-[0.24em] text-red-200/80">
              {t("危险操作", "Danger Zone")}
            </p>
            <h3 className="mt-2 text-lg font-semibold tracking-tight sm:text-2xl">
              {result ? t("恢复出厂设置已完成", "Factory Reset Complete") : t("恢复出厂设置", "Factory Reset")}
            </h3>
            <p className="mt-2 text-sm leading-7 text-white/70">
              {result
                ? t(
                    "旧登录凭据已经失效。请使用下面的新凭据重新进入控制台。",
                    "The old credentials are no longer valid. Use the credentials below to sign in again.",
                  )
                : t(
                    "这会删除所有记忆、所有日志、配置文件里的密钥字段、其它用户 key 与通道绑定，并重置管理员登录。",
                    "This deletes all memories, all logs, secret fields in config files, other user keys, and channel bindings, then resets the admin login.",
                  )}
            </p>
          </div>
        </div>

        {result ? (
          <div className="mt-5 space-y-4">
            <div className="rounded-xl border border-emerald-400/25 bg-emerald-400/10 px-4 py-3 text-sm text-emerald-100">
              {t("新的管理员凭据已生成。", "New admin credentials have been generated.")}
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="rounded-xl border border-white/10 bg-black/25 px-4 py-3">
                <p className="text-[11px] uppercase tracking-widest text-white/45">Admin Key</p>
                <p className="mt-2 break-all font-mono text-sm text-white/90">{result.admin_user_key}</p>
                <button
                  type="button"
                  onClick={() => void writeTextToClipboard(result.admin_user_key)}
                  className="mt-3 inline-flex items-center gap-2 rounded-lg border border-white/15 bg-white/5 px-2.5 py-1.5 text-xs text-white/80 hover:bg-white/10"
                >
                  <Copy className="h-3.5 w-3.5" />
                  {t("复制", "Copy")}
                </button>
              </div>
              <div className="rounded-xl border border-white/10 bg-black/25 px-4 py-3">
                <p className="text-[11px] uppercase tracking-widest text-white/45">{t("Web 登录", "Web Login")}</p>
                <p className="mt-2 text-sm text-white/75">
                  {t("用户名", "Username")}: <span className="font-mono text-white">{result.webd_username}</span>
                </p>
                <p className="mt-1 text-sm text-white/75">
                  {t("密码", "Password")}: <span className="font-mono text-white">{result.webd_password}</span>
                </p>
              </div>
            </div>
            <div className="grid gap-3 text-xs text-white/55 sm:grid-cols-3">
              <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                {t("数据库删除记录", "Database rows deleted")}: <span className="text-white/85">{deletedTotal}</span>
              </div>
              <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                {t("配置字段清空", "Config fields cleared")}: <span className="text-white/85">{result.config?.fields_cleared ?? 0}</span>
              </div>
              <div className="rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                {t("日志文件删除", "Log files deleted")}: <span className="text-white/85">{result.logs?.files_deleted ?? 0}</span>
              </div>
            </div>
            {(result.warnings?.length ?? 0) > 0 ? (
              <div className="max-h-28 overflow-auto rounded-xl border border-amber-400/25 bg-amber-400/10 px-3 py-2 text-xs leading-5 text-amber-100">
                {result.warnings?.map((warning) => <p key={warning}>{warning}</p>)}
              </div>
            ) : null}
          </div>
        ) : (
          <div className="mt-5 space-y-4">
            <div className="grid gap-2 text-sm text-white/72 sm:grid-cols-2">
              {[
                t("删除所有记忆和长期事实", "Delete all memories and long-term facts"),
                t("删除 logs 文件夹下所有日志", "Delete every log under the logs folder"),
                t("清空配置里的 key/token/secret/password", "Clear key/token/secret/password fields in configs"),
                t("删除其它用户 key 与绑定", "Delete other user keys and bindings"),
                t("重置 admin key", "Reset the admin key"),
                t("用户名重置为 rustclaw，密码重置为 123456", "Reset username to rustclaw and password to 123456"),
              ].map((item) => (
                <div key={item} className="flex items-start gap-2 rounded-xl border border-white/10 bg-white/5 px-3 py-2">
                  <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-red-200" />
                  <span>{item}</span>
                </div>
              ))}
            </div>

            <div className="rounded-xl border border-red-400/25 bg-red-500/10 px-4 py-3">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <p className="text-sm font-medium text-red-100">
                  {countdown > 0
                    ? t(`请等待 ${countdown} 秒`, `Wait ${countdown}s`)
                    : t("倒计时结束，可以继续确认。", "Countdown complete. You can continue.")}
                </p>
                <span className="rounded-full border border-red-300/25 bg-black/20 px-3 py-1 font-mono text-sm text-red-100">
                  {countdown}s
                </span>
              </div>
              <label className="mt-3 block space-y-2">
                <span className="text-xs text-red-100/75">
                  {t(`输入 ${confirmWord} 确认执行`, `Type ${confirmWord} to confirm`)}
                </span>
                <input
                  className="theme-input"
                  value={confirmText}
                  onChange={(event) => onConfirmTextChange(event.target.value)}
                  disabled={loading}
                  autoComplete="off"
                />
              </label>
            </div>
          </div>
        )}

        {error ? (
          <p className="mt-4 rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-100">
            {error}
          </p>
        ) : null}

        <div className="mt-5 flex flex-wrap items-center justify-end gap-3">
          <button
            type="button"
            onClick={onClose}
            disabled={loading}
            className="theme-secondary-btn px-4 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
          >
            {result ? t("返回登录", "Back to Sign In") : t("取消", "Cancel")}
          </button>
          {!result ? (
            <button
              type="button"
              onClick={() => void onRunFactoryReset()}
              disabled={!canConfirm}
              className="inline-flex items-center gap-2 rounded-xl border border-red-400/35 bg-red-500/20 px-4 py-2 text-sm font-semibold text-red-50 transition hover:bg-red-500/25 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Trash2 className="h-4 w-4" />}
              {loading ? t("正在恢复", "Resetting") : t("确认恢复出厂设置", "Confirm Factory Reset")}
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
