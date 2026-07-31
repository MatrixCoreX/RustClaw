import test from "node:test";
import assert from "node:assert/strict";

import {
  hasUnsavedLlmDraftChanges,
  isLlmConfigured,
  llmVendorSupportsApiFormat,
} from "./llm-config.ts";

test("detects vendors with configurable api format", () => {
  assert.equal(llmVendorSupportsApiFormat("minimax"), true);
  assert.equal(llmVendorSupportsApiFormat("mimo"), true);
  assert.equal(llmVendorSupportsApiFormat("openai"), false);
});

test("treats an active runtime provider as configured when credentials are external", () => {
  assert.equal(
    isLlmConfigured({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [
        {
          name: "minimax",
          api_key_configured: false,
        },
      ],
      runtime: {
        vendor: "minimax",
        model: "MiniMax-M3",
      },
    }),
    true,
  );
});

test("requires a configured saved vendor when no runtime provider is active", () => {
  assert.equal(
    isLlmConfigured({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [
        {
          name: "minimax",
          api_key_configured: true,
        },
      ],
      runtime: null,
    }),
    true,
  );
  assert.equal(
    isLlmConfigured({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [
        {
          name: "minimax",
          api_key_configured: false,
        },
      ],
      runtime: null,
    }),
    false,
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
          api_format: "openai_compat",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M3",
      draftBaseUrl: "https://proxy.example/minimax/v1",
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
          api_format: "openai_compat",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M3",
      draftBaseUrl: "https://api.minimaxi.com/v1",
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
          api_format: "openai_compat",
        },
      ],
      draftVendor: "minimax",
      draftModel: "MiniMax-M3",
      draftBaseUrl: "https://api.minimaxi.com/v1",
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
          api_format: "openai_compat",
        },
      ],
      draftVendor: "mimo",
      draftModel: "mimo-v2.5-pro",
      draftBaseUrl: "https://token-plan-cn.xiaomimimo.com/v1",
      draftApiFormat: "anthropic_claude",
    }),
    true,
  );
});
