import test from "node:test";
import assert from "node:assert/strict";

import { formatAttachmentSize, formatVisionResultText } from "./chat-attachments.ts";

test("keeps plain vision text unchanged", () => {
  assert.equal(formatVisionResultText("plain answer"), "plain answer");
});

test("formats structured vision result fields", () => {
  assert.equal(
    formatVisionResultText(
      JSON.stringify({
        summary: "A status card is visible.",
        objects: ["card", "logo"],
        visible_text: ["RustClaw", "OK"],
        uncertainties: ["small text"],
      }),
    ),
    "A status card is visible.\n\nObjects: card, logo\n\nVisible text: RustClaw ; OK\n\nUncertainties: small text",
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
