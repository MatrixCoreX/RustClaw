import { useCallback, useEffect, useRef, useState } from "react";
import { LoaderCircle, LogOut, RefreshCw } from "lucide-react";
import { useRequestOwner } from "../hooks/useRequestOwner";
import { accountRows, invokeAccountControl, type AccountUse } from "../lib/aipp-accounts";
import type { AippAccountControls, AippAccountSnapshot } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

function platformLabel(platform: string, t: Translate) {
  return ({ douyin: t("抖音", "Douyin"), xiaohongshu: t("小红书", "Xiaohongshu"),
    kuaishou: t("快手", "Kuaishou"), toutiao: t("头条", "Toutiao"), weibo: t("微博", "Weibo"),
    tiktok: "TikTok", youtube: "YouTube", bilibili: t("哔哩哔哩", "Bilibili"),
    wechat_channels: t("微信视频号", "WeChat Channels") } as Record<string, string>)[platform] || platform;
}

export function accountStatusLabel(status: AippAccountSnapshot["status"], t: Translate) {
  return {
    verified: t("最近已验证登录", "Login recently verified"),
    saved_session: t("已保存会话，账号待核实", "Saved session; account unverified"),
    expired: t("登录已过期", "Login expired"),
    signed_out: t("已退出登录", "Signed out"),
    unknown: t("登录状态待核实", "Login status unverified"),
  }[status];
}

export function AippAccountUse({ uses, t }: { uses?: AccountUse[]; t: Translate }) {
  if (!uses?.length) return <p className="mt-2 text-xs text-white/45">{t("本次账号使用情况未记录", "Account use was not recorded for this task")}</p>;
  return <div className="mt-2 space-y-1 text-xs text-white/55">{uses.map((row, index) => {
    const account = accountRows([row.account])[0];
    return <p key={`${row.platform}-${index}`}>
      {platformLabel(row.platform, t)} · {row.mode === "authenticated_browser" ? t("请求携带了登录会话", "Requests included login credentials") : t("无法确认是否使用登录会话", "Login-session use could not be confirmed")}
      {account?.display_name ? ` · ${account.display_name}` : ""}
    </p>;
  })}</div>;
}

export function AippAccountsPanel({ skill, controls, apiFetch, requestScope, t }: {
  skill: string; controls: AippAccountControls; apiFetch: ApiFetch; requestScope: string; t: Translate;
}) {
  const isCurrent = useRequestOwner(requestScope);
  const api = useRef(apiFetch); api.current = apiFetch;
  const [rows, setRows] = useState<AippAccountSnapshot[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const active = useRef<AbortController | null>(null);
  const perform = useCallback(async (operation: "status" | "logout", platform?: string) => {
    if (!isCurrent() || active.current) return;
    const controller = new AbortController(); active.current = controller;
    setBusy(platform || "status"); setError(null); setNotice(null);
    try {
      const updated = await invokeAccountControl((url, init) => api.current(url, init), skill, operation, controller.signal, platform);
      if (controller.signal.aborted || !isCurrent()) return;
      setRows(previous => operation === "status" ? updated : [...previous.filter(row => !updated.some(next => next.platform === row.platform)), ...updated]);
      if (operation === "logout") setNotice(platform || "");
    } catch (cause) {
      if (!controller.signal.aborted && isCurrent()) setError(cause instanceof Error ? cause.message : "aipp_account_operation_failed");
    } finally {
      if (!controller.signal.aborted && isCurrent()) setBusy(null);
      if (active.current === controller) active.current = null;
    }
  }, [isCurrent, skill]);
  useEffect(() => {
    void perform("status");
    return () => { active.current?.abort(); active.current = null; };
  }, [perform, controls.status_capability, controls.logout_capability]);
  return <section className="theme-panel-soft space-y-3 p-3 sm:p-4">
    <div className="flex items-center justify-between gap-3">
      <h2 className="text-sm font-medium text-white/85">{t("平台账号", "Platform accounts")}</h2>
      <button type="button" className="theme-secondary-btn px-3 py-1.5 text-xs" disabled={busy !== null} onClick={() => void perform("status")}>
        {busy === "status" ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}{t("刷新", "Refresh")}
      </button>
    </div>
    <p className="text-xs leading-5 text-white/50">{t("这里只显示本应用保存的账号状态；任务是否使用账号，以每条结果的记录为准。退出保留文件和历史记录。", "These are this app's saved account states. Check each result for actual session use. Signing out keeps files and history.")}</p>
    {error ? <p role="alert" className="text-xs text-amber-200">{error === "account_session_busy" ? t("应用仍在使用登录会话，请先结束相关媒体任务或关闭浏览器，再退出。", "A login session is in use. Finish the related media tasks or close the browser before signing out.") : t("账号操作未完成，请刷新后重试。", "The account operation did not complete. Refresh and retry.")}</p> : null}
    {notice ? <p role="status" className="text-xs text-white/65">{platformLabel(notice, t)} · {t("已退出，后续需要时请重新登录。", "Signed out. Sign in again when needed.")}</p> : null}
    <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">{rows.map(row => <div key={row.platform} className="flex min-w-0 items-center justify-between gap-2 rounded-lg border border-white/15 p-3">
      <div className="min-w-0"><p className="text-sm font-medium text-white/80">{platformLabel(row.platform, t)}</p>
        <p className="mt-1 truncate text-xs text-white/60">{row.display_name || accountStatusLabel(row.status, t)}</p>
        {row.display_name ? <p className="mt-1 text-xs text-white/45">{accountStatusLabel(row.status, t)}</p> : null}
        {row.checked_at ? <p className="mt-1 text-xs text-white/40">{t("最近检查", "Last checked")} · {new Date(row.checked_at * 1000).toLocaleString()}</p> : null}
      </div>
      {row.status !== "signed_out" ? <button type="button" className="theme-secondary-btn shrink-0 px-2 py-1.5 text-xs" disabled={busy !== null} onClick={() => void perform("logout", row.platform)}>
        {busy === row.platform ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <LogOut className="h-3.5 w-3.5" />}{t("退出登录", "Sign out")}
      </button> : null}
    </div>)}</div>
  </section>;
}
