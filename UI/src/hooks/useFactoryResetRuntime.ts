import { useEffect, useState } from "react";

import type { ApiResponse, FactoryResetResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

const FACTORY_RESET_CONFIRM_WORD = "RESET";
const FACTORY_RESET_COUNTDOWN_SECONDS = 10;

export interface UseFactoryResetRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  onResetComplete: (result: FactoryResetResponse) => void;
}

export function useFactoryResetRuntime({
  apiFetch,
  t,
  onResetComplete,
}: UseFactoryResetRuntimeParams) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [countdown, setCountdown] = useState(FACTORY_RESET_COUNTDOWN_SECONDS);
  const [confirmText, setConfirmText] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<FactoryResetResponse | null>(null);

  useEffect(() => {
    if (!dialogOpen || result || countdown <= 0) return;
    const timer = window.setTimeout(() => {
      setCountdown((value) => Math.max(0, value - 1));
    }, 1000);
    return () => window.clearTimeout(timer);
  }, [countdown, dialogOpen, result]);

  const readApiBody = async <T,>(res: Response, label: string): Promise<T> => {
    const body = (await res.json()) as ApiResponse<T>;
    if (!res.ok || !body.ok || body.data === undefined) {
      throw new Error(body.error || `${label} request failed (${res.status})`);
    }
    return body.data;
  };

  const openDialog = () => {
    setDialogOpen(true);
    setCountdown(FACTORY_RESET_COUNTDOWN_SECONDS);
    setConfirmText("");
    setError(null);
    setResult(null);
  };

  const closeDialog = () => {
    if (loading) return;
    setDialogOpen(false);
    setConfirmText("");
    setError(null);
    setCountdown(FACTORY_RESET_COUNTDOWN_SECONDS);
  };

  const runFactoryReset = async () => {
    if (countdown > 0 || confirmText.trim() !== FACTORY_RESET_CONFIRM_WORD) {
      setError(
        t(
          `请等待倒计时结束，并输入 ${FACTORY_RESET_CONFIRM_WORD} 后再继续。`,
          `Wait for the countdown to finish and type ${FACTORY_RESET_CONFIRM_WORD} before continuing.`,
        ),
      );
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const res = await apiFetch("/v1/admin/factory-reset", { method: "POST" });
      const data = await readApiBody<FactoryResetResponse>(res, "factory reset");
      setResult(data);
      onResetComplete(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setLoading(false);
    }
  };

  return {
    confirmWord: FACTORY_RESET_CONFIRM_WORD,
    dialogOpen,
    countdown,
    confirmText,
    setConfirmText,
    loading,
    error,
    result,
    canConfirm:
      countdown <= 0 &&
      confirmText.trim() === FACTORY_RESET_CONFIRM_WORD &&
      !loading &&
      !result,
    openDialog,
    closeDialog,
    runFactoryReset,
  };
}
