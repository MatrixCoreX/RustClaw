import { useEffect, useState } from "react";

import { sleep } from "../lib/display-format";
import type { ApiResponse, ServiceActionNotice, WhatsappWebLoginStatus } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseWhatsappWebRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  apiBase: string;
  uiAuthReady: boolean;
  whatsappWebHealthy: boolean;
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
  uiAuthReady,
  whatsappWebHealthy,
  setServiceActionMessage,
}: UseWhatsappWebRuntimeParams) {
  const [waLoginDialogOpen, setWaLoginDialogOpen] = useState(false);
  const [waLoginLoading, setWaLoginLoading] = useState(false);
  const [waLoginError, setWaLoginError] = useState<string | null>(null);
  const [waLoginStatus, setWaLoginStatus] = useState<WhatsappWebLoginStatus | null>(null);
  const [waWebBridgeReachable, setWaWebBridgeReachable] = useState(false);
  const [waLogoutLoading, setWaLogoutLoading] = useState(false);

  const fetchWhatsappWebLoginStatus = async (silent = false) => {
    if (!silent) {
      setWaLoginLoading(true);
      setWaLoginError(null);
    }
    try {
      const res = await apiFetch(`/v1/whatsapp-web/login-status`);
      const body = (await res.json()) as ApiResponse<WhatsappWebLoginStatus>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(whatsappWebRequestError(t, body.error));
      }
      setWaLoginStatus(body.data);
      setWaWebBridgeReachable(true);
      if (!silent) {
        setWaLoginError(null);
      }
    } catch (err) {
      setWaWebBridgeReachable(false);
      const message = err instanceof Error
        ? err.message
        : whatsappWebRequestError(t, "whatsapp_web.login_status_unavailable");
      if (!silent) {
        setWaLoginError(message);
      }
    } finally {
      if (!silent) {
        setWaLoginLoading(false);
      }
    }
  };

  const logoutWhatsappWeb = async () => {
    setWaLogoutLoading(true);
    setWaLoginError(null);
    try {
      const res = await apiFetch(`/v1/whatsapp-web/logout`, {
        method: "POST",
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(whatsappWebRequestError(t, body.error || "whatsapp_web.logout_failed"));
      }
      await sleep(1200);
      await fetchWhatsappWebLoginStatus();
      setServiceActionMessage({
        tone: "success",
        text: t("已发起 WhatsApp Web 退出登录。", "WhatsApp Web logout requested."),
      });
    } catch (err) {
      const message = err instanceof Error
        ? err.message
        : whatsappWebRequestError(t, "whatsapp_web.logout_failed");
      setWaLoginError(message);
    } finally {
      setWaLogoutLoading(false);
    }
  };

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!waLoginDialogOpen) return;
    if (!whatsappWebHealthy) {
      setWaWebBridgeReachable(false);
      setWaLoginError(null);
      return;
    }
    void fetchWhatsappWebLoginStatus();
    const timer = window.setInterval(() => {
      void fetchWhatsappWebLoginStatus(true);
    }, 2000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waLoginDialogOpen, apiBase, uiAuthReady, whatsappWebHealthy]);

  useEffect(() => {
    if (!uiAuthReady) return;
    if (!whatsappWebHealthy) {
      setWaWebBridgeReachable(false);
      return;
    }
    void fetchWhatsappWebLoginStatus(true);
    const timer = window.setInterval(() => {
      void fetchWhatsappWebLoginStatus(true);
    }, 5000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, whatsappWebHealthy]);

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
