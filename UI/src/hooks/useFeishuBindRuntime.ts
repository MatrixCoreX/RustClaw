import { useEffect, useMemo, useState } from "react";
import QRCode from "qrcode";

import { useUiDialog } from "../components/UiDialogProvider";
import {
  channelBindPollDelayMs,
  channelBindRecoveryGuidance,
  fetchChannelBindSession,
  isFeishuBindTerminalStatus,
  startChannelBindSession,
  type AgentAppChannel,
  type FeishuBindSessionResponse,
} from "../lib/feishu-bind";
import { formatUiError } from "../lib/ui-error";
import type { ApiResponse } from "../types/api";
import { useRequestOwner } from "./useRequestOwner";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type BindRequest = { token: symbol; promise: Promise<void> };

export interface UseFeishuBindRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  uiAuthReady: boolean;
  requestScope: string;
  isAdminIdentity: boolean;
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
  requestScope,
  isAdminIdentity,
  onConfigRefresh,
  onHealthRefresh,
  platform,
}: UseChannelBindRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const isCurrent = useRequestOwner(uiAuthReady && isAdminIdentity ? requestScope : "", platform);
  const requests = useMemo(() => ({
    session: null as FeishuBindSessionResponse | null,
    generation: Symbol("bind-session"),
    begin: null as BindRequest | null,
    read: null as symbol | null,
    reset: null as BindRequest | null,
    resetDispatched: false,
  }), [isCurrent]);
  const [bindLoading, setBindLoading] = useState(false);
  const [bindError, setBindError] = useState<string | null>(null);
  const [bindSession, setBindSession] = useState<FeishuBindSessionResponse | null>(null);
  const [bindQrDataUrl, setBindQrDataUrl] = useState<string | null>(null);
  const [resetLoading, setResetLoading] = useState(false);
  const channelLabel = platform === "lark" ? "Lark" : t("飞书", "Feishu");

  useEffect(() => {
    setBindLoading(false);
    setBindError(null);
    setBindSession(null);
    setBindQrDataUrl(null);
    setResetLoading(false);
  }, [isCurrent]);

  const refreshAcceptedView = async (ownsRequest: () => boolean) => {
    // A secondary read failure cannot undo an accepted binding or reset.
    for (const refresh of [onConfigRefresh, onHealthRefresh]) {
      if (!ownsRequest()) return;
      try { await refresh(); } catch { /* Keep the accepted primary result. */ }
    }
  };
  const validSession = (session: FeishuBindSessionResponse, sessionId = session.session_id) =>
    Number.isSafeInteger(session.session_id) && session.session_id > 0
    && session.session_id === sessionId && session.channel === platform;

  const beginBind = (): Promise<void> => {
    if (!isCurrent() || requests.reset) return Promise.resolve();
    if (requests.begin) return requests.begin.promise;
    const token = Symbol("bind-start");
    const ownsBegin = () => isCurrent() && requests.begin?.token === token;
    requests.generation = Symbol("bind-session");
    requests.read = null;
    setBindLoading(true);
    setBindError(null);
    setBindQrDataUrl(null);
    const promise = Promise.resolve().then(async () => {
      if (!ownsBegin()) return;
      try {
        const session = await startChannelBindSession(apiFetch, platform);
        if (!ownsBegin()) return;
        if (!validSession(session)) throw new Error("channel_bind_session_invalid");
        requests.session = session;
        setBindSession(session);
        if (session.status === "failed") setBindError(channelBindRecoveryGuidance(session.error_text, t));
      } catch (err) {
        if (ownsBegin()) setBindError(channelBindRecoveryGuidance(err, t)
          ?? formatUiError(err, t, `${channelLabel}绑定未能开始。`, `${channelLabel} binding could not start.`));
      } finally {
        if (ownsBegin()) {
          requests.begin = null;
          setBindLoading(false);
        }
      }
    });
    requests.begin = { token, promise };
    return promise;
  };

  const refreshBindSession = async (sessionId: number, generation: symbol) => {
    const ownsSession = () => isCurrent() && requests.generation === generation
      && requests.session?.session_id === sessionId && !requests.begin && !requests.resetDispatched;
    if (!ownsSession() || requests.read) return null;
    const token = Symbol("bind-status");
    requests.read = token;
    const ownsRead = () => ownsSession() && requests.read === token;
    try {
      const session = await fetchChannelBindSession(apiFetch, platform, sessionId);
      if (!ownsRead()) return null;
      if (!validSession(session, sessionId)) throw new Error("channel_bind_session_invalid");
      requests.session = session;
      setBindSession(session);
      if (session.status === "failed") setBindError(channelBindRecoveryGuidance(session.error_text, t));
      if (session.status === "bound") await refreshAcceptedView(ownsRead);
      return session;
    } catch (err) {
      const guidance = channelBindRecoveryGuidance(err, t);
      if (ownsRead() && guidance) setBindError(guidance);
      return null;
    } finally {
      if (ownsRead()) requests.read = null;
    }
  };

  const resetSetup = (): Promise<void> => {
    if (!isCurrent() || requests.begin) return Promise.resolve();
    if (requests.reset) return requests.reset.promise;
    const token = Symbol("bind-reset");
    const ownsReset = () => isCurrent() && requests.reset?.token === token;
    const promise = Promise.resolve().then(async () => {
      if (!ownsReset()) return;
      try {
        const confirmed = await showConfirm({
          title: t(`重置${channelLabel}接入`, `Reset ${channelLabel} setup`),
          message: t(
            `确认重置${channelLabel}接入吗？这会清空关键凭据，并删除当前 Key 的绑定状态与待绑定会话。`,
            `Reset ${channelLabel} setup? This clears its credentials and removes the current key's bindings and pending setup sessions.`,
          ),
          confirmLabel: t("重置", "Reset"),
          tone: "danger",
        });
        if (!confirmed || !ownsReset()) return;
        requests.resetDispatched = true;
        requests.generation = Symbol("bind-session");
        requests.read = null;
        setResetLoading(true);
        setBindError(null);
        setBindQrDataUrl(null);
        const res = await apiFetch(`/v1/admin/${platform}/reset`, { method: "POST" });
        const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
        if (!ownsReset()) return;
        if (!res.ok || !body.ok) throw new Error(body.error || `${platform}_reset_http_${res.status}`);
        requests.session = null;
        setBindSession(null);
        setBindQrDataUrl(null);
        await refreshAcceptedView(ownsReset);
      } catch (err) {
        if (ownsReset()) setBindError(formatUiError(err, t, `${channelLabel}接入未能重置。`, `${channelLabel} setup could not be reset.`));
      } finally {
        if (ownsReset()) {
          requests.reset = null;
          requests.resetDispatched = false;
          setResetLoading(false);
        }
      }
    });
    requests.reset = { token, promise };
    return promise;
  };

  useEffect(() => {
    if (!isCurrent() || requests.begin || requests.resetDispatched
      || requests.session?.session_id !== bindSession?.session_id) return;
    const entryUrl = bindSession?.entry_url?.trim() ?? "";
    if (!entryUrl) {
      setBindQrDataUrl(null);
      return;
    }
    const generation = requests.generation;
    const sessionId = bindSession?.session_id;
    let cancelled = false;
    const ownsEncoder = () => !cancelled && isCurrent() && requests.generation === generation
      && requests.session?.session_id === sessionId && requests.session?.entry_url?.trim() === entryUrl;
    void QRCode.toDataURL(entryUrl, {
      width: 288,
      margin: 1,
      color: {
        dark: "#111827",
        light: "#ffffff",
      },
    })
      .then((url) => {
        if (ownsEncoder()) {
          setBindQrDataUrl(url);
        }
      })
      .catch((err) => {
        if (ownsEncoder()) {
          setBindError(formatUiError(err, t, "无法生成绑定二维码。", "Could not generate the binding QR code."));
          setBindQrDataUrl(null);
        }
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isCurrent, bindSession?.session_id, bindSession?.entry_url, bindLoading, resetLoading]);

  useEffect(() => {
    if (!isCurrent() || requests.begin || requests.resetDispatched) return;
    if (!bindSession) return;
    if (requests.session?.session_id !== bindSession.session_id) return;
    if (isFeishuBindTerminalStatus(bindSession.status)) return;
    const generation = requests.generation;
    let cancelled = false;
    let timer: number | undefined;
    const scheduleNextPoll = (session: FeishuBindSessionResponse) => {
      timer = window.setTimeout(async () => {
        if (cancelled) return;
        const refreshed = await refreshBindSession(session.session_id, generation);
        if (!cancelled && refreshed && !isFeishuBindTerminalStatus(refreshed.status)) {
          scheduleNextPoll(refreshed);
        }
      }, channelBindPollDelayMs(session.poll_interval_seconds));
    };
    scheduleNextPoll(bindSession);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isCurrent, bindSession?.session_id, bindSession?.status, bindLoading, resetLoading]);

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
