import { useCallback, useEffect, useMemo, useState } from "react";
import { BookOpen, FilePlus2, Loader2, RefreshCw, Search, Wrench } from "lucide-react";

import {
  buildKbTaskBody,
  kbContinuationArgs,
  kbDocumentRows,
  kbNamespaceRows,
  kbPayload,
  kbSearchRows,
  kbTaskIsTerminal,
  type KbDocumentRow,
  type KbIdentity,
  type KbNamespaceRow,
  type KbSearchRow,
} from "../lib/kb-maintenance";
import { formatUiError } from "../lib/ui-error";
import type { ApiResponse, SubmitTaskResponse, TaskQueryResponse } from "../types/api";

type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
type Translate = (zh: string, en: string) => string;
type KbAction =
  | "list_namespaces"
  | "list_documents"
  | "ingest"
  | "search"
  | "reindex"
  | "resume_ingest"
  | "resume_vector_reindex";

type ActiveOperation = {
  taskId: string;
  action: KbAction;
  status: TaskQueryResponse["status"];
};

export interface KbMaintenancePanelProps {
  t: Translate;
  apiFetch: ApiFetch;
  identity: KbIdentity;
}

function boundedJson(value: unknown): string {
  try {
    const serialized = JSON.stringify(value, null, 2);
    return serialized.length <= 40_000 ? serialized : `${serialized.slice(0, 40_000)}\n...`;
  } catch {
    return "{}";
  }
}

