import type {
  ChatMessage,
  ConversationHistoryPage,
  ConversationHistoryTurn,
  TaskQueryResponse,
} from "../types/api";
import { normalizeTaskArtifacts } from "./task-artifacts";

type Translate = (zh: string, en: string) => string;

export function conversationHistoryScope(
  authReady: boolean,
  authMode: "key" | "webd" | null,
  userId: number | null | undefined,
  chatId: number | null | undefined,
): string {
  if (!authReady || !authMode || userId == null || chatId == null) return "";
  if (!Number.isSafeInteger(userId) || !Number.isSafeInteger(chatId)) return "";
  return `${authMode}:${userId}:${chatId}`;
}

export function conversationHistoryStorageKey(scope: string): string {
  const normalized = scope.trim();
  return normalized ? `rustclaw.ui.chatThreads.v2.${normalized}` : "";
}

export interface ServerTeachingRunProjection {
  id: string;
  taskId: string;
  userMessageId: string;
  assistantMessageId: string | null;
  userText: string;
  assistantText: string | null;
  status: TaskQueryResponse["status"] | "running";
  startedAt: number;
  completedAt: number | null;
  taskResult: TaskQueryResponse;
}

export interface ServerChatThreadProjection {
  id: string;
  externalChatId: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  updatedAt: number;
  lastTaskId: string;
  teachingRuns: ServerTeachingRunProjection[];
}

export function projectConversationHistory(
  pages: ConversationHistoryPage[],
  t: Translate,
): ServerChatThreadProjection[] {
  const turns = deduplicateTurns(pages.flatMap((page) => page.turns));
  const grouped = new Map<string, ConversationHistoryTurn[]>();
  for (const turn of turns) {
    const items = grouped.get(turn.conversation_id) ?? [];
    items.push(turn);
    grouped.set(turn.conversation_id, items);
  }
  return [...grouped.entries()]
    .map(([conversationId, items]) => projectThread(conversationId, items, t))
    .sort((left, right) => right.updatedAt - left.updatedAt);
}

export async function verifyConversationHistoryPage(
  page: ConversationHistoryPage,
): Promise<void> {
  if (
    page.schema_version !== 1 ||
    page.status !== "ok" ||
    !Array.isArray(page.turns) ||
    !/^[0-9a-f]{64}$/i.test(page.content_sha256)
  ) {
    throw new Error("conversation_history_schema_invalid");
  }
  if (
    page.truncated &&
    (typeof page.next_cursor !== "string" || !page.next_cursor.trim())
  ) {
    throw new Error("conversation_history_cursor_missing");
  }
  for (const turn of page.turns) {
    if (
      turn.schema_version !== 1 ||
      !machineRef(turn.task_id) ||
      !machineRef(turn.conversation_id) ||
      !["queued", "running", "succeeded", "failed", "canceled", "timeout"].includes(
        turn.status,
      ) ||
      !Number.isSafeInteger(turn.attachment_count) ||
      turn.attachment_count < 0 ||
      !Array.isArray(turn.attachment_kinds) ||
      (turn.artifacts != null && !Array.isArray(turn.artifacts)) ||
      !Number.isSafeInteger(turn.created_at) ||
      !Number.isSafeInteger(turn.updated_at)
    ) {
      throw new Error("conversation_history_turn_invalid");
    }
  }
  if (!globalThis.crypto?.subtle) return;
  const digest = await sha256Hex(JSON.stringify(page.turns));
  if (digest !== page.content_sha256.toLowerCase()) {
    throw new Error("conversation_history_digest_mismatch");
  }
}

function deduplicateTurns(turns: ConversationHistoryTurn[]): ConversationHistoryTurn[] {
  const byTask = new Map<string, ConversationHistoryTurn>();
  for (const turn of turns) {
    if (
      turn.schema_version !== 1 ||
      !machineRef(turn.task_id) ||
      !machineRef(turn.conversation_id)
    ) {
      continue;
    }
    byTask.set(turn.task_id, turn);
  }
  return [...byTask.values()];
}

function projectThread(
  conversationId: string,
  sourceTurns: ConversationHistoryTurn[],
  t: Translate,
): ServerChatThreadProjection {
  const turns = [...sourceTurns].sort(
    (left, right) =>
      left.created_at - right.created_at || left.task_id.localeCompare(right.task_id),
  );
  const messages: ChatMessage[] = [];
  const teachingRuns: ServerTeachingRunProjection[] = [];
  for (const turn of turns) {
    const startedAt = timestampMs(turn.created_at);
    const completedAt = terminalStatus(turn.status) ? timestampMs(turn.updated_at) : null;
    const userMessageId = `u-${turn.task_id}`;
    const assistantMessageId =
      turn.assistant_text || turn.error_text ? `a-${turn.task_id}` : null;
    const userText =
      turn.user_text?.trim() ||
      (turn.attachment_count > 0
        ? t("附件消息", "Attachment message")
        : t("空消息", "Empty message"));
    messages.push({
      id: userMessageId,
      role: "user",
      text: userText,
      ts: startedAt,
    });
    const assistantText = turn.assistant_text?.trim() || turn.error_text?.trim() || null;
    if (assistantMessageId && assistantText) {
      messages.push({
        id: assistantMessageId,
        role: turn.assistant_text ? "assistant" : "system",
        text: assistantText,
        ts: completedAt ?? timestampMs(turn.updated_at),
        artifacts: normalizeTaskArtifacts(turn.artifacts),
      });
    }
    teachingRuns.push({
      id: `teach-${turn.task_id}`,
      taskId: turn.task_id,
      userMessageId,
      assistantMessageId,
      userText,
      assistantText,
      status: turn.status,
      startedAt,
      completedAt,
      taskResult: {
        task_id: turn.task_id,
        status: turn.status,
        result_json: turn.assistant_text
          ? { text: turn.assistant_text, artifacts: normalizeTaskArtifacts(turn.artifacts) }
          : null,
        error_text: turn.error_text ?? null,
      },
    });
  }
  const firstUserText = teachingRuns[0]?.userText.trim() ?? "";
  const inferredTitle =
    firstUserText.length > 28 ? `${firstUserText.slice(0, 28)}...` : firstUserText;
  const customTitle = [...turns]
    .reverse()
    .find((turn) => turn.conversation_title?.trim())
    ?.conversation_title?.trim();
  const latest = turns[turns.length - 1];
  return {
    id: conversationId,
    externalChatId:
      [...turns].reverse().find((turn) => turn.external_chat_id?.trim())?.external_chat_id ??
      `ui-${conversationId}`,
    title: customTitle || inferredTitle || t("未命名任务", "Untitled task"),
    messages,
    createdAt: timestampMs(turns[0]?.created_at ?? 0),
    updatedAt: timestampMs(latest?.updated_at ?? 0),
    lastTaskId: latest?.task_id ?? "",
    teachingRuns,
  };
}

function terminalStatus(status: TaskQueryResponse["status"]): boolean {
  return ["succeeded", "failed", "canceled", "timeout"].includes(status);
}

function timestampMs(value: number): number {
  return value > 0 && value < 1_000_000_000_000 ? value * 1000 : Math.max(0, value);
}

function machineRef(value: string): boolean {
  return (
    value.length > 0 &&
    value.length <= 128 &&
    /^[A-Za-z0-9_.:-]+$/.test(value)
  );
}

async function sha256Hex(value: string): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(value),
  );
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}
