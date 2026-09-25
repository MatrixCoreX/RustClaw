import assert from "node:assert/strict";
import test from "node:test";

import {
  loadChatThreadState,
  mergeServerConversationHistory,
  persistChatThreadState,
  retainLocalDraftsForPagedRestore,
  threadHasPendingTask,
  type ChatThreadRecord,
  type ChatThreadState,
} from "./useChatRuntime";
import type { ServerChatThreadProjection } from "../lib/chat-history";
import { projectConversationHistory } from "../lib/chat-history";

const t = (zh: string, _en: string) => zh;

function thread(id: string, status: "queued" | "running" | "succeeded"): ChatThreadRecord {
  return {
    id,
    agentId: "main",
    title: id,
    messages: [{ id: `u-${id}`, role: "user", text: id, ts: 1 }],
    input: id === "draft" ? "尚未发送的内容" : "",
    createdAt: 1,
    updatedAt: 2,
    teachingMode: false,
    externalChatId: `ui-${id}`,
    lastTaskId: status === "succeeded" ? `task-${id}` : null,
    activeTeachingRunId: `run-${id}`,
    teachingRuns: [
      {
        id: `run-${id}`,
        taskId: status === "succeeded" ? `task-${id}` : `pending-${id}`,
        conversationInputId: status === "running" ? `input-${id}` : null,
        conversationInputClientMessageId:
          status === "running" ? `ui:${id}:message` : null,
        conversationInputRevision: status === "running" ? 3 : null,
        userMessageId: `u-${id}`,
        userText: id,
        status,
        startedAt: 1,
      },
    ],
  };
}

test("identity cache restores every local thread and the selected conversation", () => {
  const values = new Map<string, string>();
  const previousWindow = globalThis.window;
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      localStorage: {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => values.set(key, value),
      },
    },
  });
  try {
    const state: ChatThreadState = {
      activeThreadId: "running",
      threads: [thread("draft", "queued"), thread("running", "running")],
    };
    persistChatThreadState(state, "key:42:7");
    const restored = loadChatThreadState(t, "key:42:7");

    assert.equal(restored.activeThreadId, "running");
    assert.deepEqual(restored.threads.map((item) => item.id), ["draft", "running"]);
    assert.equal(restored.threads[0].input, "尚未发送的内容");
    assert.ok(threadHasPendingTask(restored.threads[1]));
    assert.equal(restored.threads[1].teachingRuns?.[0].conversationInputId, "input-running");
    assert.equal(restored.threads[1].teachingRuns?.[0].conversationInputRevision, 3);
  } finally {
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: previousWindow,
    });
  }
});

test("server restore keeps local drafts, active selection, and unfinished task recovery", () => {
  const localDraft = thread("draft", "queued");
  localDraft.teachingRuns = [];
  localDraft.lastTaskId = null;
  const running = thread("server-thread", "running");
  const current: ChatThreadState = {
    activeThreadId: "server-thread",
    threads: [localDraft, running],
  };
  const server: ServerChatThreadProjection[] = [
    {
      id: "server-thread",
      externalChatId: "ui-server-thread",
      title: "服务端会话",
      messages: [
        { id: "u-task-server", role: "user", text: "继续", ts: 10 },
      ],
      createdAt: 10,
      updatedAt: 11,
      lastTaskId: "task-server",
      teachingRuns: [
        {
          id: "teach-task-server",
          taskId: "task-server",
          userMessageId: "u-task-server",
          assistantMessageId: null,
          userText: "继续",
          assistantText: null,
          status: "running",
          startedAt: 10,
          completedAt: null,
          taskResult: {
            task_id: "task-server",
            status: "running",
            result_json: null,
            error_text: null,
          },
        },
      ],
    },
  ];

  const merged = mergeServerConversationHistory(current, server, t);

  assert.equal(merged.activeThreadId, "server-thread");
  assert.deepEqual(merged.threads.map((item) => item.id), ["server-thread", "draft"]);
  assert.equal(merged.threads[1].input, "尚未发送的内容");
  assert.ok(threadHasPendingTask(merged.threads[0]));
});

