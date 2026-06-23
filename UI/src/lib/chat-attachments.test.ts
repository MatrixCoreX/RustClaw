import test from "node:test";
import assert from "node:assert/strict";

import { formatVisionResultText } from "./chat-attachments.ts";

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
