import { useEffect, useState } from "react";

import type {
  ApiResponse,
  WechatLoginStatus,
  WechatQrStartResponse,
  WechatQrWaitResponse,
} from "../types/api";
import { formatUiError } from "../lib/ui-error";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export function wechatLoginRequestError(
  t: Translate,
  code: string | null | undefined,
  fallbackZh: string,
  fallbackEn: string,
): string {
  switch (code) {
    case "wechat.login_session_in_use":
      return t(
        "另一个用户正在完成微信扫码，请稍后再试。",
        "Another user is completing WeChat QR sign-in. Try again shortly.",
      );
    case "wechat.login_session_expired":
    case "wechat.login_session_not_ready":
      return t(
        "本次二维码会话已失效，请重新生成二维码。",
        "This QR session is no longer valid. Generate a new QR code.",
      );
    case "wechat.login_session_owner_mismatch":
      return t(
        "本次二维码不属于当前登录用户，请重新生成。",
        "This QR code belongs to another signed-in user. Generate a new one.",
      );
    case "wechat.confirmed_identity_missing":
    case "wechat.auto_bind_failed":
      return t(
        "微信已确认登录，但自动绑定未完成，请重新扫码。",
        "WeChat confirmed sign-in, but automatic binding did not finish. Scan again.",
      );
    default:
      return t(fallbackZh, fallbackEn);
  }
}

export function wechatLoginSessionRequiresRestart(code: string | null | undefined): boolean {
  return code === "wechat.login_session_expired"
    || code === "wechat.login_session_not_ready"
    || code === "wechat.login_session_owner_mismatch";
}

export interface UseWechatRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  apiBase: string;
  uiAuthReady: boolean;
  enabled: boolean;
  serviceHealthy: boolean;
}

export function useWechatRuntime({
  apiFetch,
  t,
  apiBase,
  uiAuthReady,
  enabled,
  serviceHealthy,
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
        throw new Error(body.error || `wechat_login_status_http_${res.status}`);
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
      const message = formatUiError(err, t, "微信登录状态暂时无法读取。", "WeChat sign-in status is temporarily unavailable.");
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
        throw new Error(wechatLoginRequestError(
          t,
          body.error,
          "微信登录二维码生成失败，请稍后重试。",
          "The WeChat sign-in QR code could not be generated. Try again shortly.",
        ));
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
      const message = formatUiError(err, t, "微信登录二维码生成失败，请稍后重试。", "The WeChat sign-in QR code could not be generated. Try again shortly.");
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
        if (wechatLoginSessionRequiresRestart(body.error)) {
          setWechatSessionKey(null);
        }
        throw new Error(wechatLoginRequestError(
          t,
          body.error,
          "微信登录确认失败，请刷新二维码后重试。",
          "WeChat sign-in confirmation failed. Refresh the QR code and try again.",
        ));
      }
      if (body.data.connected) {
        setWechatSessionKey(null);
        await fetchWechatLoginStatus(true);
        return;
      }
      if (body.data.qr_status === "expired") {
        setWechatSessionKey(null);
        setWechatLoginStatus((prev) => ({
          ...(prev ?? {}),
          connected: false,
          qr_ready: false,
          qrcode_url: null,
          qr_status: "expired",
          message: body.data?.message ?? prev?.message,
          status: "qr_expired",
        }));
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
      const message = formatUiError(err, t, "微信登录确认失败，请刷新二维码后重试。", "WeChat sign-in confirmation failed. Refresh the QR code and try again.");
      setWechatLoginError(message);
    }
  };

  useEffect(() => {
    if (!uiAuthReady || !enabled || !serviceHealthy) {
      setWechatLoginStatus(null);
      setWechatSessionKey(null);
      return;
    }
    void fetchWechatLoginStatus(true);
    const timer = window.setInterval(() => {
      void fetchWechatLoginStatus(true);
    }, 5000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, enabled, serviceHealthy]);

  useEffect(() => {
    if (!uiAuthReady || !enabled || !serviceHealthy) return;
    if (!wechatSessionKey) return;
    if (wechatLoginStatus?.connected) return;
    const timer = window.setInterval(() => {
      void pollWechatQrLogin(wechatSessionKey);
      void fetchWechatLoginStatus(true);
    }, 2000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wechatSessionKey, wechatLoginStatus?.connected, apiBase, uiAuthReady, enabled, serviceHealthy]);

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