test("incremental server pages merge older turns without replacing newer turns", () => {
  const existing = thread("server-thread", "succeeded");
  existing.messages = [
    { id: "u-new", role: "user", text: "new", ts: 20 },
    { id: "a-new", role: "assistant", text: "new answer", ts: 21 },
  ];
  existing.updatedAt = 21;
  existing.teachingRuns = [];
  const current: ChatThreadState = {
    activeThreadId: existing.id,
    threads: [existing],
  };
  const older: ServerChatThreadProjection = {
    id: existing.id,
    externalChatId: existing.externalChatId,
    title: "first question",
    messages: [
      { id: "u-old", role: "user", text: "old", ts: 10 },
      { id: "a-old", role: "assistant", text: "old answer", ts: 11 },
    ],
    createdAt: 10,
    updatedAt: 11,
    lastTaskId: "task-old",
    teachingRuns: [],
  };

  const merged = mergeServerConversationHistory(current, [older], t);

  assert.deepEqual(
    merged.threads[0].messages.map((message) => message.id),
    ["u-old", "a-old", "u-new", "a-new"],
  );
  assert.equal(merged.threads[0].lastTaskId, "task-server-thread");
});

test("server restore replaces local optimistic messages for the same task", () => {
  const existing = thread("same-task", "succeeded");
  existing.messages = [
    { id: "u-local-time", role: "user", text: "question", ts: 10 },
    { id: "a-local-time", role: "assistant", text: "answer", ts: 11 },
  ];
  existing.teachingRuns = [
    {
      id: "teach-local",
      taskId: "task-same-task",
      userMessageId: "u-local-time",
      assistantMessageId: "a-local-time",
      userText: "question",
      assistantText: "answer",
      status: "succeeded",
      startedAt: 10,
      completedAt: 11,
    },
  ];
  const server: ServerChatThreadProjection = {
    id: existing.id,
    externalChatId: existing.externalChatId,
    title: existing.title,
    messages: [
      { id: "u-task-same-task", role: "user", text: "question", ts: 10 },
      { id: "a-task-same-task", role: "assistant", text: "answer", ts: 11 },
    ],
    createdAt: 10,
    updatedAt: 11,
    lastTaskId: "task-same-task",
    teachingRuns: [
      {
        id: "teach-task-same-task",
        taskId: "task-same-task",
        userMessageId: "u-task-same-task",
        assistantMessageId: "a-task-same-task",
        userText: "question",
        assistantText: "answer",
        status: "succeeded",
        startedAt: 10,
        completedAt: 11,
        taskResult: {
          task_id: "task-same-task",
          status: "succeeded",
          result_json: { text: "answer" },
          error_text: null,
        },
      },
    ],
  };

  const merged = mergeServerConversationHistory(
    { activeThreadId: existing.id, threads: [existing] },
    [server],
    t,
  );

  assert.deepEqual(
    merged.threads[0].messages.map((message) => message.id),
    ["u-task-same-task", "a-task-same-task"],
  );
});

test("server restore keeps ordered followups as distinct teaching turns", () => {
  const projected = projectConversationHistory(
    [
      {
        schema_version: 1,
        status: "ok",
        turns: [
          {
            schema_version: 1,
            conversation_id: "thread-followups",
            agent_id: "main",
            external_chat_id: "ui-thread-followups",
            conversation_title: null,
            task_id: "task-followups",
            status: "succeeded",
            user_text: "Start the task",
            assistant_text: "The amended task completed",
            error_text: null,
            user_text_result: null,
            assistant_text_result: null,
            error_text_result: null,
            attachment_count: 0,
            attachment_kinds: [],
            artifacts: [],
            artifact_delivery: null,
            created_at: 100,
            updated_at: 140,
            conversation_inputs: [
              {
                schema_version: 1,
                input_id: "input-followup-1",
                client_message_id: "client-followup-1",
                input_seq: 1,
                text: "Apply the first amendment",
                disposition: "applied",
                decision_ref: "continue_or_amend",
                instruction_revision: 1,
                accepted_at: 110,
                updated_at: 111,
              },
              {
                schema_version: 1,
                input_id: "input-followup-2",
                client_message_id: "client-followup-2",
                input_seq: 2,
                text: "Use the revised constraint",
                disposition: "applied",
                decision_ref: "respond",
                instruction_revision: 2,
                accepted_at: 120,
                updated_at: 121,
              },
            ],
          },
        ],
        next_cursor: null,
        truncated: false,
        content_sha256: "0".repeat(64),
      },
    ],
    t,
  );

  assert.deepEqual(
    projected[0].messages.map((message) => [message.role, message.text]),
    [
      ["user", "Start the task"],
      ["user", "Apply the first amendment"],
      ["user", "Use the revised constraint"],
      ["assistant", "The amended task completed"],
    ],
  );
  assert.deepEqual(
    projected[0].teachingRuns.map((run) => [
      run.conversationInputId,
      run.conversationInputRevision,
      run.assistantMessageId,
    ]),
    [
      [null, null, null],
      ["input-followup-1", 1, null],
      ["input-followup-2", 2, "a-task-followups"],
    ],
  );
});

