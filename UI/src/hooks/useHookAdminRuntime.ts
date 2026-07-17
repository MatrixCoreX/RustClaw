import { useState } from "react";

import type { ApiResponse, HookAdminStatus } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export function useHookAdminRuntime(apiFetch: ApiFetch) {
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
      setHookStatusError(error instanceof Error ? error.message : "hook_admin_unknown_error");
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
