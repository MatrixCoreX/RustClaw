import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import type { ConversationInputRecord } from "../types/api";
import { useDeferredChatInputs } from "./useDeferredChatInputs";

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

function response(data: unknown): Response {
  return new Response(JSON.stringify({ ok: true, data }), {
    headers: { "content-type": "application/json" },
  });
}

function record(disposition: ConversationInputRecord["receipt"]["disposition"]): ConversationInputRecord {
  return {
    receipt: {
      schema_version: 1,
      input_id: "input-external-1",
      client_message_id: "message-external-1",
      input_seq: 1,
      preparation_state: "ready",
      disposition,
      target_task_id: null,
      decision_ref: null,
      instruction_revision: 0,
      execution_epoch: 0,
      accepted_at_ts: 10,
      updated_at_ts: 10,
      replayed: false,
    },
    content: [{ kind: "text", text: "Run this later." }],
    delivery_mode: "defer",
    source: {},
  };
}

test("deferred inputs reconcile across windows from conversation events", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let records: ConversationInputRecord[] = [];
  let eventStream: ReadableStreamDefaultController<Uint8Array> | null = null;
  let listRequests = 0;
  const errors: Array<string | null> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    if (path.startsWith("/v1/conversation-inputs?")) {
      listRequests += 1;
      return response({ schema_version: 1, items: records, next_after_input_seq: null });
    }
    if (path.startsWith("/v1/conversations/") && path.includes("/events?")) {
      return new Response(
        new ReadableStream<Uint8Array>({
          start(controller) {
            eventStream = controller;
            init?.signal?.addEventListener("abort", () => controller.close(), { once: true });
          },
        }),
        { headers: { "content-type": "text/event-stream" } },
      );
    }
    throw new Error(`unexpected_request:${path}`);
  };
  let runtime!: ReturnType<typeof useDeferredChatInputs>;
  function Probe() {
    runtime = useDeferredChatInputs({
      apiFetch,
      t: (zh) => zh,
      enabled: true,
      conversation: {
        id: "conversation-1",
        agentId: "main",
        externalChatId: "browser-1",
      },
      onError: (message) => errors.push(message),
      onActivated: () => undefined,
    });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => {
    renderer = create(React.createElement(Probe));
    await flush();
  });
  assert.deepEqual(runtime.items, []);
  assert.ok(eventStream);

  records = [record("deferred")];
  await act(async () => {
    eventStream!.enqueue(
      new TextEncoder().encode(
        `data: ${JSON.stringify({
          schema_version: 1,
          event_seq: 1,
          input_id: "input-external-1",
          event_kind: "accepted",
          payload: { disposition: "deferred" },
          created_at_ts: 10,
        })}\n\n`,
      ),
    );
    await flush();
  });
  assert.deepEqual(runtime.items.map((item) => item.inputId), ["input-external-1"]);

  records = [record("withdrawn")];
  await act(async () => {
    eventStream!.enqueue(
      new TextEncoder().encode(
        `data: ${JSON.stringify({
          schema_version: 1,
          event_seq: 2,
          input_id: "input-external-1",
          event_kind: "withdrawn",
          payload: { disposition: "withdrawn" },
          created_at_ts: 11,
        })}\n\n`,
      ),
    );
    await flush();
  });
  assert.deepEqual(runtime.items, []);
  assert.ok(listRequests >= 3);
  assert.deepEqual(errors, []);

  await act(async () => {
    renderer.unmount();
    await flush();
  });
});
