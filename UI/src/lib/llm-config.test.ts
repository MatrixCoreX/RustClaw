import test from "node:test";
import assert from "node:assert/strict";

import {
  hasUnsavedLlmDraftChanges,
  initialLlmDraft,
  isHostedRelayDraft,
  isLlmConfigured,
  llmVendorSupportsApiFormat,
} from "./llm-config.ts";

const hostedRelay = {
  vendor: "custom",
  model: "minimax",
  base_url: "https://relay.example/v1",
  api_format: "openai_compat",
};

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

test("uses the hosted relay as the initial draft when no usable direct provider exists", () => {
  assert.deepEqual(
    initialLlmDraft({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [{
        name: "minimax",
        base_url: "https://api.minimaxi.com/v1",
        api_key_configured: false,
      }],
      hostedRelay,
      runtime: null,
    }),
    {
      vendor: "custom",
      model: "minimax",
      baseUrl: "https://relay.example/v1",
      apiFormat: "openai_compat",
    },
  );
});

test("keeps an active direct provider instead of overriding it with the hosted relay", () => {
  assert.deepEqual(
    initialLlmDraft({
      selectedVendor: "minimax",
      selectedModel: "MiniMax-M3",
      vendors: [{
        name: "minimax",
        base_url: "https://api.minimaxi.com/v1",
        api_format: "openai_compat",
        api_key_configured: true,
      }],
      hostedRelay,
      runtime: { vendor: "minimax", model: "MiniMax-M3" },
    }),
    {
      vendor: "minimax",
      model: "MiniMax-M3",
      baseUrl: "https://api.minimaxi.com/v1",
      apiFormat: "openai_compat",
    },
  );
});

test("recognizes only the complete hosted relay draft as the default mode", () => {
  assert.equal(
    isHostedRelayDraft(hostedRelay, {
      vendor: "custom",
      model: "minimax",
      baseUrl: "https://relay.example/v1",
      apiFormat: "openai_compat",
    }),
    true,
  );
  assert.equal(
    isHostedRelayDraft(hostedRelay, {
      vendor: "custom",
      model: "another-model",
      baseUrl: "https://relay.example/v1",
      apiFormat: "openai_compat",
    }),
    false,
  );
});
