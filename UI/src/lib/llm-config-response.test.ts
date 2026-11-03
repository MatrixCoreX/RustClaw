import assert from "node:assert/strict";
import test from "node:test";
import { readLlmConfig } from "./llm-config-response";

const valid = { config_path: "configs/config.toml", selected_vendor: "custom", selected_model: "fixture-model",
  restart_required: false, vendors: [{ name: "custom", default_model: "fixture-model", models: ["fixture-model"],
    base_url: "https://provider.example/v1", api_key_configured: false }] };

test("accepts valid configured and explicitly unconfigured model settings without inventing defaults", () => {
  assert.equal(readLlmConfig(valid), valid);
  const empty = { ...valid, selected_vendor: "", selected_model: "", vendors: [] };
  assert.equal(readLlmConfig(empty), empty);
});

test("rejects incomplete fields, wrong collection shapes and malformed optional runtime/relay data", () => {
  for (const value of [null, [], {}, { ...valid, vendors: {} }, { ...valid, vendors: [null] },
    { ...valid, restart_required: "false" }, { ...valid, vendors: [{ ...valid.vendors[0], models: [1] }] },
    { ...valid, runtime: { vendor: "custom" } }, { ...valid, hosted_relay: { vendor: "custom" } }]) {
    assert.throws(() => readLlmConfig(value), /^Error: invalid_llm_config_response$/);
  }
});
