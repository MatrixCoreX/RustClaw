import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import {
  audioExtensionForMime,
  attachmentIsAudio,
  attachmentIsImage,
  CHAT_MAX_ATTACHMENTS,
  fileToChatAttachment,
  formatVisionResultText,
} from "../lib/chat-attachments";
import {
  AssistantPresentationReducer,
  decodeAssistantPresentationEvent,
} from "../lib/assistant-presentation";
import {
  conversationHistoryStorageKey,
  projectConversationHistory,
  verifyConversationHistoryPage,
  type ServerChatThreadProjection,
} from "../lib/chat-history";
import { followTaskEventStream } from "../lib/task-event-stream";
import { extractTaskText } from "../lib/task-result";
import {
  preferredVoiceRecorderMimeType,
  shouldRetryVoiceCaptureWithDefault,
  voiceAudioTrackConstraints,
  voiceInputDeviceOptions,
  voiceRecorderOptions,
  type VoiceInputDeviceOption,
} from "../lib/voice-recording";
import type {
  ApiResponse,
  ChatAttachment,
  ChannelName,
  ChatMessage,
  SubmitTaskResponse,
  ConversationArchiveUpdate,
  ConversationHistoryPage,
  ConversationTitleUpdate,
  TaskLlmDebugResponse,
  TaskQueryResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface ChatThreadSummary {
  id: string;
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

interface ChatTeachingRunRecord {
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

interface ChatThreadRecord {
  id: string;
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

interface ChatThreadState {
  activeThreadId: string;
  threads: ChatThreadRecord[];
}

const LEGACY_CHAT_THREAD_STORAGE_KEY = "rustclaw.ui.chatThreads.v1";
const MAX_CHAT_THREADS = 30;
const MAX_PERSISTED_MESSAGES_PER_THREAD = 120;
const MAX_TEACHING_RUNS_PER_THREAD = 80;

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
  fetchTaskById,
  onTaskSubmitted,
  onTaskResult,
}: UseChatRuntimeParams) {
  const [chatThreadState, setChatThreadState] = useState<ChatThreadState>(() =>
    emptyChatThreadState(t),
  );
  const activeChatThread =
    chatThreadState.threads.find((thread) => thread.id === chatThreadState.activeThreadId) ??
    chatThreadState.threads[0] ??
    createChatThread(t);
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
  const chatThreadSummaries = buildChatThreadSummaries(chatThreadState.threads, t);
  const [chatAttachments, setChatAttachments] = useState<ChatAttachment[]>([]);
  const [chatTeachingLlmDebugLoading, setChatTeachingLlmDebugLoading] = useState(false);
  const [chatSending, setChatSending] = useState(false);
  const [chatWorking, setChatWorking] = useState(false);
  const [chatRecording, setChatRecording] = useState(false);
  const [chatVoiceRecordingSupported] = useState(canUseDirectVoiceRecording);
  const [chatAudioInputDevices, setChatAudioInputDevices] = useState<
    VoiceInputDeviceOption[]
  >([]);
  const [chatAudioInputDeviceId, setChatAudioInputDeviceIdState] = useState(
    loadSelectedVoiceInputDeviceId,
  );
  const [chatError, setChatError] = useState<string | null>(null);
  const chatAttachmentInputRef = useRef<HTMLInputElement | null>(null);
  const chatMediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chatAudioChunksRef = useRef<Blob[]>([]);
  const chatInputValueRef = useRef("");
  const chatAttachmentsValueRef = useRef<ChatAttachment[]>([]);
  const chatSendingValueRef = useRef(false);
  const chatRecordingValueRef = useRef(false);
  const chatTeachingModeValueRef = useRef(false);
  const chatAudioInputDeviceIdRef = useRef(chatAudioInputDeviceId);
  const activeChatThreadRef = useRef(activeChatThread);
  const apiFetchRef = useRef(apiFetch);
  const conversationHistoryScopeRef = useRef("");
  const teachingTraceAutoLoadKeysRef = useRef<Set<string>>(new Set());
  const voiceStopRequestedRef = useRef(false);

  chatInputValueRef.current = chatInput;
  chatAttachmentsValueRef.current = chatAttachments;
  chatSendingValueRef.current = chatSending;
  chatRecordingValueRef.current = chatRecording;
  chatTeachingModeValueRef.current = chatTeachingMode;
  chatAudioInputDeviceIdRef.current = chatAudioInputDeviceId;
  activeChatThreadRef.current = activeChatThread;
  apiFetchRef.current = apiFetch;

  useEffect(() => {
    const scope = conversationHistoryScope.trim();
    if (!scope || conversationHistoryScopeRef.current !== scope) return;
    persistChatThreadState(chatThreadState, scope);
  }, [chatThreadState, conversationHistoryScope]);

  useEffect(() => {
    const scope = conversationHistoryScope.trim();
    if (!scope) {
      if (conversationHistoryScopeRef.current) {
        conversationHistoryScopeRef.current = "";
        setChatThreadState(emptyChatThreadState(t));
      }
      return;
    }
    if (conversationHistoryScopeRef.current !== scope) {
      conversationHistoryScopeRef.current = scope;
      setChatThreadState(loadChatThreadState(t, scope));
      window.localStorage.removeItem(LEGACY_CHAT_THREAD_STORAGE_KEY);
    }
    let cancelled = false;
    const restore = async () => {
      try {
        const pages: ConversationHistoryPage[] = [];
        let cursor: string | null = null;
        for (let pageIndex = 0; pageIndex < 5; pageIndex += 1) {
          const query = new URLSearchParams({ limit: "200" });
          if (cursor) query.set("cursor", cursor);
          const response = await apiFetchRef.current(
            `/v1/tasks/conversation-history?${query}`,
          );
          const body = (await response.json()) as ApiResponse<ConversationHistoryPage>;
          if (!response.ok || !body.ok || !body.data) {
            throw new Error(body.error || `conversation_history_http_${response.status}`);
          }
          await verifyConversationHistoryPage(body.data);
          pages.push(body.data);
          cursor = body.data.truncated ? body.data.next_cursor?.trim() || null : null;
          if (!cursor) break;
        }
        if (cancelled) return;
        const restored = projectConversationHistory(pages, t);
        setChatThreadState((current) =>
          mergeServerConversationHistory(current, restored, t),
        );
      } catch (error) {
        if (!cancelled) {
          console.warn(
            "conversation_history_restore_failed",
            error instanceof Error ? error.message : "unknown",
          );
        }
      }
    };
    void restore();
    return () => {
      cancelled = true;
    };
  }, [conversationHistoryScope, lang]);

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
      threads: prev.threads
        .map((thread) => (thread.id === threadId ? updater(thread) : thread))
        .slice(0, MAX_CHAT_THREADS),
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
    const nextThread = createChatThread(t);
    setChatThreadState((prev) => ({
      activeThreadId: nextThread.id,
      threads: [nextThread, ...prev.threads].slice(0, MAX_CHAT_THREADS),
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
        const replacement = createChatThread(t);
        return { activeThreadId: replacement.id, threads: [replacement] };
      }
      const remaining = prev.threads.filter((thread) => thread.id !== threadId);
      const activeThreadId =
        prev.activeThreadId === threadId
          ? remaining[0]?.id ?? createChatThread(t).id
          : prev.activeThreadId;
      return { activeThreadId, threads: remaining };
    });
    chatInputValueRef.current = "";
    chatAttachmentsValueRef.current = [];
    setChatAttachments([]);
    setChatTeachingLlmDebugLoading(false);
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
      !window.confirm(
        t(
          "删除这个任务及其对话记录？任务执行证据会安全保留，但不会再显示在对话列表中。",
          "Remove this task and its conversation history? Execution evidence will be retained safely but hidden from the conversation list.",
        ),
      )
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
      !window.confirm(
        t(
          "清空当前对话并开始一个新任务？任务执行证据会安全保留。",
          "Clear this conversation and start a new task? Execution evidence will be retained safely.",
        ),
      )
    ) {
      return false;
    }
    try {
      await archiveChatThreadOnServer(thread);
      const replacement = createChatThread(t);
      setChatThreadState((prev) => ({
        activeThreadId: replacement.id,
        threads: prev.threads
          .map((item) => (item.id === thread.id ? replacement : item))
          .slice(0, MAX_CHAT_THREADS),
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

  const handleChatAttachmentSelection = async (fileList: FileList | null) => {
    if (!fileList || fileList.length === 0) return;
    try {
      const selected = Array.from(fileList);
      if (selected.length === 0) {
        return;
      }
      const nextAttachments = await Promise.all(selected.map((file) => fileToChatAttachment(file)));
      setChatAttachments((prev) => {
        const merged = [...prev, ...nextAttachments];
        const next = merged.slice(0, CHAT_MAX_ATTACHMENTS);
        if (merged.length > CHAT_MAX_ATTACHMENTS) {
          setChatError(t("最多只能一次发送 6 个附件。", "You can send up to 6 attachments at once."));
        } else {
          setChatError(null);
        }
        chatAttachmentsValueRef.current = next;
        return next;
      });
      if (chatAttachmentInputRef.current) {
        chatAttachmentInputRef.current.value = "";
      }
    } catch (err) {
      setChatError(
        err instanceof Error ? err.message : t("读取文件失败。", "Failed to read files."),
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
    if (!canUseDirectVoiceRecording()) {
      setChatError(
        t(
          "当前浏览器不允许直接录音。请用 HTTPS 或 localhost 打开页面，或点“上传图片/文件”选择音频。",
          "This browser cannot record directly here. Open the page with HTTPS or localhost, or choose an audio file from Upload image/file.",
        ),
      );
      return;
    }
    try {
      voiceStopRequestedRef.current = false;
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
      const recorderMimeType = preferredVoiceRecorderMimeType();
      const recorder = new MediaRecorder(stream, voiceRecorderOptions(recorderMimeType));
      chatAudioChunksRef.current = [];
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) {
          chatAudioChunksRef.current.push(event.data);
        }
      };
      recorder.onerror = () => {
        stream.getTracks().forEach((track) => track.stop());
        chatRecordingValueRef.current = false;
        setChatRecording(false);
        setChatError(t("录音失败，请重新尝试。", "Recording failed. Please try again."));
      };
      recorder.onstop = async () => {
        stream.getTracks().forEach((track) => track.stop());
        chatRecordingValueRef.current = false;
        chatMediaRecorderRef.current = null;
        setChatRecording(false);
        const mimeType = recorder.mimeType || "audio/webm";
        const blob = new Blob(chatAudioChunksRef.current, { type: mimeType });
        chatAudioChunksRef.current = [];
        if (blob.size <= 0) {
          setChatError(t("没有录到声音，请重新尝试。", "No audio was recorded. Please try again."));
          return;
        }
        try {
          const file = new File(
            [blob],
            `voice-${Date.now()}.${audioExtensionForMime(mimeType)}`,
            { type: mimeType },
          );
          const attachment = await fileToChatAttachment(file, "audio");
          const attached = [...chatAttachmentsValueRef.current, attachment].slice(
            0,
            CHAT_MAX_ATTACHMENTS,
          );
          setChatError(null);
          chatAttachmentsValueRef.current = attached;
          setChatAttachments(attached);
        } catch (err) {
          setChatError(
            err instanceof Error
              ? err.message
              : t("读取录音失败。", "Failed to read the recording."),
          );
        }
      };
      chatMediaRecorderRef.current = recorder;
      recorder.start();
      if (voiceStopRequestedRef.current) {
        recorder.stop();
      } else {
        chatRecordingValueRef.current = true;
        setChatRecording(true);
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
    const recorder = chatMediaRecorderRef.current;
    if (recorder && recorder.state === "recording") {
      recorder.stop();
    }
  };

  const submitChatMessageSnapshot = async (
    rawText: string,
    rawAttachments: ChatAttachment[],
    options: { clearInput: boolean; clearAttachments: boolean },
  ) => {
    const text = rawText.trim();
    const attached = rawAttachments.slice(0, CHAT_MAX_ATTACHMENTS);
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
          "X-RustClaw-Client": "ui",
        },
        body: JSON.stringify(submitBody),
      });
      const submitData = (await submitRes.json()) as ApiResponse<SubmitTaskResponse>;
      if (!submitRes.ok || !submitData.ok || !submitData.data?.task_id) {
        throw new Error(submitData.error || `chat task submit failed (${submitRes.status})`);
      }

      const submittedTaskId = submitData.data.task_id;
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
      chatSendingValueRef.current = false;
      setChatSending(false);
      setChatWorking(false);
    }
  };

  const sendChatMessage = async () => {
    if (chatRecordingValueRef.current) return;
    await submitChatMessageSnapshot(chatInputValueRef.current, chatAttachmentsValueRef.current, {
      clearInput: true,
      clearAttachments: true,
    });
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
    chatSending,
    chatWorking,
    chatRecording,
    chatVoiceRecordingSupported,
    chatAudioInputDevices,
    chatAudioInputDeviceId,
    chatError,
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
    setChatAudioInputDeviceId,
    sendChatMessage,
    queryChatTeachingLlmDebug,
    chatThreads: chatThreadSummaries,
    activeChatThreadId: chatThreadState.activeThreadId,
    createNewChatThread,
    selectChatThread,
    renameChatThread,
    deleteChatThread,
  };
}

function threadHasServerHistory(thread: ChatThreadRecord): boolean {
  return (
    Boolean(thread.lastTaskId) ||
    (thread.teachingRuns ?? []).some((run) => Boolean(run.taskId))
  );
}

function emptyChatThreadState(t: Translate): ChatThreadState {
  const fallback = createChatThread(t);
  return { activeThreadId: fallback.id, threads: [fallback] };
}

function loadChatThreadState(t: Translate, scope: string): ChatThreadState {
  const fallback = emptyChatThreadState(t);
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
          .map((thread) => normalizeStoredChatThread(thread, t))
          .filter((thread): thread is ChatThreadRecord => Boolean(thread))
          .slice(0, MAX_CHAT_THREADS)
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

function persistChatThreadState(state: ChatThreadState, scope: string) {
  if (typeof window === "undefined") return;
  try {
    const storageKey = conversationHistoryStorageKey(scope);
    if (!storageKey) return;
    const payload: ChatThreadState = {
      activeThreadId: state.activeThreadId,
      threads: state.threads.slice(0, MAX_CHAT_THREADS).map((thread) => ({
        ...thread,
        teachingTaskResult: thread.teachingTaskResult
          ? compactTaskResultForChatStorage(thread.teachingTaskResult)
          : null,
        teachingLlmDebug: null,
        teachingLlmDebugError: null,
        activeTeachingRunId: thread.activeTeachingRunId ?? null,
        teachingRuns: (thread.teachingRuns ?? [])
          .slice(-MAX_TEACHING_RUNS_PER_THREAD)
          .map(compactTeachingRunForChatStorage),
        messages: thread.messages
          .slice(-MAX_PERSISTED_MESSAGES_PER_THREAD)
          .map(stripAttachmentPayloadsFromMessage),
      })),
    };
    window.localStorage.setItem(storageKey, JSON.stringify(payload));
  } catch {
    // Local history is a convenience cache; quota/private-mode failures must not block chat.
  }
}

function mergeServerConversationHistory(
  current: ChatThreadState,
  restored: ServerChatThreadProjection[],
  t: Translate,
): ChatThreadState {
  const existingById = new Map(current.threads.map((thread) => [thread.id, thread]));
  const serverThreads = restored.slice(0, MAX_CHAT_THREADS).map((thread) => {
    const existing = existingById.get(thread.id);
    const existingRunsByTask = new Map(
      (existing?.teachingRuns ?? [])
        .filter((run) => run.taskId)
        .map((run) => [run.taskId as string, run]),
    );
    const teachingRuns = thread.teachingRuns.map((run) => {
      const local = existingRunsByTask.get(run.taskId);
      return {
        ...run,
        llmDebug: local?.llmDebug ?? null,
        llmDebugError: local?.llmDebugError ?? null,
        callCount: local?.callCount ?? debugCallCount(local?.llmDebug),
      };
    });
    const latestRun = teachingRuns[teachingRuns.length - 1] ?? null;
    const activeTeachingRunId =
      existing?.activeTeachingRunId &&
      teachingRuns.some((run) => run.id === existing.activeTeachingRunId)
        ? existing.activeTeachingRunId
        : latestRun?.id ?? null;
    return {
      id: thread.id,
      title: thread.title || t("未命名任务", "Untitled task"),
      messages: thread.messages.slice(-MAX_PERSISTED_MESSAGES_PER_THREAD),
      input: existing?.input ?? "",
      createdAt: thread.createdAt,
      updatedAt: thread.updatedAt,
      teachingMode: existing?.teachingMode ?? false,
      externalChatId: thread.externalChatId,
      lastTaskId: thread.lastTaskId,
      teachingTaskResult: latestRun?.taskResult ?? null,
      teachingLlmDebug: null,
      teachingLlmDebugError: null,
      activeTeachingRunId,
      teachingRuns,
    } satisfies ChatThreadRecord;
  });
  const localDrafts = current.threads.filter(
    (thread) =>
      !restored.some((candidate) => candidate.id === thread.id) &&
      !thread.lastTaskId &&
      !(thread.teachingRuns ?? []).some((run) => run.taskId),
  );
  const threads = [...localDrafts, ...serverThreads]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .slice(0, MAX_CHAT_THREADS);
  if (threads.length === 0) {
    const fallback = createChatThread(t);
    return { activeThreadId: fallback.id, threads: [fallback] };
  }
  const activeThreadId = threads.some((thread) => thread.id === current.activeThreadId)
    ? current.activeThreadId
    : threads[0].id;
  return { activeThreadId, threads };
}

function normalizeStoredChatThread(raw: unknown, t: Translate): ChatThreadRecord | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Partial<ChatThreadRecord>;
  if (typeof record.id !== "string" || !record.id.trim()) return null;
  const now = Date.now();
  const messages = Array.isArray(record.messages)
    ? record.messages
        .map(normalizeStoredChatMessage)
        .filter((message): message is ChatMessage => Boolean(message))
        .slice(-MAX_PERSISTED_MESSAGES_PER_THREAD)
    : [];
  return {
    id: record.id,
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
          .slice(-MAX_TEACHING_RUNS_PER_THREAD)
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
  };
}

function stripAttachmentPayloadsFromMessage(message: ChatMessage): ChatMessage {
  return {
    id: message.id,
    role: message.role,
    text: message.text,
    ts: message.ts,
  };
}

function createChatThread(t: Translate): ChatThreadRecord {
  const now = Date.now();
  return {
    id: `chat-thread-${now}-${Math.random().toString(36).slice(2, 8)}`,
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
  return [...threads]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .map((thread) => {
      const latestRun = latestTeachingRun(thread);
      const taskResult = latestRun?.taskResult ?? thread.teachingTaskResult ?? null;
      return {
        id: thread.id,
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
  return [...(runs ?? []), run].slice(-MAX_TEACHING_RUNS_PER_THREAD);
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
  return [...messages, message].slice(-MAX_PERSISTED_MESSAGES_PER_THREAD);
}

function upsertThreadMessage(messages: ChatMessage[], message: ChatMessage): ChatMessage[] {
  const index = messages.findIndex((item) => item.id === message.id);
  if (index < 0) return appendThreadMessages(messages, message);
  const next = [...messages];
  next[index] = message;
  return next;
}

function canUseDirectVoiceRecording(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof navigator !== "undefined" &&
    window.isSecureContext &&
    Boolean(navigator.mediaDevices?.getUserMedia) &&
    typeof MediaRecorder !== "undefined"
  );
}

const SELECTED_VOICE_INPUT_DEVICE_STORAGE_KEY =
  "rustclaw.ui.chat.selected_voice_input_device.v1";

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