export function KbMaintenancePanel({ t, apiFetch, identity }: KbMaintenancePanelProps) {
  const [namespace, setNamespace] = useState("");
  const [paths, setPaths] = useState("");
  const [query, setQuery] = useState("");
  const [namespaces, setNamespaces] = useState<KbNamespaceRow[]>([]);
  const [documents, setDocuments] = useState<KbDocumentRow[]>([]);
  const [hits, setHits] = useState<KbSearchRow[]>([]);
  const [operation, setOperation] = useState<ActiveOperation | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastPayload, setLastPayload] = useState<Record<string, unknown> | null>(null);
  const continuation = useMemo(() => kbContinuationArgs(lastPayload), [lastPayload]);
  const busy = operation != null && !kbTaskIsTerminal({ task_id: operation.taskId, status: operation.status });

  const submit = useCallback(async (action: KbAction, args: Record<string, unknown>) => {
    if (busy) return;
    setError(null);
    setMessage(null);
    try {
      const response = await apiFetch("/v1/tasks", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(buildKbTaskBody({ action, ...args }, identity)),
      });
      const body = (await response.json()) as ApiResponse<SubmitTaskResponse>;
      if (!response.ok || !body.ok || !body.data?.task_id) {
        throw new Error(body.error || `kb_task_submit_http_${response.status}`);
      }
      setOperation({ taskId: body.data.task_id, action, status: "queued" });
    } catch (cause) {
      setError(formatUiError(
        cause,
        t,
        "知识库操作没有提交成功，请检查服务状态后重试。",
        "The knowledge-base operation was not submitted. Check service status and try again.",
      ));
    }
  }, [apiFetch, busy, identity, t]);

  useEffect(() => {
    if (!operation || kbTaskIsTerminal({ task_id: operation.taskId, status: operation.status })) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const poll = async () => {
      try {
        const response = await apiFetch(`/v1/tasks/${encodeURIComponent(operation.taskId)}`, {
          headers: { Accept: "application/json" },
        });
        const body = (await response.json()) as ApiResponse<TaskQueryResponse>;
        if (!response.ok || !body.ok || !body.data) {
          throw new Error(body.error || `kb_task_query_http_${response.status}`);
        }
        if (cancelled) return;
        const task = body.data;
        setOperation((current) => current?.taskId === operation.taskId
          ? { ...current, status: task.status }
          : current);
        if (!kbTaskIsTerminal(task)) {
          timer = setTimeout(() => void poll(), 1_200);
          return;
        }
        if (task.status !== "succeeded") {
          setError(formatUiError(
            new Error(task.error_text || "kb_operation_failed"),
            t,
            "知识库操作未完成。请展开诊断信息，检查路径、资料库名称或任务状态后重试。",
            "The knowledge-base operation did not complete. Open diagnostics, check the path, library name, or task state, then try again.",
          ));
          return;
        }
        const payload = kbPayload(task.result_json);
        setLastPayload(payload);
        if (operation.action === "list_namespaces") {
          const rows = kbNamespaceRows(payload);
          setNamespaces(rows);
          setNamespace((current) => current || rows[0]?.namespace || "");
          setMessage(t("资料库列表已刷新。", "The library list is up to date."));
        } else if (operation.action === "list_documents") {
          setDocuments(kbDocumentRows(payload));
          setMessage(t("文档状态已刷新。", "Document status is up to date."));
        } else if (operation.action === "search") {
          setHits(kbSearchRows(payload));
          setMessage(t("搜索完成。", "Search completed."));
        } else {
          setMessage(kbContinuationArgs(payload)
            ? t("本批已完成，还有后续工作可继续。", "This batch finished and has a continuation.")
            : t("知识库操作已完成。请刷新资料库状态。", "The knowledge-base operation completed. Refresh library status."));
        }
      } catch (cause) {
        if (cancelled) return;
        setError(formatUiError(
          cause,
          t,
          "暂时无法读取知识库任务状态；任务可能仍在后台运行，请稍后刷新。",
          "The knowledge-base task status is temporarily unavailable. It may still be running in the background; refresh shortly.",
        ));
      }
    };

    void poll();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [apiFetch, operation?.action, operation?.taskId, t]);

  const selectedNamespace = namespace.trim();
  const sourcePaths = paths.split(/\r?\n/).map((value) => value.trim()).filter(Boolean);
  const statusLabel = operation?.status === "queued"
    ? t("排队中", "Queued")
    : operation?.status === "running"
      ? t("处理中", "Working")
      : operation?.status === "succeeded"
        ? t("已完成", "Completed")
        : operation?.status === "failed"
          ? t("未完成", "Failed")
          : operation?.status === "canceled"
            ? t("已取消", "Canceled")
            : operation?.status === "timeout"
              ? t("等待恢复", "Waiting to resume")
              : "";

  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <BookOpen className="h-4 w-4 text-sky-300" />
            <h4 className="text-sm font-semibold text-white">{t("知识库", "Knowledge base")}</h4>
          </div>
          <p className="mt-2 max-w-3xl text-xs leading-5 text-white/55">
            {t(
              "把本机或服务器上的资料加入命名资料库，再搜索、查看处理状态或重建索引。耗时操作会在后台分批继续。",
              "Add files on this device or server to a named library, then search, inspect processing status, or rebuild its index. Long operations continue in background batches.",
            )}
          </p>
        </div>
        <button
          type="button"
          className="theme-secondary-btn px-3 py-2 text-xs disabled:opacity-50"
          disabled={busy}
          onClick={() => void submit("list_namespaces", {})}
        >
          {busy && operation?.action === "list_namespaces" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
          {t("刷新资料库", "Refresh libraries")}
        </button>
      </div>

      {error ? <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-100">{error}</p> : null}
      {message ? <p className="mt-3 rounded-lg border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100">{message}</p> : null}
      {operation ? (
        <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-white/55">
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
          <span>{statusLabel}</span>
          <code className="break-all text-white/40">{operation.taskId}</code>
        </div>
      ) : null}

      <div className="mt-4 grid gap-4 xl:grid-cols-3">
        <div className="rounded-xl border border-white/10 bg-[#12151f] p-4">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <FilePlus2 className="h-4 w-4" />{t("添加资料", "Add material")}
          </div>
          <label className="mt-3 block text-xs text-white/55">
            {t("资料库名称", "Library name")}
            <input className="theme-input mt-1 w-full" value={namespace} maxLength={128} onChange={(event) => setNamespace(event.target.value)} />
          </label>
          <label className="mt-3 block text-xs text-white/55">
            {t("文件或文件夹路径（每行一个）", "File or folder paths (one per line)")}
            <textarea className="theme-input mt-1 min-h-24 w-full resize-y" value={paths} onChange={(event) => setPaths(event.target.value)} />
          </label>
          <button
            type="button"
            className="theme-accent-btn mt-3 w-full justify-center disabled:opacity-50"
            disabled={busy || !selectedNamespace || sourcePaths.length === 0}
            onClick={() => void submit("ingest", { namespace: selectedNamespace, paths: sourcePaths, overwrite: false })}
          >
            {busy && operation?.action === "ingest" ? <Loader2 className="h-4 w-4 animate-spin" /> : <FilePlus2 className="h-4 w-4" />}
            {t("开始添加", "Start adding")}
          </button>
        </div>

        <div className="rounded-xl border border-white/10 bg-[#12151f] p-4">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Search className="h-4 w-4" />{t("搜索资料", "Search material")}
          </div>
          <label className="mt-3 block text-xs text-white/55">
            {t("选择资料库", "Choose a library")}
            <select className="theme-input mt-1 w-full" value={namespace} onChange={(event) => setNamespace(event.target.value)}>
              <option value="">{t("输入或刷新资料库", "Enter or refresh a library")}</option>
              {namespaces.map((item) => <option key={item.namespace} value={item.namespace}>{item.namespace}</option>)}
            </select>
          </label>
          <label className="mt-3 block text-xs text-white/55">
            {t("想查找什么", "What do you want to find?")}
            <textarea className="theme-input mt-1 min-h-24 w-full resize-y" value={query} maxLength={8_192} onChange={(event) => setQuery(event.target.value)} />
          </label>
          <button
            type="button"
            className="theme-accent-btn mt-3 w-full justify-center disabled:opacity-50"
            disabled={busy || !selectedNamespace || !query.trim()}
            onClick={() => void submit("search", { namespace: selectedNamespace, query: query.trim(), top_k: 5 })}
          >
            {busy && operation?.action === "search" ? <Loader2 className="h-4 w-4 animate-spin" /> : <Search className="h-4 w-4" />}
            {t("搜索", "Search")}
          </button>
        </div>

        <div className="rounded-xl border border-white/10 bg-[#12151f] p-4">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Wrench className="h-4 w-4" />{t("状态与修复", "Status and repair")}
          </div>
          <p className="mt-2 text-xs leading-5 text-white/50">
            {t("先选择资料库。查看文档不会读取正文；重建会按已记录的来源重新处理。", "Choose a library first. Document status does not read bodies; rebuild reprocesses recorded sources.")}
          </p>
          <div className="mt-3 grid gap-2">
            <button type="button" className="theme-secondary-btn justify-center px-3 py-2 text-xs disabled:opacity-50" disabled={busy || !selectedNamespace} onClick={() => void submit("list_documents", { namespace: selectedNamespace, limit: 100 })}>
              {t("查看文档状态", "View document status")}
            </button>
            <button type="button" className="theme-secondary-btn justify-center px-3 py-2 text-xs disabled:opacity-50" disabled={busy || !selectedNamespace} onClick={() => void submit("reindex", { namespace: selectedNamespace })}>
              {t("重建资料库索引", "Rebuild library index")}
            </button>
            {continuation ? (
              <button type="button" className="theme-accent-btn justify-center disabled:opacity-50" disabled={busy} onClick={() => void submit(continuation.action as KbAction, continuation)}>
                {t("继续未完成工作", "Continue unfinished work")}
              </button>
            ) : null}
          </div>
        </div>
      </div>

      {namespaces.length > 0 ? (
        <div className="mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
          {namespaces.map((item) => (
            <button key={item.namespace} type="button" className="rounded-xl border border-white/10 bg-[#12151f] p-3 text-left hover:border-sky-400/35" onClick={() => setNamespace(item.namespace)}>
              <p className="truncate text-sm font-semibold text-white">{item.namespace}</p>
              <p className="mt-1 text-xs text-white/50">{t("文档", "Documents")} {item.docs} · chunks {item.chunks}</p>
              <details className="mt-2 text-[11px] text-white/40">
                <summary className="cursor-pointer">{t("索引详情", "Index details")}</summary>
                <p className="mt-1 break-all">profile: {item.embeddingVersion || "--"}<br />generation: {item.activeGeneration ?? "--"}</p>
              </details>
            </button>
          ))}
        </div>
      ) : null}

      {documents.length > 0 ? (
        <details className="mt-4 rounded-xl border border-white/10 bg-[#12151f] p-4" open>
          <summary className="cursor-pointer text-sm font-semibold text-white">{t("文档处理状态", "Document processing status")} ({documents.length})</summary>
          <div className="mt-3 grid gap-2 lg:grid-cols-2">
            {documents.map((item) => (
              <div key={item.path} className="min-w-0 rounded-lg border border-white/10 px-3 py-2">
                <p className="break-all text-xs text-white/75">{item.path}</p>
                <p className="mt-1 text-[11px] text-white/40">{item.fileType || "--"} · {item.size} bytes · {item.chunks} chunks</p>
                <details className="mt-1 text-[11px] text-white/40"><summary className="cursor-pointer">{t("解析详情", "Parser details")}</summary><p className="mt-1 break-all">parser: {item.parserVersion || "--"}<br />chunker: {item.chunkerVersion || "--"}</p></details>
              </div>
            ))}
          </div>
        </details>
      ) : null}

      {hits.length > 0 ? (
        <div className="mt-4 space-y-2">
          {hits.map((item, index) => (
            <article key={item.evidenceId || `${item.sourceRef}:${index}`} className="rounded-xl border border-white/10 bg-[#12151f] p-4">
              <p className="break-all text-xs font-medium text-sky-200">{item.sourceRef || t("知识库证据", "Knowledge-base evidence")}</p>
              <p className="mt-2 whitespace-pre-wrap text-sm leading-6 text-white/75">{item.text}</p>
              <details className="mt-2 text-[11px] text-white/40"><summary className="cursor-pointer">{t("检索详情", "Retrieval details")}</summary><pre className="mt-2 overflow-auto whitespace-pre-wrap">{boundedJson({ evidence_id: item.evidenceId, score: item.score, location: item.location })}</pre></details>
            </article>
          ))}
        </div>
      ) : null}

      {lastPayload ? (
        <details className="mt-4 rounded-xl border border-white/10 bg-[#12151f] p-4 text-xs text-white/45">
          <summary className="cursor-pointer">{t("诊断信息", "Diagnostics")}</summary>
          <pre className="mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-all">{boundedJson(lastPayload)}</pre>
        </details>
      ) : null}
    </section>
  );
}
