import { useMemo, useState, type KeyboardEvent, type RefObject } from "react";
import {
  FileText,
  Loader2,
  MessageSquare,
  Mic,
  Paperclip,
  Plus,
  RefreshCw,
  Search,
  Square,
  Trash2,
  X,
} from "lucide-react";
import ReactMarkdown from "react-markdown";

import {
  attachmentIsAudio,
  attachmentIsImage,
  formatAttachmentSize,
} from "../lib/chat-attachments";
import type { ChatAttachment, ChatMessage, TaskLlmDebugResponse, TaskQueryResponse } from "../types/api";
import { TaskLlmTracePanel } from "./TaskLlmTracePanel";

type Translate = (zh: string, en: string) => string;
type TranslateSlash = (text: string) => string;

interface ChatThreadSummary {
  id: string;
  title: string;
  preview: string;
  updatedAt: number;
  messageCount: number;
  agentMode: boolean;
  teachingMode: boolean;
  taskId: string | null;
  taskStatus: TaskQueryResponse["status"] | "running" | null;
  llmCallCount: number | null;
}

interface ChatTeachingRunSummary {
  id: string;
  taskId: string | null;
  userText: string;
  assistantText: string | null;
  status: TaskQueryResponse["status"] | "running";
  startedAt: number;
  completedAt: number | null;
  callCount: number | null;
  hasTrace: boolean;
  traceError: string | null;
  selected: boolean;
}

export interface ChatPageProps {
  t: Translate;
  tSlash: TranslateSlash;
  chatMessages: ChatMessage[];
  chatThreads: ChatThreadSummary[];
  activeChatThreadId: string;
  chatInput: string;
  chatAttachments: ChatAttachment[];
  chatAgentMode: boolean;
  chatTeachingMode: boolean;
  chatTeachingTaskResult: TaskQueryResponse | null;
  chatTeachingLlmDebug: TaskLlmDebugResponse | null;
  chatTeachingLlmDebugLoading: boolean;
  chatTeachingLlmDebugError: string | null;
  chatTeachingRuns: ChatTeachingRunSummary[];
  activeChatTeachingRunId: string | null;
  chatSending: boolean;
  chatRecording: boolean;
  chatVoiceRecordingSupported: boolean;
  chatError: string | null;
  chatAttachmentInputRef: RefObject<HTMLInputElement | null>;
  toLocalTime: (value: number | null | undefined) => string;
  onChatAgentModeChange: (value: boolean) => void;
  onChatTeachingModeChange: (value: boolean) => void;
  onSelectChatTeachingRun: (runId: string) => void;
  onCreateNewChatThread: () => void;
  onSelectChatThread: (threadId: string) => void;
  onDeleteChatThread: (threadId: string) => void;
  onClearMessages: () => void;
  onChatInputChange: (value: string) => void;
  onChatInputKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onAttachmentSelection: (fileList: FileList | null) => unknown | Promise<unknown>;
  onRemoveAttachment: (index: number) => void;
  onStartVoiceRecording: () => unknown | Promise<unknown>;
  onStopVoiceRecording: () => unknown | Promise<unknown>;
  onSendMessage: () => unknown | Promise<unknown>;
  onQueryChatTeachingLlmDebug: (taskId?: string) => unknown | Promise<unknown>;
}

