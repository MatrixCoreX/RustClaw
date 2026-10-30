import { useCallback, useEffect, useMemo, useState, type SetStateAction } from "react";

import { sleep } from "../lib/display-format";
import { useRequestOwner } from "./useRequestOwner";
import type { ApiResponse, ServiceActionNotice, WhatsappWebLoginStatus } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type LoginRequest = { token: symbol; promise: Promise<void> };
type StatusRequest = LoginRequest & { foreground: boolean };

export interface UseWhatsappWebRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  apiBase: string;
  requestScope: string;
  uiAuthReady: boolean;
  whatsappWebHealthy: boolean;
  statusPollingEnabled?: boolean;
  setServiceActionMessage: (notice: ServiceActionNotice | null) => void;
}

export function whatsappWebRequestError(t: Translate, code: string | null | undefined): string {
  switch (code) {
    case "whatsapp_web.not_configured":
      return t("WhatsApp Web 尚未完成配置。", "WhatsApp Web is not configured yet.");
    case "whatsapp_web.login_status_invalid":
      return t("连接状态返回异常，请重启服务。", "The connection returned an invalid status. Restart the service.");
    case "whatsapp_web.logout_failed":
      return t("退出登录失败，请重启服务后重试。", "Sign-out failed. Restart the service and try again.");
    case "whatsapp_web.logout_unavailable":
      return t("暂时无法连接退出登录服务。", "The sign-out service is temporarily unreachable.");
    case "whatsapp_web.login_status_unavailable":
    default:
      return t("暂时无法读取 WhatsApp Web 连接状态。", "The WhatsApp Web connection status is temporarily unavailable.");
  }
}

