import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import { useUiDialog } from "../components/UiDialogProvider";
import {
  assertChatAttachmentConstraints,
  attachmentIsAudio,
  attachmentIsImage,
  ChatAttachmentConstraintError,
  DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS,
  fetchChatAttachmentConstraints,
  fileToChatAttachment,
  formatVisionResultText,
} from "../lib/chat-attachments";
import {
  AssistantPresentationReducer,
  decodeAssistantPresentationEvent,
} from "../lib/assistant-presentation";
import {
  advanceConversationBodyDescriptor,
  fetchConversationHistoryPage,
  fetchNextConversationBodyPage,
  projectConversationHistory,
} from "../lib/chat-history";
import {
  emptyChatActivity,
  reduceChatActivity,
} from "../lib/chat-activity";
import { followTaskEventStream } from "../lib/task-event-stream";
import {
  CLIENT_ORIGIN_HEADER,
} from "../lib/product-identity";
import { extractTaskText } from "../lib/task-result";
import { formatUiError } from "../lib/ui-error";
import {
  extractTaskArtifactDeliverySummary,
  extractTaskArtifacts,
} from "../lib/task-artifacts";
import {
  PcmWavRecordingError,
  shouldRetryVoiceCaptureWithDefault,
  startPcmWavRecording,
  voiceAudioTrackConstraints,
  voiceRecordingAvailability,
  voiceInputDeviceOptions,
  type PcmWavRecordingSession,
  type VoiceInputDeviceOption,
} from "../lib/voice-recording";
import type {
  ApiResponse,
  ChatAttachment,
  ChannelName,
  ChatMessage,
  ConversationInputClientTaskReceipt,
  ConversationInputPage,
  ConversationInputReceipt,
  SubmitTaskResponse,
  ConversationArchiveUpdate,
  ConversationTitleUpdate,
  TaskLlmDebugResponse,
  TaskEventEnvelope,
  TaskQueryResponse,
  UiAttachmentConstraints,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export function conversationReplyMessage(event: TaskEventEnvelope): ChatMessage | null {
  const eventType = event.event_type?.trim() || event.event_kind.trim();
  if (eventType !== "conversation_reply_item") return null;
  const payload = event.payload ?? {};
  const relation = typeof payload.relation === "string" ? payload.relation.trim() : "";
  if (relation !== "side_reply" && relation !== "clarification") return null;
  const replyId = typeof payload.reply_id === "string" ? payload.reply_id.trim() : "";
  const text = typeof payload.text === "string" ? payload.text.trim() : "";
  if (!replyId || !text || payload.terminal === true) return null;
  return {
    id: `conversation-${replyId}`,
    role: "assistant",
    text,
    ts: typeof event.timestamp_ms === "number" ? event.timestamp_ms : Date.now(),
  };
}

import {
  threadHasServerHistory,
  emptyChatThreadState,
  loadChatThreadState,
  persistChatThreadState,
  mergeServerConversationHistory,
  retainLocalDraftsForPagedRestore,
  activeTaskStatus,
  terminalTaskStatus,
  formatChatAttachmentError,
  threadHasPendingTask,
  createChatThread,
  buildChatThreadSummaries,
  selectedTeachingRun,
  buildChatTeachingRunSummaries,
  appendTeachingRun,
  updateTeachingRunById,
  updateTeachingRunsByTaskId,
  debugCallCount,
  titleForThreadAfterUserMessage,
  appendThreadMessages,
  upsertThreadMessage,
  loadSelectedVoiceInputDeviceId,
  persistSelectedVoiceInputDeviceId,
  defaultAttachmentPrompt,
  defaultAttachmentMessage,
} from "../lib/chat-thread-state";
import type {
  ChatThreadRecord,
  ChatThreadState,
} from "../types/chat-runtime";
import { useDeferredChatInputs } from "./useDeferredChatInputs";
export type { ChatDeferredInputSummary, ChatThreadRecord, ChatThreadState, ChatThreadSummary, ChatTeachingRunRecord, ChatTeachingRunSummary } from "../types/chat-runtime";
export { loadChatThreadState, persistChatThreadState, mergeServerConversationHistory, retainLocalDraftsForPagedRestore, threadHasPendingTask } from "../lib/chat-thread-state";

export interface UseChatRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  lang: "zh" | "en";
  interactionAdapter: string;
  interactionChannel: ChannelName;
  activeUserKey: string;
  activeIdentityIds: Record<string, unknown>;
  conversationHistoryScope: string;
  interactionExternalUserId: string;
  interactionExternalChatId: string;
  availableAgents?: Array<{ id: string; name: string }>;
  defaultAgentId?: string;
  fetchTaskById: (id: string) => Promise<TaskQueryResponse>;
  onTaskSubmitted: (taskId: string) => void;
  onTaskResult: (taskId: string, result: TaskQueryResponse) => void;
}

