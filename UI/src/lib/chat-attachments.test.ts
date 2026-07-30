import test from "node:test";
import assert from "node:assert/strict";

import {
  assertChatAttachmentConstraints,
  ChatAttachmentConstraintError,
  DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS,
  fetchChatAttachmentConstraints,
  formatAttachmentSize,
  formatVisionResultText,
} from "./chat-attachments.ts";

test("keeps plain vision text unchanged", () => {
  assert.equal(formatVisionResultText("plain answer"), "plain answer");
});

test("formats structured vision result fields", () => {
  assert.equal(
    formatVisionResultText(
      JSON.stringify({
        summary: "A status card is visible.",
        objects: ["card", "logo"],
        visible_text: ["Agent Runtime", "OK"],
        uncertainties: ["small text"],
      }),
    ),
    "A status card is visible.\n\nObjects: card, logo\n\nVisible text: Agent Runtime ; OK\n\nUncertainties: small text",
  );
});

test("keeps malformed JSON unchanged", () => {
  assert.equal(formatVisionResultText("{not-json"), "{not-json");
});

test("formats attachment sizes", () => {
  assert.equal(formatAttachmentSize(0), "0 B");
  assert.equal(formatAttachmentSize(1024), "1.0 KB");
  assert.equal(formatAttachmentSize(1536), "1.5 KB");
  assert.equal(formatAttachmentSize(20 * 1024 * 1024), "20 MB");
});

test("uses the same count, item, and aggregate attachment contract as the server", () => {
  assert.equal(DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS.max_attachments, 10);
  assert.equal(DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS.max_attachment_bytes, 20 * 1024 * 1024);
  assert.equal(
    DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS.max_total_attachment_bytes,
    60 * 1024 * 1024,
  );
  assert.throws(
    () => assertChatAttachmentConstraints(Array.from({ length: 11 }, () => ({ size: 1 }))),
    (error) =>
      error instanceof ChatAttachmentConstraintError && error.code === "ui_attachments_too_many",
  );
  assert.throws(
    () => assertChatAttachmentConstraints([{ size: 20 * 1024 * 1024 + 1 }]),
    (error) =>
      error instanceof ChatAttachmentConstraintError && error.code === "ui_attachment_too_large",
  );
  assert.throws(
    () => assertChatAttachmentConstraints(Array.from({ length: 4 }, () => ({ size: 16 * 1024 * 1024 }))),
    (error) =>
      error instanceof ChatAttachmentConstraintError &&
      error.code === "ui_attachments_total_too_large",
  );
});

test("loads the authoritative attachment contract from the API", async () => {
  const value = await fetchChatAttachmentConstraints(async (path) => {
    assert.equal(path, "/v1/ui/attachment-constraints");
    return new Response(
      JSON.stringify({ ok: true, data: DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  assert.deepEqual(value, DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS);
});
