import assert from "node:assert/strict";
import test from "node:test";

import {
  teachingMessageInteractive,
  teachingRunByMessageId,
} from "./chat-teaching";

test("links both user and assistant messages to the same teaching run", () => {
  const runs = [
    {
      id: "teach-1",
      userMessageId: "u-1",
      assistantMessageId: "a-1",
    },
    {
      id: "teach-2",
      userMessageId: "u-2",
      assistantMessageId: null,
    },
  ];

  const byMessage = teachingRunByMessageId(runs);

  assert.equal(byMessage.get("u-1")?.id, "teach-1");
  assert.equal(byMessage.get("a-1")?.id, "teach-1");
  assert.equal(byMessage.get("u-2")?.id, "teach-2");
  assert.equal(byMessage.get("a-2"), undefined);
});

test("message teaching click is active only when teaching mode is selected", () => {
  const run = {
    id: "teach-1",
    userMessageId: "u-1",
    assistantMessageId: "a-1",
  };

  assert.equal(teachingMessageInteractive(false, run), false);
  assert.equal(teachingMessageInteractive(true, null), false);
  assert.equal(teachingMessageInteractive(true, run), true);
});
