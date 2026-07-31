import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import {
  conversationHistoryScope,
  conversationHistoryStorageKey,
  advanceConversationBodyDescriptor,
  fetchAllConversationHistory,
  fetchConversationHistoryPage,
  fetchNextConversationBodyPage,
  projectConversationHistory,
} from "./chat-history";
import type { ConversationHistoryPage } from "../types/api";

const t = (zh: string, _en: string) => zh;

function page(): ConversationHistoryPage {
  return {
    schema_version: 1,
    status: "ok",
    next_cursor: null,
    truncated: false,
    content_sha256: "a".repeat(64),
    turns: [
      {
        schema_version: 1,
        conversation_id: "chat-thread-one",
        external_chat_id: "ui-chat-one",
        task_id: "task-2",
        status: "succeeded",
        user_text: "继续测试",
        assistant_text: "测试通过",
        error_text: null,
        attachment_count: 0,
        attachment_kinds: [],
        created_at: 102,
        updated_at: 103,
      },
      {
        schema_version: 1,
        conversation_id: "chat-thread-one",
        external_chat_id: "ui-chat-one",
        task_id: "task-1",
        status: "succeeded",
        user_text: "检查代码",
        assistant_text: "检查完成",
        error_text: null,
        attachment_count: 0,
        attachment_kinds: [],
        created_at: 100,
        updated_at: 101,
      },
    ],
  };
}

test("projects server turns into deterministic messages and teaching runs", () => {
  const threads = projectConversationHistory([page()], t);

  assert.equal(threads.length, 1);
  assert.equal(threads[0].id, "chat-thread-one");
  assert.equal(threads[0].externalChatId, "ui-chat-one");
  assert.equal(threads[0].title, "检查代码");
  assert.deepEqual(
    threads[0].messages.map((message) => [message.id, message.text]),
    [
      ["u-task-1", "检查代码"],
      ["a-task-1", "检查完成"],
      ["u-task-2", "继续测试"],
      ["a-task-2", "测试通过"],
    ],
  );
  assert.deepEqual(
    threads[0].teachingRuns.map((run) => run.taskId),
    ["task-1", "task-2"],
  );
});

test("deduplicates replayed pages and localizes attachment-only user display", () => {
  const input = page();
  input.turns = [
    {
      ...input.turns[0],
      user_text: null,
      attachment_count: 1,
      attachment_kinds: ["image"],
    },
  ];

  const threads = projectConversationHistory([input, input], t);

  assert.equal(threads[0].messages.length, 2);
  assert.equal(threads[0].messages[0].text, "附件消息");
  assert.equal(threads[0].teachingRuns.length, 1);
});

test("uses the persisted custom title instead of inferring it from the first turn", () => {
  const input = page();
  input.turns[0].conversation_title = "发布检查";
  input.turns[1].conversation_title = "发布检查";

  const threads = projectConversationHistory([input], t);

  assert.equal(threads[0].title, "发布检查");
});

test("restores assistant artifacts from server conversation history", () => {
  const input = page();
  input.turns[0].artifacts = [
    {
      schema_version: 1,
      id: "artifact-1",
      filename: "report.pdf",
      kind: "pdf",
      mime_type: "application/pdf",
      size_bytes: 42,
      sha256: "a".repeat(64),
      download_url: "/v1/tasks/task-2/artifacts/artifact-1/content",
      preview_url: "/v1/tasks/task-2/artifacts/artifact-1/content?disposition=inline",
    },
  ];

  const threads = projectConversationHistory([input], t);
  const assistant = threads[0].messages.find((message) => message.id === "a-task-2");

  assert.equal(assistant?.artifacts?.[0].filename, "report.pdf");
  assert.equal(
    threads[0].teachingRuns[1].taskResult.result_json &&
      (threads[0].teachingRuns[1].taskResult.result_json as { artifacts?: unknown[] }).artifacts?.length,
    1,
  );
});

