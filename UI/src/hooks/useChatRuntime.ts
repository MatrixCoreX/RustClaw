import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import {
  audioExtensionForMime,
  attachmentIsAudio,
  attachmentIsImage,
  CHAT_MAX_ATTACHMENTS,
  fileToChatAttachment,
  formatVisionResultText,
} from "../lib/chat-attachments";
import { sleep } from "../lib/display-format";
import { extractTaskText } from "../lib/task-result";
import type {
  ApiResponse,
  ChatAttachment,
  ChannelName,
  ChatMessage,
  SubmitTaskResponse,
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
  agentMode: boolean;
  teachingMode: boolean;
  taskId: string | null;
  taskStatus: TaskQueryResponse["status"] | "running" | null;
  llmCallCount: number | null;
}

export interface ChatTeachingRunSummary {
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
  agentMode: boolean;
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

const CHAT_THREAD_STORAGE_KEY = "rustclaw.ui.chatThreads.v1";
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
  interactionExternalUserId,
  interactionExternalChatId,
  fetchTaskById,
  onTaskSubmitted,
  onTaskResult,
}: UseChatRuntimeParams) {
  const [chatThreadState, setChatThreadState] = useState<ChatThreadState>(() =>
    loadChatThreadState(t),
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
  const [chatRecording, setChatRecording] = useState(false);
  const [chatVoiceRecordingSupported] = useState(canUseDirectVoiceRecording);
  const [chatError, setChatError] = useState<string | null>(null);
  const chatAttachmentInputRef = useRef<HTMLInputElement | null>(null);
  const chatMediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chatAudioChunksRef = useRef<Blob[]>([]);
  const chatInputValueRef = useRef("");
  const chatAttachmentsValueRef = useRef<ChatAttachment[]>([]);
  const chatSendingValueRef = useRef(false);
  const chatRecordingValueRef = useRef(false);
  const chatTeachingModeValueRef = useRef(false);
  const activeChatThreadRef = useRef(activeChatThread);
  const teachingTraceAutoLoadKeysRef = useRef<Set<string>>(new Set());
  const voiceSendOnStopRef = useRef(false);
  const voiceStopRequestedRef = useRef(false);

  chatInputValueRef.current = chatInput;
  chatAttachmentsValueRef.current = chatAttachments;
  chatSendingValueRef.current = chatSending;
  chatRecordingValueRef.current = chatRecording;
  chatTeachingModeValueRef.current = chatTeachingMode;
  activeChatThreadRef.current = activeChatThread;

  useEffect(() => {
    persistChatThreadState(chatThreadState);
  }, [chatThreadState]);

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
    updateActiveChatThread((thread) => {
      const run = (thread.teachingRuns ?? []).find((item) => item.id === runId);
      if (!run) return thread;
      return {
        ...thread,
        activeTeachingRunId: run.id,
        teachingTaskResult: run.taskResult ?? thread.teachingTaskResult ?? null,
        teachingLlmDebug: run.llmDebug ?? null,
        teachingLlmDebugError: run.llmDebugError ?? null,
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

  const deleteChatThread = (threadId: string) => {
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

  const clearChatMessages = () => {
    updateActiveChatThread((thread) => ({
      ...thread,
      messages: [clearedChatMessage(t)],
      input: "",
      updatedAt: Date.now(),
      lastTaskId: null,
      teachingTaskResult: null,
      teachingLlmDebug: null,
      teachingLlmDebugError: null,
      activeTeachingRunId: null,
      teachingRuns: [],
    }));
    chatInputValueRef.current = "";
    setChatTeachingLlmDebugLoading(false);
  };

  const fetchChatTeachingLlmDebugById = async (id: string): Promise<TaskLlmDebugResponse> => {
    const normalizedId = encodeURIComponent(id.trim());
    const res = await apiFetch(`/v1/debug/tasks/${normalizedId}`);
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
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      if (voiceStopRequestedRef.current) {
        stream.getTracks().forEach((track) => track.stop());
        voiceSendOnStopRef.current = false;
        return;
      }
      const recorderMimeType = preferredRecorderMimeType();
      const recorder = recorderMimeType
        ? new MediaRecorder(stream, { mimeType: recorderMimeType })
        : new MediaRecorder(stream);
      chatAudioChunksRef.current = [];
      voiceSendOnStopRef.current = true;
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) {
          chatAudioChunksRef.current.push(event.data);
        }
      };
      recorder.onerror = () => {
        stream.getTracks().forEach((track) => track.stop());
        chatRecordingValueRef.current = false;
        voiceSendOnStopRef.current = false;
        setChatRecording(false);
        setChatError(t("录音失败，请重新尝试。", "Recording failed. Please try again."));
      };
      recorder.onstop = async () => {
        stream.getTracks().forEach((track) => track.stop());
        const shouldSend = voiceSendOnStopRef.current;
        voiceSendOnStopRef.current = false;
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
          if (shouldSend) {
            void submitChatMessageSnapshot(chatInputValueRef.current, attached, {
              clearInput: true,
              clearAttachments: true,
            });
          } else {
            chatAttachmentsValueRef.current = attached;
            setChatAttachments(attached);
          }
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
      voiceSendOnStopRef.current = false;
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
    const teachingRunId = teachingModeAtSubmit
      ? `teach-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
      : null;
    chatSendingValueRef.current = true;
    setChatSending(true);
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
      ...(teachingRunId
        ? {
            activeTeachingRunId: teachingRunId,
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
          }
        : {}),
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
          agent_mode: true,
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
        headers: { "Content-Type": "application/json" },
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
        activeTeachingRunId: teachingRunId ?? thread.activeTeachingRunId ?? null,
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
        teachingRuns: teachingRunId
          ? updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
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
            }))
          : thread.teachingRuns,
        updatedAt: Date.now(),
      }));

      let finalResult: TaskQueryResponse | null = null;
      for (let i = 0; i < 90; i += 1) {
        const current = await fetchTaskById(submittedTaskId);
        if (["succeeded", "failed", "canceled", "timeout"].includes(current.status)) {
          finalResult = current;
          break;
        }
        await sleep(1200);
      }
      if (!finalResult) {
        throw new Error(t("轮询超时：任务仍在运行，请稍后在任务查询区查看。", "Polling timed out: the task is still running. Check it later in the task query area."));
      }
      onTaskResult(submittedTaskId, finalResult);
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        lastTaskId: submittedTaskId,
        teachingTaskResult: teachingModeAtSubmit ? finalResult : thread.teachingTaskResult,
        teachingRuns: teachingRunId
          ? updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
              ...run,
              taskId: submittedTaskId,
              status: finalResult.status,
              completedAt: Date.now(),
              taskResult: finalResult,
            }))
          : thread.teachingRuns,
        updatedAt: Date.now(),
      }));
      if (teachingModeAtSubmit && activeChatThreadRef.current.id === submitThreadId) {
        void queryChatTeachingLlmDebug(submittedTaskId);
      }

      const assistantMsg: ChatMessage = {
        id: `a-${Date.now()}`,
        role: "assistant",
        text: attachedImages.length > 0 ? formatVisionResultText(extractTaskText(finalResult)) : extractTaskText(finalResult),
        ts: Date.now(),
      };
      updateChatThreadById(submitThreadId, (thread) => ({
        ...thread,
        messages: appendThreadMessages(thread.messages, assistantMsg),
        teachingRuns: teachingRunId
          ? updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
              ...run,
              assistantMessageId: assistantMsg.id,
              assistantText: assistantMsg.text,
              completedAt: run.completedAt ?? assistantMsg.ts,
            }))
          : thread.teachingRuns,
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
        teachingRuns: teachingRunId
          ? updateTeachingRunById(thread.teachingRuns, teachingRunId, (run) => ({
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
            }))
          : thread.teachingRuns,
        updatedAt: Date.now(),
      }));
    } finally {
      chatSendingValueRef.current = false;
      setChatSending(false);
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
    chatRecording,
    chatVoiceRecordingSupported,
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
    sendChatMessage,
    queryChatTeachingLlmDebug,
    chatThreads: chatThreadSummaries,
    activeChatThreadId: chatThreadState.activeThreadId,
    createNewChatThread,
    selectChatThread,
    deleteChatThread,
  };
}

function loadChatThreadState(t: Translate): ChatThreadState {
  const fallback = createChatThread(t);
  if (typeof window === "undefined") {
    return { activeThreadId: fallback.id, threads: [fallback] };
  }
  try {
    const raw = window.localStorage.getItem(CHAT_THREAD_STORAGE_KEY);
    if (!raw) {
      return { activeThreadId: fallback.id, threads: [fallback] };
    }
    const parsed = JSON.parse(raw) as Partial<ChatThreadState>;
    const threads = Array.isArray(parsed.threads)
      ? parsed.threads
          .map((thread) => normalizeStoredChatThread(thread, t))
          .filter((thread): thread is ChatThreadRecord => Boolean(thread))
          .slice(0, MAX_CHAT_THREADS)
      : [];
    if (threads.length === 0) {
      return { activeThreadId: fallback.id, threads: [fallback] };
    }
    const activeThreadId =
      typeof parsed.activeThreadId === "string" &&
      threads.some((thread) => thread.id === parsed.activeThreadId)
        ? parsed.activeThreadId
        : threads[0].id;
    return { activeThreadId, threads };
  } catch {
    return { activeThreadId: fallback.id, threads: [fallback] };
  }
}

function persistChatThreadState(state: ChatThreadState) {
  if (typeof window === "undefined") return;
  try {
    const payload: ChatThreadState = {
      activeThreadId: state.activeThreadId,
      threads: state.threads.slice(0, MAX_CHAT_THREADS).map((thread) => ({
        ...thread,
        teachingTaskResult: thread.teachingTaskResult
          ? compactTaskResultForChatStorage(thread.teachingTaskResult)
          : null,
        teachingLlmDebug: null,
        activeTeachingRunId: thread.activeTeachingRunId ?? null,
        teachingRuns: (thread.teachingRuns ?? [])
          .slice(-MAX_TEACHING_RUNS_PER_THREAD)
          .map(compactTeachingRunForChatStorage),
        messages: thread.messages
          .slice(-MAX_PERSISTED_MESSAGES_PER_THREAD)
          .map(stripAttachmentPayloadsFromMessage),
      })),
    };
    window.localStorage.setItem(CHAT_THREAD_STORAGE_KEY, JSON.stringify(payload));
  } catch {
    // Local history is a convenience cache; quota/private-mode failures must not block chat.
  }
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
    agentMode: typeof record.agentMode === "boolean" ? record.agentMode : true,
    teachingMode: typeof record.teachingMode === "boolean" ? record.teachingMode : false,
    externalChatId:
      typeof record.externalChatId === "string" && record.externalChatId.trim()
        ? record.externalChatId.trim()
        : createThreadExternalChatId(),
    lastTaskId: typeof record.lastTaskId === "string" ? record.lastTaskId : null,
    teachingTaskResult: normalizeStoredTaskResult(record.teachingTaskResult),
    teachingLlmDebug: null,
    teachingLlmDebugError:
      typeof record.teachingLlmDebugError === "string" ? record.teachingLlmDebugError : null,
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
    llmDebugError: run.llmDebugError ?? null,
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
    llmDebugError: typeof record.llmDebugError === "string" ? record.llmDebugError : null,
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
    agentMode: true,
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
        agentMode: thread.agentMode,
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
  return runs.find((run) => run.id === activeId) ?? runs[runs.length - 1] ?? null;
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

function canUseDirectVoiceRecording(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof navigator !== "undefined" &&
    window.isSecureContext &&
    Boolean(navigator.mediaDevices?.getUserMedia) &&
    typeof MediaRecorder !== "undefined"
  );
}

function preferredRecorderMimeType(): string | undefined {
  if (typeof MediaRecorder === "undefined" || !MediaRecorder.isTypeSupported) {
    return undefined;
  }
  return [
    "audio/webm;codecs=opus",
    "audio/webm",
    "audio/mp4",
    "audio/ogg;codecs=opus",
  ].find((mimeType) => MediaRecorder.isTypeSupported(mimeType));
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
