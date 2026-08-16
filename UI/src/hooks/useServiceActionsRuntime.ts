import { useState } from "react";

import { useUiDialog } from "../components/UiDialogProvider";
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
type ServiceAction = "start" | "stop" | "restart" | "reset";

export interface UseServiceActionsRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  onHealthRefresh: () => Promise<void>;
  onConfigRefresh: () => Promise<void>;
}

export function useServiceActionsRuntime({
  apiFetch,
  t,
  onHealthRefresh,
  onConfigRefresh,
}: UseServiceActionsRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const [serviceActionLoading, setServiceActionLoading] = useState<Record<string, boolean>>({});
  const [serviceActionMessage, setServiceActionMessage] = useState<ServiceActionNotice | null>(null);

  const controlService = async (serviceName: ServiceName, action: ServiceAction): Promise<boolean> => {
    if (action === "reset") {
      const confirmed = await showConfirm({
        title: t("重置通信接入", "Reset communication setup"),
        message: t(
          "这会关闭该通信端，清除它保存的凭据、绑定和本地登录状态。其他通信端和 Agent 数据不会受影响。确定继续吗？",
          "This stops the communication service and clears its saved credentials, bindings, and local sign-in state. Other channels and Agent data are not affected. Continue?",
        ),
        confirmLabel: t("确认重置", "Reset"),
        tone: "danger",
      });
      if (!confirmed) return false;
    }
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
        return false;
      }
      setServiceActionMessage({
        tone: "success",
        text: serviceActionSuccessMessage(serviceName, action, t),
      });
      await sleep(800);
      await Promise.all([onHealthRefresh(), onConfigRefresh()]);
      return true;
    } catch {
      setServiceActionMessage({
        tone: "error",
        text: formatServiceActionError(serviceName, action, "service_action_request_failed", t),
      });
      return false;
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
