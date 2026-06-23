import test from "node:test";
import assert from "node:assert/strict";

import type { ModelConfigResponse } from "../types/api";
import {
  MULTIMODAL_KEYS,
  buildMultimodalDraft,
  buildMultimodalMetaView,
  buildMultimodalSavePayload,
  formatMultimodalToken,
  hasUnsavedMultimodalDraftChanges,
  providerUnsupportedLabel,
  updateMultimodalDraftField,
} from "./model-config.ts";

function model(vendor = "", modelName = "") {
  return { vendor, model: modelName, base_url: "", api_key: "" };
}

function configFixture(): ModelConfigResponse {
  return {
    llm: model("minimax", "MiniMax-M3"),
    image_edit: model("minimax", "image-edit"),
    image_generation: model("minimax", "image-gen"),
    image_vision: model("minimax", "vision"),
    audio_transcribe: model("minimax", "asr"),
    audio_synthesize: model("minimax", "tts"),
    video_generation: model("minimax", "video"),
    music_generation: model("minimax", "music"),
    restart_required: false,
  };
}

test("builds multimodal drafts from configured sections", () => {
  const draft = buildMultimodalDraft(configFixture());
  assert.deepEqual(Object.keys(draft).sort(), [...MULTIMODAL_KEYS].sort());
  assert.equal(draft.image_generation.model, "image-gen");
  assert.equal(draft.music_generation.vendor, "minimax");
});

test("trims multimodal save payload values", () => {
  const draft = buildMultimodalDraft(configFixture());
  const updated = updateMultimodalDraftField(draft, "image_generation", "base_url", " https://api.example/v1 ");
  const payload = buildMultimodalSavePayload(updated);
  assert.equal(payload.image_generation?.base_url, "https://api.example/v1");
  assert.equal(payload.image_generation?.model, "image-gen");
});

test("detects unsaved multimodal draft changes", () => {
  const config = configFixture();
  const draft = buildMultimodalDraft(config);
  assert.equal(hasUnsavedMultimodalDraftChanges(config, draft), false);
  const changed = updateMultimodalDraftField(draft, "audio_synthesize", "model", "new-tts");
  assert.equal(hasUnsavedMultimodalDraftChanges(config, changed), true);
});

test("formats multimodal machine tokens for compact badges", () => {
  assert.equal(formatMultimodalToken("image_generation.dry-run"), "image / generation / dry / run");
});

test("formats provider unsupported labels", () => {
  assert.equal(providerUnsupportedLabel("provider_not_configured", "en"), "Provider not configured");
  assert.equal(providerUnsupportedLabel("model_not_configured", "zh"), "未选择模型");
  assert.equal(providerUnsupportedLabel("unknown", "en"), "Provider unavailable");
});

test("builds multimodal meta view from structured model fields", () => {
  const view = buildMultimodalMetaView(
    {
      vendor: "minimax",
      model: "MiniMax-Hailuo-02",
      capabilities: ["video.generate"],
      available_models: ["a", "b", "c", "d", "e"],
      risk_level: "medium",
      dry_run_supported: true,
      external_provider: true,
      provider_supported: false,
      unsupported_reason: "model_not_in_available_models",
      api_key_configured: true,
      api_key_masked: "mi***ey",
    },
    "en",
  );
  assert.deepEqual(view?.capabilityBadges, ["video / generate"]);
  assert.deepEqual(view?.visibleModels, ["a", "b", "c", "d"]);
  assert.equal(view?.hiddenModelCount, 1);
  assert.deepEqual(view?.metaBadges, [
    "Risk: medium",
    "Dry-run supported",
    "Quota/blockers managed by provider",
    "Model is not in the available list",
    "Key: mi***ey",
  ]);
});

test("omits empty multimodal meta", () => {
  assert.equal(buildMultimodalMetaView(model(), "en"), null);
});
