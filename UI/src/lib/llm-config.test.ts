import test from "node:test";
import assert from "node:assert/strict";

import { hasUnsavedLlmDraftChanges } from "./llm-config";

test("marks api key edits as unsaved for the current vendor", () => {
  assert.equal(
    hasUnsavedLlmDraftChanges({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M2.7",
      vendors: [
        {
          name: "minimax",
          base_url: "https://api.minimax.io/v1",
          api_key: "old-key",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M2.7",
      draftBaseUrl: "https://api.minimax.io/v1",
      draftApiKey: "new-key",
    }),
    true,
  );
});

test("marks base url edits as unsaved for the current vendor", () => {
  assert.equal(
    hasUnsavedLlmDraftChanges({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M2.7",
      vendors: [
        {
          name: "minimax",
          base_url: "https://api.minimax.io/v1",
          api_key: "same-key",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M2.7",
      draftBaseUrl: "https://api.minimax.cn/v1",
      draftApiKey: "same-key",
    }),
    true,
  );
});

test("does not mark unchanged drafts as unsaved", () => {
  assert.equal(
    hasUnsavedLlmDraftChanges({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M2.7",
      vendors: [
        {
          name: "minimax",
          base_url: "https://api.minimax.io/v1",
          api_key: "same-key",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M2.7",
      draftBaseUrl: "https://api.minimax.io/v1",
      draftApiKey: "same-key",
    }),
    false,
  );
});
