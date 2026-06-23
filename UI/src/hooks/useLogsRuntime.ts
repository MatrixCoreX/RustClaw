import { useEffect, useState, type RefObject } from "react";

import type { ApiResponse, LogLatestResponse } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type Translate = (zh: string, en: string) => string;

export interface UseLogsRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  apiBase: string;
  currentPage: string;
  pollingSeconds: number;
  uiAuthReady: boolean;
  logContainerRef: RefObject<HTMLPreElement | null>;
}

export function useLogsRuntime({
  apiFetch,
  t,
  apiBase,
  currentPage,
  pollingSeconds,
  uiAuthReady,
  logContainerRef,
}: UseLogsRuntimeParams) {
  const [selectedLogFile, setSelectedLogFile] = useState("clawd.log");
  const [logTailLines, setLogTailLines] = useState(200);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [logLastUpdated, setLogLastUpdated] = useState<number | null>(null);
  const [logFollowTail, setLogFollowTail] = useState(true);

  const fetchLatestLog = async () => {
    setLogLoading(true);
    setLogError(null);
    try {
      const params = new URLSearchParams({
        file: selectedLogFile,
        lines: String(logTailLines),
      });
      const res = await apiFetch(`/v1/logs/latest?${params.toString()}`);
      const body = (await res.json()) as ApiResponse<LogLatestResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || t(`日志读取失败 (${res.status})`, `Log read failed (${res.status})`));
      }
      setLogText(body.data.text || "");
      setLogLastUpdated(Date.now());
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setLogError(message);
    } finally {
      setLogLoading(false);
    }
  };

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage !== "logs") return;
    void fetchLatestLog();
    const timer = window.setInterval(() => {
      void fetchLatestLog();
    }, Math.max(2, pollingSeconds) * 1000);
    return () => window.clearInterval(timer);
    // Mirrors the previous App.tsx polling boundary; apiFetch is intentionally represented by apiBase here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage, apiBase, selectedLogFile, logTailLines, pollingSeconds, uiAuthReady]);

  useEffect(() => {
    if (!logFollowTail) return;
    const el = logContainerRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [logText, logFollowTail, logContainerRef]);

  return {
    selectedLogFile,
    setSelectedLogFile,
    logTailLines,
    setLogTailLines,
    logLoading,
    logError,
    logText,
    logLastUpdated,
    logFollowTail,
    setLogFollowTail,
    fetchLatestLog,
  };
}