export function ChatPage({
  t,
  tSlash,
  chatMessages,
  chatThreads,
  activeChatThreadId,
  chatInput,
  chatAttachments,
  chatAgentMode,
  chatTeachingMode,
  chatTeachingTaskResult,
  chatTeachingLlmDebug,
  chatTeachingLlmDebugLoading,
  chatTeachingLlmDebugError,
  chatTeachingRuns,
  activeChatTeachingRunId,
  chatSending,
  chatRecording,
  chatVoiceRecordingSupported,
  chatError,
  chatAttachmentInputRef,
  toLocalTime,
  onChatAgentModeChange,
  onChatTeachingModeChange,
  onSelectChatTeachingRun,
  onCreateNewChatThread,
  onSelectChatThread,
  onDeleteChatThread,
  onClearMessages,
  onChatInputChange,
  onChatInputKeyDown,
  onAttachmentSelection,
  onRemoveAttachment,
  onStartVoiceRecording,
  onStopVoiceRecording,
  onSendMessage,
  onQueryChatTeachingLlmDebug,
}: ChatPageProps) {
  const [threadSearch, setThreadSearch] = useState("");
  const normalizedThreadSearch = threadSearch.trim().toLowerCase();
  const visibleChatThreads = useMemo(() => {
    if (!normalizedThreadSearch) return chatThreads;
    return chatThreads.filter((thread) => {
      const searchText = [
        thread.title,
        thread.preview,
        thread.taskId ?? "",
        thread.taskStatus ?? "",
        thread.agentMode ? "agent" : "",
        thread.teachingMode ? "teaching" : "",
      ]
        .join(" ")
        .toLowerCase();
      return searchText.includes(normalizedThreadSearch);
    });
  }, [chatThreads, normalizedThreadSearch]);
  const activeTeachingRun = useMemo(
    () =>
      chatTeachingRuns.find((run) => run.id === activeChatTeachingRunId) ??
      chatTeachingRuns.find((run) => run.selected) ??
      null,
    [activeChatTeachingRunId, chatTeachingRuns],
  );

  return (
    <section className="grid gap-4 lg:grid-cols-[18rem_minmax(0,1fr)]">
      <aside className="rounded-2xl border border-white/10 bg-white/5 p-3">
        <div className="mb-3 flex items-center justify-between gap-2">
          <h3 className="text-sm font-semibold">{t("任务历史", "Task history")}</h3>
          <button
            type="button"
            onClick={onCreateNewChatThread}
            className="inline-flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-2.5 py-1.5 text-xs hover:bg-white/10"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("新任务", "New")}
          </button>
        </div>
        <label className="mb-3 flex items-center gap-2 rounded-lg border border-white/10 bg-black/20 px-2.5 py-2 text-xs text-white/55">
          <Search className="h-3.5 w-3.5 shrink-0" />
          <input
            type="search"
            value={threadSearch}
            onChange={(event) => setThreadSearch(event.target.value)}
            placeholder={t("搜索标题、任务 ID、状态", "Search title, task ID, status")}
            className="min-w-0 flex-1 bg-transparent text-white/80 outline-none placeholder:text-white/35"
          />
        </label>
        <div className="max-h-[34rem] space-y-2 overflow-auto pr-1">
          {visibleChatThreads.map((thread) => {
            const active = thread.id === activeChatThreadId;
            return (
              <div
                key={thread.id}
                className={
                  active
                    ? "grid grid-cols-[minmax(0,1fr)_auto] gap-1 rounded-xl border border-emerald-400/35 bg-emerald-500/15 p-2"
                    : "grid grid-cols-[minmax(0,1fr)_auto] gap-1 rounded-xl border border-white/10 bg-black/20 p-2 hover:bg-white/5"
                }
              >
                <button
                  type="button"
                  onClick={() => onSelectChatThread(thread.id)}
                  className="min-w-0 text-left"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <MessageSquare className="h-3.5 w-3.5 shrink-0 text-white/55" />
                    <span className="min-w-0 truncate text-sm font-medium text-white/90" title={thread.title}>
                      {thread.title}
                    </span>
                  </div>
                  <p className="mt-1 line-clamp-2 min-h-8 break-words text-xs text-white/50">
                    {thread.preview}
                  </p>
                  <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[10px] text-white/45">
                    <span>{toLocalTime(thread.updatedAt)}</span>
                    <span>{thread.messageCount}</span>
                    {thread.taskStatus ? (
                      <span className={chatStatusBadgeClass(thread.taskStatus)}>
                        {chatStatusLabel(thread.taskStatus, t)}
                      </span>
                    ) : null}
                    {thread.agentMode ? (
                      <span className="rounded-full border border-white/10 px-1.5 py-0.5">
                        {t("自动执行", "Agent")}
                      </span>
                    ) : null}
                    {thread.teachingMode ? (
                      <span className="rounded-full border border-white/10 px-1.5 py-0.5">
                        {t("教学", "Teach")}
                      </span>
                    ) : null}
                    {typeof thread.llmCallCount === "number" ? (
                      <span className="rounded-full border border-white/10 px-1.5 py-0.5">
                        LLM {thread.llmCallCount}
                      </span>
                    ) : null}
                  </div>
                </button>
                <button
                  type="button"
                  onClick={() => onDeleteChatThread(thread.id)}
                  className="h-7 w-7 rounded-lg border border-white/10 bg-white/5 p-1.5 text-white/55 hover:bg-white/10 hover:text-white/80"
                  title={t("删除任务", "Delete task")}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
            );
          })}
          {visibleChatThreads.length === 0 ? (
            <div className="rounded-xl border border-white/10 bg-black/20 p-3 text-xs text-white/50">
              {t("没有匹配的任务。", "No matching tasks.")}
            </div>
          ) : null}
        </div>
      </aside>

      <div className="min-w-0 rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
          <h3 className="text-base font-semibold">{t("和 RustClaw 对话", "Chat with RustClaw")}</h3>
        <div className="flex flex-wrap items-center gap-3 text-sm">
          <label className="inline-flex items-center gap-2 text-white/80">
            <input type="checkbox" checked={chatAgentMode} onChange={(event) => onChatAgentModeChange(event.target.checked)} />
            agent_mode
          </label>
          <label className="inline-flex items-center gap-2 text-white/80">
            <input
              type="checkbox"
              checked={chatTeachingMode}
              onChange={(event) => onChatTeachingModeChange(event.target.checked)}
            />
            {t("教学模式", "Teaching mode")}
          </label>
          <button
            type="button"
            onClick={onClearMessages}
            className="rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10"
          >
            {t("清空记录", "Clear")}
          </button>
        </div>
      </div>

      <div className="h-80 space-y-3 overflow-auto rounded-xl border border-white/10 bg-black/30 p-3">
        {chatMessages.map((message) => (
          <div key={message.id} className="space-y-1">
            <div className="flex items-center gap-2 text-[11px] text-white/50">
              <span>{message.role}</span>
              <span>{toLocalTime(message.ts)}</span>
            </div>
            <div
              className={
                message.role === "user"
                  ? "theme-user-bubble max-w-[95%] rounded-xl px-3 py-2 text-sm text-white"
                  : message.role === "assistant"
                    ? "max-w-[95%] rounded-xl bg-emerald-500/15 px-3 py-2 text-sm text-white"
                    : "max-w-[95%] rounded-xl bg-white/10 px-3 py-2 text-sm text-white/80"
              }
            >
              {message.role === "assistant" ? (
                <div className="chat-markdown">
                  <ReactMarkdown>{message.text}</ReactMarkdown>
                </div>
              ) : (
                <pre className="whitespace-pre-wrap break-words font-sans">{message.text}</pre>
              )}
              {(message.attachments ?? message.images)?.length ? (
                <div className="mt-3 flex flex-wrap gap-2">
                  {(message.attachments ?? message.images ?? []).map((attachment, index) => (
                    <AttachmentPreview
                      key={`${message.id}-${attachment.name}-${index}`}
                      attachment={attachment}
                      t={t}
                    />
                  ))}
                </div>
              ) : null}
            </div>
          </div>
        ))}
      </div>

      {chatTeachingMode ? (
        <div className="mt-4 space-y-3">
          <TeachingRunSnapshot
            t={t}
            run={activeTeachingRun}
            debug={chatTeachingLlmDebug}
          />
          <TeachingRunHistory
            t={t}
            runs={chatTeachingRuns}
            activeRunId={activeChatTeachingRunId}
            toLocalTime={toLocalTime}
            onSelectRun={onSelectChatTeachingRun}
          />
          {chatTeachingTaskResult ? (
            <TaskLlmTracePanel
              t={t}
              tSlash={tSlash}
              taskResult={chatTeachingTaskResult}
              taskLlmDebug={chatTeachingLlmDebug}
              taskLlmDebugLoading={chatTeachingLlmDebugLoading}
              taskLlmDebugError={chatTeachingLlmDebugError}
              onQueryTaskLlmDebug={onQueryChatTeachingLlmDebug}
            />
          ) : (
            <div className="rounded-xl border border-white/10 bg-[#12151f] p-3 text-xs text-white/55">
              {t(
                "教学模式已开启。发送一条消息后，这里会保留本轮对话，并按 LLM #1、LLM #2 展示请求数据和返回数据。",
                "Teaching mode is on. After you send a message, this area will keep that turn and show request and response data as LLM #1, LLM #2, and so on.",
              )}
            </div>
          )}
        </div>
      ) : null}

      <div className="mt-4 grid shrink-0 gap-3 md:grid-cols-[1fr_auto]">
        <div className="min-w-0">
          {chatAttachments.length > 0 ? (
            <div className="mb-3 flex flex-wrap gap-2 rounded-xl border border-white/10 bg-white/5 p-2">
              {chatAttachments.map((attachment, index) => (
                <div key={`${attachment.name}-${index}`} className="relative">
                  <AttachmentPreview attachment={attachment} t={t} compact />
                  <button
                    type="button"
                    onClick={() => onRemoveAttachment(index)}
                    className="absolute -right-2 -top-2 rounded-full border border-white/15 bg-black/70 p-1 text-white/80 hover:bg-black/85"
                    title={t("移除附件", "Remove attachment")}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          ) : null}
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <input
              ref={chatAttachmentInputRef}
              type="file"
              multiple
              className="hidden"
              onChange={(event) => void onAttachmentSelection(event.target.files)}
            />
            <button
              type="button"
              onClick={() => chatAttachmentInputRef.current?.click()}
              disabled={chatSending || chatRecording}
              className="inline-flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Paperclip className="h-3.5 w-3.5" />
              {t("上传图片/文件", "Upload image/file")}
            </button>
            {chatVoiceRecordingSupported ? (
              <button
                type="button"
                onPointerDown={(event) => {
                  if (event.button !== 0) return;
                  event.preventDefault();
                  event.currentTarget.setPointerCapture?.(event.pointerId);
                  void onStartVoiceRecording();
                }}
                onPointerUp={(event) => {
                  event.preventDefault();
                  onStopVoiceRecording();
                }}
                onPointerCancel={() => onStopVoiceRecording()}
                onKeyDown={(event) => {
                  if (event.repeat || (event.key !== " " && event.key !== "Enter")) return;
                  event.preventDefault();
                  void onStartVoiceRecording();
                }}
                onKeyUp={(event) => {
                  if (event.key !== " " && event.key !== "Enter") return;
                  event.preventDefault();
                  onStopVoiceRecording();
                }}
                onContextMenu={(event) => event.preventDefault()}
                disabled={chatSending}
                className={
                  chatRecording
                    ? "inline-flex select-none items-center gap-1.5 rounded-lg border border-emerald-400/35 bg-emerald-500/15 px-3 py-1.5 text-xs text-emerald-100 disabled:cursor-not-allowed disabled:opacity-50"
                    : "inline-flex select-none items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
                }
              >
                {chatRecording ? (
                  <Square className="h-3.5 w-3.5" />
                ) : (
                  <Mic className="h-3.5 w-3.5" />
                )}
                {chatRecording ? t("松开发送", "Release to send") : t("按住说话", "Hold to talk")}
              </button>
            ) : null}
            <span className="text-xs text-white/45">
              {chatVoiceRecordingSupported
                ? t(
                    "可直接发送图片、文件或语音，也可以带一句说明。",
                    "Send images, files, or voice directly, with an optional note.",
                  )
                : t(
                    "可直接发送图片或文件，也可以带一句说明。",
                    "Send images or files directly, with an optional note.",
                  )}
            </span>
          </div>
          <textarea
            className="theme-input min-h-24 w-full resize-none"
            placeholder={t(
              "例如：你好，请告诉我你现在能做什么；或上传附件让我看看",
              "For example: Hello, tell me what you can do; or upload an attachment for review",
            )}
            value={chatInput}
            onChange={(event) => onChatInputChange(event.target.value)}
            onKeyDown={onChatInputKeyDown}
          />
        </div>
        <button
          type="button"
          onClick={() => void onSendMessage()}
          disabled={
            chatSending || chatRecording || (!chatInput.trim() && chatAttachments.length === 0)
          }
          className="theme-accent-btn shrink-0"
        >
          {chatSending ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="h-4 w-4" />
          )}
          {t("发送", "Send")}
        </button>
      </div>
      {chatError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("聊天错误", "Chat error")}: {chatError}
        </p>
      ) : null}
      </div>
    </section>
  );
}

