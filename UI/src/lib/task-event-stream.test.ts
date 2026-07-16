import { TaskSseParser } from "./task-event-stream.ts";

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
