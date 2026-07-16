import test from "node:test";
import assert from "node:assert/strict";

import { hasUnsavedLlmDraftChanges, llmVendorSupportsApiFormat } from "./llm-config.ts";

test("detects vendors with configurable api format", () => {
  assert.equal(llmVendorSupportsApiFormat("minimax"), true);
  assert.equal(llmVendorSupportsApiFormat("mimo"), true);
  assert.equal(llmVendorSupportsApiFormat("openai"), false);
});

test("marks api key edits as unsaved for the current vendor", () => {
  assert.equal(
    hasUnsavedLlmDraftChanges({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [
        {
          name: "minimax",
          base_url: "https://api.minimaxi.com/v1",
          api_key: "old-key",
          api_format: "openai_compat",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M3",
      draftBaseUrl: "https://api.minimaxi.com/v1",
      draftApiKey: "new-key",
      draftApiFormat: "openai_compat",
    }),
    true,
  );
});

test("marks base url edits as unsaved for the current vendor", () => {
  assert.equal(
    hasUnsavedLlmDraftChanges({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [
        {
          name: "minimax",
          base_url: "https://api.minimaxi.com/v1",
          api_key: "same-key",
          api_format: "openai_compat",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M3",
      draftBaseUrl: "https://proxy.example/minimax/v1",
      draftApiKey: "same-key",
      draftApiFormat: "openai_compat",
    }),
    true,
  );
});

test("does not mark unchanged drafts as unsaved", () => {
  assert.equal(
    hasUnsavedLlmDraftChanges({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [
        {
          name: "minimax",
          base_url: "https://api.minimaxi.com/v1",
          api_key: "same-key",
          api_format: "openai_compat",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M3",
      draftBaseUrl: "https://api.minimaxi.com/v1",
      draftApiKey: "same-key",
      draftApiFormat: "openai_compat",
    }),
    false,
  );
});

test("marks minimax api format edits as unsaved", () => {
  assert.equal(
    hasUnsavedLlmDraftChanges({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [
        {
          name: "minimax",
          base_url: "https://api.minimaxi.com/v1",
          api_key: "same-key",
          api_format: "openai_compat",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M3",
      draftBaseUrl: "https://api.minimaxi.com/v1",
      draftApiKey: "same-key",
      draftApiFormat: "anthropic_claude",
    }),
    true,
  );
});

test("marks mimo api format edits as unsaved", () => {
  assert.equal(
    hasUnsavedLlmDraftChanges({
      selectedVendor: "mimo",
      selectedModel: "mimo-v2.5-pro",
      vendors: [
        {
          name: "mimo",
          base_url: "https://token-plan-cn.xiaomimimo.com/v1",
          api_key: "same-key",
          api_format: "openai_compat",
        },
      ],
      draftVendor: "mimo",
      draftModel: "mimo-v2.5-pro",
      draftBaseUrl: "https://token-plan-cn.xiaomimimo.com/v1",
      draftApiKey: "same-key",
      draftApiFormat: "anthropic_claude",
    }),
    true,
  );
});
