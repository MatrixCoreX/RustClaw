import { useEffect, useState } from "react";
import QRCode from "qrcode";

import { useUiDialog } from "../components/UiDialogProvider";
import {
  fetchChannelBindSession,
  isFeishuBindTerminalStatus,
  startChannelBindSession,
  type AgentAppChannel,
  type FeishuBindSessionResponse,
} from "../lib/feishu-bind";
import { formatUiError } from "../lib/ui-error";
import type { ApiResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseFeishuBindRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  uiAuthReady: boolean;
  onConfigRefresh: () => Promise<void>;
  onHealthRefresh: () => Promise<void>;
}

export interface UseChannelBindRuntimeParams extends UseFeishuBindRuntimeParams {
  platform: AgentAppChannel;
}

export function useChannelBindRuntime({
  apiFetch,
  t,
  uiAuthReady,
  onConfigRefresh,
  onHealthRefresh,
  platform,
}: UseChannelBindRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const [bindLoading, setBindLoading] = useState(false);
  const [bindError, setBindError] = useState<string | null>(null);
  const [bindSession, setBindSession] = useState<FeishuBindSessionResponse | null>(null);
  const [bindQrDataUrl, setBindQrDataUrl] = useState<string | null>(null);
  const [resetLoading, setResetLoading] = useState(false);
  const channelLabel = platform === "lark" ? "Lark" : t("飞书", "Feishu");

  const beginBind = async () => {
    setBindLoading(true);
    setBindError(null);
    try {
      const session = await startChannelBindSession(apiFetch, platform);
      setBindSession(session);
    } catch (err) {
      setBindError(formatUiError(err, t, `${channelLabel}绑定未能开始。`, `${channelLabel} binding could not start.`));
    } finally {
      setBindLoading(false);
    }
  };

  const refreshBindSession = async (sessionId: number, silent = false) => {
    if (!silent) {
      setBindLoading(true);
      setBindError(null);
    }
    try {
      const session = await fetchChannelBindSession(apiFetch, platform, sessionId);
      setBindSession(session);
      if (session.status === "bound") {
        await onConfigRefresh();
        await onHealthRefresh();
      }
      return session;
    } catch (err) {
      if (!silent) {
        setBindError(formatUiError(err, t, `无法刷新${channelLabel}绑定状态。`, `Could not refresh ${channelLabel} binding status.`));
      }
      return null;
    } finally {
      if (!silent) {
        setBindLoading(false);
      }
    }
  };

  const resetSetup = async () => {
    const confirmed = await showConfirm({
      title: t(`重置${channelLabel}接入`, `Reset ${channelLabel} setup`),
      message: t(
        `确认重置${channelLabel}接入吗？这会清空关键凭据，并删除当前 Key 的绑定状态与待绑定会话。`,
        `Reset ${channelLabel} setup? This clears its credentials and removes the current key's bindings and pending setup sessions.`,
      ),
      confirmLabel: t("重置", "Reset"),
      tone: "danger",
    });
    if (!confirmed) return;
    setResetLoading(true);
    setBindError(null);
    try {
      const res = await apiFetch(`/v1/admin/${platform}/reset`, { method: "POST" });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `${platform}_reset_http_${res.status}`);
      }
      setBindSession(null);
      setBindQrDataUrl(null);
      await onConfigRefresh();
      await onHealthRefresh();
    } catch (err) {
      setBindError(formatUiError(err, t, `${channelLabel}接入未能重置。`, `${channelLabel} setup could not be reset.`));
    } finally {
      setResetLoading(false);
    }
  };

  useEffect(() => {
    const entryUrl = bindSession?.entry_url?.trim() ?? "";
    if (!entryUrl) {
      setBindQrDataUrl(null);
      return;
    }
    let cancelled = false;
    void QRCode.toDataURL(entryUrl, {
      width: 288,
      margin: 1,
      color: {
        dark: "#111827",
        light: "#ffffff",
      },
    })
      .then((url) => {
        if (!cancelled) {
          setBindQrDataUrl(url);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setBindError(formatUiError(err, t, "无法生成绑定二维码。", "Could not generate the binding QR code."));
          setBindQrDataUrl(null);
        }
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bindSession?.entry_url]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!bindSession) return;
    if (isFeishuBindTerminalStatus(bindSession.status)) return;
    let cancelled = false;
    let timer: number | undefined;
    const scheduleNextPoll = (session: FeishuBindSessionResponse) => {
      const intervalSeconds = Math.max(1, session.poll_interval_seconds ?? 5);
      timer = window.setTimeout(async () => {
        const refreshed = await refreshBindSession(session.session_id, true);
        if (!cancelled && refreshed && !isFeishuBindTerminalStatus(refreshed.status)) {
          scheduleNextPoll(refreshed);
        }
      }, intervalSeconds * 1000);
    };
    scheduleNextPoll(bindSession);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [uiAuthReady, bindSession?.session_id, bindSession?.status]);

  return {
    bindLoading,
    bindError,
    bindSession,
    bindQrDataUrl,
    resetLoading,
    beginBind,
    resetSetup,
  };
}

export function useFeishuBindRuntime(params: UseFeishuBindRuntimeParams) {
  const runtime = useChannelBindRuntime({ ...params, platform: "feishu" });
  return {
    feishuBindLoading: runtime.bindLoading,
    feishuBindError: runtime.bindError,
    feishuBindSession: runtime.bindSession,
    feishuBindQrDataUrl: runtime.bindQrDataUrl,
    feishuResetLoading: runtime.resetLoading,
    beginFeishuBind: runtime.beginBind,
    resetFeishuSetup: runtime.resetSetup,
  };
}
