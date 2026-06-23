import { useEffect, useState } from "react";

import type {
  ApiResponse,
  WechatLoginStatus,
  WechatQrStartResponse,
  WechatQrWaitResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseWechatRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  apiBase: string;
  uiAuthReady: boolean;
}

export function useWechatRuntime({
  apiFetch,
  t,
  apiBase,
  uiAuthReady,
}: UseWechatRuntimeParams) {
  const [wechatLoginLoading, setWechatLoginLoading] = useState(false);
  const [wechatLoginError, setWechatLoginError] = useState<string | null>(null);
  const [wechatLoginStatus, setWechatLoginStatus] = useState<WechatLoginStatus | null>(null);
  const [wechatSessionKey, setWechatSessionKey] = useState<string | null>(null);
  const [wechatQrStarting, setWechatQrStarting] = useState(false);
  const [wechatQrPreviewRequested, setWechatQrPreviewRequested] = useState(false);

  const fetchWechatLoginStatus = async (silent = false) => {
    if (!silent) {
      setWechatLoginLoading(true);
      setWechatLoginError(null);
    }
    try {
      const res = await apiFetch(`/v1/wechat/login-status`);
      const body = (await res.json()) as ApiResponse<WechatLoginStatus>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `wechat login status failed (${res.status})`);
      }
      setWechatLoginStatus(body.data);
      if (body.data.qr_ready && body.data.session_key) {
        setWechatSessionKey(body.data.session_key);
      } else if (!body.data.qr_ready || body.data.connected) {
        setWechatSessionKey(null);
      }
      if (!silent) {
        setWechatLoginError(null);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      if (!silent) {
        setWechatLoginError(message);
      }
    } finally {
      if (!silent) {
        setWechatLoginLoading(false);
      }
    }
  };

  const startWechatQrLogin = async (force = true) => {
    setWechatQrStarting(true);
    setWechatQrPreviewRequested(true);
    setWechatLoginError(null);
    setWechatSessionKey(null);
    setWechatLoginStatus((prev) => ({
      ...(prev ?? {}),
      connected: false,
      qr_ready: false,
      qrcode_url: null,
      qr_status: "generating",
      message: t("正在生成二维码...", "Generating QR code..."),
      last_error: null,
      status: "qr_generating",
      last_update_ts: Date.now(),
    }));
    try {
      const res = await apiFetch(`/v1/wechat/login-qr/start`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ force }),
      });
      const body = (await res.json()) as ApiResponse<WechatQrStartResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `wechat QR start failed (${res.status})`);
      }
      setWechatSessionKey(body.data.session_key);
      setWechatLoginStatus((prev) => ({
        ...(prev ?? {}),
        connected: false,
        qr_ready: true,
        qr_status: "wait",
        qrcode_url: body.data.qrcode_url,
        message: body.data.message,
        last_error: null,
        status: "qr_ready",
        last_update_ts: Date.now(),
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setWechatLoginError(message);
    } finally {
      setWechatQrStarting(false);
    }
  };

  const pollWechatQrLogin = async (sessionKey: string) => {
    try {
      const res = await apiFetch(`/v1/wechat/login-qr/wait`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_key: sessionKey, timeout_ms: 1500 }),
      });
      const body = (await res.json()) as ApiResponse<WechatQrWaitResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `wechat QR wait failed (${res.status})`);
      }
      if (body.data.connected) {
        setWechatSessionKey(null);
        await fetchWechatLoginStatus(true);
        return;
      }
      if (body.data.qr_status || body.data.message) {
        setWechatLoginStatus((prev) => ({
          ...(prev ?? {}),
          connected: false,
          qr_ready: true,
          qr_status: body.data.qr_status ?? prev?.qr_status ?? "wait",
          message: body.data.message ?? prev?.message,
          status: "qr_ready",
        }));
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setWechatLoginError(message);
    }
  };

  useEffect(() => {
    if (!uiAuthReady) return;
    void fetchWechatLoginStatus(true);
    const timer = window.setInterval(() => {
      void fetchWechatLoginStatus(true);
    }, 5000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!wechatSessionKey) return;
    if (wechatLoginStatus?.connected) return;
    const timer = window.setInterval(() => {
      void pollWechatQrLogin(wechatSessionKey);
      void fetchWechatLoginStatus(true);
    }, 2000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wechatSessionKey, wechatLoginStatus?.connected, apiBase, uiAuthReady]);

  return {
    wechatLoginLoading,
    wechatLoginError,
    wechatLoginStatus,
    wechatQrStarting,
    wechatQrPreviewRequested,
    fetchWechatLoginStatus,
    startWechatQrLogin,
  };
}