test("paged restore drops completed cache copies but preserves drafts and pending tasks", () => {
  const draft = thread("draft", "queued");
  draft.lastTaskId = null;
  draft.teachingRuns = [];
  const pending = thread("pending", "running");
  const completed = thread("completed", "succeeded");
  const retained = retainLocalDraftsForPagedRestore({
    activeThreadId: completed.id,
    threads: [draft, pending, completed],
  });

  assert.deepEqual(retained.threads.map((item) => item.id), ["draft", "pending"]);
  assert.equal(retained.activeThreadId, completed.id);
});

test("refresh restore selects newest server history instead of an empty welcome thread", () => {
  const pristineWelcome: ChatThreadRecord = {
    id: "empty-refresh-thread",
    agentId: "main",
    title: "新任务",
    messages: [
      {
        id: "chat-system-welcome-1",
        role: "system",
        text: "欢迎",
        ts: 100,
      },
    ],
    input: "",
    createdAt: 100,
    updatedAt: 100,
    teachingMode: false,
    externalChatId: "ui-empty-refresh-thread",
    lastTaskId: null,
    teachingRuns: [],
  };
  const restored = mergeServerConversationHistory(
    retainLocalDraftsForPagedRestore({
      activeThreadId: pristineWelcome.id,
      threads: [pristineWelcome],
    }),
    [
      {
        id: "restored-history-thread",
        externalChatId: "ui-restored-history-thread",
        title: "最近的历史聊天",
        messages: [{ id: "u-restored", role: "user", text: "继续", ts: 10 }],
        createdAt: 10,
        updatedAt: 11,
        lastTaskId: "task-restored",
        teachingRuns: [],
      },
    ],
    t,
  );

  assert.deepEqual(restored.threads.map((item) => item.id), ["restored-history-thread"]);
  assert.equal(restored.activeThreadId, "restored-history-thread");
});

test("more than 80 teaching runs and 120 messages remain reachable through incremental pages", () => {
  let state: ChatThreadState = { activeThreadId: "long-thread", threads: [] };
  for (let pageIndex = 0; pageIndex < 3; pageIndex += 1) {
    const offset = pageIndex * 41;
    const messages = Array.from({ length: 41 }, (_, index) => {
      const sequence = offset + index;
      return [
        { id: `u-${sequence}`, role: "user" as const, text: `q${sequence}`, ts: sequence * 2 },
        {
          id: `a-${sequence}`,
          role: "assistant" as const,
          text: `a${sequence}`,
          ts: sequence * 2 + 1,
        },
      ];
    }).flat();
    const teachingRuns = Array.from({ length: 41 }, (_, index) => {
      const sequence = offset + index;
      return {
        id: `teach-${sequence}`,
        taskId: `task-${sequence}`,
        userMessageId: `u-${sequence}`,
        assistantMessageId: `a-${sequence}`,
        userText: `q${sequence}`,
        assistantText: `a${sequence}`,
        status: "succeeded" as const,
        startedAt: sequence * 2,
        completedAt: sequence * 2 + 1,
        taskResult: {
          task_id: `task-${sequence}`,
          status: "succeeded" as const,
          result_json: { text: `a${sequence}` },
          error_text: null,
        },
      };
    });
    state = mergeServerConversationHistory(
      state,
      [
        {
          id: "long-thread",
          externalChatId: "ui-long-thread",
          title: "long thread",
          messages,
          createdAt: offset * 2,
          updatedAt: (offset + 40) * 2 + 1,
          lastTaskId: `task-${offset + 40}`,
          teachingRuns,
        },
      ],
      t,
    );
  }

  assert.equal(state.threads[0].messages.length, 246);
  assert.equal(state.threads[0].teachingRuns?.length, 123);
  assert.equal(new Set(state.threads[0].messages.map((message) => message.id)).size, 246);
});
