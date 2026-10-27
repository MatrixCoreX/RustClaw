import type { useUiDialog } from "../components/UiDialogProvider";
import { threadHasServerHistory } from "../lib/chat-thread-state";
import type { ApiResponse, ConversationArchiveUpdate, ConversationTitleUpdate } from "../types/api";
import type { ChatThreadRecord, ChatThreadState } from "../types/chat-runtime";

export function createChatHistoryControls({ apiFetch, chatThreadState, t, showConfirm, removeChatThreadLocally, setChatError, updateChatThreadById, isCurrent }: {
  apiFetch: (path: string, init?: RequestInit) => Promise<Response>;
  chatThreadState: ChatThreadState;
  t: (zh: string, en: string) => string;
  showConfirm: ReturnType<typeof useUiDialog>["confirm"];
  removeChatThreadLocally: (threadId: string) => void;
  setChatError: (error: string | null) => void;
  updateChatThreadById: (id: string, update: (thread: ChatThreadRecord) => ChatThreadRecord) => void;
  isCurrent: () => boolean;
}) {
  const archiveChatThreadOnServer = async (thread: ChatThreadRecord) => {
    if (!isCurrent()) throw new Error("chat_scope_obsolete");
    if (!threadHasServerHistory(thread)) return;
    const response = await apiFetch(
      `/v1/tasks/conversations/${encodeURIComponent(thread.id)}`,
      { method: "DELETE" },
    );
    const body = (await response.json()) as ApiResponse<ConversationArchiveUpdate>;
    if (!isCurrent()) throw new Error("chat_scope_obsolete");
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
    if (!isCurrent()) return false;
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
      if (!isCurrent()) return false;
      await archiveChatThreadOnServer(thread);
      if (!isCurrent()) return false;
      removeChatThreadLocally(threadId);
      setChatError(null);
      return true;
    } catch {
      if (!isCurrent()) return false;
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
    if (!isCurrent()) return false;
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
        if (!isCurrent()) return false;
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
      if (!isCurrent()) return false;
      setChatError(
        t(
          "任务名称保存失败，请检查连接后重试。",
          "The task name could not be saved. Check the connection and try again.",
        ),
      );
      return false;
    }
  };

  return { archiveChatThreadOnServer, deleteChatThread, renameChatThread };
}
