import { useState } from "react";

import { sleep } from "../lib/display-format";
import {
  formatServiceActionError,
  serviceActionErrorCode,
  serviceActionSuccessMessage,
} from "../lib/service-actions";
import type { ApiResponse, ServiceActionNotice } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type ServiceName = "telegramd" | "whatsappd" | "whatsapp_webd" | "wechatd" | "feishud" | "larkd";
type ServiceAction = "start" | "stop" | "restart";

export interface UseServiceActionsRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  onHealthRefresh: () => Promise<void>;
}

export function useServiceActionsRuntime({
  apiFetch,
  t,
  onHealthRefresh,
}: UseServiceActionsRuntimeParams) {
  const [serviceActionLoading, setServiceActionLoading] = useState<Record<string, boolean>>({});
  const [serviceActionMessage, setServiceActionMessage] = useState<ServiceActionNotice | null>(null);

  const controlService = async (serviceName: ServiceName, action: ServiceAction) => {
    setServiceActionMessage(null);
    setServiceActionLoading((prev) => ({ ...prev, [serviceName]: true }));
    try {
      const res = await apiFetch(`/v1/services/${serviceName}/${action}`, {
        method: "POST",
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        setServiceActionMessage({
          tone: "error",
          text: formatServiceActionError(serviceName, action, serviceActionErrorCode(body), t),
        });
        return;
      }
      setServiceActionMessage({
        tone: "success",
        text: serviceActionSuccessMessage(serviceName, action, t),
      });
      await sleep(800);
      await onHealthRefresh();
    } catch {
      setServiceActionMessage({
        tone: "error",
        text: formatServiceActionError(serviceName, action, "service_action_request_failed", t),
      });
    } finally {
      setServiceActionLoading((prev) => ({ ...prev, [serviceName]: false }));
    }
  };

  return {
    serviceActionLoading,
    serviceActionMessage,
    setServiceActionMessage,
    controlService,
  };
}
