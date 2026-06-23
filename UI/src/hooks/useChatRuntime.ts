import { useRef, useState, type KeyboardEvent } from "react";

import { fileToDataUrl, formatVisionResultText } from "../lib/chat-attachments";
import { sleep } from "../lib/display-format";
import { extractTaskText } from "../lib/task-result";
import type {
  ApiResponse,
  ChannelName,
  ChatImageAttachment,
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
  const [chatImageAttachments, setChatImageAttachments] = useState<ChatImageAttachment[]>([]);
  const [chatAgentMode, setChatAgentMode] = useState(true);
  const [chatSending, setChatSending] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const chatImageInputRef = useRef<HTMLInputElement | null>(null);

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

  const handleChatImageSelection = async (fileList: FileList | null) => {
    if (!fileList || fileList.length === 0) return;
    try {
      const selected = Array.from(fileList).filter((file) => file.type.startsWith("image/"));
      if (selected.length === 0) {
        setChatError(t("请选择图片文件。", "Please choose image files."));
        return;
      }
      const nextImages = await Promise.all(
        selected.map(async (file) => ({
          name: file.name,
          dataUrl: await fileToDataUrl(file),
        })),
      );
      setChatImageAttachments((prev) => [...prev, ...nextImages].slice(0, 6));
      setChatError(null);
    } catch (err) {
      setChatError(err instanceof Error ? err.message : t("读取图片失败。", "Failed to read images."));
    }
  };

  const removeChatImageAttachment = (index: number) => {
    setChatImageAttachments((prev) => prev.filter((_, i) => i !== index));
  };

  const sendChatMessage = async () => {
    const text = chatInput.trim();
    const attachedImages = chatImageAttachments;
    if ((!text && attachedImages.length === 0) || chatSending) return;
    const requestText = text || t("请描述这张图片", "Please describe this image");
    setChatSending(true);
    setChatError(null);
    const userMsg: ChatMessage = {
      id: `u-${Date.now()}`,
      role: "user",
      text: text || t("发送了一张图片", "Sent an image"),
      ts: Date.now(),
      images: attachedImages,
    };
    setChatMessages((prev) => [...prev, userMsg]);
    setChatInput("");
    setChatImageAttachments([]);
    if (chatImageInputRef.current) {
      chatImageInputRef.current.value = "";
    }

    try {
      const adapterName = interactionAdapter.trim();
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
          ...(adapterName ? { adapter: adapterName } : {}),
          ...(attachedImages.length > 0
            ? {
                images: attachedImages.map((image) => ({ base64: image.dataUrl })),
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
    chatImageAttachments,
    chatAgentMode,
    chatSending,
    chatError,
    chatImageInputRef,
    setChatAgentMode,
    clearChatMessages,
    setChatInput,
    handleChatInputKeyDown,
    handleChatImageSelection,
    removeChatImageAttachment,
    sendChatMessage,
  };
}
