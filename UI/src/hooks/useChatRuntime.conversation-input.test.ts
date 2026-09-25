import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider } from "../components/UiDialogProvider";
import {
  conversationReplyMessage,
  useChatRuntime,
  type UseChatRuntimeParams,
} from "./useChatRuntime";
import { DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS } from "../lib/chat-attachments";
import type { TaskQueryResponse } from "../types/api";

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
const response = (data: unknown, status = 200) => new Response(JSON.stringify({ ok: true, data }), { status, headers: { "content-type": "application/json" } });

test("projects model-authored nonterminal replies without treating machine status as chat text", () => {
  const clarification = conversationReplyMessage({
    schema_version: 1,
    seq: 3,
    timestamp_ms: 1234,
    task_id: "task-1",
    event_kind: "conversation_reply_item",
    payload: {
      reply_id: "reply-1",
      relation: "clarification",
      lifecycle_stage: "accepted",
      text: "Which directory should I use?",
      terminal: false,
    },
  });
  assert.deepEqual(clarification, {
    id: "conversation-reply-1",
    role: "assistant",
    text: "Which directory should I use?",
    ts: 1234,
  });
  assert.equal(
    conversationReplyMessage({
      schema_version: 1,
      task_id: "task-1",
      event_kind: "conversation_reply_item",
      payload: {
        reply_id: "reply-2",
        relation: "control_status",
        lifecycle_stage: "stop_requested",
        message_key: "channel.control.cancel_requested",
        text: "",
        terminal: false,
      },
    }),
    null,
  );
});

