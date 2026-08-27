import type { TaskEventEnvelope } from "../types/api";
import { prefixedSha256 } from "./sha256";

export type AssistantPresentationEventKind =
  | "assistant_output_started"
  | "assistant_output_delta"
  | "assistant_output_completed"
  | "assistant_output_aborted"
  | "assistant_output_replaced";

export interface AssistantPresentationEvent {
  kind: AssistantPresentationEventKind;
  schemaVersion: number;
  taskId: string;
  conversationId: string;
  turnId: string;
  streamId: string;
  attemptId: string;
  sequence: number;
  contentOffsetBytes: number;
  createdAt: number;
  content?: string;
  totalContentBytes?: number;
  contentSha256?: string;
  errorCode?: string;
  messageKey?: string;
  retryable?: boolean;
  oldStreamId?: string;
  newStreamId?: string;
}

export interface AssistantPresentationState {
  streamId: string;
  attemptId: string;
  taskId: string;
  conversationId: string;
  turnId: string;
  status: "streaming" | "completed" | "aborted" | "replaced";
  content: string;
  contentBytes: number;
  nextSequence: number;
  contentSha256: string | null;
  errorCode: string | null;
  messageKey: string | null;
  retryable: boolean | null;
}

const PRESENTATION_KINDS = new Set<AssistantPresentationEventKind>([
  "assistant_output_started",
  "assistant_output_delta",
  "assistant_output_completed",
  "assistant_output_aborted",
  "assistant_output_replaced",
]);

const encoder = new TextEncoder();

export function decodeAssistantPresentationEvent(
  envelope: TaskEventEnvelope,
): AssistantPresentationEvent | null {
  if (!PRESENTATION_KINDS.has(envelope.event_kind as AssistantPresentationEventKind)) {
    return null;
  }
  const kind = envelope.event_kind as AssistantPresentationEventKind;
  const payload = record(envelope.payload, "assistant_presentation_payload_invalid");
  const event: AssistantPresentationEvent = {
    kind,
    schemaVersion: integer(payload.schema_version, "assistant_presentation_schema_version_invalid"),
    taskId: token(payload.task_id, "assistant_presentation_task_id_invalid"),
    conversationId: token(
      payload.conversation_id,
      "assistant_presentation_conversation_id_invalid",
    ),
    turnId: token(payload.turn_id, "assistant_presentation_turn_id_invalid"),
    streamId: token(payload.stream_id, "assistant_presentation_stream_id_invalid"),
    attemptId: token(payload.attempt_id, "assistant_presentation_attempt_id_invalid"),
    sequence: integer(payload.sequence, "assistant_presentation_sequence_invalid"),
    contentOffsetBytes: integer(
      payload.content_offset_bytes,
      "assistant_presentation_offset_invalid",
    ),
    createdAt: integer(payload.created_at, "assistant_presentation_created_at_invalid"),
  };
  if (event.schemaVersion !== 1 || event.taskId !== envelope.task_id) {
    throw new Error("assistant_presentation_identity_mismatch");
  }
  if (kind === "assistant_output_delta") {
    event.content = string(payload.content, "assistant_presentation_content_invalid");
  } else if (kind === "assistant_output_completed") {
    event.totalContentBytes = integer(
      payload.total_content_bytes,
      "assistant_presentation_total_bytes_invalid",
    );
    event.contentSha256 = sha256(payload.content_sha256);
  } else if (kind === "assistant_output_aborted") {
    event.errorCode = token(payload.error_code, "assistant_presentation_error_code_invalid");
    event.messageKey = token(payload.message_key, "assistant_presentation_message_key_invalid");
    if (typeof payload.retryable !== "boolean") {
      throw new Error("assistant_presentation_retryable_invalid");
    }
    event.retryable = payload.retryable;
  } else if (kind === "assistant_output_replaced") {
    event.oldStreamId = token(
      payload.old_stream_id,
      "assistant_presentation_old_stream_id_invalid",
    );
    event.newStreamId = token(
      payload.new_stream_id,
      "assistant_presentation_new_stream_id_invalid",
    );
  }
  return event;
}

