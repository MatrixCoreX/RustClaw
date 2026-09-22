import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useModelConfigRuntime } from "./useModelConfigRuntime";
import type { LlmConfigResponse } from "../types/api";

const vendors = ["openai", "google", "anthropic", "grok", "deepseek", "qwen", "minimax", "mimo", "custom"];
const t = (zh: string) => zh;

test("all direct vendors send draft keys for testing and saving, never reuse a key after switching", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  for (const vendor of vendors) {
    const config: LlmConfigResponse = {
      config_path: "configs/config.toml", selected_vendor: vendor, selected_model: "fixture-model", restart_required: false,
      vendors: vendors.map(name => ({ name, default_model: "fixture-model", models: ["fixture-model"], base_url: "https://provider.example/v1", api_key_configured: false })),
      hosted_relay: { vendor: "custom", model: "relay-model", base_url: "https://relay.example/v1", api_format: "openai_compat", daily_request_limit: 1000 },
    };
    const requests: Array<{ path: string; body: Record<string, unknown> }> = [];
    const apiFetch = async (path: string, init?: RequestInit) => {
      if (init?.body) requests.push({ path, body: JSON.parse(String(init.body)) });
      const data = path === "/v1/llm/test" ? { success: true, vendor, model: "fixture-model", response_text: "ok" }
        : path === "/v1/llm/config" ? config : { entries: [] };
      return new Response(JSON.stringify({ ok: true, data }));
    };
    let runtime: ReturnType<typeof useModelConfigRuntime>;
    function Probe() { runtime = useModelConfigRuntime({ apiFetch, t }); return null; }
    let renderer: ReactTestRenderer;
    await act(async () => { renderer = create(React.createElement(Probe)); });
    try {
      await act(async () => { await runtime.fetchLlmConfig(); });
      assert.equal(runtime!.llmDraftVendor, vendor);
      assert.equal(runtime!.hasUnsavedLlmChanges, false);
      await act(async () => { runtime.setLlmDraftApiKey("  fixture-draft-key  "); });
      assert.equal(runtime!.hasUnsavedLlmChanges, true);
      await act(async () => { await runtime.testLlmConfig(); });
      assert.equal(requests.at(-1)!.body.vendor_api_key, "fixture-draft-key");
      assert.ok(runtime!.llmTestMessage);
      await act(async () => { runtime.setLlmDraftApiKey("fixture-updated-key"); });
      assert.equal(runtime!.llmTestMessage, null);
      await act(async () => { await runtime.saveLlmConfig(); });
      assert.equal(requests.at(-1)!.body.vendor_api_key, "fixture-updated-key");
      assert.equal(runtime!.llmDraftApiKey, "");
      await act(async () => { await runtime.saveLlmConfig(); });
      assert.equal("vendor_api_key" in requests.at(-1)!.body, false);
      await act(async () => { runtime.setLlmDraftApiKey("fixture-for-one-vendor-only"); });
      await act(async () => { runtime.applyLlmVendorDraft(vendor === "mimo" ? "minimax" : "mimo"); });
      assert.equal(runtime!.llmDraftApiKey, "");
      await act(async () => { runtime.setLlmDraftApiKey("fixture-for-direct-only"); });
      await act(async () => { runtime.applyHostedRelayDraft(); });
      assert.equal(runtime!.llmDraftApiKey, "");
    } finally { await act(async () => { renderer.unmount(); }); }
  }
});