async function setup(options: {
  loseFirstConversationInputResponse?: boolean;
  terminalRaceOnExpectedTarget?: boolean;
  holdConversationInputResponses?: boolean;
} = {}) {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requests: Array<{ payload: { text: string; conversation_id: string; agent_id: string; attachments?: Array<{ name: string; base64: string }> }; idempotency_key: string }> = [];
  const clientTasks: Array<{
    input: Record<string, unknown>;
    task: { payload: { attachments?: Array<{ name: string; base64: string }> } };
  }> = [];
  const conversationInputs: Array<{
    client_message_id: string;
    expected_task_id?: string;
    delivery_mode?: "auto" | "defer";
    content: Array<{ kind: string; text: string }>;
    scope: { conversation_id: string; agent_id: string; channel: string; channel_account_id: string };
  }> = [];
  const streams = new Map<string, ReadableStreamDefaultController<Uint8Array>>();
  const results = new Map<string, TaskQueryResponse>();
  const conversationTasks = new Map<string, string>();
  const conversationReceipts = new Map<string, Record<string, unknown>>();
  const conversationRecords = new Map<string, Record<string, unknown>>();
  const activatedInputs: string[] = [];
  const withdrawnInputs: string[] = [];
  const cancelRequests: Array<Record<string, unknown>> = [];
  let lostConversationInputResponse = false;
  let conversationInputRecoveryQueries = 0;
  let nextTaskSequence = 1;
  let releaseConversationInputResponses = () => undefined;
  const conversationInputResponseGate = new Promise<void>((resolve) => {
    releaseConversationInputResponses = resolve;
  });
  const params: UseChatRuntimeParams = {
    apiFetch: async (path, init) => {
      if (path.startsWith("/v1/tasks/conversation-history")) return response({ schema_version: 1, status: "ok", turns: [], truncated: false, content_sha256: createHash("sha256").update("[]").digest("hex") });
      if (path === "/v1/ui/attachment-constraints") return response(DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS);
      if (path === "/v1/conversation-inputs/cancel-current" && init?.method === "POST") {
        const request = JSON.parse(String(init.body)) as Record<string, unknown>;
        cancelRequests.push(request);
        return response({
          schema_version: 1,
          status: "cancel_requested",
          task_id: request.expected_task_id,
          canceled: 1,
        });
      }
      if (path === "/v1/tasks" && init?.method === "POST") {
        requests.push(JSON.parse(String(init.body)));
        const task_id = `task-${nextTaskSequence++}`;
        results.set(task_id, { task_id, status: "running", result_json: null, error_text: null });
        return response({ task_id });
      }
      const inputAction = path.match(/^\/v1\/conversation-inputs\/([^/]+)\/(activate|withdraw)$/);
      if (inputAction && init?.method === "POST") {
        const inputId = decodeURIComponent(inputAction[1]);
        const record = [...conversationRecords.values()].find(
          (candidate) => (candidate.receipt as Record<string, unknown>).input_id === inputId,
        );
        if (!record) {
          return new Response(JSON.stringify({ ok: false, error: "conversation_input_not_found" }), {
            status: 404,
            headers: { "content-type": "application/json" },
          });
        }
        const receipt = record.receipt as Record<string, unknown>;
        if (inputAction[2] === "withdraw") {
          withdrawnInputs.push(inputId);
          receipt.disposition = "withdrawn";
          receipt.updated_at_ts = 2;
          return response(receipt);
        }
        activatedInputs.push(inputId);
        const scope = record.scope as { conversation_id: string };
        let taskId = conversationTasks.get(scope.conversation_id);
        if (!taskId) {
          taskId = `task-${nextTaskSequence++}`;
          conversationTasks.set(scope.conversation_id, taskId);
          results.set(taskId, { task_id: taskId, status: "running", result_json: null, error_text: null });
        }
        receipt.disposition = "applied";
        receipt.target_task_id = taskId;
        receipt.instruction_revision = Number(receipt.instruction_revision) + 1;
        receipt.updated_at_ts = 2;
        return response(receipt);
      }
      if (
        (path === "/v1/conversation-inputs" || path === "/v1/conversation-inputs/client-task") &&
        init?.method === "POST"
      ) {
        const submitted = JSON.parse(String(init.body));
        const input = submitted.input ?? submitted;
        if (submitted.input) clientTasks.push(submitted);
        conversationInputs.push(input);
        if (options.terminalRaceOnExpectedTarget && input.expected_task_id) {
          const expiredTaskId = input.expected_task_id as string;
          results.set(expiredTaskId, {
            task_id: expiredTaskId,
            status: "succeeded",
            result_json: { text: `${expiredTaskId} reply` },
            error_text: null,
          });
          conversationTasks.delete(input.scope.conversation_id);
          return new Response(
            JSON.stringify({ ok: false, error: "conversation_input_target_conflict" }),
            { status: 409, headers: { "content-type": "application/json" } },
          );
        }
        let taskId = input.expected_task_id as string | undefined;
        const focusedTaskId = conversationTasks.get(input.scope.conversation_id);
        const deferred = input.delivery_mode === "defer";
        const adoptedExistingTask = !deferred && Boolean(taskId && !focusedTaskId);
        if (deferred) {
          taskId = undefined;
        } else if (!taskId) {
          taskId = focusedTaskId;
          if (!taskId) {
            taskId = `task-${nextTaskSequence++}`;
            conversationTasks.set(input.scope.conversation_id, taskId);
            results.set(taskId, { task_id: taskId, status: "running", result_json: null, error_text: null });
          }
        } else if (adoptedExistingTask) {
          conversationTasks.set(input.scope.conversation_id, taskId);
        }
        const receipt = {
          schema_version: 1,
          input_id: `input-${conversationInputs.length}`,
          client_message_id: input.client_message_id,
          input_seq: conversationInputs.length + 1,
          preparation_state: "ready",
          disposition: deferred
            ? "deferred"
            : adoptedExistingTask || !input.expected_task_id
              ? "applied"
              : "pending",
          target_task_id: taskId ?? null,
          instruction_revision: conversationInputs.length + 1,
          execution_epoch: 0,
          accepted_at_ts: 1,
          updated_at_ts: 1,
          replayed: false,
        };
        conversationReceipts.set(input.client_message_id, receipt);
        conversationRecords.set(input.client_message_id, {
          receipt,
          scope: input.scope,
          content: input.content,
          delivery_mode: input.delivery_mode ?? "auto",
          source: { received_at_ts: 1 },
        });
        if (options.loseFirstConversationInputResponse && !lostConversationInputResponse) {
          lostConversationInputResponse = true;
          throw new TypeError("simulated response loss");
        }
        if (options.holdConversationInputResponses) {
          await conversationInputResponseGate;
        }
        return response(
          submitted.input
            ? {
                schema_version: 1,
                input: receipt,
                handoff_state: adoptedExistingTask || focusedTaskId
                  ? "bound_existing_task"
                  : "task_created",
              }
            : receipt,
        );
      }
      if (path.startsWith("/v1/conversation-inputs?") && !init?.method) {
        const query = new URL(path, "http://runtime.invalid").searchParams;
        const clientMessageId = query.get("client_message_id");
        if (clientMessageId) {
          conversationInputRecoveryQueries += 1;
          const record = conversationRecords.get(clientMessageId);
          if (!record) {
            return new Response(JSON.stringify({ ok: false, error: "conversation_input_not_found" }), {
              status: 404,
              headers: { "content-type": "application/json" },
            });
          }
          return response({ schema_version: 1, items: [record], next_after_input_seq: null });
        }
        const records = [...conversationRecords.values()].filter((record) => {
          const scope = record.scope as Record<string, string>;
          return scope.conversation_id === query.get("conversation_id")
            && scope.agent_id === query.get("agent_id")
            && scope.channel === query.get("channel")
            && scope.channel_account_id === query.get("channel_account_id");
        });
        return response({ schema_version: 1, items: records, next_after_input_seq: null });
      }
      if (path.startsWith("/v1/conversations/") && path.includes("/events?")) {
        return new Response(new ReadableStream<Uint8Array>({
          start(controller) {
            init?.signal?.addEventListener("abort", () => controller.close(), { once: true });
          },
        }), { headers: { "content-type": "text/event-stream" } });
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
    runtime: () => runtime, requests, clientTasks, conversationInputs, activatedInputs,
    withdrawnInputs, cancelRequests, params,
    releaseConversationInputResponses,
    conversationInputRecoveryQueries: () => conversationInputRecoveryQueries,
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

test("messages submitted before the first receipt are durably bound without a browser FIFO", async () => {
  const app = await setup({ holdConversationInputResponses: true });
  try {
    await app.send("first while the task id is unknown");
    await app.send("second before the first receipt arrives");
    assert.equal(app.conversationInputs.length, 2);
    assert.equal(app.conversationInputs[0].expected_task_id, undefined);
    assert.equal(app.conversationInputs[1].expected_task_id, undefined);

    await act(async () => {
      app.releaseConversationInputResponses();
      await flush();
      await flush();
    });
    assert.equal(app.runtime().chatTeachingRuns.length, 2);
    assert.deepEqual(
      new Set(app.runtime().chatTeachingRuns.map((run) => run.taskId)),
      new Set(["task-1"]),
    );
    await app.complete("task-1");
  } finally {
    app.releaseConversationInputResponses();
    await app.unmount();
  }
});

test("busy text sends steer the active task immediately and preserve per-input teaching IDs", async () => {
  const app = await setup();
  try {
    await act(async () => app.runtime().setChatTeachingMode(true));
    await app.send("first");
    await app.send("second");
    await app.send("third");
    assert.equal(app.requests.length, 0);
    assert.deepEqual(app.conversationInputs.map(item => item.content[0].text), ["first", "second", "third"]);
    assert.equal(app.runtime().chatInput, "");
    await act(async () => app.runtime().setChatInput("new unsent draft"));
    await app.complete("task-1");
    assert.deepEqual(app.requests, []);
    assert.equal(app.runtime().chatInput, "new unsent draft");
    assert.deepEqual(app.runtime().chatMessages.filter(item => item.role !== "system").map(item => item.text),
      ["first", "second", "third", "task-1 reply"]);
    assert.equal(app.runtime().chatTeachingRuns.length, 3);
    assert.deepEqual(new Set(app.runtime().chatTeachingRuns.map(item => item.taskId)), new Set(["task-1"]));
    assert.deepEqual(
      app.runtime().chatTeachingRuns.map(item => item.conversationInputId),
      ["input-3", "input-2", "input-1"],
    );
    assert.deepEqual(
      app.runtime().chatTeachingRuns.map(item => item.conversationInputRevision),
      [4, 3, 2],
    );
    assert.equal(new Set(app.conversationInputs.map(item => item.client_message_id)).size, 3);
    assert.equal(app.runtime().chatSending, false);
  } finally { await app.unmount(); }
});

test("stops the authenticated current conversation without submitting natural-language control", async () => {
  const app = await setup();
  try {
    await app.send("start a long task");
    assert.equal(app.runtime().chatCanStop, true);

    await act(async () => {
      await app.runtime().stopActiveChatTask();
      await flush();
    });

    assert.equal(app.cancelRequests.length, 1);
    assert.equal(app.cancelRequests[0].expected_task_id, "task-1");
    assert.deepEqual(app.cancelRequests[0].scope, app.conversationInputs[0].scope);
    assert.equal(app.conversationInputs.length, 1);
    assert.equal(app.runtime().chatActivity.stage, "stopping");
  } finally {
    await app.unmount();
  }
});

test("a lost conversation-input response is recovered by client message id without duplication", async () => {
  const app = await setup({ loseFirstConversationInputResponse: true });
  try {
    await app.send("persist before the response disappears");
    assert.equal(app.conversationInputs.length, 1);
    assert.equal(app.conversationInputRecoveryQueries(), 1);
    assert.equal(app.runtime().chatTeachingRuns[0].conversationInputId, "input-1");
    assert.equal(app.runtime().chatTeachingRuns[0].taskId, "task-1");
    await app.send("apply this to the recovered active task");
    assert.equal(app.conversationInputs.length, 2);
    assert.equal(app.conversationInputs[1].expected_task_id, "task-1");
    await app.complete("task-1");
  } finally {
    await app.unmount();
  }
});

test("a follow-up sent across a terminal boundary starts and observes a new task", async () => {
  const app = await setup({ terminalRaceOnExpectedTarget: true });
  try {
    await app.send("first");
    await app.send("arrived as the first task completed");
    const successfulFollowup = app.conversationInputs.find(
      item => item.content[0].text === "arrived as the first task completed" && !item.expected_task_id,
    );
    assert.ok(successfulFollowup);
    assert.equal(app.runtime().chatTeachingRuns[0].taskId, "task-2");
    assert.equal(app.runtime().chatTeachingRuns[0].conversationInputId, "input-4");
    await app.complete("task-2");
    assert.deepEqual(
      app.runtime().chatMessages.filter(item => item.role !== "system").map(item => item.text),
      ["first", "arrived as the first task completed", "task-2 reply"],
    );
  } finally {
    await app.unmount();
  }
});

test("conversation changes keep active-task inputs scoped and do not block a new conversation", async () => {
  const app = await setup();
  try {
    const firstThread = app.runtime().activeChatThreadId;
    await app.send("A1");
    await app.send("A2");
    await act(async () => app.runtime().createNewChatThread());
    const otherThread = app.runtime().activeChatThreadId;
    assert.notEqual(otherThread, firstThread);
    assert.equal(app.runtime().chatSending, false);
    await app.send("B1");
    assert.equal(app.requests.length, 0);
    assert.equal(app.conversationInputs[0].scope.conversation_id, firstThread);
    assert.equal(app.conversationInputs[1].scope.conversation_id, firstThread);
    await app.complete("task-1");
    assert.equal(app.conversationInputs[2].scope.conversation_id, otherThread);
    await app.complete("task-2");
    assert.deepEqual(app.runtime().chatMessages.filter(item => item.role !== "system").map(item => item.text), ["B1", "task-2 reply"]);
    await act(async () => app.runtime().selectChatThread(firstThread));
    assert.deepEqual(app.runtime().chatMessages.filter(item => item.role !== "system").map(item => item.text), ["A1", "A2", "task-1 reply"]);
  } finally { await app.unmount(); }
});

test("failed active task settles all already accepted follow-up inputs without a client queue", async () => {
  const app = await setup();
  try {
    await app.send("first");
    await app.send("remove me");
    await app.send("next");
    await app.complete("task-1", "failed");
    assert.equal(app.requests.length, 0);
    assert.equal(app.conversationInputs.length, 3);
    assert.ok(app.runtime().chatTeachingRuns.every(run => run.status === "failed"));
  } finally { await app.unmount(); }
});

test("account switch does not replay an input already accepted by the server", async () => {
  const app = await setup();
  try {
    await app.send("first");
    await app.send("never submit after logout");
    await app.changeScope();
    assert.equal(app.requests.length, 0);
    assert.equal(app.conversationInputs.length, 2);
    assert.equal(app.runtime().chatSending, false);
  } finally { await app.unmount(); }
});

test("IME confirmation does not send, while ordinary Enter steers the active task", async () => {
  const app = await setup();
  try {
    await app.send("first");
    await act(async () => app.runtime().setChatInput("输入中文"));
    await act(async () => app.runtime().handleChatInputKeyDown({ key: "Enter", shiftKey: false,
      nativeEvent: { isComposing: true }, preventDefault: () => assert.fail("IME must not be interrupted") } as never));
    await act(async () => app.runtime().handleChatInputKeyDown({ key: "Enter", shiftKey: false,
      nativeEvent: { isComposing: false }, preventDefault: () => undefined } as never));
    assert.equal(app.conversationInputs[1].content[0].text, "输入中文");
    await app.complete("task-1");
  } finally { await app.unmount(); }
});

test("active-task attachments submit immediately and do not erase the next draft's attachments", async () => {
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
    assert.equal(app.requests.length, 0);
    assert.equal(app.conversationInputs.length, 2);
    assert.equal(app.clientTasks[1].task.payload.attachments?.[0].name, "old.txt");
    assert.equal(
      app.clientTasks[1].task.payload.attachments?.[0].base64,
      "data:text/plain;base64,b2xk",
    );
    assert.equal(app.runtime().chatAttachments[0].name, "new.txt");
    await app.complete("task-1");
  } finally {
    await app.unmount();
    if (original) Object.defineProperty(globalThis, "FileReader", original);
    else Reflect.deleteProperty(globalThis, "FileReader");
  }
});

test("an attachment task is adopted before later text steers the same task", async () => {
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
    await act(async () => {
      await app.runtime().handleChatAttachmentSelection([
        new File(["report"], "report.txt", { type: "text/plain" }),
      ] as unknown as FileList);
    });
    await app.send("inspect this attachment");
    assert.equal(app.requests.length, 0);
    assert.equal(app.conversationInputs.length, 1);
    assert.equal(app.conversationInputs[0].expected_task_id, undefined);

    await app.send("focus on the unresolved findings");
    assert.equal(app.requests.length, 0);
    assert.equal(app.conversationInputs.length, 2);
    assert.equal(app.conversationInputs[1].expected_task_id, "task-1");
    await app.complete("task-1");
  } finally {
    await app.unmount();
    if (original) Object.defineProperty(globalThis, "FileReader", original);
    else Reflect.deleteProperty(globalThis, "FileReader");
  }
});

test("deferred messages remain durable without starting work and can later run or withdraw", async () => {
  const app = await setup();
  try {
    await act(async () => app.runtime().setChatDeliveryMode("defer"));
    await app.send("handle this after the current work");
    assert.equal(app.requests.length, 0);
    assert.equal(app.conversationInputs.length, 1);
    assert.equal(app.conversationInputs[0].delivery_mode, "defer");
    assert.equal(app.runtime().chatDeliveryMode, "auto");
    assert.deepEqual(app.runtime().chatDeferredInputs.map(item => item.text), [
      "handle this after the current work",
    ]);

    await act(async () => {
      await app.runtime().activateDeferredChatInput(app.runtime().chatDeferredInputs[0].inputId);
      await flush();
    });
    assert.deepEqual(app.activatedInputs, ["input-1"]);
    assert.deepEqual(app.runtime().chatDeferredInputs, []);
    assert.equal(app.runtime().chatTeachingRuns[0].taskId, "task-1");

    await act(async () => app.runtime().setChatDeliveryMode("defer"));
    await app.send("discard this deferred message");
    const withdrawnId = app.runtime().chatDeferredInputs[0].inputId;
    await act(async () => {
      await app.runtime().withdrawDeferredChatInput(withdrawnId);
      await flush();
    });
    assert.deepEqual(app.withdrawnInputs, [withdrawnId]);
    assert.deepEqual(app.runtime().chatDeferredInputs, []);
    await app.complete("task-1");
  } finally {
    await app.unmount();
  }
});
