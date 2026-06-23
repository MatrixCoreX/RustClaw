import { useEffect, useState } from "react";
import QRCode from "qrcode";

import {
  fetchFeishuBindSession,
  isFeishuBindTerminalStatus,
  startFeishuBindSession,
  type FeishuBindSessionResponse,
} from "../lib/feishu-bind";
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

export function useFeishuBindRuntime({
  apiFetch,
  t,
  uiAuthReady,
  onConfigRefresh,
  onHealthRefresh,
}: UseFeishuBindRuntimeParams) {
  const [feishuBindLoading, setFeishuBindLoading] = useState(false);
  const [feishuBindError, setFeishuBindError] = useState<string | null>(null);
  const [feishuBindSession, setFeishuBindSession] = useState<FeishuBindSessionResponse | null>(null);
  const [feishuBindQrDataUrl, setFeishuBindQrDataUrl] = useState<string | null>(null);
  const [feishuResetLoading, setFeishuResetLoading] = useState(false);

  const beginFeishuBind = async () => {
    setFeishuBindLoading(true);
    setFeishuBindError(null);
    try {
      const session = await startFeishuBindSession(apiFetch);
      setFeishuBindSession(session);
    } catch (err) {
      setFeishuBindError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setFeishuBindLoading(false);
    }
  };

  const refreshFeishuBindSession = async (sessionId: number, silent = false) => {
    if (!silent) {
      setFeishuBindLoading(true);
      setFeishuBindError(null);
    }
    try {
      const session = await fetchFeishuBindSession(apiFetch, sessionId);
      setFeishuBindSession(session);
      if (session.status === "bound") {
        await onConfigRefresh();
        await onHealthRefresh();
      }
      return session;
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      if (!silent) {
        setFeishuBindError(message);
      }
      return null;
    } finally {
      if (!silent) {
        setFeishuBindLoading(false);
      }
    }
  };

  const resetFeishuSetup = async () => {
    const confirmed = window.confirm(
      t(
        "确认重置飞书接入吗？这会清空飞书配置里的关键凭据，并删除当前 Key 的飞书绑定状态与待绑定会话。",
        "Reset Feishu setup? This clears the Feishu credentials and removes the current key's Feishu bindings and pending setup sessions.",
      ),
    );
    if (!confirmed) return;
    setFeishuResetLoading(true);
    setFeishuBindError(null);
    try {
      const res = await apiFetch(`/v1/admin/feishu/reset`, { method: "POST" });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `feishu reset failed (${res.status})`);
      }
      setFeishuBindSession(null);
      setFeishuBindQrDataUrl(null);
      await onConfigRefresh();
      await onHealthRefresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setFeishuBindError(message);
    } finally {
      setFeishuResetLoading(false);
    }
  };

  useEffect(() => {
    const entryUrl = feishuBindSession?.entry_url?.trim() ?? "";
    if (!entryUrl) {
      setFeishuBindQrDataUrl(null);
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
          setFeishuBindQrDataUrl(url);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setFeishuBindError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
          setFeishuBindQrDataUrl(null);
        }
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [feishuBindSession?.entry_url]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!feishuBindSession) return;
    if (isFeishuBindTerminalStatus(feishuBindSession.status)) return;
    const timer = window.setInterval(() => {
      void refreshFeishuBindSession(feishuBindSession.session_id, true);
    }, 1800);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [uiAuthReady, feishuBindSession?.session_id, feishuBindSession?.status]);

  return {
    feishuBindLoading,
    feishuBindError,
    feishuBindSession,
    feishuBindQrDataUrl,
    feishuResetLoading,
    beginFeishuBind,
    resetFeishuSetup,
  };
}
