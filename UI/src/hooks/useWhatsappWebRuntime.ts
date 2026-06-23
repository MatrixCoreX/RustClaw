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
        throw new Error(body.error || `whatsapp web login status failed (${res.status})`);
      }
      setWaLoginStatus(body.data);
      setWaWebBridgeReachable(true);
      if (!silent) {
        setWaLoginError(null);
      }
    } catch (err) {
      setWaWebBridgeReachable(false);
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
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
        throw new Error(body.error || `whatsapp web logout failed (${res.status})`);
      }
      await sleep(1200);
      await fetchWhatsappWebLoginStatus();
      setServiceActionMessage({
        tone: "success",
        text: t("已发起 WhatsApp Web 退出登录。", "WhatsApp Web logout requested."),
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
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
