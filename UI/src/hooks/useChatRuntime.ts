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
  formatAttachmentSize,
  formatVisionResultText,
} from "../lib/chat-attachments";
import {
  AssistantPresentationReducer,
  decodeAssistantPresentationEvent,
} from "../lib/assistant-presentation";
import {
  advanceConversationBodyDescriptor,
  conversationHistoryStorageKey,
  fetchConversationHistoryPage,
  fetchNextConversationBodyPage,
  projectConversationHistory,
  type ServerChatThreadProjection,
} from "../lib/chat-history";
import {
  emptyChatActivity,
  reduceChatActivity,
} from "../lib/chat-activity";
import { followTaskEventStream } from "../lib/task-event-stream";
import {
  appStorageKey,
  CLIENT_ORIGIN_HEADER,
} from "../lib/product-identity";
import { extractTaskText } from "../lib/task-result";
import { extractTaskArtifacts, normalizeTaskArtifacts } from "../lib/task-artifacts";
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
  SubmitTaskResponse,
  ConversationArchiveUpdate,
  ConversationTitleUpdate,
  TaskLlmDebugResponse,
  TaskQueryResponse,
  UiAttachmentConstraints,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface ChatThreadSummary {
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

export interface ChatTeachingRunSummary {
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

export interface ChatTeachingRunRecord {
  id: string;
  taskId: string | null;
  userMessageId: string;
  assistantMessageId?: string | null;
  userText: string;
  assistantText?: string | null;
  status: TaskQueryResponse["status"] | "running";
  startedAt: number;
  completedAt?: number | null;
  taskResult?: TaskQueryResponse | null;
  llmDebug?: TaskLlmDebugResponse | null;
  llmDebugError?: string | null;
  callCount?: number | null;
}

export interface ChatThreadRecord {
  id: string;
  agentId: string;
  title: string;
  messages: ChatMessage[];
  input: string;
  createdAt: number;
  updatedAt: number;
  teachingMode: boolean;
  externalChatId: string;
  lastTaskId?: string | null;
  teachingTaskResult?: TaskQueryResponse | null;
  teachingLlmDebug?: TaskLlmDebugResponse | null;
  teachingLlmDebugError?: string | null;
  activeTeachingRunId?: string | null;
  teachingRuns?: ChatTeachingRunRecord[];
}

export interface ChatThreadState {
  activeThreadId: string;
  threads: ChatThreadRecord[];
}

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
  const [chatTeachingLlmDebugLoading, setChatTeachingLlmDebugLoading] = useState(false);
  const [chatSending, setChatSending] = useState(false);
  const [chatCompacting, setChatCompacting] = useState(false);
  const [chatWorking, setChatWorking] = useState(false);
  const [chatActivity, setChatActivity] = useState(emptyChatActivity);
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
  const chatSendingValueRef = useRef(false);
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
  const recoveryAbortControllersRef = useRef<Map<string, AbortController>>(new Map());
  const voiceStopRequestedRef = useRef(false);

  chatInputValueRef.current = chatInput;
  chatAttachmentsValueRef.current = chatAttachments;
  chatSendingValueRef.current = chatSending;
  chatRecordingValueRef.current = chatRecording;
  chatTeachingModeValueRef.current = chatTeachingMode;
  chatAudioInputDeviceIdRef.current = chatAudioInputDeviceId;
  activeChatThreadRef.current = activeChatThread;
  apiFetchRef.current = apiFetch;

  useEffect(
    () => () => {
      const recorder = chatVoiceRecorderRef.current;
      chatVoiceRecorderRef.current = null;
      if (recorder) void recorder.cancel().catch(() => undefined);
      for (const controller of recoveryAbortControllersRef.current.values()) {
        controller.abort();
      }
      recoveryAbortControllersRef.current.clear();
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
      setChatError(
        error instanceof Error
          ? error.message
          : t("加载更早的任务失败。", "Failed to load earlier tasks."),
      );
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
      setChatError(
        error instanceof Error
          ? error.message
          : t("继续读取完整内容失败。", "Failed to load more of this message."),
      );
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
      throw new Error(body.error || `chat teaching trace query failed (${res.status})`);
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
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
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
    chatSendingValueRef.current = true;
    setChatSending(true);
    setChatWorking(true);
    setChatActivity(emptyChatActivity());
    try {
      const presentation = new AssistantPresentationReducer();
      let streamedAssistantMessageId: string | null = null;
      await followTaskEventStream(
        apiFetch,
        taskId,
        async (event) => {
          setChatActivity((current) => reduceChatActivity(current, event));
          const decoded = decodeAssistantPresentationEvent(event);
          if (!decoded) {
            if (event.event_kind === "task_final") setChatWorking(false);
            return;
          }
          const stream = await presentation.apply(decoded);
          if (!stream || (!stream.content && stream.status === "streaming")) return;
          setChatWorking(false);
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
          }
        : null;
      updateChatThreadById(threadId, (thread) => ({
        ...thread,
        lastTaskId: taskId,
        messages: assistantMessage
          ? upsertThreadMessage(thread.messages, assistantMessage)
          : thread.messages,
        teachingTaskResult: result,
        teachingRuns: updateTeachingRunById(
          thread.teachingRuns,
          teachingRunId,
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
        setChatError(
          error instanceof Error
            ? error.message
            : t("恢复未完成任务失败。", "Failed to resume the unfinished task."),
        );
        suspendedChatTaskIdsRef.current.add(taskId);
      }
    } finally {
      recoveryAbortControllersRef.current.delete(taskId);
      liveChatTaskIdsRef.current.delete(taskId);
      if (liveChatTaskIdsRef.current.size === 0) {
        chatSendingValueRef.current = false;
        setChatSending(false);
        setChatWorking(false);
      }
    }
  };

  useEffect(() => {
    for (const thread of chatThreadState.threads) {
      for (const run of thread.teachingRuns ?? []) {
        const taskId = run.taskId?.trim();
        if (taskId && activeTaskStatus(run.status)) {
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
    if (chatRecordingValueRef.current || chatSendingValueRef.current) return;
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
        err instanceof Error ? err.message : t("无法开始录音。", "Unable to start recording."),
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
    const attached = rawAttachments;
    if ((!text && attached.length === 0) || chatSendingValueRef.current) return;
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
    const threadAtSubmit = activeChatThreadRef.current;
    const submitThreadId = threadAtSubmit.id;
    const teachingModeAtSubmit = threadAtSubmit.teachingMode;
    const teachingRunId = `teach-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    chatSendingValueRef.current = true;
    setChatSending(true);
    setChatWorking(true);
    setChatActivity(emptyChatActivity());
    setChatError(null);
    const userMsg: ChatMessage = {
      id: `u-${Date.now()}`,
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
      input: options.clearInput ? "" : thread.input,
      updatedAt: Date.now(),
    }));
    if (options.clearInput) {
      chatInputValueRef.current = "";
    }
    if (options.clearAttachments) {
      chatAttachmentsValueRef.current = [];
      setChatAttachments([]);
    }
    if (chatAttachmentInputRef.current) {
      chatAttachmentInputRef.current.value = "";
    }

    let submittedTaskId: string | null = null;
    try {
      const adapterName = interactionAdapter.trim();
      const explicitExternalChatId = interactionExternalChatId.trim();
      const effectiveExternalChatId = explicitExternalChatId
        ? `${explicitExternalChatId}--${threadAtSubmit.externalChatId}`
        : threadAtSubmit.externalChatId;
      const attachmentPayload = attached.map((attachment) => ({
        name: attachment.name,
        mime_type: attachment.mimeType,
        size: attachment.size,
        kind: attachment.kind,
        base64: attachment.dataUrl,
      }));
      const submitBody: Record<string, unknown> = {
        channel: interactionChannel,
        kind: "ask",
        ...(activeUserKey ? { user_key: activeUserKey } : {}),
        ...activeIdentityIds,
        ...(interactionExternalUserId.trim() ? { external_user_id: interactionExternalUserId.trim() } : {}),
        ...(effectiveExternalChatId ? { external_chat_id: effectiveExternalChatId } : {}),
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
      const submitRes = await apiFetch(`/v1/tasks`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          [CLIENT_ORIGIN_HEADER]: "ui",
        },
        body: JSON.stringify(submitBody),
      });
      const submitData = (await submitRes.json()) as ApiResponse<SubmitTaskResponse>;
      if (!submitRes.ok || !submitData.ok || !submitData.data?.task_id) {
        throw new Error(submitData.error || `chat task submit failed (${submitRes.status})`);
      }

      submittedTaskId = submitData.data.task_id;
      liveChatTaskIdsRef.current.add(submittedTaskId);
      onTaskSubmitted(submittedTaskId);
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

      const presentation = new AssistantPresentationReducer();
      let streamedAssistantMessageId: string | null = null;
      let completedPresentationText: string | null = null;
      await followTaskEventStream(apiFetch, submittedTaskId, async (event) => {
        setChatActivity((current) => reduceChatActivity(current, event));
        const decoded = decodeAssistantPresentationEvent(event);
        if (!decoded) {
          if (event.event_kind === "task_final") setChatWorking(false);
          return;
        }
        const stream = await presentation.apply(decoded);
        if (decoded.kind === "assistant_output_aborted") {
          setChatWorking(true);
        }
        if (!stream || (!stream.content && stream.status === "streaming")) return;
        setChatWorking(false);
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
      });
      const finalResult = await fetchTaskById(submittedTaskId);
      const finalTaskText = extractTaskText(finalResult);
      if (
        completedPresentationText !== null &&
        completedPresentationText !== finalTaskText
      ) {
        setChatError("assistant_presentation_final_mismatch");
      }
      onTaskResult(submittedTaskId, finalResult);
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        lastTaskId: submittedTaskId,
        teachingTaskResult: teachingModeAtSubmit ? finalResult : thread.teachingTaskResult,
        teachingRuns: updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
          ...run,
          taskId: submittedTaskId,
          status: finalResult.status,
          completedAt: Date.now(),
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
      };
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        messages: upsertThreadMessage(thread.messages, assistantMsg),
        teachingRuns: updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
          ...run,
          assistantMessageId: assistantMsg.id,
          assistantText: assistantMsg.text,
          completedAt: run.completedAt ?? assistantMsg.ts,
        })),
        updatedAt: Date.now(),
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setChatError(message);
      const systemErrMsg: ChatMessage = {
        id: `e-${Date.now()}`,
        role: "system",
        text: `${t("发送失败", "Send failed")}: ${message}`,
        ts: Date.now(),
      };
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        messages: appendThreadMessages(thread.messages, systemErrMsg),
        teachingRuns: updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
          ...run,
          status: "failed",
          assistantMessageId: systemErrMsg.id,
          assistantText: systemErrMsg.text,
          completedAt: Date.now(),
          taskResult: run.taskId
            ? {
                task_id: run.taskId,
                status: "failed",
                result_json: null,
                error_text: message,
              }
            : run.taskResult ?? null,
          llmDebugError: run.llmDebugError,
        })),
        updatedAt: Date.now(),
      }));
    } finally {
      if (submittedTaskId) liveChatTaskIdsRef.current.delete(submittedTaskId);
      if (liveChatTaskIdsRef.current.size === 0) {
        chatSendingValueRef.current = false;
        setChatSending(false);
        setChatWorking(false);
      }
    }
  };

  const sendChatMessage = async () => {
    if (chatRecordingValueRef.current) return;
    await submitChatMessageSnapshot(chatInputValueRef.current, chatAttachmentsValueRef.current, {
      clearInput: true,
      clearAttachments: true,
    });
  };

  const compactChatContext = async (focus?: string) => {
    if (chatSendingValueRef.current || chatCompacting) return false;
    const thread = activeChatThreadRef.current;
    const normalizedFocus = focus?.trim() ?? "";
    if (normalizedFocus.length > 4_000) {
      setChatError(t("压缩重点不能超过 4000 个字符。", "Compaction focus cannot exceed 4,000 characters."));
      return false;
    }
    setChatCompacting(true);
    setChatError(null);
    let submittedTaskId: string | null = null;
    try {
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
        throw new Error(submitted.error || `context compaction submit failed (${submitRes.status})`);
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
      setChatError(
        error instanceof Error
          ? error.message
          : t("压缩上下文失败。", "Failed to compact context."),
      );
      return false;
    } finally {
      setChatCompacting(false);
    }
  };

  const handleChatInputKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
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

function threadHasServerHistory(thread: ChatThreadRecord): boolean {
  return (
    Boolean(thread.lastTaskId) ||
    (thread.teachingRuns ?? []).some((run) => Boolean(run.taskId))
  );
}

function emptyChatThreadState(t: Translate, defaultAgentId = "main"): ChatThreadState {
  const fallback = createChatThread(t, defaultAgentId);
  return { activeThreadId: fallback.id, threads: [fallback] };
}

export function loadChatThreadState(
  t: Translate,
  scope: string,
  defaultAgentId = "main",
): ChatThreadState {
  const fallback = emptyChatThreadState(t, defaultAgentId);
  if (typeof window === "undefined") {
    return fallback;
  }
  try {
    const storageKey = conversationHistoryStorageKey(scope);
    const raw = storageKey ? window.localStorage.getItem(storageKey) : null;
    if (!raw) {
      return fallback;
    }
    const parsed = JSON.parse(raw) as Partial<ChatThreadState>;
    const threads = Array.isArray(parsed.threads)
      ? parsed.threads
          .map((thread) => normalizeStoredChatThread(thread, t, defaultAgentId))
          .filter((thread): thread is ChatThreadRecord => Boolean(thread))
      : [];
    if (threads.length === 0) {
      return fallback;
    }
    const activeThreadId =
      typeof parsed.activeThreadId === "string" &&
      threads.some((thread) => thread.id === parsed.activeThreadId)
        ? parsed.activeThreadId
        : threads[0].id;
    return { activeThreadId, threads };
  } catch {
    return fallback;
  }
}

export function persistChatThreadState(state: ChatThreadState, scope: string) {
  if (typeof window === "undefined") return;
  try {
    const storageKey = conversationHistoryStorageKey(scope);
    if (!storageKey) return;
    const payload: ChatThreadState = {
      activeThreadId: state.activeThreadId,
      threads: state.threads.map((thread) => ({
        ...thread,
        teachingTaskResult: thread.teachingTaskResult
          ? compactTaskResultForChatStorage(thread.teachingTaskResult)
          : null,
        teachingLlmDebug: null,
        teachingLlmDebugError: null,
        activeTeachingRunId: thread.activeTeachingRunId ?? null,
        teachingRuns: (thread.teachingRuns ?? []).map(compactTeachingRunForChatStorage),
        messages: thread.messages.map(stripAttachmentPayloadsFromMessage),
      })),
    };
    window.localStorage.setItem(storageKey, JSON.stringify(payload));
  } catch {
    // Local history is a convenience cache; quota/private-mode failures must not block chat.
  }
}

export function mergeServerConversationHistory(
  current: ChatThreadState,
  restored: ServerChatThreadProjection[],
  t: Translate,
  defaultAgentId = "main",
): ChatThreadState {
  const existingById = new Map(current.threads.map((thread) => [thread.id, thread]));
  const serverThreads = restored.map((thread) => {
    const existing = existingById.get(thread.id);
    const existingRunsByTask = new Map(
      (existing?.teachingRuns ?? [])
        .filter((run) => run.taskId)
        .map((run) => [run.taskId as string, run]),
    );
    const restoredRuns = thread.teachingRuns.map((run) => {
      const local = existingRunsByTask.get(run.taskId);
      return {
        ...run,
        llmDebug: local?.llmDebug ?? null,
        llmDebugError: local?.llmDebugError ?? null,
        callCount: local?.callCount ?? debugCallCount(local?.llmDebug),
      };
    });
    const restoredTaskIds = new Set(restoredRuns.map((run) => run.taskId));
    const teachingRuns = [
      ...(existing?.teachingRuns ?? []).filter((run) => !restoredTaskIds.has(run.taskId)),
      ...restoredRuns,
    ].sort((left, right) => left.startedAt - right.startedAt);
    const restoredMessageIds = new Set(thread.messages.map((message) => message.id));
    const messages = [
      ...(existing?.messages ?? []).filter((message) => !restoredMessageIds.has(message.id)),
      ...thread.messages,
    ].sort((left, right) => left.ts - right.ts || left.id.localeCompare(right.id));
    const latestRun = teachingRuns[teachingRuns.length - 1] ?? null;
    const activeTeachingRunId =
      existing?.activeTeachingRunId &&
      teachingRuns.some((run) => run.id === existing.activeTeachingRunId)
        ? existing.activeTeachingRunId
        : latestRun?.id ?? null;
    return {
      id: thread.id,
      agentId: thread.agentId || existing?.agentId || defaultAgentId,
      title: thread.title || t("未命名任务", "Untitled task"),
      messages,
      input: existing?.input ?? "",
      createdAt: Math.min(existing?.createdAt ?? thread.createdAt, thread.createdAt),
      updatedAt: Math.max(existing?.updatedAt ?? thread.updatedAt, thread.updatedAt),
      teachingMode: existing?.teachingMode ?? false,
      externalChatId: thread.externalChatId,
      lastTaskId:
        !existing || thread.updatedAt >= existing.updatedAt
          ? thread.lastTaskId
          : existing.lastTaskId ?? thread.lastTaskId,
      teachingTaskResult: latestRun?.taskResult ?? null,
      teachingLlmDebug: null,
      teachingLlmDebugError: null,
      activeTeachingRunId,
      teachingRuns,
    } satisfies ChatThreadRecord;
  });
  const retainedThreads = current.threads.filter(
    (thread) => !restored.some((candidate) => candidate.id === thread.id),
  );
  const threads = [...retainedThreads, ...serverThreads].sort(
    (left, right) => right.updatedAt - left.updatedAt,
  );
  if (threads.length === 0) {
    const fallback = createChatThread(t, defaultAgentId);
    return { activeThreadId: fallback.id, threads: [fallback] };
  }
  const activeThreadId = threads.some((thread) => thread.id === current.activeThreadId)
    ? current.activeThreadId
    : threads[0].id;
  return { activeThreadId, threads };
}

export function retainLocalDraftsForPagedRestore(
  current: ChatThreadState,
): ChatThreadState {
  const threads = current.threads.filter(
    (thread) =>
      threadHasPendingTask(thread) ||
      (!threadHasServerHistory(thread) && !threadIsPristineWelcome(thread)),
  );
  return {
    activeThreadId: current.activeThreadId,
    threads,
  };
}

function threadIsPristineWelcome(thread: ChatThreadRecord): boolean {
  return (
    !thread.input.trim() &&
    !thread.teachingMode &&
    !thread.lastTaskId &&
    (thread.teachingRuns ?? []).length === 0 &&
    thread.messages.length === 1 &&
    thread.messages[0].role === "system" &&
    thread.messages[0].id.startsWith("chat-system-welcome-")
  );
}

function normalizeStoredChatThread(
  raw: unknown,
  t: Translate,
  defaultAgentId = "main",
): ChatThreadRecord | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<ChatThreadRecord>;
  if (typeof record.id !== "string" || !record.id.trim()) return null;
  const now = Date.now();
  const messages = Array.isArray(record.messages)
    ? record.messages
        .map(normalizeStoredChatMessage)
        .filter((message): message is ChatMessage => Boolean(message))
    : [];
  return {
    id: record.id,
    agentId:
      typeof record.agentId === "string" && record.agentId.trim()
        ? record.agentId.trim()
        : defaultAgentId,
    title:
      typeof record.title === "string" && record.title.trim()
        ? record.title.trim()
        : t("未命名任务", "Untitled task"),
    messages: messages.length > 0 ? messages : [welcomeChatMessage(t)],
    input: typeof record.input === "string" ? record.input : "",
    createdAt: typeof record.createdAt === "number" ? record.createdAt : now,
    updatedAt: typeof record.updatedAt === "number" ? record.updatedAt : now,
    teachingMode: typeof record.teachingMode === "boolean" ? record.teachingMode : false,
    externalChatId:
      typeof record.externalChatId === "string" && record.externalChatId.trim()
        ? record.externalChatId.trim()
        : createThreadExternalChatId(),
    lastTaskId: typeof record.lastTaskId === "string" ? record.lastTaskId : null,
    teachingTaskResult: normalizeStoredTaskResult(record.teachingTaskResult),
    teachingLlmDebug: null,
    teachingLlmDebugError: null,
    activeTeachingRunId:
      typeof record.activeTeachingRunId === "string" ? record.activeTeachingRunId : null,
    teachingRuns: Array.isArray(record.teachingRuns)
      ? record.teachingRuns
          .map(normalizeStoredTeachingRun)
          .filter((run): run is ChatTeachingRunRecord => Boolean(run))
      : [],
  };
}

function compactTeachingRunForChatStorage(run: ChatTeachingRunRecord): ChatTeachingRunRecord {
  return {
    id: run.id,
    taskId: run.taskId ?? null,
    userMessageId: run.userMessageId,
    assistantMessageId: run.assistantMessageId ?? null,
    userText: run.userText,
    assistantText: run.assistantText ?? null,
    status: run.status,
    startedAt: run.startedAt,
    completedAt: run.completedAt ?? null,
    taskResult: run.taskResult ? compactTaskResultForChatStorage(run.taskResult) : null,
    llmDebug: null,
    llmDebugError: null,
    callCount: run.callCount ?? debugCallCount(run.llmDebug),
  };
}

function compactTaskResultForChatStorage(result: TaskQueryResponse): TaskQueryResponse {
  return {
    task_id: result.task_id,
    status: result.status,
    goal: result.goal ?? null,
    result_json: null,
    error_text: result.error_text ?? null,
  };
}

function normalizeStoredTeachingRun(raw: unknown): ChatTeachingRunRecord | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<ChatTeachingRunRecord>;
  if (
    typeof record.id !== "string" ||
    typeof record.userMessageId !== "string" ||
    typeof record.userText !== "string" ||
    typeof record.startedAt !== "number"
  ) {
    return null;
  }
  const status = isTaskStatusOrRunning(record.status) ? record.status : "running";
  return {
    id: record.id,
    taskId: typeof record.taskId === "string" && record.taskId.trim() ? record.taskId : null,
    userMessageId: record.userMessageId,
    assistantMessageId:
      typeof record.assistantMessageId === "string" ? record.assistantMessageId : null,
    userText: record.userText,
    assistantText: typeof record.assistantText === "string" ? record.assistantText : null,
    status,
    startedAt: record.startedAt,
    completedAt: typeof record.completedAt === "number" ? record.completedAt : null,
    taskResult: normalizeStoredTaskResult(record.taskResult),
    llmDebug: null,
    llmDebugError: null,
    callCount: typeof record.callCount === "number" ? record.callCount : null,
  };
}

function isTaskStatusOrRunning(value: unknown): value is ChatTeachingRunRecord["status"] {
  return ["queued", "running", "succeeded", "failed", "canceled", "timeout"].includes(String(value));
}

function activeTaskStatus(status: ChatTeachingRunRecord["status"]): boolean {
  return status === "queued" || status === "running";
}

function terminalTaskStatus(status: TaskQueryResponse["status"]): boolean {
  return !activeTaskStatus(status);
}

function formatChatAttachmentError(
  error: unknown,
  constraints: UiAttachmentConstraints,
  t: Translate,
): string {
  if (!(error instanceof ChatAttachmentConstraintError)) {
    return error instanceof Error ? error.message : t("读取文件失败。", "Failed to read files.");
  }
  switch (error.code) {
    case "ui_attachments_too_many":
      return t(
        `一次最多发送 ${constraints.max_attachments} 个附件。`,
        `You can send up to ${constraints.max_attachments} attachments at once.`,
      );
    case "ui_attachment_too_large":
      return t(
        `单个附件不能超过 ${formatAttachmentSize(constraints.max_attachment_bytes)}。`,
        `Each attachment must be no larger than ${formatAttachmentSize(constraints.max_attachment_bytes)}.`,
      );
    case "ui_attachments_total_too_large":
      return t(
        `附件总大小不能超过 ${formatAttachmentSize(constraints.max_total_attachment_bytes)}。`,
        `The total attachment size must not exceed ${formatAttachmentSize(constraints.max_total_attachment_bytes)}.`,
      );
    default:
      return t("附件不符合上传要求。", "The attachments do not meet the upload requirements.");
  }
}

export function threadHasPendingTask(thread: ChatThreadRecord): boolean {
  return (thread.teachingRuns ?? []).some(
    (run) => Boolean(run.taskId) && activeTaskStatus(run.status),
  );
}

function normalizeStoredTaskResult(raw: unknown): TaskQueryResponse | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<TaskQueryResponse>;
  if (typeof record.task_id !== "string" || !record.task_id.trim()) return null;
  return {
    task_id: record.task_id,
    status: typeof record.status === "string" ? record.status : "succeeded",
    goal: record.goal ?? null,
    result_json: null,
    error_text: typeof record.error_text === "string" ? record.error_text : null,
  };
}

function normalizeStoredChatMessage(raw: unknown): ChatMessage | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<ChatMessage>;
  if (
    typeof record.id !== "string" ||
    typeof record.text !== "string" ||
    typeof record.ts !== "number" ||
    !["user", "assistant", "system"].includes(String(record.role))
  ) {
    return null;
  }
  return {
    id: record.id,
    role: record.role as ChatMessage["role"],
    text: record.text,
    ts: record.ts,
    artifacts: normalizeTaskArtifacts(record.artifacts),
    bodyResult: normalizeStoredConversationBodyDescriptor(record.bodyResult),
  };
}

function stripAttachmentPayloadsFromMessage(message: ChatMessage): ChatMessage {
  return {
    id: message.id,
    role: message.role,
    text: message.text,
    ts: message.ts,
    artifacts: normalizeTaskArtifacts(message.artifacts),
    bodyResult: normalizeStoredConversationBodyDescriptor(message.bodyResult),
  };
}

function normalizeStoredConversationBodyDescriptor(
  raw: ChatMessage["bodyResult"],
): ChatMessage["bodyResult"] {
  if (!raw || typeof raw !== "object") return null;
  if (
    raw.schema_version !== 1 ||
    typeof raw.complete !== "boolean" ||
    !Number.isSafeInteger(raw.original_size_bytes) ||
    !Number.isSafeInteger(raw.returned_size_bytes) ||
    raw.original_size_bytes < raw.returned_size_bytes ||
    !/^[0-9a-f]{64}$/i.test(raw.content_sha256)
  ) {
    return null;
  }
  if (
    !raw.complete &&
    (!raw.continuation ||
      raw.continuation.kind !== "conversation_body_range" ||
      typeof raw.continuation.url !== "string" ||
      !Number.isSafeInteger(raw.continuation.next_start_byte))
  ) {
    return null;
  }
  return raw;
}

function createChatThread(t: Translate, agentId = "main"): ChatThreadRecord {
  const now = Date.now();
  return {
    id: `chat-thread-${now}-${Math.random().toString(36).slice(2, 8)}`,
    agentId,
    title: t("新任务", "New task"),
    messages: [welcomeChatMessage(t)],
    input: "",
    createdAt: now,
    updatedAt: now,
    teachingMode: false,
    externalChatId: createThreadExternalChatId(),
    lastTaskId: null,
    teachingTaskResult: null,
    teachingLlmDebug: null,
    teachingLlmDebugError: null,
    activeTeachingRunId: null,
    teachingRuns: [],
  };
}

function createThreadExternalChatId(): string {
  return `ui-chat-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function welcomeChatMessage(t: Translate): ChatMessage {
  return {
    id: `chat-system-welcome-${Date.now()}`,
    role: "system",
    text: t(
      "会话窗口已连接 clawd。发送消息后会自动提交 ask 任务并轮询结果。",
      "The chat window is connected to clawd. Messages submit ask tasks and poll for results automatically.",
    ),
    ts: Date.now(),
  };
}

function clearedChatMessage(t: Translate): ChatMessage {
  return {
    id: `chat-clear-${Date.now()}`,
    role: "system",
    text: t("当前任务的聊天记录已清空。", "This task's chat history was cleared."),
    ts: Date.now(),
  };
}

function buildChatThreadSummaries(
  threads: ChatThreadRecord[],
  t: Translate,
): ChatThreadSummary[] {
  return threads.map((thread) => {
      const latestRun = latestTeachingRun(thread);
      const taskResult = latestRun?.taskResult ?? thread.teachingTaskResult ?? null;
      return {
        id: thread.id,
        agentId: thread.agentId,
        title: thread.title,
        preview: threadPreview(thread, t),
        updatedAt: thread.updatedAt,
        messageCount: thread.messages.filter((message) => message.role !== "system").length,
        teachingMode: thread.teachingMode,
        taskId: latestRun?.taskId ?? taskResult?.task_id ?? thread.lastTaskId ?? null,
        taskStatus: latestRun?.status ?? taskResult?.status ?? null,
        llmCallCount:
          latestRun?.callCount ??
          debugCallCount(latestRun?.llmDebug) ??
          debugCallCount(thread.teachingLlmDebug),
      };
    });
}

function selectedTeachingRun(thread: ChatThreadRecord): ChatTeachingRunRecord | null {
  const runs = thread.teachingRuns ?? [];
  if (runs.length === 0) return null;
  const activeId = thread.activeTeachingRunId;
  const activeRun = runs.find((run) => run.id === activeId);
  if (activeRun) return activeRun;
  return thread.teachingMode ? (runs[runs.length - 1] ?? null) : null;
}

function latestTeachingRun(thread: ChatThreadRecord): ChatTeachingRunRecord | null {
  const runs = thread.teachingRuns ?? [];
  return runs.reduce<ChatTeachingRunRecord | null>((latest, run) => {
    if (!latest) return run;
    return run.startedAt >= latest.startedAt ? run : latest;
  }, null);
}

function buildChatTeachingRunSummaries(thread: ChatThreadRecord): ChatTeachingRunSummary[] {
  const activeId = selectedTeachingRun(thread)?.id ?? null;
  return [...(thread.teachingRuns ?? [])]
    .sort((left, right) => right.startedAt - left.startedAt)
    .map((run) => ({
      id: run.id,
      taskId: run.taskId ?? null,
      userMessageId: run.userMessageId,
      assistantMessageId: run.assistantMessageId ?? null,
      userText: run.userText,
      assistantText: run.assistantText ?? null,
      status: run.status,
      startedAt: run.startedAt,
      completedAt: run.completedAt ?? null,
      callCount: run.callCount ?? debugCallCount(run.llmDebug),
      hasTrace: Boolean(run.llmDebug),
      traceError: run.llmDebugError ?? null,
      selected: run.id === activeId,
    }));
}

function appendTeachingRun(
  runs: ChatTeachingRunRecord[] | undefined,
  run: ChatTeachingRunRecord,
): ChatTeachingRunRecord[] {
  return [...(runs ?? []), run];
}

function updateTeachingRunById(
  runs: ChatTeachingRunRecord[] | undefined,
  runId: string,
  updater: (run: ChatTeachingRunRecord) => ChatTeachingRunRecord,
): ChatTeachingRunRecord[] {
  return (runs ?? []).map((run) => (run.id === runId ? updater(run) : run));
}

function updateTeachingRunsByTaskId(
  runs: ChatTeachingRunRecord[] | undefined,
  taskId: string,
  updater: (run: ChatTeachingRunRecord) => ChatTeachingRunRecord,
): ChatTeachingRunRecord[] {
  return (runs ?? []).map((run) => (run.taskId === taskId ? updater(run) : run));
}

function debugCallCount(debug: TaskLlmDebugResponse | null | undefined): number | null {
  if (!debug) return null;
  if (typeof debug.call_count === "number") return debug.call_count;
  return debug.calls?.length ?? debug.entries?.length ?? null;
}

function threadPreview(thread: ChatThreadRecord, t: Translate): string {
  const latest = [...thread.messages]
    .reverse()
    .find((message) => message.role === "user" || message.role === "assistant");
  return latest?.text.trim() || t("还没有消息", "No messages yet");
}

function titleForThreadAfterUserMessage(
  thread: ChatThreadRecord,
  message: ChatMessage,
  t: Translate,
): string {
  const hasPriorUserMessage = thread.messages.some((item) => item.role === "user");
  const defaultTitles = new Set([t("新任务", "New task"), t("未命名任务", "Untitled task")]);
  if (hasPriorUserMessage || !defaultTitles.has(thread.title)) {
    return thread.title;
  }
  const cleaned = message.text.replace(/\s+/g, " ").trim();
  if (!cleaned) {
    return t("附件任务", "Attachment task");
  }
  return cleaned.length > 28 ? `${cleaned.slice(0, 28)}...` : cleaned;
}

function appendThreadMessages(messages: ChatMessage[], message: ChatMessage): ChatMessage[] {
  return [...messages, message];
}

function upsertThreadMessage(messages: ChatMessage[], message: ChatMessage): ChatMessage[] {
  const index = messages.findIndex((item) => item.id === message.id);
  if (index < 0) return appendThreadMessages(messages, message);
  const next = [...messages];
  next[index] = message;
  return next;
}

const SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY =
  appStorageKey("ui.chat.selected_voice_input_device.v1");

function loadSelectedVoiceInputDeviceId(): string {
  if (typeof window === "undefined") return "";
  try {
    return window.localStorage.getItem(SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY)?.trim() ?? "";
  } catch {
    return "";
  }
}

function persistSelectedVoiceInputDeviceId(deviceId: string): void {
  if (typeof window === "undefined") return;
  try {
    if (deviceId) {
      window.localStorage.setItem(SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY, deviceId);
    } else {
      window.localStorage.removeItem(SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY);
    }
  } catch {
    // Browser privacy settings may disable local storage; recording still works.
  }
}

function defaultAttachmentPrompt(
  t: Translate,
  imageCount: number,
  audioCount: number,
  fileCount: number,
): string {
  if (audioCount > 0 && imageCount === 0 && fileCount === 0) {
    return t("请根据这段语音继续对话", "Please continue the conversation based on this voice message");
  }
  if (imageCount > 0 && fileCount === 0 && audioCount === 0) {
    return t("请描述这张图片", "Please describe this image");
  }
  return t("请查看我上传的附件", "Please review the attachments I uploaded");
}

function defaultAttachmentMessage(
  t: Translate,
  imageCount: number,
  audioCount: number,
  fileCount: number,
): string {
  if (audioCount > 0 && imageCount === 0 && fileCount === 0) {
    return t("发送了一段语音", "Sent a voice message");
  }
  if (imageCount > 0 && fileCount === 0 && audioCount === 0) {
    return t("发送了一张图片", "Sent an image");
  }
  return t("发送了附件", "Sent attachments");
}
