import { useEffect, useRef, useState, type RefObject } from "react";

import { formatUiError } from "../lib/ui-error";
import type { ApiResponse, LogFilesResponse, LogLatestResponse } from "../types/api";

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
  const [selectedLogFile, setSelectedLogFileState] = useState("");
  const selectedLogFileRef = useRef("");
  const [logFiles, setLogFiles] = useState<string[]>([]);
  const [logFilesLoading, setLogFilesLoading] = useState(false);
  const [logFilesError, setLogFilesError] = useState<string | null>(null);
  const [logTailLines, setLogTailLines] = useState(200);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [logLastUpdated, setLogLastUpdated] = useState<number | null>(null);
  const [logFollowTail, setLogFollowTail] = useState(true);

  const setSelectedLogFile = (value: string) => {
    selectedLogFileRef.current = value;
    setSelectedLogFileState(value);
  };

  const fetchLogFiles = async (): Promise<string> => {
    setLogFilesLoading(true);
    setLogFilesError(null);
    try {
      const res = await apiFetch("/v1/logs/files");
      const body = (await res.json()) as ApiResponse<LogFilesResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(
          body.error || `log_files_http_${res.status}`,
        );
      }
      const files = Array.isArray(body.data.files)
        ? body.data.files.filter((file): file is string => typeof file === "string" && file.length > 0)
        : [];
      setLogFiles(files);
      const current = selectedLogFileRef.current;
      const next = files.includes(current) ? current : files[0] || "";
      if (next !== current) setSelectedLogFile(next);
      if (!next) {
        setLogText("");
        setLogLastUpdated(null);
      }
      return next;
    } catch (err) {
      setLogFilesError(formatUiError(err, t, "无法读取日志列表。", "Could not load the log list."));
      return selectedLogFileRef.current;
    } finally {
      setLogFilesLoading(false);
    }
  };

  const fetchLatestLog = async (fileName = selectedLogFileRef.current) => {
    if (!fileName) return;
    setLogLoading(true);
    setLogError(null);
    try {
      const params = new URLSearchParams({
        file: fileName,
        lines: String(logTailLines),
      });
      const res = await apiFetch(`/v1/logs/latest?${params.toString()}`);
      const body = (await res.json()) as ApiResponse<LogLatestResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `log_latest_http_${res.status}`);
      }
      setLogText(body.data.text || "");
      setLogLastUpdated(Date.now());
    } catch (err) {
      setLogError(formatUiError(err, t, "无法读取日志内容。", "Could not load the log content."));
    } finally {
      setLogLoading(false);
    }
  };

  const refreshLogs = async () => {
    const fileName = await fetchLogFiles();
    if (fileName) await fetchLatestLog(fileName);
  };

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage !== "logs") return;
    void fetchLogFiles();
    const timer = window.setInterval(() => {
      void fetchLogFiles();
    }, Math.max(2, pollingSeconds) * 1000);
    return () => window.clearInterval(timer);
    // Mirrors the previous App.tsx polling boundary; apiFetch is intentionally represented by apiBase here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage, apiBase, pollingSeconds, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady || currentPage !== "logs" || !selectedLogFile) return;
    void fetchLatestLog(selectedLogFile);
    const timer = window.setInterval(() => {
      void fetchLatestLog(selectedLogFile);
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
    logFiles,
    logFilesLoading,
    logFilesError,
    logTailLines,
    setLogTailLines,
    logLoading,
    logError,
    logText,
    logLastUpdated,
    logFollowTail,
    setLogFollowTail,
    refreshLogs,
  };
}
