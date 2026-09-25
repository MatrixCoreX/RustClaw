import type {
  ChatMessage,
  ConversationBodyDescriptor,
  ConversationBodyPage,
  ConversationHistoryPage,
  ConversationHistoryTurn,
  TaskQueryResponse,
} from "../types/api";
import {
  normalizeTaskArtifactDeliverySummary,
  normalizeTaskArtifacts,
} from "./task-artifacts";
import { appStorageKey } from "./product-identity";
import { sha256Hex } from "./sha256";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export const CONVERSATION_HISTORY_PAGE_SIZE = 60;

export function conversationHistoryScope(
  authReady: boolean,
  authMode: "key" | "webd" | null,
  userId: number | null | undefined,
  chatId: number | null | undefined,
): string {
  if (!authReady || !authMode || userId == null || chatId == null) return "";
  // Runtime identities are signed 64-bit values. JSON parses them as finite
  // JavaScript numbers even when they exceed Number.MAX_SAFE_INTEGER; they are
  // still stable enough to isolate this browser-local cache across refreshes.
  if (!Number.isFinite(userId) || !Number.isInteger(userId)) return "";
  if (!Number.isFinite(chatId) || !Number.isInteger(chatId)) return "";
  return `${authMode}:${userId}:${chatId}`;
}

export function conversationHistoryStorageKey(scope: string): string {
  const normalized = scope.trim();
  return normalized ? appStorageKey(`ui.chatThreads.v2.${normalized}`) : "";
}

export interface ServerTeachingRunProjection {
  id: string;
  taskId: string;
  conversationInputId?: string | null;
  conversationInputClientMessageId?: string | null;
  conversationInputRevision?: number | null;
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
  agentId?: string;
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

export async function fetchAllConversationHistory(
  apiFetch: ApiFetch,
): Promise<ConversationHistoryPage[]> {
  const pages: ConversationHistoryPage[] = [];
  const seenCursors = new Set<string>();
  let cursor: string | null = null;
  for (;;) {
    const page = await fetchConversationHistoryPage(apiFetch, cursor);
    pages.push(page);
    const next = page.truncated ? page.next_cursor?.trim() || null : null;
    if (!next) return pages;
    if (seenCursors.has(next)) {
      throw new Error("conversation_history_cursor_cycle");
    }
    seenCursors.add(next);
    cursor = next;
  }
}

export async function fetchConversationHistoryPage(
  apiFetch: ApiFetch,
  cursor: string | null = null,
): Promise<ConversationHistoryPage> {
  const query = new URLSearchParams({ limit: String(CONVERSATION_HISTORY_PAGE_SIZE) });
  if (cursor) query.set("cursor", cursor);
  const response = await apiFetch(`/v1/tasks/conversation-history?${query}`);
  const body = (await response.json()) as {
    ok?: boolean;
    data?: ConversationHistoryPage | null;
    error?: string | null;
  };
  if (!response.ok || !body.ok || !body.data) {
    throw new Error(body.error || `conversation_history_http_${response.status}`);
  }
  await verifyConversationHistoryPage(body.data);
  return body.data;
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
      !validConversationBodyDescriptor(turn.user_text_result) ||
      !validConversationBodyDescriptor(turn.assistant_text_result) ||
      !validConversationBodyDescriptor(turn.error_text_result) ||
      (turn.artifacts != null && !Array.isArray(turn.artifacts)) ||
      (turn.conversation_inputs != null &&
        (!Array.isArray(turn.conversation_inputs) ||
          turn.conversation_inputs.some(
            (input) =>
              input.schema_version !== 1 ||
              !machineRef(input.input_id) ||
              !machineRef(input.client_message_id) ||
              !Number.isSafeInteger(input.input_seq) ||
              input.input_seq < 1 ||
              typeof input.text !== "string" ||
              ![
                "pending",
                "deferred",
                "needs_clarification",
                "applied",
                "rejected",
                "withdrawn",
              ].includes(input.disposition) ||
              !Number.isSafeInteger(input.instruction_revision) ||
              !Number.isSafeInteger(input.accepted_at) ||
              !Number.isSafeInteger(input.updated_at),
          ))) ||
      (turn.conversation_replies != null &&
        (!Array.isArray(turn.conversation_replies) ||
          turn.conversation_replies.some(
            (reply) =>
              reply.schema_version !== 1 ||
              !machineRef(reply.reply_id) ||
              (reply.input_id != null && !machineRef(reply.input_id)) ||
              !["side_reply", "clarification"].includes(reply.relation) ||
              !["accepted", "stop_requested", "settled"].includes(
                reply.lifecycle_stage,
              ) ||
              typeof reply.text !== "string" ||
              !reply.text.trim() ||
              !Number.isSafeInteger(reply.instruction_revision) ||
              !Number.isSafeInteger(reply.execution_epoch) ||
              !Number.isSafeInteger(reply.created_at),
          ))) ||
      !Number.isSafeInteger(turn.created_at) ||
      !Number.isSafeInteger(turn.updated_at)
    ) {
      throw new Error("conversation_history_turn_invalid");
    }
  }
  const digest = await sha256Hex(JSON.stringify(page.turns));
  if (digest !== page.content_sha256.toLowerCase()) {
    throw new Error("conversation_history_digest_mismatch");
  }
}