test("builds a credential-free history scope only after authentication is ready", () => {
  assert.equal(conversationHistoryScope(false, "webd", 42, 7), "");
  assert.equal(conversationHistoryScope(true, null, 42, 7), "");
  assert.equal(conversationHistoryScope(true, "key", null, 7), "");
  assert.equal(conversationHistoryScope(true, "webd", 42, 7), "webd:42:7");
  assert.equal(conversationHistoryScope(true, "key", 42, 7), "key:42:7");
  const runtimeUserId = 2_403_753_217_836_067_397;
  const runtimeChatId = 2_640_685_665_509_587_865;
  assert.equal(
    conversationHistoryScope(true, "key", runtimeUserId, runtimeChatId),
    `key:${runtimeUserId}:${runtimeChatId}`,
  );
});

test("uses an identity-scoped local cache key without credentials", () => {
  assert.equal(conversationHistoryStorageKey(""), "");
  assert.equal(
    conversationHistoryStorageKey("webd:42:7"),
    "agent-runtime.ui.chatThreads.v2.webd:42:7",
  );
});

test("loads every server history page without a fixed page or conversation cap", async () => {
  const pages = Array.from({ length: 7 }, (_, pageIndex) => {
    const turns = Array.from({ length: 5 }, (_, turnIndex) => {
      const sequence = pageIndex * 5 + turnIndex;
      return {
        schema_version: 1,
        conversation_id: `chat-thread-${sequence}`,
        external_chat_id: `ui-chat-${sequence}`,
        task_id: `task-${sequence}`,
        status: "succeeded" as const,
        user_text: `问题 ${sequence}`,
        assistant_text: `回答 ${sequence}`,
        error_text: null,
        attachment_count: 0,
        attachment_kinds: [],
        created_at: 100 + sequence,
        updated_at: 100 + sequence,
      };
    });
    const truncated = pageIndex < 6;
    return {
      schema_version: 1,
      status: "ok" as const,
      turns,
      next_cursor: truncated ? `${pageIndex + 1}:task-${pageIndex + 1}` : null,
      truncated,
      content_sha256: createHash("sha256").update(JSON.stringify(turns)).digest("hex"),
    } satisfies ConversationHistoryPage;
  });
  let requestCount = 0;
  const loaded = await fetchAllConversationHistory(async () => {
    const page = pages[requestCount];
    requestCount += 1;
    return new Response(JSON.stringify({ ok: true, data: page }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  });

  assert.equal(requestCount, 7);
  assert.equal(loaded.length, 7);
  assert.equal(projectConversationHistory(loaded, t).length, 35);
});

test("loads one history page at a time for browser memory isolation", async () => {
  const value = page();
  value.next_cursor = "101:task-older";
  value.truncated = true;
  value.content_sha256 = createHash("sha256")
    .update(JSON.stringify(value.turns))
    .digest("hex");
  let requested = "";
  const loaded = await fetchConversationHistoryPage(async (path) => {
    requested = path;
    return new Response(JSON.stringify({ ok: true, data: value }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  });

  assert.match(requested, /limit=60/);
  assert.equal(loaded.next_cursor, "101:task-older");
});

test("validates and advances a snapshot-bound conversation body page", async () => {
  const descriptor = {
    schema_version: 1 as const,
    complete: false,
    original_size_bytes: 12,
    returned_size_bytes: 6,
    content_sha256: "b".repeat(64),
    continuation: {
      kind: "conversation_body_range" as const,
      url: `/v1/tasks/task-2/conversation-body/assistant?start_byte=6&sha256=${"b".repeat(64)}`,
      next_start_byte: 6,
    },
  };
  const next = await fetchNextConversationBodyPage(async () => {
    return new Response(
      JSON.stringify({
        ok: true,
        data: {
          schema_version: 1,
          status: "ok",
          task_id: "task-2",
          field: "assistant",
          text: "second",
          start_byte: 6,
          end_byte: 12,
          total_size_bytes: 12,
          complete: true,
          next_start_byte: null,
          content_sha256: "b".repeat(64),
        },
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  }, descriptor);
  const advanced = advanceConversationBodyDescriptor(descriptor, next);

  assert.equal(next.text, "second");
  assert.equal(advanced.complete, true);
  assert.equal(advanced.returned_size_bytes, 12);
  assert.equal(advanced.continuation, null);
});