export function useChatRuntime({
  apiFetch,
  t,
  lang,
  interactionAdapter,
  interactionChannel,
  activeUserKey,
  activeIdentityIds,
  conversationHistoryScope,
  interactionExternalUserId,
  interactionExternalChatId,
  availableAgents = [],
  defaultAgentId = "main",
  fetchTaskById,
  onTaskSubmitted,
  onTaskResult,
}: UseChatRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const [chatThreadState, setChatThreadState] = useState<ChatThreadState>(() =>
    emptyChatThreadState(t, defaultAgentId),
  );
  const activeChatThread =
    chatThreadState.threads.find((thread) => thread.id === chatThreadState.activeThreadId) ??
    chatThreadState.threads[0] ??
    createChatThread(t, defaultAgentId);
  const chatMessages = activeChatThread.messages;
  const chatInput = activeChatThread.input;
  const chatTeachingMode = activeChatThread.teachingMode;
  const activeTeachingRun = selectedTeachingRun(activeChatThread);
  const chatTeachingTaskResult = activeTeachingRun
    ? (activeTeachingRun.taskResult ?? null)
    : (activeChatThread.teachingTaskResult ?? null);
  const chatTeachingLlmDebug = activeTeachingRun
    ? (activeTeachingRun.llmDebug ?? null)
    : (activeChatThread.teachingLlmDebug ?? null);
  const chatTeachingLlmDebugError =
    activeTeachingRun
      ? (activeTeachingRun.llmDebugError ?? null)
      : (activeChatThread.teachingLlmDebugError ?? null);
  const chatTeachingRuns = buildChatTeachingRunSummaries(activeChatThread);
  const activeChatTeachingRunId = activeTeachingRun?.id ?? null;
  const activeChatAgentId = activeChatThread.agentId;
  const activeChatCanChangeAgent =
    !threadHasServerHistory(activeChatThread) &&
    activeChatThread.messages.every((message) => message.role === "system");
  const chatThreadSummaries = buildChatThreadSummaries(chatThreadState.threads, t);
  const [chatAttachments, setChatAttachments] = useState<ChatAttachment[]>([]);
  const [chatDeliveryMode, setChatDeliveryMode] = useState<"auto" | "defer">("auto");
  const [chatStopping, setChatStopping] = useState(false);
  const [chatTeachingLlmDebugLoading, setChatTeachingLlmDebugLoading] = useState(false);
  const [chatCompacting, setChatCompacting] = useState(false);
  const [compactingThreadId, setCompactingThreadId] = useState<string | null>(null);
  const [liveThreads, setLiveThreads] = useState<Record<string, { working: boolean; activity: ReturnType<typeof emptyChatActivity> }>>({});
  const chatSending = Boolean(liveThreads[activeChatThread.id]) || threadHasPendingTask(activeChatThread)
    || compactingThreadId === activeChatThread.id;
  const chatWorking = liveThreads[activeChatThread.id]?.working ?? false;
  const activeChatTaskId = [...(activeChatThread.teachingRuns ?? [])]
    .reverse()
    .find((run) => run.taskId && activeTaskStatus(run.status))
    ?.taskId?.trim() || null;
  const chatActivity = liveThreads[activeChatThread.id]?.activity ?? emptyChatActivity();
  const [chatRecording, setChatRecording] = useState(false);
  const [chatVoiceRecordingAvailability] = useState(voiceRecordingAvailability);
  const chatVoiceRecordingSupported = chatVoiceRecordingAvailability === "available";
  const [chatAudioInputDevices, setChatAudioInputDevices] = useState<
    VoiceInputDeviceOption[]
  >([]);
  const [chatAudioInputDeviceId, setChatAudioInputDeviceIdState] = useState(
    loadSelectedVoiceInputDeviceId,
  );
  const [chatError, setChatError] = useState<string | null>(null);
  const [chatHistoryCursor, setChatHistoryCursor] = useState<string | null>(null);
  const [chatHistoryLoading, setChatHistoryLoading] = useState(false);
  const [chatBodyLoadingMessageId, setChatBodyLoadingMessageId] = useState<string | null>(null);
  const [chatAttachmentConstraints, setChatAttachmentConstraints] =
    useState<UiAttachmentConstraints>(DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS);
  const chatAttachmentInputRef = useRef<HTMLInputElement | null>(null);
  const chatVoiceRecorderRef = useRef<PcmWavRecordingSession | null>(null);
  const chatInputValueRef = useRef("");
  const chatAttachmentsValueRef = useRef<ChatAttachment[]>([]);
  const chatRecordingValueRef = useRef(false);
  const chatTeachingModeValueRef = useRef(false);
  const chatAudioInputDeviceIdRef = useRef(chatAudioInputDeviceId);
  const activeChatThreadRef = useRef(activeChatThread);
  const apiFetchRef = useRef(apiFetch);
  const conversationHistoryScopeRef = useRef("");
  const chatHistoryLoadingRef = useRef(false);
  const teachingTraceAutoLoadKeysRef = useRef<Set<string>>(new Set());
  const liveChatTaskIdsRef = useRef<Set<string>>(new Set());
  const suspendedChatTaskIdsRef = useRef<Set<string>>(new Set());
  const conversationInputTaskIdsRef = useRef<Set<string>>(new Set());
  const recoveryAbortControllersRef = useRef<Map<string, AbortController>>(new Map());
  const submissionAbortControllersRef = useRef<
    Map<string, { threadId: string; controller: AbortController }>
  >(new Map());
  const voiceStopRequestedRef = useRef(false);

  chatInputValueRef.current = chatInput;
  chatAttachmentsValueRef.current = chatAttachments;
  chatRecordingValueRef.current = chatRecording;
  chatTeachingModeValueRef.current = chatTeachingMode;
  chatAudioInputDeviceIdRef.current = chatAudioInputDeviceId;
  activeChatThreadRef.current = activeChatThread;
  apiFetchRef.current = apiFetch;

  const beginLiveThread = (threadId: string) => {
    if (conversationHistoryScopeRef.current !== conversationHistoryScope.trim()) return;
    setLiveThreads(current => ({ ...current, [threadId]: { working: true, activity: emptyChatActivity() } }));
  };
  const finishLiveThread = (threadId: string) => {
    if (conversationHistoryScopeRef.current !== conversationHistoryScope.trim()) return;
    setLiveThreads(current => {
      const next = { ...current };
      delete next[threadId];
      return next;
    });
  };
  const updateLiveThread = (threadId: string, update: (value: { working: boolean; activity: ReturnType<typeof emptyChatActivity> }) => { working: boolean; activity: ReturnType<typeof emptyChatActivity> }) => {
    if (conversationHistoryScopeRef.current !== conversationHistoryScope.trim()) return;
    setLiveThreads(current => current[threadId] ? { ...current, [threadId]: update(current[threadId]) } : current);
  };

  useEffect(() => {
    setLiveThreads({});
    setChatAttachments([]);
    setChatError(null);
    suspendedChatTaskIdsRef.current.clear();
    for (const controller of recoveryAbortControllersRef.current.values()) controller.abort();
    recoveryAbortControllersRef.current.clear();
    for (const submission of submissionAbortControllersRef.current.values()) {
      submission.controller.abort();
    }
    submissionAbortControllersRef.current.clear();
    liveChatTaskIdsRef.current.clear();
    conversationInputTaskIdsRef.current.clear();
  }, [conversationHistoryScope]);

  useEffect(
    () => () => {
      const recorder = chatVoiceRecorderRef.current;
      chatVoiceRecorderRef.current = null;
      if (recorder) void recorder.cancel().catch(() => undefined);
      for (const controller of recoveryAbortControllersRef.current.values()) {
        controller.abort();
      }
      recoveryAbortControllersRef.current.clear();
      for (const submission of submissionAbortControllersRef.current.values()) {
        submission.controller.abort();
      }
      submissionAbortControllersRef.current.clear();
    },
    [],
  );

  useEffect(() => {
    const scope = conversationHistoryScope.trim();
    if (!scope || conversationHistoryScopeRef.current !== scope) return;
    persistChatThreadState(chatThreadState, scope);
  }, [chatThreadState, conversationHistoryScope]);

  useEffect(() => {
    if (availableAgents.length === 0) return;
    const known = new Set(availableAgents.map((agent) => agent.id));
    const fallbackAgentId = known.has(defaultAgentId)
      ? defaultAgentId
      : availableAgents[0]?.id ?? "main";
    if (chatThreadState.threads.every((thread) => known.has(thread.agentId))) return;
    setChatThreadState((current) => ({
      ...current,
      threads: current.threads.map((thread) =>
        known.has(thread.agentId) ? thread : { ...thread, agentId: fallbackAgentId },
      ),
    }));
    setChatError(
      t(
        "原任务使用的 Agent 已不存在，已切换到主 Agent。",
        "The Agent used by this task no longer exists, so it was switched to the main Agent.",
      ),
    );
  }, [availableAgents, chatThreadState.threads, defaultAgentId, t]);

  useEffect(() => {
    const scope = conversationHistoryScope.trim();
    if (!scope) {
      if (conversationHistoryScopeRef.current) {
        conversationHistoryScopeRef.current = "";
        setChatThreadState(emptyChatThreadState(t, defaultAgentId));
        setChatHistoryCursor(null);
      }
      return;
    }
    if (conversationHistoryScopeRef.current !== scope) {
      conversationHistoryScopeRef.current = scope;
      setChatThreadState(loadChatThreadState(t, scope, defaultAgentId));
      setChatHistoryCursor(null);
    }
    let cancelled = false;
    const restore = async () => {
      chatHistoryLoadingRef.current = true;
      setChatHistoryLoading(true);
      try {
        const page = await fetchConversationHistoryPage(apiFetchRef.current);
        if (cancelled) return;
        const restored = projectConversationHistory([page], t);
        setChatThreadState((current) =>
          mergeServerConversationHistory(
            retainLocalDraftsForPagedRestore(current),
            restored,
            t,
            defaultAgentId,
          ),
        );
        setChatHistoryCursor(page.truncated ? page.next_cursor?.trim() || null : null);
      } catch (error) {
        if (!cancelled) {
          console.warn(
            "conversation_history_restore_failed",
            error instanceof Error ? error.message : "unknown",
          );
        }
      }
      finally {
        if (!cancelled) setChatHistoryLoading(false);
        chatHistoryLoadingRef.current = false;
      }
    };
    void restore();
    return () => {
      cancelled = true;
    };
  }, [conversationHistoryScope, lang, defaultAgentId]);

  useEffect(() => {
    if (!conversationHistoryScope.trim()) return;
    let active = true;
    void fetchChatAttachmentConstraints(apiFetchRef.current)
      .then((constraints) => {
        if (active) setChatAttachmentConstraints(constraints);
      })
      .catch(() => {
        if (active) setChatAttachmentConstraints(DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS);
      });
    return () => {
      active = false;
    };
  }, [conversationHistoryScope]);

  const loadEarlierConversationHistory = async () => {
    const cursor = chatHistoryCursor?.trim();
    if (!cursor || chatHistoryLoadingRef.current) return;
    chatHistoryLoadingRef.current = true;
    setChatHistoryLoading(true);
    try {
      const page = await fetchConversationHistoryPage(apiFetchRef.current, cursor);
      const restored = projectConversationHistory([page], t);
      setChatThreadState((current) =>
        mergeServerConversationHistory(current, restored, t, defaultAgentId),
      );
      setChatHistoryCursor(page.truncated ? page.next_cursor?.trim() || null : null);
      setChatError(null);
    } catch (error) {
      setChatError(formatUiError(error, t, "加载更早的任务失败。", "Failed to load earlier tasks."));
    } finally {
      chatHistoryLoadingRef.current = false;
      setChatHistoryLoading(false);
    }
  };

  const loadNextChatMessageBody = async (messageId: string) => {
    if (chatBodyLoadingMessageId) return;
    const thread = activeChatThreadRef.current;
    const message = thread.messages.find((item) => item.id === messageId);
    const descriptor = message?.bodyResult;
    if (!message || !descriptor || descriptor.complete || !descriptor.continuation) return;
    setChatBodyLoadingMessageId(messageId);
    try {
      const page = await fetchNextConversationBodyPage(apiFetchRef.current, descriptor);
      const text = `${message.text}${page.text}`;
      const bodyResult = advanceConversationBodyDescriptor(descriptor, page);
      updateChatThreadById(thread.id, (current) => ({
        ...current,
        messages: current.messages.map((item) =>
          item.id === messageId ? { ...item, text, bodyResult } : item,
        ),
        teachingRuns: (current.teachingRuns ?? []).map((run) => ({
          ...run,
          ...(run.userMessageId === messageId ? { userText: text } : {}),
          ...(run.assistantMessageId === messageId ? { assistantText: text } : {}),
        })),
      }));
      setChatError(null);
    } catch (error) {
      setChatError(formatUiError(error, t, "继续读取完整内容失败。", "Failed to load more of this message."));
    } finally {
      setChatBodyLoadingMessageId(null);
    }
  };

  useEffect(() => {
    if (!chatVoiceRecordingSupported || !navigator.mediaDevices?.enumerateDevices) return;
    let active = true;
    const refresh = async () => {
      try {
        const devices = voiceInputDeviceOptions(
          await navigator.mediaDevices.enumerateDevices(),
        );
        if (active) setChatAudioInputDevices(devices);
      } catch {
        if (active) setChatAudioInputDevices([]);
      }
    };
    const handleDeviceChange = () => {
      void refresh();
    };
    void refresh();
    navigator.mediaDevices.addEventListener?.("devicechange", handleDeviceChange);
    return () => {
      active = false;
      navigator.mediaDevices.removeEventListener?.("devicechange", handleDeviceChange);
    };
  }, [chatVoiceRecordingSupported]);

  const setChatAudioInputDeviceId = (deviceId: string) => {
    const normalized = deviceId.trim();
    chatAudioInputDeviceIdRef.current = normalized;
    setChatAudioInputDeviceIdState(normalized);
    persistSelectedVoiceInputDeviceId(normalized);
  };

  const updateChatThreadById = (
    threadId: string,
    updater: (thread: ChatThreadRecord) => ChatThreadRecord,
  ) => {
    if (conversationHistoryScopeRef.current !== conversationHistoryScope.trim()) return;
    setChatThreadState((prev) => ({
      ...prev,
      threads: prev.threads.map((thread) =>
        thread.id === threadId ? updater(thread) : thread,
      ),
    }));
  };

  const updateActiveChatThread = (updater: (thread: ChatThreadRecord) => ChatThreadRecord) => {
    const threadId = activeChatThreadRef.current.id;
    updateChatThreadById(threadId, updater);
  };

  const setChatInput = (value: string) => {
    chatInputValueRef.current = value;
    updateActiveChatThread((thread) => ({ ...thread, input: value, updatedAt: Date.now() }));
  };

  const setChatTeachingMode = (value: boolean) => {
    chatTeachingModeValueRef.current = value;
    updateActiveChatThread((thread) => ({
      ...thread,
      teachingMode: value,
      updatedAt: Date.now(),
    }));
  };

  const selectChatTeachingRun = (runId: string) => {
    const selected = (activeChatThreadRef.current.teachingRuns ?? []).find(
      (item) => item.id === runId,
    );
    const selectedTaskId = selected?.taskId?.trim();
    if (selectedTaskId) {
      teachingTraceAutoLoadKeysRef.current.delete(
        `${activeChatThreadRef.current.id}:${selectedTaskId}`,
      );
    }
    updateActiveChatThread((thread) => {
      const run = (thread.teachingRuns ?? []).find((item) => item.id === runId);
      if (!run) return thread;
      return {
        ...thread,
        activeTeachingRunId: run.id,
        teachingTaskResult: run.taskResult ?? thread.teachingTaskResult ?? null,
        teachingLlmDebug: run.llmDebug ?? null,
        teachingLlmDebugError: null,
        teachingRuns: updateTeachingRunById(thread.teachingRuns, run.id, (item) => ({
          ...item,
          llmDebugError: null,
        })),
        updatedAt: Date.now(),
      };
    });
  };

  const selectChatThread = (threadId: string) => {
    if (!chatThreadState.threads.some((thread) => thread.id === threadId)) return;
    setChatThreadState((prev) => ({ ...prev, activeThreadId: threadId }));
    chatAttachmentsValueRef.current = [];
    setChatAttachments([]);
    setChatTeachingLlmDebugLoading(false);
  };

  const createNewChatThread = () => {
    const nextThread = createChatThread(t, defaultAgentId);
    setChatThreadState((prev) => ({
      activeThreadId: nextThread.id,
      threads: [nextThread, ...prev.threads],
    }));
    chatInputValueRef.current = "";
    chatAttachmentsValueRef.current = [];
    setChatAttachments([]);
    setChatTeachingLlmDebugLoading(false);
    setChatError(null);
  };

  const removeChatThreadLocally = (threadId: string) => {
    for (const [runId, submission] of submissionAbortControllersRef.current) {
      if (submission.threadId !== threadId) continue;
      submission.controller.abort();
      submissionAbortControllersRef.current.delete(runId);
    }
    setChatThreadState((prev) => {
      if (prev.threads.length <= 1) {
        const replacement = createChatThread(t, defaultAgentId);
        return { activeThreadId: replacement.id, threads: [replacement] };
      }
      const remaining = prev.threads.filter((thread) => thread.id !== threadId);
      const activeThreadId =
        prev.activeThreadId === threadId
          ? remaining[0]?.id ?? createChatThread(t, defaultAgentId).id
          : prev.activeThreadId;
      return { activeThreadId, threads: remaining };
    });
    chatInputValueRef.current = "";
    chatAttachmentsValueRef.current = [];
    setChatAttachments([]);
    setChatTeachingLlmDebugLoading(false);
  };

  const setActiveChatAgentId = (agentId: string) => {
    if (!availableAgents.some((agent) => agent.id === agentId)) return;
    if (!activeChatCanChangeAgent) {
      setChatError(
        t(
          "已有消息的任务不能切换 Agent，请新建任务后再选择。",
          "An existing task cannot switch Agents. Create a new task, then choose one.",
        ),
      );
      return;
    }
    updateActiveChatThread((thread) => ({ ...thread, agentId, updatedAt: Date.now() }));
    setChatError(null);
  };

  const archiveChatThreadOnServer = async (thread: ChatThreadRecord) => {
    if (!threadHasServerHistory(thread)) return;
    const response = await apiFetch(
      `/v1/tasks/conversations/${encodeURIComponent(thread.id)}`,
      { method: "DELETE" },
    );
    const body = (await response.json()) as ApiResponse<ConversationArchiveUpdate>;
    if (
      !response.ok ||
      !body.ok ||
      !body.data ||
      body.data.status !== "ok" ||
      body.data.conversation_id !== thread.id
    ) {
      throw new Error(body.error || `conversation_archive_http_${response.status}`);
    }
  };

  const deleteChatThread = async (threadId: string): Promise<boolean> => {
    const thread = chatThreadState.threads.find((candidate) => candidate.id === threadId);
    if (!thread) return false;
    if (
      !(await showConfirm({
        title: t("删除任务记录", "Remove task history"),
        message: t(
          "删除这个任务及其对话记录？任务执行证据会安全保留，但不会再显示在对话列表中。",
          "Remove this task and its conversation history? Execution evidence will be retained safely but hidden from the conversation list.",
        ),
        confirmLabel: t("删除", "Remove"),
        tone: "danger",
      }))
    ) {
      return false;
    }
    try {
      await archiveChatThreadOnServer(thread);
      removeChatThreadLocally(threadId);
      setChatError(null);
      return true;
    } catch {
      setChatError(
        t(
          "任务记录删除失败，请检查连接后重试。",
          "The task history could not be removed. Check the connection and try again.",
        ),
      );
      return false;
    }
  };

  const renameChatThread = async (threadId: string, rawTitle: string): Promise<boolean> => {
    const title = rawTitle.trim();
    if (!title || Array.from(title).length > 120) {
      setChatError(
        t(
          "任务名称需要填写，并且不能超过 120 个字符。",
          "Enter a task name of no more than 120 characters.",
        ),
      );
      return false;
    }
    try {
      const thread = chatThreadState.threads.find((candidate) => candidate.id === threadId);
      let persistedTitle = title;
      if (thread && threadHasServerHistory(thread)) {
        const response = await apiFetch(
          `/v1/tasks/conversations/${encodeURIComponent(threadId)}/title`,
          {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ title }),
          },
        );
        const body = (await response.json()) as ApiResponse<ConversationTitleUpdate>;
        if (
          !response.ok ||
          !body.ok ||
          !body.data ||
          body.data.status !== "ok" ||
          body.data.conversation_id !== threadId
        ) {
          throw new Error(body.error || `conversation_title_http_${response.status}`);
        }
        persistedTitle = body.data.title;
      }
      updateChatThreadById(threadId, (thread) => ({
        ...thread,
        title: persistedTitle,
        updatedAt: Date.now(),
      }));
      setChatError(null);
      return true;
    } catch {
      setChatError(
        t(
          "任务名称保存失败，请检查连接后重试。",
          "The task name could not be saved. Check the connection and try again.",
        ),
      );
      return false;
    }
  };

  const clearChatMessages = async (): Promise<boolean> => {
    const thread = activeChatThreadRef.current;
    if (
      !(await showConfirm({
        title: t("清空当前对话", "Clear current conversation"),
        message: t(
          "清空当前对话并开始一个新任务？任务执行证据会安全保留。",
          "Clear this conversation and start a new task? Execution evidence will be retained safely.",
        ),
        confirmLabel: t("清空并新建", "Clear and start new"),
        tone: "danger",
      }))
    ) {
      return false;
    }
    try {
      await archiveChatThreadOnServer(thread);
      const replacement = createChatThread(t, defaultAgentId);
      for (const [runId, submission] of submissionAbortControllersRef.current) {
        if (submission.threadId !== thread.id) continue;
        submission.controller.abort();
        submissionAbortControllersRef.current.delete(runId);
      }
      setChatThreadState((prev) => ({
        activeThreadId: replacement.id,
        threads: prev.threads.map((item) =>
          item.id === thread.id ? replacement : item,
        ),
      }));
      chatInputValueRef.current = "";
      chatAttachmentsValueRef.current = [];
      setChatAttachments([]);
      setChatTeachingLlmDebugLoading(false);
      setChatError(null);
      return true;
    } catch {
      setChatError(
        t(
          "当前对话清空失败，请检查连接后重试。",
          "The conversation could not be cleared. Check the connection and try again.",
        ),
      );
      return false;
    }
  };

  const fetchChatTeachingLlmDebugById = async (id: string): Promise<TaskLlmDebugResponse> => {
    const normalizedId = encodeURIComponent(id.trim());
    const res = await apiFetch(`/v1/debug/tasks/${normalizedId}?teaching=true`);
    const body = (await res.json()) as ApiResponse<TaskLlmDebugResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(body.error || `chat_teaching_trace_query_http_${res.status}`);
    }
    return body.data;
  };

  const queryChatTeachingLlmDebug = async (taskId?: string) => {
    const threadAtQuery = activeChatThreadRef.current;
    const targetTaskId = (
      taskId ??
      threadAtQuery.teachingTaskResult?.task_id ??
      threadAtQuery.lastTaskId ??
      ""
    ).trim();
    if (!targetTaskId) return null;
    setChatTeachingLlmDebugLoading(true);
    updateChatThreadById(threadAtQuery.id, (thread) => ({
      ...thread,
      teachingLlmDebugError: null,
    }));
    try {
      const result = await fetchChatTeachingLlmDebugById(targetTaskId);
      updateChatThreadById(threadAtQuery.id, (thread) => ({
        ...thread,
        lastTaskId: targetTaskId,
        teachingLlmDebug: result,
        teachingLlmDebugError: null,
        teachingRuns: updateTeachingRunsByTaskId(thread.teachingRuns, targetTaskId, (run) => ({
          ...run,
          llmDebug: result,
          llmDebugError: null,
          callCount: debugCallCount(result),
        })),
        updatedAt: Date.now(),
      }));
      return result;
    } catch (err) {
      const message = formatUiError(err, t, "教学过程暂时无法读取。", "The teaching trace is temporarily unavailable.");
      updateChatThreadById(threadAtQuery.id, (thread) => ({
        ...thread,
        lastTaskId: targetTaskId,
        teachingLlmDebug: null,
        teachingLlmDebugError: message,
        teachingRuns: updateTeachingRunsByTaskId(thread.teachingRuns, targetTaskId, (run) => ({
          ...run,
          llmDebug: null,
          llmDebugError: message,
        })),
        updatedAt: Date.now(),
      }));
      return null;
    } finally {
      setChatTeachingLlmDebugLoading(false);
    }
  };

  useEffect(() => {
    const targetTaskId = (
      activeTeachingRun?.taskId ??
      activeTeachingRun?.taskResult?.task_id ??
      ""
    ).trim();
    if (
      !chatTeachingMode ||
      !targetTaskId ||
      activeTeachingRun?.llmDebug ||
      activeTeachingRun?.llmDebugError ||
      chatTeachingLlmDebugLoading
    ) {
      return;
    }
    const autoLoadKey = `${activeChatThread.id}:${targetTaskId}`;
    if (teachingTraceAutoLoadKeysRef.current.has(autoLoadKey)) {
      return;
    }
    teachingTraceAutoLoadKeysRef.current.add(autoLoadKey);
    void queryChatTeachingLlmDebug(targetTaskId);
  }, [
    activeChatThread.id,
    activeTeachingRun?.id,
    activeTeachingRun?.taskId,
    activeTeachingRun?.taskResult?.task_id,
    activeTeachingRun?.llmDebug,
    activeTeachingRun?.llmDebugError,
    chatTeachingMode,
    chatTeachingLlmDebugLoading,
  ]);

  const recoverPendingChatTask = async (
    threadId: string,
    teachingRunId: string,
    taskId: string,
  ) => {
    if (
      liveChatTaskIdsRef.current.has(taskId) ||
      suspendedChatTaskIdsRef.current.has(taskId)
    ) {
      return;
    }
    const controller = new AbortController();
    liveChatTaskIdsRef.current.add(taskId);
    recoveryAbortControllersRef.current.set(taskId, controller);
    beginLiveThread(threadId);
    try {
      const presentation = new AssistantPresentationReducer();
      let streamedAssistantMessageId: string | null = null;
      await followTaskEventStream(
        apiFetch,
        taskId,
        async (event) => {
          if (controller.signal.aborted) return;
          updateLiveThread(threadId, current => ({ ...current, activity: reduceChatActivity(current.activity, event) }));
          const replyMessage = conversationReplyMessage(event);
          if (replyMessage) {
            updateChatThreadById(threadId, (thread) => ({
              ...thread,
              messages: upsertThreadMessage(thread.messages, replyMessage),
              updatedAt: Date.now(),
            }));
            if (event.payload?.relation === "clarification") {
              updateLiveThread(threadId, current => ({ ...current, working: false }));
            }
          }
          const decoded = decodeAssistantPresentationEvent(event);
          if (!decoded) {
            if (event.event_kind === "task_final") updateLiveThread(threadId, current => ({ ...current, working: false }));
            return;
          }
          const stream = await presentation.apply(decoded);
          if (!stream || (!stream.content && stream.status === "streaming")) return;
          updateLiveThread(threadId, current => ({ ...current, working: false }));
          streamedAssistantMessageId ??= `a-${taskId}`;
          const streamedMessage: ChatMessage = {
            id: streamedAssistantMessageId,
            role: "assistant",
            text: stream.content,
            ts: Date.now(),
          };
          updateChatThreadById(threadId, (thread) => ({
            ...thread,
            messages: upsertThreadMessage(thread.messages, streamedMessage),
            teachingRuns: updateTeachingRunById(
              thread.teachingRuns,
              teachingRunId,
              (run) => ({
                ...run,
                assistantMessageId: streamedMessage.id,
                assistantText: streamedMessage.text,
              }),
            ),
            updatedAt: Date.now(),
          }));
        },
        controller.signal,
      );
      if (controller.signal.aborted) return;
      const result = await fetchTaskById(taskId);
      if (controller.signal.aborted) return;
      onTaskResult(taskId, result);
      const terminal = terminalTaskStatus(result.status);
      const resultText = terminal ? extractTaskText(result) : "";
      const assistantMessage = resultText
        ? {
            id: streamedAssistantMessageId ?? `a-${taskId}`,
            role: "assistant" as const,
            text: resultText,
            ts: Date.now(),
            artifacts: extractTaskArtifacts(result),
            artifactDelivery: extractTaskArtifactDeliverySummary(result),
          }
        : null;
      updateChatThreadById(threadId, (thread) => ({
        ...thread,
        lastTaskId: taskId,
        messages: assistantMessage
          ? upsertThreadMessage(thread.messages, assistantMessage)
          : thread.messages,
        teachingTaskResult: result,
        teachingRuns: updateTeachingRunsByTaskId(
          thread.teachingRuns,
          taskId,
          (run) => ({
            ...run,
            status: result.status,
            completedAt: terminal ? Date.now() : null,
            taskResult: result,
            assistantMessageId: assistantMessage?.id ?? run.assistantMessageId ?? null,
            assistantText: assistantMessage?.text ?? run.assistantText ?? null,
          }),
        ),
        updatedAt: Date.now(),
      }));
      if (!terminal) suspendedChatTaskIdsRef.current.add(taskId);
    } catch (error) {
      if (!controller.signal.aborted) {
        if (activeChatThreadRef.current.id === threadId) {
          setChatError(formatUiError(error, t, "恢复未完成任务失败。", "Failed to resume the unfinished task."));
        }
        suspendedChatTaskIdsRef.current.add(taskId);
      }
    } finally {
      recoveryAbortControllersRef.current.delete(taskId);
      liveChatTaskIdsRef.current.delete(taskId);
      finishLiveThread(threadId);
    }
  };

  const deferredInputs = useDeferredChatInputs({
    apiFetch,
    t,
    enabled: Boolean(conversationHistoryScope.trim()),
    conversation: {
      id: activeChatThread.id,
      agentId: activeChatThread.agentId,
      externalChatId: activeChatThread.externalChatId,
    },
    onError: setChatError,
    onActivated: (input, receipt, taskId) => {
      const thread = activeChatThreadRef.current;
      const teachingRunId = `teach-${input.inputId}`;
      conversationInputTaskIdsRef.current.add(taskId);
      updateChatThreadById(thread.id, (current) => ({
        ...current,
        lastTaskId: taskId,
        activeTeachingRunId: current.teachingMode
          ? teachingRunId
          : current.activeTeachingRunId ?? null,
        teachingRuns: appendTeachingRun(current.teachingRuns, {
          id: teachingRunId,
          taskId,
          conversationInputId: input.inputId,
          conversationInputClientMessageId: receipt.client_message_id,
          conversationInputRevision: receipt.instruction_revision,
          userMessageId: `u-${input.inputId}`,
          assistantMessageId: null,
          userText: input.text || input.attachmentNames.join(" · "),
          assistantText: null,
          status: "running",
          startedAt: input.acceptedAt,
          completedAt: null,
          taskResult: {
            task_id: taskId,
            status: "running",
            result_json: null,
            error_text: null,
          },
          llmDebug: null,
          llmDebugError: null,
          callCount: null,
        }),
        updatedAt: Date.now(),
      }));
      onTaskSubmitted(taskId);
      void recoverPendingChatTask(thread.id, teachingRunId, taskId);
    },
  });
  const chatDeferredInputs = deferredInputs.items;
  const chatDeferredActionInputId = deferredInputs.actionInputId;
  const activateDeferredChatInput = deferredInputs.activate;
  const withdrawDeferredChatInput = deferredInputs.withdraw;

  useEffect(() => {
    for (const thread of chatThreadState.threads) {
      for (const run of thread.teachingRuns ?? []) {
        const taskId = run.taskId?.trim();
        if (taskId && activeTaskStatus(run.status)) {
          if (run.conversationInputId) {
            conversationInputTaskIdsRef.current.add(taskId);
          }
          void recoverPendingChatTask(thread.id, run.id, taskId);
        }
      }
    }
  }, [chatThreadState]);

  const handleChatAttachmentSelection = async (fileList: FileList | null) => {
    if (!fileList || fileList.length === 0) return;
    try {
      const selected = Array.from(fileList);
      if (selected.length === 0) {
        return;
      }
      assertChatAttachmentConstraints(
        [...chatAttachmentsValueRef.current, ...selected],
        chatAttachmentConstraints,
      );
      const nextAttachments = await Promise.all(
        selected.map((file) => fileToChatAttachment(file, undefined, chatAttachmentConstraints)),
      );
      setChatAttachments((prev) => {
        const merged = [...prev, ...nextAttachments];
        setChatError(null);
        chatAttachmentsValueRef.current = merged;
        return merged;
      });
      if (chatAttachmentInputRef.current) {
        chatAttachmentInputRef.current.value = "";
      }
    } catch (err) {
      setChatError(
        formatChatAttachmentError(err, chatAttachmentConstraints, t),
      );
    }
  };

  const removeChatAttachment = (index: number) => {
    setChatAttachments((prev) => {
      const next = prev.filter((_, i) => i !== index);
      chatAttachmentsValueRef.current = next;
      return next;
    });
  };

  const startChatVoiceRecording = async () => {
    if (chatRecordingValueRef.current) return;
    const availability = voiceRecordingAvailability();
    if (availability !== "available") {
      setChatError(
        availability === "insecure_context"
          ? t(
              "浏览器禁止 HTTP IP 地址使用麦克风。请通过受信任的 HTTPS 地址访问；如果浏览器就在 {product_name} 主机上，也可以使用 localhost。",
              "Browsers block microphone access on HTTP IP addresses. Use a trusted HTTPS address, or localhost when the browser runs on the {product_name} host.",
            )
          : t(
              "当前浏览器不支持直接录音，请改用支持录音的现代浏览器或上传音频文件。",
              "This browser does not support direct recording. Use a modern browser with recording support or upload an audio file.",
            ),
      );
      return;
    }
    try {
      voiceStopRequestedRef.current = false;
      chatRecordingValueRef.current = true;
      setChatRecording(true);
      const selectedDeviceId = chatAudioInputDeviceIdRef.current;
      let stream: MediaStream;
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          audio: voiceAudioTrackConstraints(selectedDeviceId),
        });
      } catch (error) {
        if (!selectedDeviceId || !shouldRetryVoiceCaptureWithDefault(error)) throw error;
        setChatAudioInputDeviceId("");
        stream = await navigator.mediaDevices.getUserMedia({
          audio: voiceAudioTrackConstraints(),
        });
      }
      if (voiceStopRequestedRef.current) {
        stream.getTracks().forEach((track) => track.stop());
        chatRecordingValueRef.current = false;
        setChatRecording(false);
        return;
      }
      const actualDeviceId = stream
        .getAudioTracks()[0]
        ?.getSettings()
        .deviceId?.trim();
      if (actualDeviceId && actualDeviceId !== chatAudioInputDeviceIdRef.current) {
        setChatAudioInputDeviceId(actualDeviceId);
      }
      if (navigator.mediaDevices.enumerateDevices) {
        void navigator.mediaDevices
          .enumerateDevices()
          .then((devices) => setChatAudioInputDevices(voiceInputDeviceOptions(devices)))
          .catch(() => undefined);
      }
      const recorder = await startPcmWavRecording(stream);
      chatVoiceRecorderRef.current = recorder;
      if (voiceStopRequestedRef.current) {
        void finishChatVoiceRecording(recorder);
      }
      setChatError(null);
    } catch (err) {
      chatRecordingValueRef.current = false;
      setChatRecording(false);
      setChatError(
        formatUiError(err, t, "无法开始录音。", "Unable to start recording."),
      );
    }
  };

  const stopChatVoiceRecording = () => {
    voiceStopRequestedRef.current = true;
    const recorder = chatVoiceRecorderRef.current;
    if (recorder) {
      void finishChatVoiceRecording(recorder);
    }
  };

  const cancelChatVoiceRecording = () => {
    voiceStopRequestedRef.current = true;
    const recorder = chatVoiceRecorderRef.current;
    chatVoiceRecorderRef.current = null;
    chatRecordingValueRef.current = false;
    setChatRecording(false);
    if (recorder) {
      void recorder.cancel().catch(() => undefined);
    }
  };

  const finishChatVoiceRecording = async (recorder: PcmWavRecordingSession) => {
    if (chatVoiceRecorderRef.current !== recorder) return;
    chatVoiceRecorderRef.current = null;
    chatRecordingValueRef.current = false;
    setChatRecording(false);
    try {
      const blob = await recorder.stop();
      const file = new File([blob], `voice-${Date.now()}.wav`, { type: "audio/wav" });
      const attachment = await fileToChatAttachment(file, "audio", chatAttachmentConstraints);
      const attached = [...chatAttachmentsValueRef.current, attachment];
      assertChatAttachmentConstraints(attached, chatAttachmentConstraints);
      setChatError(null);
      await submitChatMessageSnapshot(chatInputValueRef.current, attached, {
        clearInput: true,
        clearAttachments: true,
      });
    } catch (err) {
      setChatError(
        err instanceof PcmWavRecordingError && err.code === "empty"
          ? t("没有录到声音，请重新尝试。", "No audio was recorded. Please try again.")
          : err instanceof ChatAttachmentConstraintError
            ? formatChatAttachmentError(err, chatAttachmentConstraints, t)
            : t("读取录音失败，请重新尝试。", "Failed to read the recording. Please try again."),
      );
    }
  };

  const submitChatMessageSnapshot = async (
    rawText: string,
    rawAttachments: ChatAttachment[],
    options: { clearInput: boolean; clearAttachments: boolean },
  ) => {
    const text = rawText.trim();
    try {
      assertChatAttachmentConstraints(rawAttachments, chatAttachmentConstraints);
    } catch (error) {
      setChatError(formatChatAttachmentError(error, chatAttachmentConstraints, t));
      return;
    }
    const attached = rawAttachments.map(item => ({ ...item }));
    if (!text && attached.length === 0) return;
    const thread = { ...activeChatThreadRef.current };
    const runId = `teach-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    if (options.clearInput) {
      chatInputValueRef.current = "";
      updateChatThreadById(thread.id, current => ({ ...current, input: "" }));
    }
    if (options.clearAttachments) {
      chatAttachmentsValueRef.current = [];
      setChatAttachments([]);
    }
    if (chatAttachmentInputRef.current) chatAttachmentInputRef.current.value = "";
    setChatError(null);
    const deliveryMode = chatDeliveryMode;
    const activeTaskId = [...(thread.teachingRuns ?? [])]
      .reverse()
      .find((run) => run.taskId && activeTaskStatus(run.status))
      ?.taskId?.trim();
    if (deliveryMode === "defer") {
      await submitConversationInputSnapshot(
        text,
        attached,
        thread,
        runId,
        activeTaskId,
        "defer",
      );
      setChatDeliveryMode("auto");
      return;
    }
    if (
      activeTaskId &&
      conversationInputTaskIdsRef.current.has(activeTaskId) &&
      (text || attached.length > 0)
    ) {
      await submitConversationInputSnapshot(text, attached, thread, runId, activeTaskId, "auto");
      return;
    }
    const controller = new AbortController();
    submissionAbortControllersRef.current.set(runId, {
      threadId: thread.id,
      controller,
    });
    void executeChatMessageSnapshot(text, attached, thread, runId, controller.signal).finally(
      () => submissionAbortControllersRef.current.delete(runId),
    );
  };

  const submitConversationInputSnapshot = async (
    text: string,
    attached: ChatAttachment[],
    threadAtSubmit: ChatThreadRecord,
    teachingRunId: string,
    activeTaskId: string | undefined,
    deliveryMode: "auto" | "defer",
  ) => {
    const submittedAt = Date.now();
    const clientMessageId = `ui:${threadAtSubmit.id}:${teachingRunId}`;
    const attachedImages = attached.filter(attachmentIsImage);
    const attachedAudios = attached.filter(attachmentIsAudio);
    const attachedFiles = attached.filter(
      (attachment) => !attachmentIsImage(attachment) && !attachmentIsAudio(attachment),
    );
    const audioOnly = attachedAudios.length > 0 && attachedImages.length === 0 && attachedFiles.length === 0;
    const primaryAudio = attachedAudios[attachedAudios.length - 1];
    const requestText =
      text ||
      (audioOnly
        ? ""
        : defaultAttachmentPrompt(t, attachedImages.length, attachedAudios.length, attachedFiles.length));
    const userMsg: ChatMessage = {
      id: `u-${teachingRunId}`,
      role: "user",
      text:
        text ||
        defaultAttachmentMessage(t, attachedImages.length, attachedAudios.length, attachedFiles.length),
      ts: submittedAt,
      attachments: attached,
      images: attachedImages,
    };
    if (deliveryMode === "auto" && activeTaskId) {
      updateChatThreadById(threadAtSubmit.id, (thread) => ({
        ...thread,
        title: titleForThreadAfterUserMessage(thread, userMsg, t),
        messages: appendThreadMessages(thread.messages, userMsg),
        activeTeachingRunId: threadAtSubmit.teachingMode
          ? teachingRunId
          : thread.activeTeachingRunId ?? null,
        teachingRuns: appendTeachingRun(thread.teachingRuns, {
          id: teachingRunId,
          taskId: activeTaskId,
          conversationInputId: null,
          conversationInputClientMessageId: clientMessageId,
          conversationInputRevision: null,
          userMessageId: userMsg.id,
          assistantMessageId: null,
          userText: userMsg.text,
          assistantText: null,
          status: "running",
          startedAt: submittedAt,
          completedAt: null,
          taskResult: {
            task_id: activeTaskId,
            status: "running",
            result_json: null,
            error_text: null,
          },
          llmDebug: null,
          llmDebugError: null,
          callCount: null,
        }),
        updatedAt: submittedAt,
      }));
    }
    try {
      const scope = {
        conversation_id: threadAtSubmit.id,
        agent_id: threadAtSubmit.agentId,
        channel: "ui",
        channel_account_id: threadAtSubmit.externalChatId,
      };
      const submission = {
        schema_version: 1,
        client_message_id: clientMessageId,
        scope,
        content: [{ kind: "text" as const, text: requestText }],
        delivery_mode: deliveryMode,
        ...(activeTaskId ? { expected_task_id: activeTaskId } : {}),
        source: { received_at_ts: Math.floor(submittedAt / 1000) },
      };
      const attachmentPayload = attached.map((attachment) => ({
        name: attachment.name,
        mime_type: attachment.mimeType,
        size: attachment.size,
        kind: attachment.kind,
        base64: attachment.dataUrl,
      }));
      const task = {
        channel: "ui",
        kind: "ask",
        idempotency_key: clientMessageId,
        ...(activeUserKey ? { user_key: activeUserKey } : {}),
        ...activeIdentityIds,
        external_user_id: threadAtSubmit.externalChatId,
        external_chat_id: threadAtSubmit.id,
        payload: {
          text: requestText,
          conversation_id: threadAtSubmit.id,
          agent_id: threadAtSubmit.agentId,
          ...(audioOnly ? { source: "voice" } : {}),
          ...(attached.length > 0
            ? {
                attachments: attachmentPayload,
                images: attachedImages.map((image) => ({
                  name: image.name,
                  mime_type: image.mimeType,
                  size: image.size,
                  base64: image.dataUrl,
                })),
                ...(primaryAudio
                  ? {
                      audio: {
                        name: primaryAudio.name,
                        mime_type: primaryAudio.mimeType,
                        size: primaryAudio.size,
                        base64: primaryAudio.dataUrl,
                      },
                    }
                  : {}),
                response_language: lang === "zh" ? "zh-CN" : "en",
              }
            : {}),
        },
      };
      let receipt: ConversationInputReceipt;
      try {
        receipt = await submitConversationClientTaskWithRecovery(submission, task);
      } catch (error) {
        if (!(error instanceof Error) || error.message !== "conversation_input_target_conflict") {
          throw error;
        }
        if (!activeTaskId) throw error;
        const latestTarget = await fetchTaskById(activeTaskId);
        if (!terminalTaskStatus(latestTarget.status)) throw error;
        receipt = await submitConversationClientTaskWithRecovery(
          { ...submission, expected_task_id: undefined },
          task,
        );
      }
      if (deliveryMode === "defer") {
        const deferredUserMessage = { ...userMsg, id: `u-${receipt.input_id}` };
        updateChatThreadById(threadAtSubmit.id, (thread) => ({
          ...thread,
          title: titleForThreadAfterUserMessage(thread, deferredUserMessage, t),
          messages: upsertThreadMessage(thread.messages, deferredUserMessage),
          updatedAt: Date.now(),
        }));
        deferredInputs.add({
          inputId: receipt.input_id,
          text: userMsg.text,
          attachmentNames: attached.map((item) => item.name),
          acceptedAt: receipt.accepted_at_ts * 1_000,
        });
        return;
      }
      const acceptedTaskId = receipt.target_task_id?.trim();
      if (!acceptedTaskId) throw new Error("conversation_input_task_binding_missing");
      conversationInputTaskIdsRef.current.add(acceptedTaskId);
      updateChatThreadById(threadAtSubmit.id, (thread) => ({
        ...thread,
        lastTaskId: acceptedTaskId,
        teachingRuns: updateTeachingRunById(
          thread.teachingRuns,
          teachingRunId,
          (run) => ({
            ...run,
            taskId: acceptedTaskId,
            conversationInputId: receipt.input_id,
            conversationInputClientMessageId: receipt.client_message_id,
            conversationInputRevision: receipt.instruction_revision,
            taskResult: {
              task_id: acceptedTaskId,
              status: "running",
              result_json: null,
              error_text: null,
            },
          }),
        ),
        updatedAt: Date.now(),
      }));
      if (acceptedTaskId !== activeTaskId) {
        onTaskSubmitted(acceptedTaskId);
        void recoverPendingChatTask(threadAtSubmit.id, teachingRunId, acceptedTaskId);
      }
    } catch (error) {
      const message = formatUiError(
        error,
        t,
        deliveryMode === "defer"
          ? "延后消息未能保存，请重试。"
          : "补充要求未能提交，请重试。",
        deliveryMode === "defer"
          ? "The deferred message could not be saved. Please retry."
          : "The follow-up instruction could not be submitted. Please retry.",
      );
      if (activeChatThreadRef.current.id === threadAtSubmit.id) setChatError(message);
      const systemMessage: ChatMessage = {
        id: `e-${teachingRunId}`,
        role: "system",
        text: `${t("发送失败", "Send failed")}: ${message}`,
        ts: Date.now(),
      };
      updateChatThreadById(threadAtSubmit.id, (thread) => ({
        ...thread,
        messages: appendThreadMessages(thread.messages, systemMessage),
        teachingRuns:
          deliveryMode === "auto" && activeTaskId
            ? updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
                ...run,
                status: "failed",
                completedAt: systemMessage.ts,
                assistantMessageId: systemMessage.id,
                assistantText: systemMessage.text,
                taskResult: {
                  task_id: activeTaskId,
                  status: "failed",
                  result_json: null,
                  error_text: message,
                },
              }))
            : thread.teachingRuns,
        updatedAt: Date.now(),
      }));
    }
  };

  const recoverConversationInputReceipt = async (
    scope: {
      conversation_id: string;
      agent_id: string;
      channel: string;
      channel_account_id: string;
    },
    clientMessageId: string,
    signal?: AbortSignal,
  ): Promise<ConversationInputReceipt | null> => {
    const query = new URLSearchParams({
      conversation_id: scope.conversation_id,
      agent_id: scope.agent_id,
      channel: scope.channel,
      channel_account_id: scope.channel_account_id,
      client_message_id: clientMessageId,
    });
    const response = await apiFetch(`/v1/conversation-inputs?${query.toString()}`, { signal });
    const body = (await response.json()) as ApiResponse<ConversationInputPage>;
    if (response.status === 404) return null;
    if (!response.ok || !body.ok || !body.data) {
      throw new Error(body.error || `conversation_input_recovery_http_${response.status}`);
    }
    return body.data.items[0]?.receipt ?? null;
  };

  const submitConversationClientTaskWithRecovery = async (
    submission: {
      schema_version: number;
      client_message_id: string;
      scope: {
        conversation_id: string;
        agent_id: string;
        channel: string;
        channel_account_id: string;
      };
      content: Array<{ kind: "text"; text: string }>;
      delivery_mode: "auto" | "defer";
      expected_task_id?: string;
      source: { received_at_ts: number };
    },
    task: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<ConversationInputReceipt> => {
    const submit = async () => {
      const response = await apiFetch("/v1/conversation-inputs/client-task", {
        method: "POST",
        signal,
        headers: {
          "Content-Type": "application/json",
          [CLIENT_ORIGIN_HEADER]: "ui",
        },
        body: JSON.stringify({ input: submission, task }),
      });
      const body = (await response.json()) as ApiResponse<ConversationInputClientTaskReceipt>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `conversation_input_submit_http_${response.status}`);
      }
      return body.data.input;
    };
    try {
      return await submit();
    } catch (submitError) {
      if (signal?.aborted) throw submitError;
      try {
        const recovered = await recoverConversationInputReceipt(
          submission.scope,
          submission.client_message_id,
          signal,
        );
        if (recovered?.target_task_id || submission.delivery_mode === "defer") return recovered;
      } catch {
        // Retry the idempotent client-task handoff below.
      }
      try {
        return await submit();
      } catch {
        throw submitError;
      }
    }
  };

  const executeChatMessageSnapshot = async (
    text: string,
    attached: ChatAttachment[],
    threadAtSubmit: ChatThreadRecord,
    teachingRunId: string,
    signal: AbortSignal,
  ): Promise<boolean | "waiting"> => {
    if (signal.aborted || conversationHistoryScopeRef.current !== conversationHistoryScope.trim()) return false;
    const attachedImages = attached.filter(attachmentIsImage);
    const attachedAudios = attached.filter(attachmentIsAudio);
    const attachedFiles = attached.filter(
      (attachment) => !attachmentIsImage(attachment) && !attachmentIsAudio(attachment),
    );
    const audioOnly = attachedAudios.length > 0 && attachedImages.length === 0 && attachedFiles.length === 0;
    const primaryAudio = attachedAudios[attachedAudios.length - 1];
    const requestText =
      text ||
      (audioOnly
        ? ""
        : defaultAttachmentPrompt(
            t,
            attachedImages.length,
            attachedAudios.length,
            attachedFiles.length,
          ));
    const submitThreadId = threadAtSubmit.id;
    const conversationInputClientMessageId = `ui:${submitThreadId}:${teachingRunId}`;
    const teachingModeAtSubmit = threadAtSubmit.teachingMode;
    const setTurnError = (message: string | null) => {
      if (activeChatThreadRef.current.id === submitThreadId) setChatError(message);
    };
    beginLiveThread(submitThreadId);
    setTurnError(null);
    const userMsg: ChatMessage = {
      id: `u-${teachingRunId}`,
      role: "user",
      text:
        text ||
        defaultAttachmentMessage(
          t,
          attachedImages.length,
          attachedAudios.length,
          attachedFiles.length,
        ),
      ts: Date.now(),
      attachments: attached,
      images: attachedImages,
    };
    updateChatThreadById(submitThreadId, (thread) => ({
      ...thread,
      title: titleForThreadAfterUserMessage(thread, userMsg, t),
      messages: appendThreadMessages(thread.messages, userMsg),
      activeTeachingRunId: teachingModeAtSubmit
        ? teachingRunId
        : thread.activeTeachingRunId ?? null,
      teachingRuns: appendTeachingRun(thread.teachingRuns, {
        id: teachingRunId,
        taskId: null,
        conversationInputId: null,
        conversationInputClientMessageId,
        conversationInputRevision: null,
        userMessageId: userMsg.id,
        assistantMessageId: null,
        userText: userMsg.text,
        assistantText: null,
        status: "running",
        startedAt: userMsg.ts,
        completedAt: null,
        taskResult: null,
        llmDebug: null,
        llmDebugError: null,
        callCount: null,
      }),
      updatedAt: Date.now(),
    }));

    let submittedTaskId: string | null = null;
    let ownsTaskFollower = false;
    try {
      const adapterName = interactionAdapter.trim();
      const attachmentPayload = attached.map((attachment) => ({
        name: attachment.name,
        mime_type: attachment.mimeType,
        size: attachment.size,
        kind: attachment.kind,
        base64: attachment.dataUrl,
      }));
      const submitBody: Record<string, unknown> = {
        channel: "ui",
        kind: "ask",
        idempotency_key: `ui:${submitThreadId}:${teachingRunId}`,
        ...(activeUserKey ? { user_key: activeUserKey } : {}),
        ...activeIdentityIds,
        ...(interactionExternalUserId.trim() ? { external_user_id: interactionExternalUserId.trim() } : {}),
        external_chat_id: submitThreadId,
        payload: {
          text: requestText,
          conversation_id: threadAtSubmit.id,
          agent_id: threadAtSubmit.agentId,
          ...(audioOnly ? { source: "voice" } : {}),
          ...(adapterName ? { adapter: adapterName } : {}),
          ...(attached.length > 0
            ? {
                attachments: attachmentPayload,
                images: attachedImages.map((image) => ({
                  name: image.name,
                  mime_type: image.mimeType,
                  size: image.size,
                  base64: image.dataUrl,
                })),
                ...(primaryAudio
                  ? {
                      audio: {
                        name: primaryAudio.name,
                        mime_type: primaryAudio.mimeType,
                        size: primaryAudio.size,
                        base64: primaryAudio.dataUrl,
                      },
                    }
                  : {}),
                response_language: lang === "zh" ? "zh-CN" : "en",
              }
            : {}),
        },
      };
      const receipt = await submitConversationClientTaskWithRecovery(
        {
          schema_version: 1,
          client_message_id: conversationInputClientMessageId,
          scope: {
            conversation_id: submitThreadId,
            agent_id: threadAtSubmit.agentId,
            channel: "ui",
            channel_account_id: threadAtSubmit.externalChatId,
          },
          content: [{ kind: "text", text: requestText }],
          delivery_mode: "auto",
          source: { received_at_ts: Math.floor(userMsg.ts / 1000) },
        },
        submitBody,
        signal,
      );
      if (signal.aborted) return false;
      const taskId = receipt.target_task_id?.trim();
      if (!taskId) throw new Error("conversation_input_task_binding_missing");
      submittedTaskId = taskId;
      conversationInputTaskIdsRef.current.add(taskId);
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        teachingRuns: updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
          ...run,
          conversationInputId: receipt.input_id,
          conversationInputClientMessageId: receipt.client_message_id,
          conversationInputRevision: receipt.instruction_revision,
        })),
        updatedAt: Date.now(),
      }));
      const taskAlreadyFollowed = liveChatTaskIdsRef.current.has(submittedTaskId);
      if (!taskAlreadyFollowed) {
        liveChatTaskIdsRef.current.add(submittedTaskId);
        ownsTaskFollower = true;
        onTaskSubmitted(submittedTaskId);
      }
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        lastTaskId: submittedTaskId,
        activeTeachingRunId: teachingModeAtSubmit
          ? teachingRunId
          : thread.activeTeachingRunId ?? null,
        teachingTaskResult: teachingModeAtSubmit
          ? {
              task_id: submittedTaskId,
              status: "running",
              result_json: null,
              error_text: null,
            }
          : thread.teachingTaskResult,
        teachingLlmDebug: teachingModeAtSubmit ? null : thread.teachingLlmDebug,
        teachingLlmDebugError: teachingModeAtSubmit ? null : thread.teachingLlmDebugError,
        teachingRuns: updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
          ...run,
          taskId: submittedTaskId,
          status: "running",
          taskResult: {
            task_id: submittedTaskId,
            status: "running",
            result_json: null,
            error_text: null,
          },
          llmDebug: null,
          llmDebugError: null,
        })),
        updatedAt: Date.now(),
      }));
      if (taskAlreadyFollowed) return "waiting";

      const presentation = new AssistantPresentationReducer();
      let streamedAssistantMessageId: string | null = null;
      let completedPresentationText: string | null = null;
      await followTaskEventStream(apiFetch, submittedTaskId, async (event) => {
        if (signal.aborted) return;
        updateLiveThread(submitThreadId, current => ({ ...current, activity: reduceChatActivity(current.activity, event) }));
        const replyMessage = conversationReplyMessage(event);
        if (replyMessage) {
          updateChatThreadById(submitThreadId, (thread) => ({
            ...thread,
            messages: upsertThreadMessage(thread.messages, replyMessage),
            updatedAt: Date.now(),
          }));
          if (event.payload?.relation === "clarification") {
            updateLiveThread(submitThreadId, current => ({ ...current, working: false }));
          }
        }
        const decoded = decodeAssistantPresentationEvent(event);
        if (!decoded) {
          if (event.event_kind === "task_final") updateLiveThread(submitThreadId, current => ({ ...current, working: false }));
          return;
        }
        const stream = await presentation.apply(decoded);
        if (decoded.kind === "assistant_output_aborted") {
          updateLiveThread(submitThreadId, current => ({ ...current, working: true }));
        }
        if (!stream || (!stream.content && stream.status === "streaming")) return;
        updateLiveThread(submitThreadId, current => ({ ...current, working: false }));
        if (stream.status === "completed") completedPresentationText = stream.content;
        streamedAssistantMessageId ??= `a-${submittedTaskId}`;
        const streamedMessage: ChatMessage = {
          id: streamedAssistantMessageId,
          role: "assistant",
          text: stream.content,
          ts: Date.now(),
        };
        updateChatThreadById(submitThreadId, (thread) => ({
          ...thread,
          messages: upsertThreadMessage(thread.messages, streamedMessage),
          teachingRuns: updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
            ...run,
            assistantMessageId: streamedMessage.id,
            assistantText: streamedMessage.text,
          })),
          updatedAt: Date.now(),
        }));
      }, signal);
      if (signal.aborted) return false;
      const finalResult = await fetchTaskById(submittedTaskId);
      if (signal.aborted) return false;
      if (!terminalTaskStatus(finalResult.status)) suspendedChatTaskIdsRef.current.add(submittedTaskId);
      const finalTaskText = extractTaskText(finalResult);
      if (
        completedPresentationText !== null &&
        completedPresentationText !== finalTaskText
      ) {
        setTurnError(t(
          "流式回复与最终结果不一致，页面已保留最终结果。",
          "The streamed reply differed from the final result. The final result has been kept.",
        ));
      }
      onTaskResult(submittedTaskId, finalResult);
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        lastTaskId: submittedTaskId,
        teachingTaskResult: teachingModeAtSubmit ? finalResult : thread.teachingTaskResult,
        teachingRuns: updateTeachingRunsByTaskId(thread.teachingRuns, submittedTaskId, (run) => ({
          ...run,
          status: finalResult.status,
          completedAt: terminalTaskStatus(finalResult.status) ? Date.now() : null,
          taskResult: finalResult,
        })),
        updatedAt: Date.now(),
      }));
      if (teachingModeAtSubmit && activeChatThreadRef.current.id === submitThreadId) {
        void queryChatTeachingLlmDebug(submittedTaskId);
      }

      const assistantMsg: ChatMessage = {
        id: streamedAssistantMessageId ?? `a-${Date.now()}`,
        role: "assistant",
        text: attachedImages.length > 0 ? formatVisionResultText(finalTaskText) : finalTaskText,
        ts: Date.now(),
        artifacts: extractTaskArtifacts(finalResult),
        artifactDelivery: extractTaskArtifactDeliverySummary(finalResult),
      };
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        messages: upsertThreadMessage(thread.messages, assistantMsg),
        teachingRuns: updateTeachingRunsByTaskId(thread.teachingRuns, submittedTaskId, (run) => ({
          ...run,
          assistantMessageId: assistantMsg.id,
          assistantText: assistantMsg.text,
          completedAt: terminalTaskStatus(finalResult.status) ? run.completedAt ?? assistantMsg.ts : null,
        })),
        updatedAt: Date.now(),
      }));
      return terminalTaskStatus(finalResult.status) ? finalResult.status === "succeeded" : "waiting";
    } catch (err) {
      if (signal.aborted) return false;
      if (submittedTaskId) suspendedChatTaskIdsRef.current.add(submittedTaskId);
      const message = formatUiError(err, t, "连接中断，请检查任务记录后再继续。", "The connection was interrupted. Check the task history before continuing.");
      setTurnError(message);
      const systemErrMsg: ChatMessage = {
        id: `e-${Date.now()}`,
        role: "system",
        text: `${submittedTaskId ? t("任务状态待确认", "Task status unconfirmed") : t("发送失败", "Send failed")}: ${message}`,
        ts: Date.now(),
      };
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        messages: appendThreadMessages(thread.messages, systemErrMsg),
        teachingRuns: updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
          ...run,
          status: submittedTaskId ? "running" : "failed",
          assistantMessageId: systemErrMsg.id,
          assistantText: systemErrMsg.text,
          completedAt: submittedTaskId ? null : Date.now(),
          taskResult: run.taskId
            ? {
                task_id: run.taskId,
                status: "running",
                result_json: null,
                error_text: message,
              }
            : run.taskResult ?? null,
          llmDebugError: run.llmDebugError,
        })),
        updatedAt: Date.now(),
      }));
      return submittedTaskId ? "waiting" : false;
    } finally {
      if (submittedTaskId && ownsTaskFollower) {
        liveChatTaskIdsRef.current.delete(submittedTaskId);
      }
      if (ownsTaskFollower || !submittedTaskId) finishLiveThread(submitThreadId);
    }
  };

  const sendChatMessage = async () => {
    if (chatRecordingValueRef.current) return;
    await submitChatMessageSnapshot(chatInputValueRef.current, chatAttachmentsValueRef.current, {
      clearInput: true,
      clearAttachments: true,
    });
  };

  const stopActiveChatTask = async () => {
    const thread = activeChatThreadRef.current;
    const taskId = [...(thread.teachingRuns ?? [])]
      .reverse()
      .find((run) => run.taskId && activeTaskStatus(run.status))
      ?.taskId?.trim();
    if (!taskId || chatStopping) return;

    setChatStopping(true);
    setChatError(null);
    try {
      const response = await apiFetch("/v1/conversation-inputs/cancel-current", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          schema_version: 1,
          client_request_id: `ui-stop:${thread.id}:${crypto.randomUUID()}`,
          scope: {
            conversation_id: thread.id,
            agent_id: thread.agentId,
            channel: "ui",
            channel_account_id: thread.externalChatId,
          },
          expected_task_id: taskId,
        }),
      });
      const body = (await response.json()) as ApiResponse<{
        status?: string;
        task_id?: string | null;
      }>;
      if (!response.ok || !body.ok) {
        throw new Error(body.error || `conversation_cancel_http_${response.status}`);
      }
      updateLiveThread(thread.id, (current) => ({
        ...current,
        working: true,
        activity: reduceChatActivity(current.activity, {
          schema_version: 1,
          task_id: taskId,
          seq: 0,
          event_type: "conversation_reply_item",
          event_kind: "conversation_reply_item",
          payload: {
            relation: "control_status",
            lifecycle_stage: "stop_requested",
          },
        }),
      }));
    } catch (error) {
      setChatError(formatUiError(
        error,
        t,
        "停止请求未能提交，请稍后重试。",
        "The stop request could not be submitted. Try again shortly.",
      ));
    } finally {
      setChatStopping(false);
    }
  };

  const compactChatContext = async (focus?: string) => {
    if (chatSending || chatCompacting) return false;
    const thread = activeChatThreadRef.current;
    const normalizedFocus = focus?.trim() ?? "";
    if (normalizedFocus.length > 4_000) {
      setChatError(t("压缩重点不能超过 4000 个字符。", "Compaction focus cannot exceed 4,000 characters."));
      return false;
    }
    setChatCompacting(true);
    setCompactingThreadId(thread.id);
    setChatError(null);
    let submittedTaskId: string | null = null;
    try {
      const compactionRequestId = `ui-compact-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      const payload: Record<string, unknown> = {
        entrypoint: "compact_conversation",
        source: "ui_machine",
        conversation_id: thread.id,
        thread_id: thread.id,
        session_id: thread.id,
        ...(thread.lastTaskId ? { resume_task_id: thread.lastTaskId } : {}),
        ...(normalizedFocus ? { compaction_focus: normalizedFocus } : {}),
      };
      const submitRes = await apiFetch("/v1/tasks", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          [CLIENT_ORIGIN_HEADER]: "ui",
        },
        body: JSON.stringify({
          channel: interactionChannel,
          kind: "ask",
          idempotency_key: `ui:${thread.id}:${compactionRequestId}`,
          ...(activeUserKey ? { user_key: activeUserKey } : {}),
          ...activeIdentityIds,
          ...(interactionExternalUserId.trim()
            ? { external_user_id: interactionExternalUserId.trim() }
            : {}),
          ...(interactionExternalChatId.trim()
            ? { external_chat_id: `${interactionExternalChatId.trim()}--${thread.externalChatId}` }
            : { external_chat_id: thread.externalChatId }),
          payload,
        }),
      });
      const submitted = (await submitRes.json()) as ApiResponse<SubmitTaskResponse>;
      if (!submitRes.ok || !submitted.ok || !submitted.data?.task_id) {
        throw new Error(submitted.error || `context_compaction_submit_http_${submitRes.status}`);
      }
      submittedTaskId = submitted.data.task_id;
      onTaskSubmitted(submittedTaskId);
      await followTaskEventStream(apiFetch, submittedTaskId, async () => undefined);
      const finalResult = await fetchTaskById(submittedTaskId);
      onTaskResult(submittedTaskId, finalResult);
      if (finalResult.status !== "succeeded") {
        throw new Error(finalResult.error_text || t("压缩没有完成。", "Compaction did not complete."));
      }
      updateChatThreadById(thread.id, (current) => ({
        ...current,
        lastTaskId: submittedTaskId,
        messages: appendThreadMessages(current.messages, {
          id: `context-compacted-${submittedTaskId}`,
          role: "system",
          text: t(
            "已整理较早的对话内容；当前任务、重要引用和未完成事项会继续保留。",
            "Earlier conversation context was compacted. Current work, important references, and open items remain available.",
          ),
          ts: Date.now(),
        }),
        updatedAt: Date.now(),
      }));
      return true;
    } catch (error) {
      setChatError(formatUiError(error, t, "压缩上下文失败。", "Failed to compact context."));
      return false;
    } finally {
      setChatCompacting(false);
      setCompactingThreadId(null);
    }
  };

  const handleChatInputKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && e.nativeEvent.keyCode !== 229) {
      e.preventDefault();
      void sendChatMessage();
    }
  };

  return {
    chatMessages,
    chatInput,
    chatAttachments,
    chatTeachingMode,
    chatTeachingTaskResult,
    chatTeachingLlmDebug,
    chatTeachingLlmDebugLoading,
    chatTeachingLlmDebugError,
    chatTeachingRuns,
    activeChatTeachingRunId,
    activeChatAgentId,
    activeChatCanChangeAgent,
    chatSending,
    chatCanStop: Boolean(activeChatTaskId),
    chatStopping,
    chatDeliveryMode,
    setChatDeliveryMode,
    chatDeferredInputs,
    chatDeferredActionInputId,
    activateDeferredChatInput,
    withdrawDeferredChatInput,
    chatCompacting,
    chatWorking,
    chatActivity,
    chatRecording,
    chatVoiceRecordingSupported,
    chatVoiceRecordingAvailability,
    chatAudioInputDevices,
    chatAudioInputDeviceId,
    chatError,
    chatHistoryHasMore: Boolean(chatHistoryCursor),
    chatHistoryLoading,
    chatBodyLoadingMessageId,
    chatAttachmentInputRef,
    setChatTeachingMode,
    selectChatTeachingRun,
    clearChatMessages,
    setChatInput,
    handleChatInputKeyDown,
    handleChatAttachmentSelection,
    removeChatAttachment,
    startChatVoiceRecording,
    stopChatVoiceRecording,
    cancelChatVoiceRecording,
    setChatAudioInputDeviceId,
    sendChatMessage,
    stopActiveChatTask,
    compactChatContext,
    queryChatTeachingLlmDebug,
    chatThreads: chatThreadSummaries,
    activeChatThreadId: chatThreadState.activeThreadId,
    createNewChatThread,
    selectChatThread,
    setActiveChatAgentId,
    renameChatThread,
    deleteChatThread,
    loadEarlierConversationHistory,
    loadNextChatMessageBody,
  };
}