export async function fetchNextConversationBodyPage(
  apiFetch: ApiFetch,
  descriptor: ConversationBodyDescriptor,
): Promise<ConversationBodyPage> {
  const rawUrl = descriptor.continuation?.url?.trim();
  if (descriptor.complete || !rawUrl || !safeConversationBodyUrl(rawUrl)) {
    throw new Error("conversation_body_continuation_invalid");
  }
  const response = await apiFetch(rawUrl);
  const body = (await response.json()) as {
    ok?: boolean;
    data?: ConversationBodyPage | null;
    error?: string | null;
  };
  if (!response.ok || !body.ok || !body.data) {
    throw new Error(body.error || `conversation_body_http_${response.status}`);
  }
  const page = body.data;
  if (
    page.schema_version !== 1 ||
    page.status !== "ok" ||
    !machineRef(page.task_id) ||
    !["user", "assistant", "error"].includes(page.field) ||
    typeof page.text !== "string" ||
    !Number.isSafeInteger(page.start_byte) ||
    !Number.isSafeInteger(page.end_byte) ||
    !Number.isSafeInteger(page.total_size_bytes) ||
    page.start_byte !== descriptor.returned_size_bytes ||
    page.end_byte < page.start_byte ||
    new TextEncoder().encode(page.text).byteLength !== page.end_byte - page.start_byte ||
    page.total_size_bytes !== descriptor.original_size_bytes ||
    page.content_sha256.toLowerCase() !== descriptor.content_sha256.toLowerCase() ||
    (!page.complete && !Number.isSafeInteger(page.next_start_byte))
  ) {
    throw new Error("conversation_body_page_invalid");
  }
  return page;
}

