import assert from "node:assert/strict";

import { followTaskEventStream, taskEventClosesLiveStream, TaskSseParser } from "./task-event-stream.ts";

const events: Array<{ seq?: number; event_kind: string }> = [];
const parser = new TaskSseParser((event) => events.push(event));
parser.push(": heart");
parser.push("beat\n\nid: 1\nevent: tool_started\ndata: {\"schema_version\":1,\"seq\":1,");
parser.push("\"task_id\":\"task-a\",\"event_kind\":\"tool_started\"}\n\n");
parser.push("data: {\"schema_version\":1,\"seq\":2,\"task_id\":\"task-a\",\"event_kind\":\"task_final\"}\n\n");
parser.finish();

if (events.length !== 2 || events[0]?.seq !== 1 || events[1]?.event_kind !== "task_final") {
  throw new Error("task SSE parser did not preserve chunked event ordering");
}

const encoder = new TextEncoder();
const requestPaths: string[] = [];
let requestCount = 0;
const receivedEvents: Array<{ seq?: number; event_kind: string }> = [];

const interruptedApiFetch = async (path: string): Promise<Response> => {
  requestPaths.push(path);
  requestCount += 1;
  if (requestCount === 1) {
    let pullCount = 0;
    return new Response(
      new ReadableStream<Uint8Array>({
        pull(controller) {
          if (pullCount === 0) {
            pullCount += 1;
            controller.enqueue(
              encoder.encode(
                'id: 1\ndata: {"schema_version":1,"seq":1,"task_id":"task-reconnect","event_kind":"tool_started"}\n\n',
              ),
            );
            return;
          }
          controller.error(new Error("network error"));
        },
      }),
      { status: 200 },
    );
  }
  return new Response(
    encoder.encode(
      'id: 2\ndata: {"schema_version":1,"seq":2,"task_id":"task-reconnect","event_kind":"task_final"}\n\n',
    ),
    { status: 200 },
  );
};

await followTaskEventStream(
  interruptedApiFetch,
  "task-reconnect",
  (event) => {
    receivedEvents.push(event);
  },
);

assert.deepEqual(
  receivedEvents.map((event) => [event.seq, event.event_kind]),
  [
    [1, "tool_started"],
    [2, "task_final"],
  ],
);
assert.equal(requestCount, 2);
assert.equal(requestPaths[0], "/v1/tasks/task-reconnect/events?cursor=0");
assert.equal(requestPaths[1], "/v1/tasks/task-reconnect/events?cursor=1");

const handlerOrder: string[] = [];
await followTaskEventStream(
  async () =>
    new Response(
      encoder.encode(
        [
          'data: {"schema_version":1,"seq":1,"task_id":"task-ordered","event_kind":"tool_started"}',
          "",
          'data: {"schema_version":1,"seq":2,"task_id":"task-ordered","event_kind":"task_final"}',
          "",
        ].join("\n"),
      ),
      { status: 200 },
    ),
  "task-ordered",
  async (event) => {
    if (event.seq === 1) {
      await new Promise((resolve) => globalThis.setTimeout(resolve, 10));
    }
    handlerOrder.push(`${event.seq}:${event.event_kind}`);
  },
);
assert.deepEqual(handlerOrder, ["1:tool_started", "2:task_final"]);

assert.equal(
  taskEventClosesLiveStream({
    schema_version: 1,
    task_id: "task-needs-user",
    event_kind: "task_state",
    payload: { execution_state: "needs_confirmation", lifecycle: { state: "needs_user" } },
  }),
  true,
);
assert.equal(
  taskEventClosesLiveStream({
    schema_version: 1,
    task_id: "task-background",
    event_kind: "task_state",
    payload: { execution_state: "background", lifecycle: { state: "background" } },
  }),
  false,
);
assert.equal(
  taskEventClosesLiveStream({
    schema_version: 1,
    task_id: "task-blocked",
    event_kind: "task_state",
    payload: { execution_state: "blocked", lifecycle: { state: "blocked" } },
  }),
  true,
);