export class AssistantPresentationReducer {
  private readonly streams = new Map<string, AssistantPresentationState>();
  private readonly seen = new Map<string, Map<number, string>>();

  async apply(event: AssistantPresentationEvent): Promise<AssistantPresentationState | null> {
    if (event.kind === "assistant_output_replaced") {
      const previous = this.streams.get(event.oldStreamId ?? "");
      if (previous) previous.status = "replaced";
      return previous ?? null;
    }
    const fingerprint = JSON.stringify(event);
    const streamSeen = this.seen.get(event.streamId) ?? new Map<number, string>();
    const prior = streamSeen.get(event.sequence);
    if (prior !== undefined) {
      if (prior !== fingerprint) throw new Error("assistant_presentation_duplicate_conflict");
      return this.streams.get(event.streamId) ?? null;
    }

    if (event.kind === "assistant_output_started") {
      if (event.sequence !== 0 || event.contentOffsetBytes !== 0) {
        throw new Error("assistant_presentation_start_invalid");
      }
      if (this.streams.has(event.streamId)) {
        throw new Error("assistant_presentation_stream_conflict");
      }
      const state: AssistantPresentationState = {
        streamId: event.streamId,
        attemptId: event.attemptId,
        taskId: event.taskId,
        conversationId: event.conversationId,
        turnId: event.turnId,
        status: "streaming",
        content: "",
        contentBytes: 0,
        nextSequence: 1,
        contentSha256: null,
        errorCode: null,
        messageKey: null,
        retryable: null,
      };
      this.streams.set(event.streamId, state);
      streamSeen.set(event.sequence, fingerprint);
      this.seen.set(event.streamId, streamSeen);
      return state;
    }

    const state = this.streams.get(event.streamId);
    if (!state) throw new Error("assistant_presentation_start_missing");
    if (state.status !== "streaming") throw new Error("assistant_presentation_stream_terminal");
    if (event.sequence !== state.nextSequence) {
      throw new Error("assistant_presentation_sequence_gap");
    }
    if (event.contentOffsetBytes !== state.contentBytes) {
      throw new Error("assistant_presentation_offset_mismatch");
    }

    if (event.kind === "assistant_output_delta") {
      const content = event.content ?? "";
      state.content += content;
      state.contentBytes += encoder.encode(content).byteLength;
    } else if (event.kind === "assistant_output_completed") {
      if (
        event.totalContentBytes !== state.contentBytes ||
        event.contentOffsetBytes !== event.totalContentBytes
      ) {
        throw new Error("assistant_presentation_completion_size_mismatch");
      }
      const expectedDigest = await prefixedSha256(state.content);
      if (event.contentSha256 !== expectedDigest) {
        throw new Error("assistant_presentation_digest_mismatch");
      }
      state.status = "completed";
      state.contentSha256 = event.contentSha256 ?? null;
    } else if (event.kind === "assistant_output_aborted") {
      state.status = "aborted";
      state.errorCode = event.errorCode ?? null;
      state.messageKey = event.messageKey ?? null;
      state.retryable = event.retryable ?? null;
    }
    state.nextSequence += 1;
    streamSeen.set(event.sequence, fingerprint);
    this.seen.set(event.streamId, streamSeen);
    return state;
  }

  get(streamId: string): AssistantPresentationState | null {
    return this.streams.get(streamId) ?? null;
  }
}

function record(value: unknown, errorCode: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(errorCode);
  return value as Record<string, unknown>;
}

function string(value: unknown, errorCode: string): string {
  if (typeof value !== "string") throw new Error(errorCode);
  return value;
}

function token(value: unknown, errorCode: string): string {
  const result = string(value, errorCode).trim();
  if (!result || result.length > 512) throw new Error(errorCode);
  return result;
}

function integer(value: unknown, errorCode: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new Error(errorCode);
  }
  return value;
}

function sha256(value: unknown): string {
  const result = token(value, "assistant_presentation_digest_invalid").toLowerCase();
  if (!/^sha256:[0-9a-f]{64}$/.test(result)) {
    throw new Error("assistant_presentation_digest_invalid");
  }
  return result;
}