function TeachingRunSnapshot({
  t,
  run,
  debug,
}: {
  t: Translate;
  run: ChatTeachingRunSummary | null;
  debug: TaskLlmDebugResponse | null;
}) {
  if (!run) return null;
  const flow = debug?.flow_summary ?? null;
  const tokens = [
    run.taskId ? `task_id=${run.taskId}` : null,
    `status=${run.status}`,
    `trace_loaded=${run.hasTrace}`,
    `llm_calls=${flow?.call_count ?? run.callCount ?? 0}`,
    flow ? `stage_count=${flow.stage_count}` : null,
    flow ? `verifier_call_count=${flow.verifier_call_count}` : null,
    flow ? `finalizer_call_count=${flow.finalizer_call_count}` : null,
  ].filter((item): item is string => Boolean(item));

  return (
    <div className="border-y border-white/10 py-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm font-semibold">{t("当前教学轮次", "Selected teaching turn")}</p>
        {run.traceError ? (
          <span className="rounded-md border border-red-300/25 bg-red-500/10 px-2 py-1 text-xs text-red-100">
            {t("调用明细查询失败", "Trace query failed")}
          </span>
        ) : null}
      </div>
      <div className="flex flex-wrap gap-2">
        {tokens.map((item) => (
          <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-[11px] text-white/65">
            {item}
          </span>
        ))}
      </div>
      <p className="mt-2 line-clamp-2 break-words text-xs text-white/55">
        {run.userText}
      </p>
    </div>
  );
}

