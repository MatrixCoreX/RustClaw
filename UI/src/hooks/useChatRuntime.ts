import { useRef, useState, type KeyboardEvent } from "react";

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
  TaskQueryResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

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
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([
    {
      id: "chat-system-welcome",
      role: "system",
      text: t(
        "会话窗口已连接 clawd。发送消息后会自动提交 ask 任务并轮询结果。",
        "The chat window is connected to clawd. Messages submit ask tasks and poll for results automatically.",
      ),
      ts: Date.now(),
    },
  ]);
  const [chatInput, setChatInput] = useState("");
  const [chatAttachments, setChatAttachments] = useState<ChatAttachment[]>([]);
  const [chatAgentMode, setChatAgentMode] = useState(true);
  const [chatSending, setChatSending] = useState(false);
  const [chatRecording, setChatRecording] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const chatAttachmentInputRef = useRef<HTMLInputElement | null>(null);
  const chatMediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chatAudioChunksRef = useRef<Blob[]>([]);

  const clearChatMessages = () => {
    setChatMessages([
      {
        id: `chat-clear-${Date.now()}`,
        role: "system",
        text: t("聊天记录已清空。", "Chat history cleared."),
        ts: Date.now(),
      },
    ]);
  };

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
        if (merged.length > CHAT_MAX_ATTACHMENTS) {
          setChatError(t("最多只能一次发送 6 个附件。", "You can send up to 6 attachments at once."));
        } else {
          setChatError(null);
        }
        return merged.slice(0, CHAT_MAX_ATTACHMENTS);
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
    setChatAttachments((prev) => prev.filter((_, i) => i !== index));
  };

  const startChatVoiceRecording = async () => {
    if (chatRecording || chatSending) return;
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
      setChatError(t("当前浏览器不支持录音。", "This browser does not support voice recording."));
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const recorder = new MediaRecorder(stream);
      chatAudioChunksRef.current = [];
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) {
          chatAudioChunksRef.current.push(event.data);
        }
      };
      recorder.onerror = () => {
        stream.getTracks().forEach((track) => track.stop());
        setChatRecording(false);
        setChatError(t("录音失败，请重新尝试。", "Recording failed. Please try again."));
      };
      recorder.onstop = async () => {
        stream.getTracks().forEach((track) => track.stop());
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
          setChatAttachments((prev) => [...prev, attachment].slice(0, CHAT_MAX_ATTACHMENTS));
          setChatError(null);
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
      setChatRecording(true);
      setChatError(null);
    } catch (err) {
      setChatRecording(false);
      setChatError(
        err instanceof Error ? err.message : t("无法开始录音。", "Unable to start recording."),
      );
    }
  };

  const stopChatVoiceRecording = () => {
    const recorder = chatMediaRecorderRef.current;
    if (recorder && recorder.state === "recording") {
      recorder.stop();
    }
  };

  const sendChatMessage = async () => {
    const text = chatInput.trim();
    const attached = chatAttachments;
    if ((!text && attached.length === 0) || chatSending || chatRecording) return;
    const attachedImages = attached.filter(attachmentIsImage);
    const attachedAudios = attached.filter(attachmentIsAudio);
    const attachedFiles = attached.filter(
      (attachment) => !attachmentIsImage(attachment) && !attachmentIsAudio(attachment),
    );
    const audioOnly = attachedAudios.length > 0 && attachedImages.length === 0 && attachedFiles.length === 0;
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
    setChatMessages((prev) => [...prev, userMsg]);
    setChatInput("");
    setChatAttachments([]);
    if (chatAttachmentInputRef.current) {
      chatAttachmentInputRef.current.value = "";
    }

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
        channel: interactionChannel,
        kind: "ask",
        ...(activeUserKey ? { user_key: activeUserKey } : {}),
        ...activeIdentityIds,
        ...(interactionExternalUserId.trim() ? { external_user_id: interactionExternalUserId.trim() } : {}),
        ...(interactionExternalChatId.trim() ? { external_chat_id: interactionExternalChatId.trim() } : {}),
        payload: {
          text: requestText,
          agent_mode: chatAgentMode,
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
                ...(attachedAudios.length > 0
                  ? {
                      audio: {
                        name: attachedAudios[0].name,
                        mime_type: attachedAudios[0].mimeType,
                        size: attachedAudios[0].size,
                        base64: attachedAudios[0].dataUrl,
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

      const assistantMsg: ChatMessage = {
        id: `a-${Date.now()}`,
        role: "assistant",
        text: attachedImages.length > 0 ? formatVisionResultText(extractTaskText(finalResult)) : extractTaskText(finalResult),
        ts: Date.now(),
      };
      setChatMessages((prev) => [...prev, assistantMsg]);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setChatError(message);
      const systemErrMsg: ChatMessage = {
        id: `e-${Date.now()}`,
        role: "system",
        text: `${t("发送失败", "Send failed")}: ${message}`,
        ts: Date.now(),
      };
      setChatMessages((prev) => [...prev, systemErrMsg]);
    } finally {
      setChatSending(false);
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
    chatAgentMode,
    chatSending,
    chatRecording,
    chatError,
    chatAttachmentInputRef,
    setChatAgentMode,
    clearChatMessages,
    setChatInput,
    handleChatInputKeyDown,
    handleChatAttachmentSelection,
    removeChatAttachment,
    startChatVoiceRecording,
    stopChatVoiceRecording,
    sendChatMessage,
  };
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
