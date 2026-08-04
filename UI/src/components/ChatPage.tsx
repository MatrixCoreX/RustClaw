import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";
import {
  Check,
  Download,
  ExternalLink,
  FileArchive,
  FileAudio,
  FileText,
  FileVideo,
  Image as ImageIcon,
  Loader2,
  Maximize2,
  MessageSquare,
  Mic,
  Minimize2,
  Paperclip,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Pencil,
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
import {
  teachingMessageInteractive,
  teachingRunByMessageId,
} from "../lib/chat-teaching";
import type { ChatActivitySummary } from "../lib/chat-activity";
import {
  fetchTaskArtifactBlob,
  MAX_AUTOMATIC_ARTIFACT_PREVIEW_BYTES,
  saveTaskArtifactBlob,
  taskArtifactBrowserVideoUrl,
  taskArtifactVideoPosterUrl,
  type ArtifactFetch,
} from "../lib/task-artifact-content";
import { artifactPreviewKind } from "../lib/task-artifacts";
import type { ChatAttachment, ChatMessage, TaskArtifact, TaskLlmDebugResponse, TaskQueryResponse } from "../types/api";
import type {
  VoiceInputDeviceOption,
  VoiceRecordingAvailability,
} from "../lib/voice-recording";
import { TaskLlmTracePanel } from "./TaskLlmTracePanel";

type Translate = (zh: string, en: string) => string;
type TranslateSlash = (text: string) => string;

interface ChatThreadSummary {
  id: string;
  agentId: string;
  title: string;
  preview: string;
  updatedAt: number;
  messageCount: number;
  teachingMode: boolean;
  taskId: string | null;
  taskStatus: TaskQueryResponse["status"] | "running" | null;
  llmCallCount: number | null;
}

interface ChatTeachingRunSummary {
  id: string;
  taskId: string | null;
  userMessageId: string;
  assistantMessageId: string | null;
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
  artifactFetch: ArtifactFetch;
  chatMessages: ChatMessage[];
  chatThreads: ChatThreadSummary[];
  activeChatThreadId: string;
  availableAgents: Array<{ id: string; name: string }>;
  activeChatAgentId: string;
  activeChatCanChangeAgent: boolean;
  chatInput: string;
  chatAttachments: ChatAttachment[];
  chatTeachingMode: boolean;
  chatTeachingTaskResult: TaskQueryResponse | null;
  chatTeachingLlmDebug: TaskLlmDebugResponse | null;
  chatTeachingLlmDebugLoading: boolean;
  chatTeachingLlmDebugError: string | null;
  chatTeachingRuns: ChatTeachingRunSummary[];
  activeChatTeachingRunId: string | null;
  chatSending: boolean;
  chatWorking: boolean;
  chatActivity: ChatActivitySummary;
  chatRecording: boolean;
  chatVoiceRecordingSupported: boolean;
  chatVoiceRecordingAvailability: VoiceRecordingAvailability;
  chatAudioInputDevices: VoiceInputDeviceOption[];
  chatAudioInputDeviceId: string;
  chatError: string | null;
  chatHistoryHasMore: boolean;
  chatHistoryLoading: boolean;
  chatBodyLoadingMessageId: string | null;
  chatAttachmentInputRef: RefObject<HTMLInputElement | null>;
  toLocalTime: (value: number | null | undefined) => string;
  onChatTeachingModeChange: (value: boolean) => void;
  onSelectChatTeachingRun: (runId: string) => void;
  onCreateNewChatThread: () => void;
  onSelectChatThread: (threadId: string) => void;
  onActiveChatAgentChange: (agentId: string) => void;
  onRenameChatThread: (threadId: string, title: string) => Promise<boolean>;
  onDeleteChatThread: (threadId: string) => void | Promise<boolean>;
  onLoadEarlierConversationHistory: () => void | Promise<unknown>;
  onLoadNextChatMessageBody: (messageId: string) => void | Promise<unknown>;
  onClearMessages: () => void | Promise<boolean>;
  onChatInputChange: (value: string) => void;
  onChatInputKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onAttachmentSelection: (fileList: FileList | null) => unknown | Promise<unknown>;
  onRemoveAttachment: (index: number) => void;
  onStartVoiceRecording: () => unknown | Promise<unknown>;
  onStopVoiceRecording: () => unknown | Promise<unknown>;
  onCancelVoiceRecording: () => unknown | Promise<unknown>;
  onAudioInputDeviceChange: (deviceId: string) => void;
  onSendMessage: () => unknown | Promise<unknown>;
  onQueryChatTeachingLlmDebug: (taskId?: string) => unknown | Promise<unknown>;
}

export function ChatPage({
  t,
  tSlash,
  artifactFetch,
  chatMessages,
  chatThreads,
  activeChatThreadId,
  availableAgents,
  activeChatAgentId,
  activeChatCanChangeAgent,
  chatInput,
  chatAttachments,
  chatTeachingMode,
  chatTeachingTaskResult,
  chatTeachingLlmDebug,
  chatTeachingLlmDebugLoading,
  chatTeachingLlmDebugError,
  chatTeachingRuns,
  activeChatTeachingRunId,
  chatSending,
  chatWorking,
  chatActivity,
  chatRecording,
  chatVoiceRecordingSupported,
  chatVoiceRecordingAvailability,
  chatAudioInputDevices,
  chatAudioInputDeviceId,
  chatError,
  chatHistoryHasMore,
  chatHistoryLoading,
  chatBodyLoadingMessageId,
  chatAttachmentInputRef,
  toLocalTime,
  onChatTeachingModeChange,
  onSelectChatTeachingRun,
  onCreateNewChatThread,
  onSelectChatThread,
  onActiveChatAgentChange,
  onRenameChatThread,
  onDeleteChatThread,
  onLoadEarlierConversationHistory,
  onLoadNextChatMessageBody,
  onClearMessages,
  onChatInputChange,
  onChatInputKeyDown,
  onAttachmentSelection,
  onRemoveAttachment,
  onStartVoiceRecording,
  onStopVoiceRecording,
  onCancelVoiceRecording,
  onAudioInputDeviceChange,
  onSendMessage,
  onQueryChatTeachingLlmDebug,
}: ChatPageProps) {
  const [taskHistoryExpanded, setTaskHistoryExpanded] = useState(true);
  const [chatMaximized, setChatMaximized] = useState(false);
  const [threadSearch, setThreadSearch] = useState("");
  const [renamingThreadId, setRenamingThreadId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameSaving, setRenameSaving] = useState(false);
  const messageListRef = useRef<HTMLDivElement | null>(null);
  const normalizedThreadSearch = threadSearch.trim().toLowerCase();
  const visibleChatThreads = useMemo(() => {
    if (!normalizedThreadSearch) return chatThreads;
    return chatThreads.filter((thread) => {
      const searchText = [
        thread.title,
        thread.preview,
        thread.taskId ?? "",
        thread.taskStatus ?? "",
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
  const teachingRunByMessage = useMemo(
    () => teachingRunByMessageId(chatTeachingRuns),
    [chatTeachingRuns],
  );
  const activeThread =
    chatThreads.find((thread) => thread.id === activeChatThreadId) ?? null;

  const beginRename = (thread: ChatThreadSummary) => {
    setRenamingThreadId(thread.id);
    setRenameDraft(thread.title);
  };
  const cancelRename = () => {
    if (renameSaving) return;
    setRenamingThreadId(null);
    setRenameDraft("");
  };
  const createNewChatThread = () => {
    setTaskHistoryExpanded(true);
    setThreadSearch("");
    onCreateNewChatThread();
  };
  const saveRename = async (threadId: string) => {
    if (renameSaving) return;
    setRenameSaving(true);
    const saved = await onRenameChatThread(threadId, renameDraft);
    setRenameSaving(false);
    if (saved) {
      setRenamingThreadId(null);
      setRenameDraft("");
    }
  };

  useEffect(() => {
    const messageList = messageListRef.current;
    if (!messageList) return;
    const frame = window.requestAnimationFrame(() => {
      messageList.scrollTo({
        top: messageList.scrollHeight,
        behavior: chatSending ? "smooth" : "auto",
      });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [activeChatThreadId, chatMessages.length, chatSending, chatWorking]);
  useEffect(() => {
    if (!chatMaximized) return;
    const previousBodyOverflow = document.body.style.overflow;
    const restoreFromEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") setChatMaximized(false);
    };
    document.body.style.overflow = "hidden";
    window.addEventListener("keydown", restoreFromEscape);
    return () => {
      document.body.style.overflow = previousBodyOverflow;
      window.removeEventListener("keydown", restoreFromEscape);
    };
  }, [chatMaximized]);
  const teachingPanelVisible = chatTeachingMode;
  const selectTeachingRunFromMessage = (run: ChatTeachingRunSummary) => {
    if (!chatTeachingMode) return;
    onSelectChatTeachingRun(run.id);
  };

  return (
    <section
      className={
        taskHistoryExpanded
          ? "grid gap-4 md:h-full md:min-h-0 md:grid-cols-[18rem_minmax(0,1fr)] md:overflow-hidden"
          : "grid gap-4 md:h-full md:min-h-0 md:grid-cols-[3.75rem_minmax(0,1fr)] md:overflow-hidden"
      }
    >
      <aside className="self-start rounded-2xl border border-white/10 bg-white/5 p-3 md:h-full md:min-h-0 md:overflow-hidden">
        <div
          className={
            taskHistoryExpanded
              ? "mb-3 flex items-center justify-between gap-2"
              : "flex items-center justify-between gap-2 md:flex-col"
          }
        >
          <h3
            className={
              taskHistoryExpanded
                ? "text-sm font-semibold"
                : "text-sm font-semibold md:sr-only"
            }
          >
            {t("任务历史", "Task history")}
          </h3>
          <div
            className={
              taskHistoryExpanded
                ? "flex items-center gap-1"
                : "flex items-center gap-1 md:flex-col"
            }
          >
            <button
              type="button"
              onClick={createNewChatThread}
              className={
                taskHistoryExpanded
                  ? "inline-flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-2.5 py-1.5 text-xs hover:bg-white/10"
                  : "theme-icon-btn h-8 w-8"
              }
              title={t("新建任务", "New task")}
              aria-label={t("新建任务", "New task")}
            >
              <Plus className="h-3.5 w-3.5" />
              {taskHistoryExpanded ? t("新任务", "New") : null}
            </button>
            <button
              type="button"
              onClick={() => setTaskHistoryExpanded((expanded) => !expanded)}
              className="theme-icon-btn h-8 w-8"
              title={
                taskHistoryExpanded
                  ? t("收起任务历史", "Collapse task history")
                  : t("展开任务历史", "Expand task history")
              }
              aria-label={
                taskHistoryExpanded
                  ? t("收起任务历史", "Collapse task history")
                  : t("展开任务历史", "Expand task history")
              }
              aria-expanded={taskHistoryExpanded}
              aria-controls="chat-task-history-content"
            >
              {taskHistoryExpanded ? (
                <PanelLeftClose className="h-4 w-4" />
              ) : (
                <PanelLeftOpen className="h-4 w-4" />
              )}
            </button>
          </div>
        </div>
        <div id="chat-task-history-content" hidden={!taskHistoryExpanded}>
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
            <div className="max-h-[34rem] space-y-2 overflow-auto pr-1 md:max-h-[calc(100vh-12rem)]">
          {visibleChatThreads.map((thread) => {
            const active = thread.id === activeChatThreadId;
            const renaming = thread.id === renamingThreadId;
            return (
              <div
                key={thread.id}
                className={
                  active
                    ? "grid grid-cols-[minmax(0,1fr)_auto] gap-1 rounded-xl border border-emerald-400/35 bg-emerald-500/15 p-2"
                    : "grid grid-cols-[minmax(0,1fr)_auto] gap-1 rounded-xl border border-white/10 bg-black/20 p-2 hover:bg-white/5"
                }
              >
                {renaming ? (
                  <form
                    className="col-span-2 flex min-w-0 items-center gap-1"
                    onSubmit={(event) => {
                      event.preventDefault();
                      void saveRename(thread.id);
                    }}
                  >
                    <input
                      autoFocus
                      value={renameDraft}
                      maxLength={120}
                      onChange={(event) => setRenameDraft(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Escape") cancelRename();
                      }}
                      className="min-w-0 flex-1 rounded-lg border border-emerald-300/40 bg-black/30 px-2 py-1.5 text-sm text-white outline-none focus:border-emerald-300/70"
                      aria-label={t("任务名称", "Task name")}
                    />
                    <button
                      type="submit"
                      disabled={renameSaving || !renameDraft.trim()}
                      className="h-8 w-8 rounded-lg border border-white/10 bg-white/5 p-1.5 text-white/70 hover:bg-white/10 disabled:opacity-40"
                      title={t("保存名称", "Save name")}
                    >
                      {renameSaving ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <Check className="h-4 w-4" />
                      )}
                    </button>
                    <button
                      type="button"
                      disabled={renameSaving}
                      onClick={cancelRename}
                      className="h-8 w-8 rounded-lg border border-white/10 bg-white/5 p-1.5 text-white/60 hover:bg-white/10 disabled:opacity-40"
                      title={t("取消", "Cancel")}
                    >
                      <X className="h-4 w-4" />
                    </button>
                  </form>
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={() => {
                        onSelectChatThread(thread.id);
                      }}
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
                    {availableAgents.length > 1 ? (
                      <span className="rounded-full border border-white/10 px-1.5 py-0.5">
                        {availableAgents.find((agent) => agent.id === thread.agentId)?.name ?? thread.agentId}
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
                    <div className="flex flex-col gap-1">
                      <button
                        type="button"
                        onClick={() => beginRename(thread)}
                        className="h-7 w-7 rounded-lg border border-white/10 bg-white/5 p-1.5 text-white/55 hover:bg-white/10 hover:text-white/80"
                        title={t("重命名任务", "Rename task")}
                        aria-label={t(`重命名任务：${thread.title}`, `Rename task: ${thread.title}`)}
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>
                      <button
                        type="button"
                        onClick={() => void onDeleteChatThread(thread.id)}
                        className="h-7 w-7 rounded-lg border border-red-300/15 bg-red-500/5 p-1.5 text-red-200/70 hover:bg-red-500/15 hover:text-red-100"
                        title={t("删除任务", "Delete task")}
                        aria-label={t(`删除任务：${thread.title}`, `Delete task: ${thread.title}`)}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </>
                )}
              </div>
            );
          })}
          {visibleChatThreads.length === 0 ? (
            <div className="rounded-xl border border-white/10 bg-black/20 p-3 text-xs text-white/50">
              {t("没有匹配的任务。", "No matching tasks.")}
            </div>
          ) : null}
          {chatHistoryHasMore && !normalizedThreadSearch ? (
            <button
              type="button"
              disabled={chatHistoryLoading}
              onClick={() => void onLoadEarlierConversationHistory()}
              className="mt-2 inline-flex w-full items-center justify-center gap-2 rounded-xl border border-white/10 bg-white/5 px-3 py-2 text-xs text-white/70 hover:bg-white/10 disabled:cursor-wait disabled:opacity-55"
            >
              {chatHistoryLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
              {chatHistoryLoading
                ? t("正在加载...", "Loading...")
                : t("加载更早的任务", "Load earlier tasks")}
            </button>
          ) : null}
            </div>
        </div>
      </aside>

      <div
        id="agent-chat-window"
        className={`flex min-w-0 flex-col rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5 md:min-h-0 md:overflow-hidden ${
          chatMaximized ? "chat-window-maximized" : "md:h-full"
        }`}
      >
        <div
          className="mb-4 flex shrink-0 cursor-default select-none flex-wrap items-center justify-between gap-3"
          onDoubleClick={(event) => {
            const target = event.target;
            if (
              target instanceof Element &&
              target.closest("button, input, label, select, textarea, a, [role='button']")
            ) {
              return;
            }
            setChatMaximized((maximized) => !maximized);
          }}
          title={
            chatMaximized
              ? t("双击标题栏恢复聊天窗口", "Double-click the title bar to restore the chat window")
              : t(
                  "双击标题栏占满浏览器窗口",
                  "Double-click the title bar to fill the browser window",
                )
          }
        >
          <div className="min-w-0">
            <p className="text-xs text-white/45">Agent</p>
            <h3 className="truncate text-base font-semibold" title={activeThread?.title}>
              {activeThread?.title ?? t("新任务", "New task")}
            </h3>
          </div>
        <div className="flex flex-wrap items-center gap-3 text-sm">
          {availableAgents.length > 1 ? (
            <label
              className="inline-flex items-center gap-2 text-white/75"
              title={
                activeChatCanChangeAgent
                  ? t("为这个新任务选择 Agent", "Choose an Agent for this new task")
                  : t("已有消息后不能切换；请新建任务", "Create a new task to switch Agents")
              }
            >
              <span>{t("使用", "Use")}</span>
              <select
                value={activeChatAgentId}
                disabled={!activeChatCanChangeAgent}
                onChange={(event) => onActiveChatAgentChange(event.target.value)}
                className="rounded-lg border border-white/15 bg-black/30 px-2 py-1.5 text-xs text-white outline-none disabled:cursor-not-allowed disabled:opacity-55"
                aria-label={t("选择 Agent", "Choose Agent")}
              >
                {availableAgents.map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.name}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
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
            onClick={(event) => {
              event.stopPropagation();
              setChatMaximized((maximized) => !maximized);
            }}
            className="inline-flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/5 px-2.5 py-1.5 text-xs hover:bg-white/10"
            title={
              chatMaximized
                ? t("恢复聊天窗口（也可按 Esc）", "Restore chat window (or press Esc)")
                : t("占满浏览器窗口", "Fill browser window")
            }
            aria-label={
              chatMaximized
                ? t("恢复聊天窗口", "Restore chat window")
                : t("占满浏览器窗口", "Fill browser window")
            }
            aria-pressed={chatMaximized}
            aria-controls="agent-chat-window"
          >
            {chatMaximized ? (
              <Minimize2 className="h-3.5 w-3.5" />
            ) : (
              <Maximize2 className="h-3.5 w-3.5" />
            )}
            {chatMaximized ? t("恢复", "Restore") : t("全屏", "Full screen")}
          </button>
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              void onClearMessages();
            }}
            className="rounded-lg border border-white/15 bg-white/5 px-3 py-1.5 text-xs hover:bg-white/10"
          >
            {t("清空记录", "Clear")}
          </button>
        </div>
      </div>

      <div
        ref={messageListRef}
        className="agent-chat-message-list min-h-80 flex-1 space-y-3 overflow-y-auto rounded-xl border border-white/10 bg-black/30 p-3 md:min-h-0"
      >
        {chatMessages.map((message) => {
          const messageTeachingRun = teachingRunByMessage.get(message.id) ?? null;
          const messageTeachingEnabled = teachingMessageInteractive(chatTeachingMode, messageTeachingRun);
          const messageTeachingSelected = Boolean(
            messageTeachingEnabled &&
              (messageTeachingRun.id === activeChatTeachingRunId || messageTeachingRun.selected),
          );
          const bubbleClass =
            message.role === "user"
              ? "theme-user-bubble max-w-[95%] rounded-xl px-3 py-2 text-sm text-white"
              : message.role === "assistant"
                ? "max-w-[95%] rounded-xl bg-emerald-500/15 px-3 py-2 text-sm text-white"
                : "max-w-[95%] rounded-xl bg-white/10 px-3 py-2 text-sm text-white/80";
          return (
            <div key={message.id} className="space-y-1">
              <div className="flex items-center gap-2 text-[11px] text-white/50">
                <span>{message.role}</span>
                <span>{toLocalTime(message.ts)}</span>
                {messageTeachingEnabled ? (
                  <span className="rounded border border-sky-300/25 px-1.5 py-0.5 text-sky-100">
                    {t("教学", "Teach")}
                  </span>
                ) : null}
              </div>
              <div
                role={messageTeachingEnabled ? "button" : undefined}
                tabIndex={messageTeachingEnabled ? 0 : undefined}
                title={messageTeachingEnabled ? t("点击查看这一轮的完整教学内容", "Click to show this turn's full teaching trace") : undefined}
                onClick={() => {
                  if (messageTeachingEnabled && messageTeachingRun) {
                    selectTeachingRunFromMessage(messageTeachingRun);
                  }
                }}
                onKeyDown={(event) => {
                  if (!messageTeachingEnabled || !messageTeachingRun || (event.key !== "Enter" && event.key !== " ")) return;
                  event.preventDefault();
                  selectTeachingRunFromMessage(messageTeachingRun);
                }}
                className={`${bubbleClass} ${
                  messageTeachingEnabled
                    ? "cursor-pointer transition hover:ring-1 hover:ring-sky-300/45 focus:outline-none focus:ring-2 focus:ring-sky-300/60"
                    : ""
                } ${messageTeachingSelected ? "ring-2 ring-sky-300/65" : ""}`}
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
                {message.artifacts?.length ? (
                  <div className="mt-3 grid gap-2 sm:grid-cols-2">
                    {message.artifacts.map((artifact) => (
                      <TaskArtifactCard
                        key={`${message.id}-${artifact.id}`}
                        artifact={artifact}
                        artifactFetch={artifactFetch}
                        t={t}
                      />
                    ))}
                  </div>
                ) : null}
                {message.bodyResult && !message.bodyResult.complete ? (
                  <div className="mt-3 rounded-lg border border-sky-300/20 bg-sky-500/5 p-2.5 text-xs text-sky-100/80">
                    <p>
                      {t(
                        "结果较大，可继续查看。当前已显示",
                        "This result is large and can be continued. Currently showing",
                      )}{" "}
                      {formatAttachmentSize(message.bodyResult.returned_size_bytes)} /{" "}
                      {formatAttachmentSize(message.bodyResult.original_size_bytes)}
                    </p>
                    <button
                      type="button"
                      disabled={chatBodyLoadingMessageId !== null}
                      onClick={(event) => {
                        event.stopPropagation();
                        void onLoadNextChatMessageBody(message.id);
                      }}
                      className="mt-2 inline-flex items-center gap-1.5 rounded-lg border border-sky-300/25 bg-sky-500/10 px-2.5 py-1.5 text-sky-100 hover:bg-sky-500/20 disabled:cursor-wait disabled:opacity-55"
                    >
                      {chatBodyLoadingMessageId === message.id ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : null}
                      {chatBodyLoadingMessageId === message.id
                        ? t("正在继续读取...", "Loading more...")
                        : t("继续查看完整内容", "Show more")}
                    </button>
                  </div>
                ) : null}
              </div>
            </div>
          );
        })}
        {chatSending || chatWorking ? (
          <ChatWorkingIndicator t={t} activity={chatActivity} />
        ) : null}

      {teachingPanelVisible ? (
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

      {chatError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("聊天错误", "Chat error")}: {chatError}
        </p>
      ) : null}
      </div>

      <div className="shrink-0 pt-4">
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
              <>
                <label className="inline-flex items-center gap-1.5 text-xs text-white/70">
                  <span>{t("麦克风", "Microphone")}</span>
                  <select
                    value={chatAudioInputDeviceId}
                    onChange={(event) => onAudioInputDeviceChange(event.target.value)}
                    disabled={chatSending || chatRecording}
                    className="theme-input h-8 max-w-52 py-1 text-xs"
                    title={t("选择录音使用的麦克风", "Choose the microphone used for recording")}
                  >
                    <option value="">{t("系统默认", "System default")}</option>
                    {chatAudioInputDevices.map((device, index) => (
                      <option key={device.deviceId} value={device.deviceId}>
                        {device.label || t(`麦克风 ${index + 1}`, `Microphone ${index + 1}`)}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  type="button"
                  onPointerDown={(event) => {
                    if (event.button !== 0) return;
                    event.preventDefault();
                    event.currentTarget.setPointerCapture?.(event.pointerId);
                    void onStartVoiceRecording();
                  }}
                  onPointerUp={(event) => {
                    if (event.button !== 0) return;
                    event.preventDefault();
                    onStopVoiceRecording();
                  }}
                  onPointerCancel={() => onCancelVoiceRecording()}
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
                  {chatRecording
                    ? t("松开发送", "Release to send")
                    : t("按住发言", "Hold to talk")}
                </button>
              </>
            ) : (
              <button
                type="button"
                onClick={() => void onStartVoiceRecording()}
                className="inline-flex items-center gap-1.5 rounded-lg border border-amber-300/25 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-100 hover:bg-amber-500/15"
                title={
                  chatVoiceRecordingAvailability === "insecure_context"
                    ? t("HTTP IP 地址无法获得浏览器麦克风权限", "HTTP IP addresses cannot receive browser microphone permission")
                    : t("当前浏览器不支持直接录音", "This browser does not support direct recording")
                }
              >
                <Mic className="h-3.5 w-3.5" />
                {chatVoiceRecordingAvailability === "insecure_context"
                  ? t("语音需要 HTTPS", "Voice needs HTTPS")
                  : t("语音不可用", "Voice unavailable")}
              </button>
            )}
            <span className="text-xs text-white/45">
              {chatVoiceRecordingSupported
                ? t(
                    "按住发言，松开后自动发送；也可以发送图片或文件。",
                    "Hold to talk and release to send automatically; images and files are also supported.",
                  )
                : chatVoiceRecordingAvailability === "insecure_context"
                  ? t(
                      "浏览器不允许 HTTP IP 页面使用麦克风，请改用受信任的 HTTPS 地址。",
                      "Browsers do not allow microphone access on HTTP IP pages. Use a trusted HTTPS address.",
                    )
                : t(
                    "可直接发送图片或文件，也可以带一句说明。",
                    "Send images or files directly, with an optional note.",
                  )}
            </span>
          </div>
          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-stretch gap-2 sm:gap-3">
            <textarea
              className="theme-input h-12 min-h-12 max-h-60 w-full resize-y sm:h-[72px] sm:min-h-[72px]"
              placeholder={t(
                "例如：你好，请告诉我你现在能做什么；或上传附件让我看看",
                "For example: Hello, tell me what you can do; or upload an attachment for review",
              )}
              value={chatInput}
              onChange={(event) => onChatInputChange(event.target.value)}
              onKeyDown={onChatInputKeyDown}
            />
            <button
              type="button"
              onClick={() => void onSendMessage()}
              disabled={
                chatSending || chatRecording || (!chatInput.trim() && chatAttachments.length === 0)
              }
              className="theme-accent-btn chat-send-btn min-h-12 min-w-16 shrink-0 self-stretch justify-center sm:min-h-[72px] sm:min-w-20"
            >
              {chatSending ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="h-4 w-4" />
              )}
              {t("发送", "Send")}
            </button>
          </div>
        </div>
      </div>
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

function ChatWorkingIndicator({
  t,
  activity,
}: {
  t: Translate;
  activity: ChatActivitySummary;
}) {
  const skillProgressTitle = (() => {
    switch (activity.progressDetailKey) {
      case "media_download.precheck.starting":
        return t("正在检查媒体任务所需条件", "Checking the media task requirements");
      case "media_download.download.starting":
        return t("正在下载媒体文件", "Downloading the media file");
      case "media_download.download.completed":
        return t("媒体下载完成，正在准备后续处理", "The media download is complete; preparing the next step");
      case "media_download.transcribe.extracting_audio":
        return t("视频已下载，正在提取音频", "The video is downloaded; extracting its audio");
      case "media_download.transcribe.recognizing_speech":
        return t("音频提取完成，正在转写文字", "Audio extraction is complete; transcribing speech");
      case "media_download.transcribe.completed":
        return t("文字转写完成，正在整理结果", "Transcription is complete; preparing the result");
      case "browser_web.pages.starting":
        return t("正在打开并读取网页", "Opening and reading web pages");
      case "browser_web.pages.completed":
        return t("网页读取已完成", "Web page reading is complete");
      case "kb.operation.starting":
        return t("正在准备知识库操作", "Preparing the knowledge-base operation");
      case "package_manager.operation.starting":
        return t("正在准备软件包操作", "Preparing the package operation");
      case "skill_dispatch.queue.waiting":
        return t("当前任务已进入队列，将按顺序处理", "This task is queued and will run in order");
      case "skill_dispatch.queue.started":
        return t("前一个任务已结束，正在开始处理", "The previous task finished; processing is starting");
      default:
        return null;
    }
  })();
  const activityTitle = (() => {
    switch (activity.stage) {
      case "queued":
        return t("任务已进入队列，正在等待执行", "The task is queued and waiting to run");
      case "llm_request":
        return activity.llmCallCount > 0
          ? t(
              `第 ${activity.llmCallCount} 次 LLM 调用正在处理`,
              `LLM call ${activity.llmCallCount} is processing`,
            )
          : t("LLM 正在处理", "The LLM is processing");
      case "llm_response":
        return activity.llmCallCount > 0
          ? t(
              `第 ${activity.llmCallCount} 次 LLM 调用正在生成回复`,
              `LLM call ${activity.llmCallCount} is generating a response`,
            )
          : t("正在生成回复", "Generating a response");
      case "choosing_tool":
        return t("正在选择下一步工具或技能", "Choosing the next tool or skill");
      case "running_tool":
        return skillProgressTitle
          ? skillProgressTitle
          : activity.commandPreview
          ? t(
              `正在运行系统命令：${activity.commandPreview}`,
              `Running system command: ${activity.commandPreview}`,
            )
          : activity.activeName
          ? t(
              `正在使用 ${activity.activeName}`,
              `Using ${activity.activeName}`,
            )
          : t("正在调用工具或技能", "Calling a tool or skill");
      case "tool_returned":
        return activity.activeName
          ? t(
              `${activity.activeName} 已返回，继续处理`,
              `${activity.activeName} returned; continuing`,
            )
          : t("工具已返回，继续处理", "The tool returned; continuing");
      case "finalizing":
        return t("正在整理最终结果", "Preparing the final result");
      default:
        return t("正在分析请求", "Analyzing the request");
    }
  })();
  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={t("Agent 正在处理", "Agent is working")}
      data-testid="chat-working-indicator"
      className="space-y-1"
    >
      <div className="text-[11px] text-white/50">{t("任务状态", "Task status")}</div>
      <div className="chat-activity-sweep min-h-12 max-w-xl rounded-xl border border-emerald-300/20 bg-emerald-500/12 px-3 py-2.5 text-sm text-white">
        <div className="relative z-[1] flex items-center gap-2">
          <Loader2
            aria-hidden="true"
            className="h-4 w-4 shrink-0 text-emerald-200 motion-safe:animate-spin"
          />
          <span className="min-w-0 truncate font-medium" title={activityTitle}>
            {activityTitle}
          </span>
        </div>
        <div className="relative z-[1] mt-2 flex flex-wrap gap-1.5 text-[10px] text-white/60">
          {activity.llmCallCount > 0 ? (
            <span className="rounded-full border border-white/10 bg-black/15 px-2 py-0.5">
              LLM {activity.llmCallCount}
            </span>
          ) : null}
          {activity.roundNo ? (
            <span className="rounded-full border border-white/10 bg-black/15 px-2 py-0.5">
              {t(`第 ${activity.roundNo} 轮`, `Round ${activity.roundNo}`)}
            </span>
          ) : null}
          {activity.progressCurrent !== null && activity.progressTotal !== null ? (
            <span className="rounded-full border border-white/10 bg-black/15 px-2 py-0.5">
              {t(
                `进度 ${activity.progressCurrent}/${activity.progressTotal}${activity.progressTotal > 0 ? ` · ${Math.min(100, Math.round((activity.progressCurrent / activity.progressTotal) * 100))}%` : ""}`,
                `Progress ${activity.progressCurrent}/${activity.progressTotal}${activity.progressTotal > 0 ? ` · ${Math.min(100, Math.round((activity.progressCurrent / activity.progressTotal) * 100))}%` : ""}`,
              )}
            </span>
          ) : null}
          {activity.activeName ? (
            <span
              className="max-w-64 truncate rounded-full border border-white/10 bg-black/15 px-2 py-0.5 font-mono"
              title={activity.activeName}
            >
              {activity.activeName}
            </span>
          ) : null}
          {activity.commandPreview ? (
            <span
              className="max-w-64 truncate rounded-full border border-white/10 bg-black/15 px-2 py-0.5 font-mono"
              title={activity.commandPreview}
            >
              $ {activity.commandPreview}
            </span>
          ) : null}
        </div>
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

function TaskArtifactCard({
  artifact,
  artifactFetch,
  t,
}: {
  artifact: TaskArtifact;
  artifactFetch: ArtifactFetch;
  t: Translate;
}) {
  const previewKind = artifactPreviewKind(artifact);
  const previewUrl = artifact.preview_url ?? null;
  const mediaPreviewUrl =
    previewKind === "video" && previewUrl
      ? taskArtifactBrowserVideoUrl(previewUrl)
      : previewUrl;
  const videoPosterUrl =
    previewKind === "video" && previewUrl ? taskArtifactVideoPosterUrl(previewUrl) : null;
  const automaticPreview =
    previewKind !== "none" &&
    Boolean(previewUrl) &&
    artifact.size_bytes <= MAX_AUTOMATIC_ARTIFACT_PREVIEW_BYTES;
  const [previewRequested, setPreviewRequested] = useState(automaticPreview);
  const [previewState, setPreviewState] = useState<"idle" | "loading" | "ready" | "error">(
    automaticPreview ? "loading" : "idle",
  );
  const [previewObjectUrl, setPreviewObjectUrl] = useState<string | null>(null);
  const [videoPosterObjectUrl, setVideoPosterObjectUrl] = useState<string | null>(null);
  const [videoPreviewUnsupported, setVideoPreviewUnsupported] = useState(false);
  const [downloadLoading, setDownloadLoading] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const artifactFetchRef = useRef(artifactFetch);
  artifactFetchRef.current = artifactFetch;
  const stopBubble = (event: { stopPropagation: () => void }) => event.stopPropagation();
  const icon =
    previewKind === "image" ? (
      <ImageIcon className="h-5 w-5" />
    ) : previewKind === "audio" ? (
      <FileAudio className="h-5 w-5" />
    ) : previewKind === "video" ? (
      <FileVideo className="h-5 w-5" />
    ) : artifact.kind === "archive" ? (
      <FileArchive className="h-5 w-5" />
    ) : (
      <FileText className="h-5 w-5" />
    );

  useEffect(() => {
    if (!previewRequested || previewKind === "none" || !previewUrl) {
      setPreviewState("idle");
      setPreviewObjectUrl(null);
      setVideoPreviewUnsupported(false);
      return;
    }
    const controller = new AbortController();
    let objectUrl: string | null = null;
    setPreviewState("loading");
    setPreviewObjectUrl(null);
    setVideoPreviewUnsupported(false);
    if (!mediaPreviewUrl) {
      setPreviewState("error");
      return;
    }
    void fetchTaskArtifactBlob(artifactFetchRef.current, mediaPreviewUrl, controller.signal)
      .then((blob) => {
        if (controller.signal.aborted) return;
        objectUrl = URL.createObjectURL(blob);
        setPreviewObjectUrl(objectUrl);
        setPreviewState("ready");
      })
      .catch(() => {
        if (!controller.signal.aborted) setPreviewState("error");
      });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [artifact.id, mediaPreviewUrl, previewKind, previewRequested, previewUrl]);

  useEffect(() => {
    if (!previewRequested || !videoPosterUrl) {
      setVideoPosterObjectUrl(null);
      return;
    }
    const controller = new AbortController();
    let objectUrl: string | null = null;
    setVideoPosterObjectUrl(null);
    void fetchTaskArtifactBlob(artifactFetchRef.current, videoPosterUrl, controller.signal)
      .then((blob) => {
        if (controller.signal.aborted) return;
        objectUrl = URL.createObjectURL(blob);
        setVideoPosterObjectUrl(objectUrl);
      })
      .catch(() => {
        if (!controller.signal.aborted) setVideoPosterObjectUrl(null);
      });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [artifact.id, previewRequested, videoPosterUrl]);

  const downloadArtifact = async () => {
    if (downloadLoading) return;
    setDownloadLoading(true);
    setDownloadError(null);
    try {
      const blob = await fetchTaskArtifactBlob(artifactFetchRef.current, artifact.download_url);
      saveTaskArtifactBlob(blob, artifact.filename);
    } catch {
      setDownloadError(t("下载失败，请重新登录后再试。", "Download failed. Sign in again and retry."));
    } finally {
      setDownloadLoading(false);
    }
  };

  const requestPreview = () => {
    setPreviewState("loading");
    setPreviewRequested(true);
  };

  return (
    <div
      className="min-w-0 overflow-hidden rounded-md border border-white/12 bg-black/20"
      onClick={stopBubble}
      onKeyDown={stopBubble}
    >
      {previewKind === "image" && previewObjectUrl ? (
        <a
          href={previewObjectUrl}
          target="_blank"
          rel="noreferrer"
          className="block border-b border-white/10 bg-black/20"
          title={t("打开图片预览", "Open image preview")}
        >
          <img
            src={previewObjectUrl}
            alt={artifact.filename}
            loading="lazy"
            className="h-44 w-full object-contain"
          />
        </a>
      ) : null}
      {previewKind === "audio" && previewObjectUrl ? (
        <div className="border-b border-white/10 p-3">
          <audio
            controls
            preload="metadata"
            src={previewObjectUrl}
            className="h-9 w-full"
            title={artifact.filename}
          />
        </div>
      ) : null}
      {previewKind === "video" && previewObjectUrl && !videoPreviewUnsupported ? (
        <div className="border-b border-white/10 bg-black/25">
          <video
            controls
            playsInline
            preload="auto"
            src={previewObjectUrl}
            poster={videoPosterObjectUrl ?? undefined}
            className="aspect-video w-full object-contain"
            title={artifact.filename}
            onLoadedData={(event) => {
              const video = event.currentTarget;
              if (video.videoWidth <= 0 || video.videoHeight <= 0) {
                setVideoPreviewUnsupported(true);
                return;
              }
              if (video.currentTime === 0 && Number.isFinite(video.duration) && video.duration > 0) {
                video.currentTime = Math.min(0.05, video.duration / 100);
              }
            }}
            onError={() => setVideoPreviewUnsupported(true)}
          />
        </div>
      ) : null}
      {previewKind === "video" &&
      videoPosterObjectUrl &&
      ((previewObjectUrl && videoPreviewUnsupported) || previewState === "error") ? (
        <div className="border-b border-white/10 bg-black/25">
          <img
            src={videoPosterObjectUrl}
            alt={t(`${artifact.filename} 的视频封面`, `Video poster for ${artifact.filename}`)}
            className="max-h-[32rem] w-full object-contain"
          />
          <p className="border-t border-white/10 px-3 py-2 text-xs text-white/65">
            {previewState === "error"
              ? t(
                  "网页兼容版生成失败。高清原视频仍可下载，请重试预览或下载后播放。",
                  "The browser-compatible copy could not be generated. The original high-quality video is still available to download; retry the preview or play it after downloading.",
                )
              : t(
                  "网页兼容版仍无法播放。高清原视频可正常下载。",
                  "The browser-compatible copy still cannot play. The original high-quality video remains available to download.",
                )}
          </p>
        </div>
      ) : null}
      {previewKind !== "none" &&
      previewUrl &&
      !previewObjectUrl &&
      !(previewKind === "video" && previewState === "error" && videoPosterObjectUrl) ? (
        <div className="flex min-h-24 items-center justify-center border-b border-white/10 bg-black/20 p-3 text-center text-xs text-white/60">
          {previewState === "error" ? (
            <div>
              <p>{t("预览加载失败。", "Preview failed to load.")}</p>
              <button
                type="button"
                onClick={requestPreview}
                className="mt-2 rounded-md border border-white/15 px-2.5 py-1.5 text-white/80 hover:bg-white/10"
              >
                {t("重新加载", "Retry")}
              </button>
            </div>
          ) : previewRequested ? (
            <span className="inline-flex items-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              {previewKind === "video"
                ? t(
                    "正在准备浏览器兼容版，高清原视频不会改变…",
                    "Preparing a browser-compatible copy; the original high-quality video will not be changed…",
                  )
                : t("正在加载预览…", "Loading preview…")}
            </span>
          ) : (
            <button
              type="button"
              onClick={requestPreview}
              className="rounded-md border border-white/15 px-3 py-2 text-white/80 hover:bg-white/10"
            >
              {t("加载预览", "Load preview")}
            </button>
          )}
        </div>
      ) : null}
      <div className="flex min-w-0 items-center gap-3 p-3">
        <span className="shrink-0 text-sky-200">{icon}</span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-white/85" title={artifact.filename}>
            {artifact.filename}
          </p>
          <p className="mt-0.5 truncate text-xs text-white/45" title={artifact.mime_type}>
            {artifact.mime_type} · {formatAttachmentSize(artifact.size_bytes)}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {previewKind === "pdf" && previewObjectUrl ? (
            <a
              href={previewObjectUrl}
              target="_blank"
              rel="noreferrer"
              className="theme-icon-btn h-8 w-8"
              title={t("预览", "Preview")}
              aria-label={t(`预览 ${artifact.filename}`, `Preview ${artifact.filename}`)}
            >
              <ExternalLink className="h-4 w-4" />
            </a>
          ) : null}
          <button
            type="button"
            onClick={() => void downloadArtifact()}
            disabled={downloadLoading}
            className="theme-icon-btn h-8 w-8"
            title={t("下载", "Download")}
            aria-label={t(`下载 ${artifact.filename}`, `Download ${artifact.filename}`)}
          >
            {downloadLoading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Download className="h-4 w-4" />
            )}
          </button>
        </div>
      </div>
      {downloadError ? (
        <p className="border-t border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-100">
          {downloadError}
        </p>
      ) : null}
    </div>
  );
}