function TeachingRunHistory({
  t,
  runs,
  activeRunId,
  toLocalTime,
  onSelectRun,
}: {
  t: Translate;
  runs: ChatTeachingRunSummary[];
  activeRunId: string | null;
  toLocalTime: (value: number | null | undefined) => string;
  onSelectRun: (runId: string) => void;
}) {
  if (runs.length === 0) {
    return (
      <div className="rounded-xl border border-white/10 bg-[#12151f] p-3 text-xs text-white/55">
        {t(
          "教学历史会保留每一次对话的任务、回复和模型调用入口。",
          "Teaching history keeps each turn's task, response, and model-call trace entry point.",
        )}
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-white/10 bg-[#12151f] p-3">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="text-sm font-semibold">{t("教学历史", "Teaching history")}</p>
          <p className="mt-1 text-xs text-white/50">
            {t(
              "每条记录对应一次对话。切换后可查看该任务的完整 LLM 请求和返回。",
              "Each record maps to one turn. Switch records to inspect that task's full LLM request and response trace.",
            )}
          </p>
        </div>
        <span className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-xs text-white/60">
          {runs.length}
        </span>
      </div>
      <div className="grid max-h-64 gap-2 overflow-auto pr-1 md:grid-cols-2">
        {runs.map((run) => {
          const active = run.id === activeRunId || run.selected;
          return (
            <button
              type="button"
              key={run.id}
              onClick={() => onSelectRun(run.id)}
              className={
                active
                  ? "min-w-0 rounded-lg border border-sky-300/40 bg-sky-500/15 p-3 text-left"
                  : "min-w-0 rounded-lg border border-white/10 bg-black/20 p-3 text-left hover:bg-white/5"
              }
            >
              <div className="mb-2 flex flex-wrap items-center gap-1.5 text-[10px] text-white/55">
                <span>{toLocalTime(run.startedAt)}</span>
                <span className="rounded border border-white/10 px-1.5 py-0.5 font-mono">
                  {run.status}
                </span>
                {run.callCount != null ? (
                  <span className="rounded border border-white/10 px-1.5 py-0.5 font-mono">
                    LLM={run.callCount}
                  </span>
                ) : null}
                {run.hasTrace ? (
                  <span className="rounded border border-emerald-300/25 px-1.5 py-0.5 text-emerald-100">
                    {t("已加载", "Loaded")}
                  </span>
                ) : null}
                {run.traceError ? (
                  <span className="rounded border border-red-300/25 px-1.5 py-0.5 text-red-100">
                    {t("查询失败", "Trace error")}
                  </span>
                ) : null}
              </div>
              <p className="line-clamp-2 min-h-9 break-words text-xs text-white/85">
                {run.userText}
              </p>
              {run.assistantText ? (
                <p className="mt-2 line-clamp-2 min-h-8 break-words text-[11px] text-white/50">
                  {run.assistantText}
                </p>
              ) : null}
              {run.taskId ? (
                <p className="mt-2 truncate font-mono text-[10px] text-white/40" title={run.taskId}>
                  task_id={run.taskId}
                </p>
              ) : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function chatStatusLabel(
  status: TaskQueryResponse["status"] | "running",
  t: Translate,
): string {
  switch (status) {
    case "queued":
      return t("排队中", "Queued");
    case "running":
      return t("运行中", "Running");
    case "succeeded":
      return t("已完成", "Done");
    case "failed":
      return t("失败", "Failed");
    case "canceled":
      return t("已取消", "Canceled");
    case "timeout":
      return t("超时", "Timed out");
    default:
      return status;
  }
}

function chatStatusBadgeClass(status: TaskQueryResponse["status"] | "running"): string {
  const base = "rounded-full border px-1.5 py-0.5";
  if (status === "succeeded") return `${base} border-emerald-300/30 text-emerald-100`;
  if (status === "failed" || status === "timeout") return `${base} border-rose-300/30 text-rose-100`;
  if (status === "canceled") return `${base} border-white/10 text-white/55`;
  return `${base} border-sky-300/30 text-sky-100`;
}

function AttachmentPreview({
  attachment,
  t,
  compact = false,
}: {
  attachment: ChatAttachment;
  t: Translate;
  compact?: boolean;
}) {
  if (attachmentIsImage(attachment)) {
    return (
      <img
        src={attachment.dataUrl}
        alt={attachment.name}
        className={
          compact
            ? "h-20 w-20 rounded-lg border border-white/10 object-cover"
            : "max-h-40 rounded-lg border border-white/10 object-contain"
        }
      />
    );
  }
  if (attachmentIsAudio(attachment)) {
    return (
      <div
        className={
          compact
            ? "w-52 rounded-lg border border-white/10 bg-black/25 p-2"
            : "w-64 rounded-lg border border-white/10 bg-black/25 p-2"
        }
      >
        <div className="mb-2 flex items-center gap-2 text-xs text-white/75">
          <Mic className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 truncate" title={attachment.name}>
            {attachment.name}
          </span>
        </div>
        <audio
          controls
          src={attachment.dataUrl}
          className="h-8 w-full"
          title={t("语音预览", "Voice preview")}
        />
      </div>
    );
  }
  return (
    <div
      className={
        compact
          ? "flex h-20 w-44 items-center gap-2 rounded-lg border border-white/10 bg-black/25 p-2"
          : "flex max-w-72 items-center gap-2 rounded-lg border border-white/10 bg-black/25 p-2"
      }
    >
      <FileText className="h-5 w-5 shrink-0 text-white/70" />
      <div className="min-w-0 text-xs">
        <div className="truncate text-white/80" title={attachment.name}>
          {attachment.name}
        </div>
        <div className="text-white/45">{formatAttachmentSize(attachment.size)}</div>
      </div>
    </div>
  );
}
