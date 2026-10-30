import { useEffect, useMemo, useState } from "react";

import type {
  ApiResponse,
  WechatLoginStatus,
  WechatQrStartResponse,
  WechatQrWaitResponse,
} from "../types/api";
import { formatUiError } from "../lib/ui-error";
import { useRequestOwner } from "./useRequestOwner";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type LoginRequest = { token: symbol; promise: Promise<void> };
type StatusRequest = LoginRequest & { foreground: boolean };

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
  requestScope: string;
  uiAuthReady: boolean;
  enabled: boolean;
  serviceHealthy: boolean;
  statusPollingEnabled?: boolean;
}

export function useWechatRuntime({
  apiFetch,
  t,
  apiBase,
  requestScope,
  uiAuthReady,
  enabled,
  serviceHealthy,
  statusPollingEnabled = true,
}: UseWechatRuntimeParams) {
  const isCurrent = useRequestOwner(uiAuthReady ? requestScope : "", apiBase);
  const requests = useMemo(() => ({
    status: null as StatusRequest | null,
    start: null as LoginRequest | null,
    poll: null as LoginRequest | null,
    sessionKey: null as string | null,
  }), [isCurrent]);
  const [wechatLoginLoading, setWechatLoginLoading] = useState(false);
  const [wechatLoginError, setWechatLoginError] = useState<string | null>(null);
  const [wechatLoginStatus, setWechatLoginStatus] = useState<WechatLoginStatus | null>(null);
  const [wechatSessionKey, setWechatSessionKey] = useState<string | null>(null);
  const [wechatQrStarting, setWechatQrStarting] = useState(false);
  const [wechatQrPreviewRequested, setWechatQrPreviewRequested] = useState(false);
  useEffect(() => {
    setWechatLoginLoading(false);
    setWechatLoginError(null);
    setWechatLoginStatus(null);
    setWechatSessionKey(null);
    setWechatQrStarting(false);
    setWechatQrPreviewRequested(false);
  }, [isCurrent]);

  const fetchWechatLoginStatus = (silent = false): Promise<void> => {
    if (!isCurrent() || requests.start) return Promise.resolve();
    if (requests.status) {
      if (!silent) {
        requests.status.foreground = true;
        setWechatLoginLoading(true);
        setWechatLoginError(null);
      }
      return requests.status.promise;
    }
    const token = Symbol("wechat-status");
    const ownsRead = () => isCurrent() && requests.status?.token === token;
    if (!silent) {
      setWechatLoginLoading(true);
      setWechatLoginError(null);
    }
    const promise = Promise.resolve().then(async () => {
      if (!ownsRead()) return;
      try {
        const res = await apiFetch(`/v1/wechat/login-status`);
        const body = (await res.json()) as ApiResponse<WechatLoginStatus>;
        if (!ownsRead()) return;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(body.error || `wechat_login_status_http_${res.status}`);
        }
        setWechatLoginStatus(body.data);
        let sessionKey = requests.sessionKey;
        if (body.data.connected || !body.data.qr_ready) sessionKey = null;
        else if (body.data.session_key) sessionKey = body.data.session_key;
        if (sessionKey !== requests.sessionKey) requests.poll = null;
        requests.sessionKey = sessionKey;
        setWechatSessionKey(sessionKey);
        if (requests.status?.foreground) setWechatLoginError(null);
      } catch (err) {
        if (ownsRead() && requests.status?.foreground) {
          setWechatLoginError(formatUiError(err, t, "微信登录状态暂时无法读取。", "WeChat sign-in status is temporarily unavailable."));
        }
      } finally {
        if (ownsRead()) {
          if (requests.status?.foreground) setWechatLoginLoading(false);
          requests.status = null;
        }
      }
    });
    requests.status = { token, promise, foreground: !silent };
    return promise;
  };

  const startWechatQrLogin = (force = true): Promise<void> => {
    if (!isCurrent()) return Promise.resolve();
    if (requests.start) return requests.start.promise;
    const token = Symbol("wechat-start");
    const ownsStart = () => isCurrent() && requests.start?.token === token;
    requests.status = null;
    requests.poll = null;
    requests.sessionKey = null;
    setWechatLoginLoading(false);
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
    const promise = Promise.resolve().then(async () => {
      if (!ownsStart()) return;
      try {
        const res = await apiFetch(`/v1/wechat/login-qr/start`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ force }),
        });
        const body = (await res.json()) as ApiResponse<WechatQrStartResponse>;
        if (!ownsStart()) return;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(wechatLoginRequestError(
            t,
            body.error,
            "微信登录二维码生成失败，请稍后重试。",
            "The WeChat sign-in QR code could not be generated. Try again shortly.",
          ));
        }
        requests.sessionKey = body.data.session_key;
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
        if (ownsStart()) {
          setWechatLoginError(formatUiError(err, t, "微信登录二维码生成失败，请稍后重试。", "The WeChat sign-in QR code could not be generated. Try again shortly."));
        }
      } finally {
        if (ownsStart()) {
          requests.start = null;
          setWechatQrStarting(false);
        }
      }
    });
    requests.start = { token, promise };
    return promise;
  };

  const pollWechatQrLogin = (sessionKey: string): Promise<void> => {
    if (!isCurrent() || requests.start || requests.sessionKey !== sessionKey) return Promise.resolve();
    if (requests.poll) return requests.poll.promise;
    const token = Symbol("wechat-poll");
    const ownsPoll = () => isCurrent() && requests.poll?.token === token && requests.sessionKey === sessionKey;
    const promise = Promise.resolve().then(async () => {
      if (!ownsPoll()) return;
      try {
        const res = await apiFetch(`/v1/wechat/login-qr/wait`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ session_key: sessionKey, timeout_ms: 1500 }),
        });
        const body = (await res.json()) as ApiResponse<WechatQrWaitResponse>;
        if (!ownsPoll()) return;
        if (!res.ok || !body.ok || !body.data) {
          if (wechatLoginSessionRequiresRestart(body.error)) {
            requests.sessionKey = null;
            requests.status = null;
            setWechatLoginLoading(false);
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
          const confirmation = body.data;
          const foreground = requests.status?.foreground === true;
          // A pre-confirmation status read cannot establish the new binding.
          requests.status = null;
          requests.sessionKey = null;
          setWechatLoginLoading(false);
          setWechatSessionKey(null);
          setWechatLoginStatus((prev) => ({
            ...(prev ?? {}),
            connected: confirmation.connected,
            provider_connected: confirmation.provider_connected,
            current_user_bound: confirmation.current_user_bound,
            binding_status: confirmation.binding_status,
            qr_ready: false,
            qr_status: null,
            qrcode_url: null,
            message: confirmation.message ?? null,
            status: "connected",
          }));
          await fetchWechatLoginStatus(!foreground);
          return;
        }
        if (body.data.qr_status === "expired") {
          requests.sessionKey = null;
          requests.status = null;
          setWechatLoginLoading(false);
          setWechatSessionKey(null);
          setWechatLoginStatus((prev) => ({
            ...(prev ?? {}), connected: false, qr_ready: false, qrcode_url: null, qr_status: "expired",
            message: body.data?.message ?? prev?.message, status: "qr_expired",
          }));
          return;
        }
        if (body.data.qr_status || body.data.message) {
          setWechatLoginStatus((prev) => ({
            ...(prev ?? {}), connected: false, qr_ready: true,
            qr_status: body.data.qr_status ?? prev?.qr_status ?? "wait",
            message: body.data.message ?? prev?.message, status: "qr_ready",
          }));
        }
      } catch (err) {
        if (isCurrent() && requests.poll?.token === token) {
          setWechatLoginError(formatUiError(err, t, "微信登录确认失败，请刷新二维码后重试。", "WeChat sign-in confirmation failed. Refresh the QR code and try again."));
        }
      } finally {
        if (isCurrent() && requests.poll?.token === token) requests.poll = null;
      }
    });
    requests.poll = { token, promise };
    return promise;
  };

  useEffect(() => {
    if (!uiAuthReady || !enabled || !serviceHealthy || !statusPollingEnabled) {
      requests.status = null;
      requests.poll = null;
      requests.sessionKey = null;
      setWechatLoginLoading(false);
      setWechatLoginStatus(null);
      setWechatSessionKey(null);
      return;
    }
    if (wechatSessionKey !== requests.sessionKey) return;
    if (wechatSessionKey && !wechatLoginStatus?.connected) return;
    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") void fetchWechatLoginStatus(true);
    };
    refreshWhenVisible();
    const timer = window.setInterval(refreshWhenVisible, 5000);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, enabled, serviceHealthy, statusPollingEnabled, wechatSessionKey, wechatLoginStatus?.connected, isCurrent]);

  useEffect(() => {
    if (!uiAuthReady || !enabled || !serviceHealthy || !statusPollingEnabled) return;
    if (!wechatSessionKey) return;
    if (wechatSessionKey !== requests.sessionKey) return;
    if (wechatLoginStatus?.connected) return;
    const refreshWhenVisible = () => {
      if (document.visibilityState !== "visible") return;
      void pollWechatQrLogin(wechatSessionKey);
      void fetchWechatLoginStatus(true);
    };
    refreshWhenVisible();
    const timer = window.setInterval(refreshWhenVisible, 2000);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wechatSessionKey, wechatLoginStatus?.connected, apiBase, uiAuthReady, enabled, serviceHealthy, statusPollingEnabled, isCurrent]);

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
