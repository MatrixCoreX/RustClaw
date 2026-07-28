import assert from "node:assert/strict";
import test from "node:test";

import {
  conversationHistoryScope,
  conversationHistoryStorageKey,
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
});

test("uses an identity-scoped local cache key without credentials", () => {
  assert.equal(conversationHistoryStorageKey(""), "");
  assert.equal(
    conversationHistoryStorageKey("webd:42:7"),
    "rustclaw.ui.chatThreads.v2.webd:42:7",
  );
});
