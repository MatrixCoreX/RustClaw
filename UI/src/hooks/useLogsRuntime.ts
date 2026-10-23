import { useEffect, useRef, useState, type RefObject } from "react";

import { formatUiError } from "../lib/ui-error";
import { PageRequestLifecycle } from "../lib/page-request-lifecycle";
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
  const [logTailLines, setLogTailLinesState] = useState(200);
  const logTailLinesRef = useRef(200);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [logLastUpdated, setLogLastUpdated] = useState<number | null>(null);
  const [logFollowTail, setLogFollowTail] = useState(true);
  const logFilesLifecycle = useRef(new PageRequestLifecycle());
  const logContentLifecycle = useRef(new PageRequestLifecycle());
  const logFilesRequestRef = useRef<{ key: string; request: Promise<string>; controller: AbortController } | null>(null);
  const logContentRequestRef = useRef<{ key: string; request: Promise<void>; controller: AbortController } | null>(null);

  const invalidateContent = () => {
    logContentLifecycle.current.invalidate();
    logContentRequestRef.current?.controller.abort();
    logContentRequestRef.current = null;
    setLogLoading(false);
    setLogError(null);
    setLogText("");
    setLogLastUpdated(null);
  };

  const setSelectedLogFile = (value: string) => {
    if (value !== selectedLogFileRef.current) invalidateContent();
    selectedLogFileRef.current = value;
    setSelectedLogFileState(value);
  };

  const setLogTailLines = (value: number) => {
    if (value !== logTailLinesRef.current) invalidateContent();
    logTailLinesRef.current = value;
    setLogTailLinesState(value);
  };

  const fetchLogFiles = (
    cursor = logFilesCursorRef.current,
    signal?: AbortSignal,
  ): Promise<string> => {
    const key = cursor || "first";
    if (signal?.aborted) return Promise.resolve("");
    if (logFilesRequestRef.current?.key === key && !logFilesRequestRef.current.controller.signal.aborted) {
      return logFilesRequestRef.current.request;
    }
    logFilesRequestRef.current?.controller.abort();
    const controller = new AbortController();
    const abort = () => controller.abort();
    signal?.addEventListener("abort", abort, { once: true });
    const requestId = logFilesLifecycle.current.begin(false)!;
    const isCurrent = () => !controller.signal.aborted && logFilesLifecycle.current.isCurrent(requestId);
    const request = (async () => {
      setLogFilesLoading(true);
      setLogFilesError(null);
      await Promise.resolve();
      try {
        const params = new URLSearchParams({ limit: "100" });
        if (cursor) params.set("cursor", cursor);
        const res = await apiFetch(`/v1/logs/files?${params.toString()}`, { signal: controller.signal });
        const body = (await res.json()) as ApiResponse<LogFilesResponse>;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(body.error || `log_files_http_${res.status}`);
        }
        if (!isCurrent()) return "";
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
        if (isCurrent()) {
          setLogFilesError(formatUiError(err, t, "无法读取日志列表。", "Could not load the log list."));
        }
        return "";
      } finally {
        signal?.removeEventListener("abort", abort);
        if (logFilesRequestRef.current?.controller === controller) {
          logFilesRequestRef.current = null;
          setLogFilesLoading(false);
        }
      }
    })();
    logFilesRequestRef.current = { key, request, controller };
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
    if (!fileName || signal?.aborted) return Promise.resolve();
    const lines = logTailLinesRef.current;
    const key = `${fileName}\n${lines}`;
    if (logContentRequestRef.current?.key === key && !logContentRequestRef.current.controller.signal.aborted) {
      return logContentRequestRef.current.request;
    }
    logContentRequestRef.current?.controller.abort();
    const controller = new AbortController();
    const abort = () => controller.abort();
    signal?.addEventListener("abort", abort, { once: true });
    const requestId = logContentLifecycle.current.begin(false)!;
    const isCurrent = () => !controller.signal.aborted && logContentLifecycle.current.isCurrent(requestId)
      && fileName === selectedLogFileRef.current && lines === logTailLinesRef.current;
    const request = (async () => {
      setLogLoading(true);
      setLogError(null);
      await Promise.resolve();
      try {
        const params = new URLSearchParams({
          file: fileName,
          lines: String(lines),
        });
        const res = await apiFetch(`/v1/logs/latest?${params.toString()}`, { signal: controller.signal });
        const body = (await res.json()) as ApiResponse<LogLatestResponse>;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(body.error || `log_latest_http_${res.status}`);
        }
        if (!isCurrent()) return;
        setLogText(body.data.text || "");
        setLogLastUpdated(Date.now());
      } catch (err) {
        if (isCurrent()) {
          setLogError(formatUiError(err, t, "无法读取日志内容。", "Could not load the log content."));
        }
      } finally {
        signal?.removeEventListener("abort", abort);
        if (logContentRequestRef.current?.controller === controller) {
          logContentRequestRef.current = null;
          setLogLoading(false);
        }
      }
    })();
    logContentRequestRef.current = { key, request, controller };
    return request;
  };

  const refreshLogs = async () => {
    const fileName = await fetchLogFiles();
    if (fileName) await fetchLatestLog(fileName);
  };

  useEffect(() => {
    setLogLoading(false);
    setLogFilesLoading(false);
    return () => {
      logFilesLifecycle.current.invalidate();
      logContentLifecycle.current.invalidate();
      logFilesRequestRef.current?.controller.abort();
      logContentRequestRef.current?.controller.abort();
      logFilesRequestRef.current = null;
      logContentRequestRef.current = null;
    };
  }, [currentPage, apiBase, uiAuthReady]);

  useEffect(() => {
    setLogText("");
    setLogLastUpdated(null);
    setLogError(null);
    setLogFiles([]);
    setLogFilesError(null);
    setLogFilesTotal(0);
    setLogFilesNextCursor(null);
    setLogFilesCursorHistory([]);
    logFilesCursorRef.current = null;
  }, [apiBase, uiAuthReady]);

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
