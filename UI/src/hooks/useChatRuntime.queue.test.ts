import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider } from "../components/UiDialogProvider";
import { useChatRuntime, type UseChatRuntimeParams } from "./useChatRuntime";
import { DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS } from "../lib/chat-attachments";
import type { TaskQueryResponse } from "../types/api";

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
const response = (data: unknown) => new Response(JSON.stringify({ ok: true, data }), { headers: { "content-type": "application/json" } });

async function setup() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requests: Array<{ payload: { text: string; conversation_id: string; agent_id: string; attachments?: Array<{ name: string; base64: string }> }; idempotency_key: string }> = [];
  const streams = new Map<string, ReadableStreamDefaultController<Uint8Array>>();
  const results = new Map<string, TaskQueryResponse>();
  const params: UseChatRuntimeParams = {
    apiFetch: async (path, init) => {
      if (path.startsWith("/v1/tasks/conversation-history")) return response({ schema_version: 1, status: "ok", turns: [], truncated: false, content_sha256: createHash("sha256").update("[]").digest("hex") });
      if (path === "/v1/ui/attachment-constraints") return response(DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS);
      if (path === "/v1/tasks" && init?.method === "POST") {
        requests.push(JSON.parse(String(init.body)));
        const task_id = `task-${requests.length}`;
        results.set(task_id, { task_id, status: "running", result_json: null, error_text: null });
        return response({ task_id });
      }
      const taskId = path.match(/^\/v1\/tasks\/([^/]+)\/events/)?.[1];
      if (taskId) return new Response(new ReadableStream<Uint8Array>({
        start(controller) {
          streams.set(taskId, controller);
          init?.signal?.addEventListener("abort", () => controller.close(), { once: true });
        },
      }), { headers: { "content-type": "text/event-stream" } });
      if (path.startsWith("/v1/debug/tasks/")) return response({ calls: [], call_count: 0 });
      throw new Error(`Unexpected test request: ${path}`);
    },
    t: zh => zh, lang: "zh", interactionAdapter: "", interactionChannel: "ui",
    activeUserKey: "test-user", activeIdentityIds: {}, conversationHistoryScope: "key:1:2",
    interactionExternalUserId: "", interactionExternalChatId: "",
    availableAgents: [{ id: "main", name: "Main" }], defaultAgentId: "main",
    fetchTaskById: async id => results.get(id)!,
    onTaskSubmitted: () => undefined, onTaskResult: () => undefined,
  };
  let runtime!: ReturnType<typeof useChatRuntime>;
  function Probe() { runtime = useChatRuntime(params); return null; }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(UiDialogProvider, null, React.createElement(Probe))); await flush(); });
  await act(flush);
  return {
    runtime: () => runtime, requests, params,
    send: async (text: string) => {
      await act(async () => { runtime.setChatInput(text); await runtime.sendChatMessage(); await flush(); });
      await act(flush);
    },
    complete: async (id: string, status: TaskQueryResponse["status"] = "succeeded") => {
      await act(async () => {
        results.set(id, { task_id: id, status, result_json: { text: `${id} reply` }, error_text: status === "failed" ? "test failure" : null });
        const stream = streams.get(id)!;
        stream.enqueue(new TextEncoder().encode(`data: ${JSON.stringify({ schema_version: 1, task_id: id, seq: 1, event_kind: "task_final", payload: { status } })}\n\n`));
        stream.close();
        await flush();
      });
      await act(flush);
    },
    changeScope: async () => {
      params.conversationHistoryScope = "key:3:4";
      await act(async () => { renderer.update(React.createElement(UiDialogProvider, null, React.createElement(Probe))); await flush(); });
    },
    unmount: async () => { await act(async () => { renderer.unmount(); await flush(); }); },
  };
}

test("busy send queues immediately, drains FIFO, preserves a newer draft and per-turn teaching IDs", async () => {
  const app = await setup();
  try {
    await act(async () => app.runtime().setChatTeachingMode(true));
    await app.send("first");
    await app.send("second");
    await app.send("third");
    assert.equal(app.requests.length, 1);
    assert.deepEqual(app.runtime().chatQueuedMessages.map(item => item.text), ["second", "third"]);
    assert.equal(app.runtime().chatInput, "");
    await act(async () => app.runtime().setChatInput("new unsent draft"));
    await app.complete("task-1");
    assert.deepEqual(app.requests.map(item => item.payload.text), ["first", "second"]);
    assert.equal(app.runtime().chatInput, "new unsent draft");
    await app.complete("task-2");
    await app.complete("task-3");
    assert.deepEqual(app.runtime().chatMessages.filter(item => item.role !== "system").map(item => item.text),
      ["first", "task-1 reply", "second", "task-2 reply", "third", "task-3 reply"]);
    assert.equal(app.runtime().chatTeachingRuns.length, 3);
    assert.equal(new Set(app.runtime().chatTeachingRuns.map(item => item.taskId)).size, 3);
    assert.equal(new Set(app.requests.map(item => item.idempotency_key)).size, 3);
    assert.equal(app.runtime().chatSending, false);
  } finally { await app.unmount(); }
});