export function useWhatsappWebRuntime({
  apiFetch,
  t,
  apiBase,
  requestScope,
  uiAuthReady,
  whatsappWebHealthy,
  statusPollingEnabled = true,
  setServiceActionMessage,
}: UseWhatsappWebRuntimeParams) {
  const isCurrent = useRequestOwner(uiAuthReady ? requestScope : "", apiBase);
  const requests = useMemo(() => ({ status: null as StatusRequest | null,
    logout: null as LoginRequest | null, dialogOpen: false }), [isCurrent]);
  const [waLoginDialogOpen, setDialogOpen] = useState(false);
  const [waLoginLoading, setWaLoginLoading] = useState(false);
  const [waLoginError, setWaLoginError] = useState<string | null>(null);
  const [waLoginStatus, setWaLoginStatus] = useState<WhatsappWebLoginStatus | null>(null);
  const [waWebBridgeReachable, setWaWebBridgeReachable] = useState(false);
  const [waLogoutLoading, setWaLogoutLoading] = useState(false);
  useEffect(() => {
    setDialogOpen(false);
    setWaLoginLoading(false);
    setWaLoginError(null);
    setWaLoginStatus(null);
    setWaWebBridgeReachable(false);
    setWaLogoutLoading(false);
  }, [isCurrent]);

  const setWaLoginDialogOpen = useCallback((value: SetStateAction<boolean>) => {
    if (!isCurrent()) return;
    requests.dialogOpen = typeof value === "function" ? value(requests.dialogOpen) : value;
    setDialogOpen(requests.dialogOpen);
  }, [isCurrent, requests]);

  const readLoginStatus = (silent = false, logoutToken?: symbol): Promise<void> => {
    if (!isCurrent() || (requests.logout && requests.logout.token !== logoutToken)) return Promise.resolve();
    if (requests.status) {
      if (!silent) {
        requests.status.foreground = true;
        setWaLoginLoading(true);
        setWaLoginError(null);
      }
      return requests.status.promise;
    }
    const token = Symbol("whatsapp-status");
    const ownsRead = () => isCurrent() && requests.status?.token === token;
    if (!silent) {
      setWaLoginLoading(true);
      setWaLoginError(null);
    }
    const promise = Promise.resolve().then(async () => {
      if (!ownsRead()) return;
      try {
        const res = await apiFetch(`/v1/whatsapp-web/login-status`);
        const body = (await res.json()) as ApiResponse<WhatsappWebLoginStatus>;
        if (!ownsRead()) return;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(whatsappWebRequestError(t, body.error));
        }
        setWaLoginStatus(body.data);
        setWaWebBridgeReachable(true);
        if (requests.status?.foreground) setWaLoginError(null);
      } catch (err) {
        if (!ownsRead()) return;
        setWaWebBridgeReachable(false);
        const message = err instanceof Error
          ? err.message
          : whatsappWebRequestError(t, "whatsapp_web.login_status_unavailable");
        if (requests.status?.foreground) setWaLoginError(message);
      } finally {
        if (ownsRead()) {
          if (requests.status?.foreground) setWaLoginLoading(false);
          requests.status = null;
        }
      }
    });
    requests.status = { token, promise, foreground: !silent };
    return promise;
  };
  const fetchWhatsappWebLoginStatus = (silent = false) => readLoginStatus(silent);

  const logoutWhatsappWeb = (): Promise<void> => {
    if (!isCurrent()) return Promise.resolve();
    if (requests.logout) return requests.logout.promise;
    const token = Symbol("whatsapp-logout");
    const ownsLogout = () => isCurrent() && requests.logout?.token === token;
    requests.status = null;
    setWaLoginLoading(false);
    setWaLogoutLoading(true);
    setWaLoginError(null);
    const promise = Promise.resolve().then(async () => {
      if (!ownsLogout()) return;
      try {
        try {
          const res = await apiFetch(`/v1/whatsapp-web/logout`, { method: "POST" });
          const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
          if (!ownsLogout()) return;
          if (!res.ok || !body.ok) {
            throw new Error(whatsappWebRequestError(t, body.error || "whatsapp_web.logout_failed"));
          }
        } catch (err) {
          if (ownsLogout()) setWaLoginError(err instanceof Error
            ? err.message : whatsappWebRequestError(t, "whatsapp_web.logout_failed"));
          return;
        }
        await sleep(1200);
        if (!ownsLogout()) return;
        setServiceActionMessage({
          tone: "success",
          text: t("已发起 WhatsApp Web 退出登录。", "WhatsApp Web logout requested."),
        });
        // The accepted logout is independent of this secondary status read.
        await Promise.allSettled([readLoginStatus(false, token)]);
      } finally {
        if (ownsLogout()) {
          requests.logout = null;
          setWaLogoutLoading(false);
        }
      }
    });
    requests.logout = { token, promise };
    return promise;
  };

  useEffect(() => {
    if (!uiAuthReady || !statusPollingEnabled) return;
    if (!isCurrent() || waLoginDialogOpen !== requests.dialogOpen) return;
    if (!waLoginDialogOpen) return;
    if (!whatsappWebHealthy) {
      setWaWebBridgeReachable(false);
      setWaLoginError(null);
      return;
    }
    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") void fetchWhatsappWebLoginStatus(true);
    };
    void fetchWhatsappWebLoginStatus();
    const timer = window.setInterval(refreshWhenVisible, 2000);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waLoginDialogOpen, apiBase, uiAuthReady, whatsappWebHealthy, statusPollingEnabled, isCurrent]);

  useEffect(() => {
    if (!uiAuthReady || !statusPollingEnabled) return;
    if (!isCurrent()) return;
    if (waLoginDialogOpen) return;
    if (!whatsappWebHealthy) {
      setWaWebBridgeReachable(false);
      return;
    }
    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") void fetchWhatsappWebLoginStatus(true);
    };
    refreshWhenVisible();
    const timer = window.setInterval(refreshWhenVisible, 5000);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, whatsappWebHealthy, statusPollingEnabled, waLoginDialogOpen, isCurrent]);

  return {
    waLoginDialogOpen,
    setWaLoginDialogOpen,
    waLoginLoading,
    waLoginError,
    waLoginStatus,
    waWebBridgeReachable,
    waLogoutLoading,
    fetchWhatsappWebLoginStatus,
    logoutWhatsappWeb,
  };
}
