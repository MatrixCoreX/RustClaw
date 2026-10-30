import { useCallback, useEffect, useMemo, useState } from "react";

import { useUiDialog } from "../components/UiDialogProvider";
import { sleep } from "../lib/display-format";
import { useRequestOwner } from "./useRequestOwner";
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
  requestScope: string;
  isAdminIdentity: boolean;
  onHealthRefresh: () => Promise<void>;
  onConfigRefresh: () => Promise<void>;
}

export function useServiceActionsRuntime({
  apiFetch,
  t,
  requestScope,
  isAdminIdentity,
  onHealthRefresh,
  onConfigRefresh,
}: UseServiceActionsRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const isCurrent = useRequestOwner(requestScope, isAdminIdentity ? "admin" : "user");
  const pendingActions = useMemo(() => new Map<ServiceName, symbol>(), [isCurrent]);
  const feedback = useMemo(() => ({ revision: 0 }), [isCurrent]);
  const [serviceActionLoading, setServiceActionLoading] = useState<Record<string, boolean>>({});
  const [serviceActionMessage, setNotice] = useState<ServiceActionNotice | null>(null);

  useEffect(() => {
    setServiceActionLoading({});
    setNotice(null);
  }, [isCurrent]);

  const setServiceActionMessage = useCallback((notice: ServiceActionNotice | null) => {
    if (!isCurrent()) return;
    feedback.revision++;
    setNotice(notice);
  }, [isCurrent, feedback]);

  const controlService = async (serviceName: ServiceName, action: ServiceAction): Promise<boolean> => {
    if (!isAdminIdentity || !isCurrent() || pendingActions.has(serviceName)) return false;
    const token = Symbol(serviceName);
    pendingActions.set(serviceName, token);
    const ownsAction = () => isCurrent() && pendingActions.get(serviceName) === token;
    let feedbackRevision = feedback.revision;
    const canPresent = () => ownsAction() && feedback.revision === feedbackRevision;
    try {
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
        if (!confirmed || !ownsAction()) return false;
      }
      feedbackRevision = ++feedback.revision;
      setNotice(null);
      setServiceActionLoading((prev) => ({ ...prev, [serviceName]: true }));
      const res = await apiFetch(`/v1/services/${serviceName}/${action}`, {
        method: "POST",
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!ownsAction()) return false;
      if (!res.ok || !body.ok) {
        if (canPresent()) setNotice({
          tone: "error",
          text: formatServiceActionError(serviceName, action, serviceActionErrorCode(body), t),
        });
        return false;
      }
      if (canPresent()) setNotice({
        tone: "success",
        text: serviceActionSuccessMessage(serviceName, action, t),
      });
      await sleep(800);
      if (!ownsAction()) return false;
      // A failed status read cannot undo an already accepted service operation.
      await Promise.allSettled([onHealthRefresh, onConfigRefresh].map(async refresh => {
        if (ownsAction()) await refresh();
      }));
      return ownsAction();
    } catch {
      if (canPresent()) setNotice({
        tone: "error",
        text: formatServiceActionError(serviceName, action, "service_action_request_failed", t),
      });
      return false;
    } finally {
      if (ownsAction()) {
        pendingActions.delete(serviceName);
        setServiceActionLoading((prev) => ({ ...prev, [serviceName]: false }));
      }
    }
  };

  return {
    serviceActionLoading,
    serviceActionMessage,
    setServiceActionMessage,
    controlService,
  };
}