test("conversation changes keep queued turns with their original owner and do not block a new conversation", async () => {
  const app = await setup();
  try {
    const firstThread = app.runtime().activeChatThreadId;
    await app.send("A1");
    await app.send("A2");
    await act(async () => app.runtime().createNewChatThread());
    const otherThread = app.runtime().activeChatThreadId;
    assert.notEqual(otherThread, firstThread);
    assert.equal(app.runtime().chatSending, false);
    assert.deepEqual(app.runtime().chatQueuedMessages, []);
    await app.send("B1");
    assert.equal(app.requests.length, 2);
    await app.complete("task-1");
    assert.equal(app.requests[2].payload.conversation_id, firstThread);
    assert.equal(app.requests[1].payload.conversation_id, otherThread);
    await app.complete("task-3");
    await app.complete("task-2");
    assert.deepEqual(app.runtime().chatMessages.filter(item => item.role !== "system").map(item => item.text), ["B1", "task-2 reply"]);
    await act(async () => app.runtime().selectChatThread(firstThread));
    assert.deepEqual(app.runtime().chatMessages.filter(item => item.role !== "system").map(item => item.text), ["A1", "task-1 reply", "A2", "task-3 reply"]);
  } finally { await app.unmount(); }
});

test("remove pending message and pause on failed predecessor; continuation is explicit", async () => {
  const app = await setup();
  try {
    await app.send("first");
    await app.send("remove me");
    await app.send("next");
    await act(async () => app.runtime().removeQueuedChatMessage(app.runtime().chatQueuedMessages[0].id));
    await app.complete("task-1", "failed");
    assert.equal(app.requests.length, 1);
    assert.equal(app.runtime().chatQueuePaused, true);
    await act(async () => { app.runtime().resumeChatQueue(); await flush(); });
    assert.equal(app.requests[1].payload.text, "next");
    await app.complete("task-2");
  } finally { await app.unmount(); }
});

test("account switch drops unsubmitted messages and does not replay them with the new credential", async () => {
  const app = await setup();
  try {
    await app.send("first");
    await app.send("never submit after logout");
    await app.changeScope();
    assert.equal(app.runtime().chatQueuedMessages.length, 0);
    assert.equal(app.requests.length, 1);
    assert.equal(app.runtime().chatSending, false);
  } finally { await app.unmount(); }
});

test("IME confirmation does not send, while ordinary Enter queues during execution", async () => {
  const app = await setup();
  try {
    await app.send("first");
    await act(async () => app.runtime().setChatInput("输入中文"));
    await act(async () => app.runtime().handleChatInputKeyDown({ key: "Enter", shiftKey: false,
      nativeEvent: { isComposing: true }, preventDefault: () => assert.fail("IME must not be interrupted") } as never));
    assert.equal(app.runtime().chatQueuedMessages.length, 0);
    await act(async () => app.runtime().handleChatInputKeyDown({ key: "Enter", shiftKey: false,
      nativeEvent: { isComposing: false }, preventDefault: () => undefined } as never));
    assert.equal(app.runtime().chatQueuedMessages[0].text, "输入中文");
    await app.complete("task-1");
    await app.complete("task-2");
  } finally { await app.unmount(); }
});

test("queued attachments are snapshotted and do not erase the next draft's attachments", async () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "FileReader");
  class TestReader {
    result = "";
    onload?: () => void;
    async readAsDataURL(file: File) {
      this.result = `data:${file.type};base64,${Buffer.from(await file.arrayBuffer()).toString("base64")}`;
      this.onload?.();
    }
  }
  Object.defineProperty(globalThis, "FileReader", { configurable: true, value: TestReader });
  const app = await setup();
  try {
    await app.send("first");
    await act(async () => { await app.runtime().handleChatAttachmentSelection([new File(["old"], "old.txt", { type: "text/plain" })] as unknown as FileList); });
    await app.send("attachment follow-up");
    await act(async () => { await app.runtime().handleChatAttachmentSelection([new File(["new"], "new.txt", { type: "text/plain" })] as unknown as FileList); });
    await app.complete("task-1");
    assert.equal(app.requests[1].payload.attachments?.[0].name, "old.txt");
    assert.equal(app.requests[1].payload.attachments?.[0].base64, "data:text/plain;base64,b2xk");
    assert.equal(app.runtime().chatAttachments[0].name, "new.txt");
    await app.complete("task-2");
  } finally {
    await app.unmount();
    if (original) Object.defineProperty(globalThis, "FileReader", original);
    else Reflect.deleteProperty(globalThis, "FileReader");
  }
});
