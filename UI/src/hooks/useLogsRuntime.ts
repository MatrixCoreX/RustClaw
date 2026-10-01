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
  const [logFilesTotal, setLogFilesTotal] = useState(0);
  const [logFilesNextCursor, setLogFilesNextCursor] = useState<string | null>(null);
  const [logFilesCursorHistory, setLogFilesCursorHistory] = useState<string[]>([]);
  const logFilesCursorRef = useRef<string | null>(null);
  const [logFilesLoading, setLogFilesLoading] = useState(false);
  const [logFilesError, setLogFilesError] = useState<string | null>(null);
  const [logTailLines, setLogTailLines] = useState(200);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [logLastUpdated, setLogLastUpdated] = useState<number | null>(null);
  const [logFollowTail, setLogFollowTail] = useState(true);
  const logFilesRequestRef = useRef<{ key: string; request: Promise<string> } | null>(null);
  const logContentRequestRef = useRef<{ key: string; request: Promise<void> } | null>(null);

  const setSelectedLogFile = (value: string) => {
    selectedLogFileRef.current = value;
    setSelectedLogFileState(value);
  };

  const fetchLogFiles = (
    cursor = logFilesCursorRef.current,
    signal?: AbortSignal,
  ): Promise<string> => {
    const key = cursor || "first";
    if (logFilesRequestRef.current?.key === key) return logFilesRequestRef.current.request;
    const request = (async () => {
      setLogFilesLoading(true);
      setLogFilesError(null);
      try {
        const params = new URLSearchParams({ limit: "100" });
        if (cursor) params.set("cursor", cursor);
        const res = await apiFetch(`/v1/logs/files?${params.toString()}`, { signal });
        const body = (await res.json()) as ApiResponse<LogFilesResponse>;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(body.error || `log_files_http_${res.status}`);
        }
        if (signal?.aborted) return selectedLogFileRef.current;
        const files = Array.isArray(body.data.files)
          ? body.data.files.filter((file): file is string => typeof file === "string" && file.length > 0)
          : [];
        setLogFiles(files);
        setLogFilesTotal(typeof body.data.total === "number" ? body.data.total : files.length);
        setLogFilesNextCursor(
          body.data.has_more && typeof body.data.next_cursor === "string"
            ? body.data.next_cursor
            : null,
        );
        const current = selectedLogFileRef.current;
        const next = files.includes(current) ? current : files[0] || "";
        if (next !== current) setSelectedLogFile(next);
        if (!next) {
          setLogText("");
          setLogLastUpdated(null);
        }
        return next;
      } catch (err) {
        if (!signal?.aborted) {
          setLogFilesError(formatUiError(err, t, "无法读取日志列表。", "Could not load the log list."));
        }
        return selectedLogFileRef.current;
      } finally {
        if (logFilesRequestRef.current?.request === request) {
          logFilesRequestRef.current = null;
          setLogFilesLoading(false);
        }
      }
    })();
    logFilesRequestRef.current = { key, request };
    return request;
  };

  const openNextLogFilesPage = async () => {
    if (!logFilesNextCursor || logFilesLoading) return;
    setLogFilesCursorHistory((current) => [...current, logFilesCursorRef.current || ""]);
    logFilesCursorRef.current = logFilesNextCursor;
    await fetchLogFiles(logFilesNextCursor);
  };

  const openPreviousLogFilesPage = async () => {
    if (logFilesCursorHistory.length === 0 || logFilesLoading) return;
    const previous = logFilesCursorHistory[logFilesCursorHistory.length - 1] || null;
    setLogFilesCursorHistory((current) => current.slice(0, -1));
    logFilesCursorRef.current = previous;
    await fetchLogFiles(previous);
  };

  const fetchLatestLog = (
    fileName = selectedLogFileRef.current,
    signal?: AbortSignal,
  ): Promise<void> => {
    if (!fileName) return Promise.resolve();
    const key = `${fileName}\n${logTailLines}`;
    if (logContentRequestRef.current?.key === key) return logContentRequestRef.current.request;
    const request = (async () => {
      setLogLoading(true);
      setLogError(null);
      try {
        const params = new URLSearchParams({
          file: fileName,
          lines: String(logTailLines),
        });
        const res = await apiFetch(`/v1/logs/latest?${params.toString()}`, { signal });
        const body = (await res.json()) as ApiResponse<LogLatestResponse>;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(body.error || `log_latest_http_${res.status}`);
        }
        if (signal?.aborted) return;
        setLogText(body.data.text || "");
        setLogLastUpdated(Date.now());
      } catch (err) {
        if (!signal?.aborted) {
          setLogError(formatUiError(err, t, "无法读取日志内容。", "Could not load the log content."));
        }
      } finally {
        if (logContentRequestRef.current?.request === request) {
          logContentRequestRef.current = null;
          setLogLoading(false);
        }
      }
    })();
    logContentRequestRef.current = { key, request };
    return request;
  };

  const refreshLogs = async () => {
    const fileName = await fetchLogFiles();
    if (fileName) await fetchLatestLog(fileName);
  };

  useEffect(() => {
    if (!uiAuthReady) return;
    if (currentPage !== "logs") return;
    const controller = new AbortController();
    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") void fetchLogFiles(undefined, controller.signal);
    };
    refreshWhenVisible();
    const timer = window.setInterval(() => {
      refreshWhenVisible();
    }, Math.max(2, pollingSeconds) * 1000);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      controller.abort();
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
    // Mirrors the previous App.tsx polling boundary; apiFetch is intentionally represented by apiBase here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage, apiBase, pollingSeconds, uiAuthReady]);

  useEffect(() => {
    if (!uiAuthReady || currentPage !== "logs" || !selectedLogFile) return;
    const controller = new AbortController();
    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") {
        void fetchLatestLog(selectedLogFile, controller.signal);
      }
    };
    refreshWhenVisible();
    const timer = window.setInterval(() => {
      refreshWhenVisible();
    }, Math.max(2, pollingSeconds) * 1000);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      controller.abort();
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
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
    logFilesTotal,
    logFilesHasPrevious: logFilesCursorHistory.length > 0,
    logFilesHasNext: Boolean(logFilesNextCursor),
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
    openNextLogFilesPage,
    openPreviousLogFilesPage,
    refreshLogs,
  };
}