export function advanceConversationBodyDescriptor(
  descriptor: ConversationBodyDescriptor,
  page: ConversationBodyPage,
): ConversationBodyDescriptor {
  const nextStart = page.complete ? null : page.next_start_byte ?? null;
  const previousUrl = descriptor.continuation?.url ?? "";
  return {
    ...descriptor,
    complete: page.complete,
    returned_size_bytes: page.end_byte,
    continuation:
      nextStart == null
        ? null
        : {
            kind: "conversation_body_range",
            url: withConversationBodyStart(previousUrl, nextStart),
            next_start_byte: nextStart,
          },
  };
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
      bodyResult: turn.user_text_result ?? null,
    });
    const taskResult: TaskQueryResponse = {
      task_id: turn.task_id,
      status: turn.status,
      result_json: turn.assistant_text
        ? {
            text: turn.assistant_text,
            artifacts: normalizeTaskArtifacts(turn.artifacts),
            artifact_delivery: turn.artifact_delivery,
          }
        : null,
      error_text: turn.error_text ?? null,
    };
    const followupInputs = [...(turn.conversation_inputs ?? [])].sort(
      (left, right) => left.input_seq - right.input_seq,
    );
    const followupInputIds = new Set(followupInputs.map((input) => input.input_id));
    const replies = [...(turn.conversation_replies ?? [])].sort(
      (left, right) =>
        left.created_at - right.created_at || left.reply_id.localeCompare(right.reply_id),
    );
    const initialReplies = replies.filter(
      (reply) => !reply.input_id || !followupInputIds.has(reply.input_id),
    );
    const latestInitialReply = initialReplies[initialReplies.length - 1] ?? null;
    const initialAssistantMessageId = latestInitialReply
      ? `conversation-${latestInitialReply.reply_id}`
      : followupInputs.length === 0
        ? assistantMessageId
        : null;
    const initialRun: ServerTeachingRunProjection = {
      id: `teach-${turn.task_id}`,
      taskId: turn.task_id,
      conversationInputId: null,
      conversationInputClientMessageId: null,
      conversationInputRevision: null,
      userMessageId,
      assistantMessageId: initialAssistantMessageId,
      userText,
      assistantText: latestInitialReply?.text.trim() ||
        (followupInputs.length === 0
          ? turn.assistant_text?.trim() || turn.error_text?.trim() || null
          : null),
      status: turn.status,
      startedAt,
      completedAt,
      taskResult,
    };
    teachingRuns.push(initialRun);
    for (const reply of initialReplies) {
      messages.push({
        id: `conversation-${reply.reply_id}`,
        role: "assistant",
        text: reply.text,
        ts: timestampMs(reply.created_at),
      });
    }
    for (const input of followupInputs) {
      const followupMessageId = `u-${input.input_id}`;
      messages.push({
        id: followupMessageId,
        role: "user",
        text: input.text,
        ts: timestampMs(input.accepted_at),
      });
      const latest = input === followupInputs[followupInputs.length - 1];
      const inputReplies = replies.filter((reply) => reply.input_id === input.input_id);
      const latestInputReply = inputReplies[inputReplies.length - 1] ?? null;
      teachingRuns.push({
        id: `teach-${input.input_id}`,
        taskId: turn.task_id,
        conversationInputId: input.input_id,
        conversationInputClientMessageId: input.client_message_id,
        conversationInputRevision: input.instruction_revision,
        userMessageId: followupMessageId,
        assistantMessageId: latestInputReply
          ? `conversation-${latestInputReply.reply_id}`
          : latest
            ? assistantMessageId
            : null,
        userText: input.text,
        assistantText: latestInputReply?.text.trim() ||
          (latest
            ? turn.assistant_text?.trim() || turn.error_text?.trim() || null
            : null),
        status: turn.status,
        startedAt: timestampMs(input.accepted_at),
        completedAt,
        taskResult,
      });
      for (const reply of inputReplies) {
        messages.push({
          id: `conversation-${reply.reply_id}`,
          role: "assistant",
          text: reply.text,
          ts: timestampMs(reply.created_at),
        });
      }
    }
    const assistantText = turn.assistant_text?.trim() || turn.error_text?.trim() || null;
    if (assistantMessageId && assistantText) {
      messages.push({
        id: assistantMessageId,
        role: turn.assistant_text ? "assistant" : "system",
        text: assistantText,
        ts: completedAt ?? timestampMs(turn.updated_at),
        artifacts: normalizeTaskArtifacts(turn.artifacts),
        artifactDelivery: normalizeTaskArtifactDeliverySummary(turn.artifact_delivery),
        bodyResult: turn.assistant_text
          ? (turn.assistant_text_result ?? null)
          : (turn.error_text_result ?? null),
      });
    }
  }
  const firstUserText = teachingRuns[0]?.userText.trim() ?? "";
  messages.sort((left, right) => left.ts - right.ts || left.id.localeCompare(right.id));
  const inferredTitle =
    firstUserText.length > 28 ? `${firstUserText.slice(0, 28)}...` : firstUserText;
  const customTitle = [...turns]
    .reverse()
    .find((turn) => turn.conversation_title?.trim())
    ?.conversation_title?.trim();
  const latest = turns[turns.length - 1];
  return {
    id: conversationId,
    agentId: [...turns].reverse().find((turn) => turn.agent_id?.trim())?.agent_id?.trim(),
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

function validConversationBodyDescriptor(
  value: ConversationBodyDescriptor | null | undefined,
): boolean {
  if (value == null) return true;
  return (
    value.schema_version === 1 &&
    typeof value.complete === "boolean" &&
    Number.isSafeInteger(value.original_size_bytes) &&
    Number.isSafeInteger(value.returned_size_bytes) &&
    value.original_size_bytes >= value.returned_size_bytes &&
    /^[0-9a-f]{64}$/i.test(value.content_sha256) &&
    (value.complete ||
      (value.continuation?.kind === "conversation_body_range" &&
        Number.isSafeInteger(value.continuation.next_start_byte) &&
        value.continuation.next_start_byte === value.returned_size_bytes &&
        safeConversationBodyUrl(value.continuation.url)))
  );
}

function safeConversationBodyUrl(value: string): boolean {
  return /^\/v1\/tasks\/[A-Za-z0-9-]+\/conversation-body\/(user|assistant|error)\?/.test(
    value,
  );
}

function withConversationBodyStart(value: string, startByte: number): string {
  if (!safeConversationBodyUrl(value)) {
    throw new Error("conversation_body_continuation_invalid");
  }
  const url = new URL(value, "http://agent.invalid");
  url.searchParams.set("start_byte", String(startByte));
  return `${url.pathname}?${url.searchParams.toString()}`;
}
