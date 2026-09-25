import { useEffect, useRef, useState } from "react";

import { followConversationInputEventStream } from "../lib/conversation-input-event-stream";
import { CLIENT_ORIGIN_HEADER } from "../lib/product-identity";
import { formatUiError } from "../lib/ui-error";
import type {
  ApiResponse,
  ConversationInputPage,
  ConversationInputReceipt,
  ConversationInputRecord,
} from "../types/api";
import type { ChatDeferredInputSummary } from "../types/chat-runtime";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

interface DeferredConversationRef {
  id: string;
  agentId: string;
  externalChatId: string;
}

interface UseDeferredChatInputsParams {
  apiFetch: ApiFetch;
  t: Translate;
  enabled: boolean;
  conversation: DeferredConversationRef;
  onError: (message: string | null) => void;
  onActivated: (
    input: ChatDeferredInputSummary,
    receipt: ConversationInputReceipt,
    taskId: string,
  ) => void;
}

function deferredInputSummary(record: ConversationInputRecord): ChatDeferredInputSummary {
  const text = record.content
    .filter((item) => item.kind === "text" && typeof item.text === "string")
    .map((item) => String(item.text).trim())
    .filter(Boolean)
    .join("\n");
  const attachmentNames = record.content
    .filter((item) => item.kind === "attachment")
    .map((item) =>
      typeof item.display_name === "string" && item.display_name.trim()
        ? item.display_name.trim()
        : String(item.attachment_id ?? "").trim(),
    )
    .filter(Boolean);
  return {
    inputId: record.receipt.input_id,
    text,
    attachmentNames,
    acceptedAt: record.receipt.accepted_at_ts * 1_000,
  };
}

export function useDeferredChatInputs({
  apiFetch,
  t,
  enabled,
  conversation,
  onError,
  onActivated,
}: UseDeferredChatInputsParams) {
  const [items, setItems] = useState<ChatDeferredInputSummary[]>([]);
  const [actionInputId, setActionInputId] = useState<string | null>(null);
  const apiFetchRef = useRef(apiFetch);
  const onErrorRef = useRef(onError);
  const tRef = useRef(t);
  apiFetchRef.current = apiFetch;
  onErrorRef.current = onError;
  tRef.current = t;

  useEffect(() => {
    if (!enabled) {
      setItems([]);
      return;
    }
    const controller = new AbortController();
    let loadRunning = false;
    let loadRequested = false;
    const loadOnce = async () => {
      const records: ConversationInputRecord[] = [];
      let afterInputSeq = 0;
      for (;;) {
        const query = new URLSearchParams({
          conversation_id: conversation.id,
          agent_id: conversation.agentId,
          channel: "ui",
          channel_account_id: conversation.externalChatId,
          after_input_seq: String(afterInputSeq),
          limit: "100",
        });
        const response = await apiFetchRef.current(`/v1/conversation-inputs?${query.toString()}`, {
          signal: controller.signal,
        });
        const body = (await response.json()) as ApiResponse<ConversationInputPage>;
        if (!response.ok || !body.ok || !body.data) {
          throw new Error(body.error || `conversation_input_list_http_${response.status}`);
        }
        records.push(...body.data.items);
        const next = body.data.next_after_input_seq;
        if (!next || next <= afterInputSeq) break;
        afterInputSeq = next;
      }
      if (!controller.signal.aborted) {
        setItems(
          records
            .filter((item) => item.receipt.disposition === "deferred")
            .map(deferredInputSummary),
        );
      }
    };
    const reconcile = async () => {
      if (loadRunning) {
        loadRequested = true;
        return;
      }
      loadRunning = true;
      try {
        do {
          loadRequested = false;
          await loadOnce();
        } while (loadRequested && !controller.signal.aborted);
      } finally {
        loadRunning = false;
      }
    };
    const reportReadError = (error: unknown) => {
      if (!controller.signal.aborted) {
        onErrorRef.current(
          formatUiError(
            error,
            tRef.current,
            "无法读取延后消息。",
            "Could not load deferred messages.",
          ),
        );
      }
    };
    void reconcile().catch(reportReadError);
    void followConversationInputEventStream(
      (path, init) => apiFetchRef.current(path, init),
      {
        conversationId: conversation.id,
        agentId: conversation.agentId,
        channel: "ui",
        channelAccountId: conversation.externalChatId,
      },
      reconcile,
      controller.signal,
    ).catch(reportReadError);
    return () => controller.abort();
  }, [conversation.agentId, conversation.externalChatId, conversation.id, enabled]);

  const activate = async (inputId: string) => {
    const input = items.find((item) => item.inputId === inputId);
    if (!input || actionInputId) return;
    setActionInputId(inputId);
    onError(null);
    try {
      const response = await apiFetch(
        `/v1/conversation-inputs/${encodeURIComponent(inputId)}/activate`,
        { method: "POST", headers: { [CLIENT_ORIGIN_HEADER]: "ui" } },
      );
      const body = (await response.json()) as ApiResponse<ConversationInputReceipt>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `conversation_input_activate_http_${response.status}`);
      }
      const taskId = body.data.target_task_id?.trim();
      if (!taskId) throw new Error("conversation_input_task_binding_missing");
      setItems((current) => current.filter((item) => item.inputId !== inputId));
      onActivated(input, body.data, taskId);
    } catch (error) {
      onError(
        formatUiError(
          error,
          t,
          "延后消息未能开始处理。",
          "The deferred message could not be started.",
        ),
      );
    } finally {
      setActionInputId(null);
    }
  };

  const withdraw = async (inputId: string) => {
    if (actionInputId) return;
    setActionInputId(inputId);
    onError(null);
    try {
      const response = await apiFetch(
        `/v1/conversation-inputs/${encodeURIComponent(inputId)}/withdraw`,
        { method: "POST", headers: { [CLIENT_ORIGIN_HEADER]: "ui" } },
      );
      const body = (await response.json()) as ApiResponse<ConversationInputReceipt>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(body.error || `conversation_input_withdraw_http_${response.status}`);
      }
      setItems((current) => current.filter((item) => item.inputId !== inputId));
    } catch (error) {
      onError(
        formatUiError(
          error,
          t,
          "延后消息未能撤回。",
          "The deferred message could not be withdrawn.",
        ),
      );
    } finally {
      setActionInputId(null);
    }
  };

  const add = (input: ChatDeferredInputSummary) => {
    setItems((current) => [
      ...current.filter((item) => item.inputId !== input.inputId),
      input,
    ].sort((left, right) => left.acceptedAt - right.acceptedAt));
  };

  return { items, actionInputId, activate, withdraw, add };
}
