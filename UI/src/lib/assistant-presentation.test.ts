import assert from "node:assert/strict";
import { createHash } from "node:crypto";

import type { TaskEventEnvelope } from "../types/api.ts";
import {
  AssistantPresentationReducer,
  decodeAssistantPresentationEvent,
} from "./assistant-presentation.ts";

function envelope(
  eventKind: string,
  sequence: number,
  offset: number,
  extra: Record<string, unknown> = {},
): TaskEventEnvelope {
  return {
    schema_version: 1,
    task_id: "task-1",
    event_kind: eventKind,
    payload: {
      schema_version: 1,
      task_id: "task-1",
      conversation_id: "conversation-1",
      turn_id: "turn-1",
      stream_id: "stream-1",
      attempt_id: "attempt-1",
      sequence,
      content_offset_bytes: offset,
      created_at: 10,
      ...extra,
    },
  };
}

const reducer = new AssistantPresentationReducer();
const started = decodeAssistantPresentationEvent(
  envelope("assistant_output_started", 0, 0),
);
assert.ok(started);
await reducer.apply(started);
const first = decodeAssistantPresentationEvent(
  envelope("assistant_output_delta", 1, 0, { content: "你" }),
);
assert.ok(first);
assert.equal((await reducer.apply(first))?.contentBytes, 3);
assert.equal(
  (await reducer.apply(first))?.content,
  "你",
  "identical duplicate must be idempotent",
);
const second = decodeAssistantPresentationEvent(
  envelope("assistant_output_delta", 2, 3, { content: "好" }),
);
assert.ok(second);
assert.equal((await reducer.apply(second))?.content, "你好");
const completed = decodeAssistantPresentationEvent(
  envelope("assistant_output_completed", 3, 6, {
    total_content_bytes: 6,
    content_sha256: digest("你好"),
  }),
);
assert.ok(completed);
assert.equal((await reducer.apply(completed))?.status, "completed");

assert.throws(
  () =>
    decodeAssistantPresentationEvent({
      ...envelope("assistant_output_delta", 1, 0, { content: "x" }),
      task_id: "other-task",
    }),
  /assistant_presentation_identity_mismatch/,
);

const gapReducer = new AssistantPresentationReducer();
const gapStart = decodeAssistantPresentationEvent(envelope("assistant_output_started", 0, 0));
const gapDelta = decodeAssistantPresentationEvent(
  envelope("assistant_output_delta", 2, 0, { content: "x" }),
);
assert.ok(gapStart && gapDelta);
await gapReducer.apply(gapStart);
await assert.rejects(
  () => gapReducer.apply(gapDelta),
  /assistant_presentation_sequence_gap/,
);

const offsetReducer = new AssistantPresentationReducer();
const offsetStart = decodeAssistantPresentationEvent(envelope("assistant_output_started", 0, 0));
const badOffset = decodeAssistantPresentationEvent(
  envelope("assistant_output_delta", 1, 2, { content: "x" }),
);
assert.ok(offsetStart && badOffset);
await offsetReducer.apply(offsetStart);
await assert.rejects(
  () => offsetReducer.apply(badOffset),
  /assistant_presentation_offset_mismatch/,
);

const conflictReducer = new AssistantPresentationReducer();
const conflictStart = decodeAssistantPresentationEvent(envelope("assistant_output_started", 0, 0));
const original = decodeAssistantPresentationEvent(
  envelope("assistant_output_delta", 1, 0, { content: "x" }),
);
const conflict = decodeAssistantPresentationEvent(
  envelope("assistant_output_delta", 1, 0, { content: "y" }),
);
assert.ok(conflictStart && original && conflict);
await conflictReducer.apply(conflictStart);
await conflictReducer.apply(original);
await assert.rejects(
  () => conflictReducer.apply(conflict),
  /assistant_presentation_duplicate_conflict/,
);

const abortReducer = new AssistantPresentationReducer();
const abortStart = decodeAssistantPresentationEvent(envelope("assistant_output_started", 0, 0));
const aborted = decodeAssistantPresentationEvent(
  envelope("assistant_output_aborted", 1, 0, {
    error_code: "answer_retried",
    message_key: "assistant.output.retried",
    retryable: true,
  }),
);
assert.ok(abortStart && aborted);
await abortReducer.apply(abortStart);
assert.equal((await abortReducer.apply(aborted))?.status, "aborted");
assert.equal(abortReducer.get("stream-1")?.errorCode, "answer_retried");

const replacement = decodeAssistantPresentationEvent(
  envelope("assistant_output_replaced", 2, 0, {
    old_stream_id: "stream-1",
    new_stream_id: "stream-2",
  }),
);
assert.ok(replacement);
assert.equal((await abortReducer.apply(replacement))?.status, "replaced");

const sizeReducer = new AssistantPresentationReducer();
const sizeStart = decodeAssistantPresentationEvent(envelope("assistant_output_started", 0, 0));
const sizeCompleted = decodeAssistantPresentationEvent(
  envelope("assistant_output_completed", 1, 0, {
    total_content_bytes: 1,
    content_sha256: `sha256:${"b".repeat(64)}`,
  }),
);
assert.ok(sizeStart && sizeCompleted);
await sizeReducer.apply(sizeStart);
await assert.rejects(
  () => sizeReducer.apply(sizeCompleted),
  /assistant_presentation_completion_size_mismatch/,
);

const digestReducer = new AssistantPresentationReducer();
const digestStart = decodeAssistantPresentationEvent(envelope("assistant_output_started", 0, 0));
const digestDelta = decodeAssistantPresentationEvent(
  envelope("assistant_output_delta", 1, 0, { content: "answer" }),
);
const badDigest = decodeAssistantPresentationEvent(
  envelope("assistant_output_completed", 2, 6, {
    total_content_bytes: 6,
    content_sha256: `sha256:${"b".repeat(64)}`,
  }),
);
assert.ok(digestStart && digestDelta && badDigest);
await digestReducer.apply(digestStart);
await digestReducer.apply(digestDelta);
await assert.rejects(
  () => digestReducer.apply(badDigest),
  /assistant_presentation_digest_mismatch/,
);

const cryptoDescriptor = Object.getOwnPropertyDescriptor(globalThis, "crypto");
Object.defineProperty(globalThis, "crypto", { configurable: true, value: undefined });
try {
  const httpReducer = new AssistantPresentationReducer();
  const httpContent = "局域网 HTTP 回复";
  const httpBytes = new TextEncoder().encode(httpContent).byteLength;
  const httpStart = decodeAssistantPresentationEvent(
    envelope("assistant_output_started", 0, 0),
  );
  const httpDelta = decodeAssistantPresentationEvent(
    envelope("assistant_output_delta", 1, 0, { content: httpContent }),
  );
  const httpCompleted = decodeAssistantPresentationEvent(
    envelope("assistant_output_completed", 2, httpBytes, {
      total_content_bytes: httpBytes,
      content_sha256: digest(httpContent),
    }),
  );
  assert.ok(httpStart && httpDelta && httpCompleted);
  await httpReducer.apply(httpStart);
  await httpReducer.apply(httpDelta);
  assert.equal((await httpReducer.apply(httpCompleted))?.status, "completed");
} finally {
  if (cryptoDescriptor) Object.defineProperty(globalThis, "crypto", cryptoDescriptor);
  else delete (globalThis as { crypto?: Crypto }).crypto;
}

function digest(content: string): string {
  return `sha256:${createHash("sha256").update(content).digest("hex")}`;
}
