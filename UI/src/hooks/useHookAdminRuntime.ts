import { useState } from "react";

import { formatUiError } from "../lib/ui-error";
import type { ApiResponse, HookAdminStatus } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type Translate = (zh: string, en: string) => string;

export function useHookAdminRuntime(apiFetch: ApiFetch, t: Translate) {
  const [hookStatus, setHookStatus] = useState<HookAdminStatus | null>(null);
  const [hookStatusLoading, setHookStatusLoading] = useState(false);
  const [hookStatusError, setHookStatusError] = useState<string | null>(null);

  const refreshHookStatus = async () => {
    setHookStatusLoading(true);
    setHookStatusError(null);
    try {
      const response = await apiFetch("/v1/admin/hooks/status");
      const body = (await response.json()) as ApiResponse<HookAdminStatus>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `hook_admin_http_${response.status}`);
      }
      setHookStatus(body.data);
      return body.data;
    } catch (error) {
      setHookStatusError(formatUiError(error, t, "无法读取 Hook 状态。", "Could not load hook status."));
      return null;
    } finally {
      setHookStatusLoading(false);
    }
  };

  return {
    hookStatus,
    hookStatusLoading,
    hookStatusError,
    refreshHookStatus,
  };
}
