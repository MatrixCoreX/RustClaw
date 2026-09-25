import assert from "node:assert/strict";
import test from "node:test";

import {
  ConversationInputSseParser,
  followConversationInputEventStream,
} from "./conversation-input-event-stream";
import type { ConversationInputEventRecord } from "../types/api";

function event(eventSeq: number): ConversationInputEventRecord {
  return {
    schema_version: 1,
    event_seq: eventSeq,
    input_id: `input-${eventSeq}`,
    event_kind: "accepted",
    payload: { disposition: "deferred" },
    created_at_ts: eventSeq,
  };
}

test("conversation input SSE parser accepts fragmented records", () => {
  const observed: ConversationInputEventRecord[] = [];
  const parser = new ConversationInputSseParser((value) => observed.push(value));
  const encoded = `id: 1\nevent: accepted\ndata: ${JSON.stringify(event(1))}\n\n`;

  parser.push(encoded.slice(0, 17));
  parser.push(encoded.slice(17));
  parser.finish();

  assert.deepEqual(observed, [event(1)]);
});

test("conversation input stream reconnects from the durable cursor and ignores duplicates", async () => {
  const requests: Array<{ path: string; lastEventId: string | null }> = [];
  const controller = new AbortController();
  let responseIndex = 0;
  const apiFetch = async (path: string, init?: RequestInit) => {
    requests.push({
      path,
      lastEventId: new Headers(init?.headers).get("Last-Event-ID"),
    });
    responseIndex += 1;
    const records = responseIndex === 1 ? [event(1)] : [event(1), event(2)];
    return new Response(
      new ReadableStream<Uint8Array>({
        start(stream) {
          const encoder = new TextEncoder();
          for (const record of records) {
            stream.enqueue(encoder.encode(`data: ${JSON.stringify(record)}\n\n`));
          }
          stream.close();
        },
      }),
      { headers: { "content-type": "text/event-stream" } },
    );
  };
  const observed: number[] = [];

  await followConversationInputEventStream(
    apiFetch,
    {
      conversationId: "conversation-1",
      agentId: "main",
      channel: "ui",
      channelAccountId: "browser-1",
    },
    (record) => {
      observed.push(record.event_seq);
      if (record.event_seq === 2) controller.abort();
    },
    controller.signal,
  );

  assert.deepEqual(observed, [1, 2]);
  assert.equal(requests.length, 2);
  assert.equal(requests[0].lastEventId, "0");
  assert.match(requests[1].path, /cursor=1/);
  assert.equal(requests[1].lastEventId, "1");
});
