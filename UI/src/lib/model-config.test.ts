import test from "node:test";
import assert from "node:assert/strict";

import type { ModelConfigResponse } from "../types/api";
import {
  MULTIMODAL_KEYS,
  MULTIMODAL_SKILL_BY_KEY,
  buildModelCatalogEntryViews,
  buildMultimodalDraft,
  buildMultimodalMetaView,
  buildMultimodalSavePayload,
  buildMultimodalSkillEnabledState,
  formatContextWindow,
  formatLlmTestMessage,
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

test("maps every multimodal module to its independently switchable skill", () => {
  assert.deepEqual(
    MULTIMODAL_SKILL_BY_KEY,
    {
      image_edit: "image_edit",
      image_generation: "image_generate",
      image_vision: "image_vision",
      audio_synthesize: "audio_synthesize",
      audio_transcribe: "audio_transcribe",
      video_generation: "video_generate",
      music_generation: "music_generate",
    },
  );
});

test("builds multimodal switch state from the live runtime allow-set", () => {
  const enabled = buildMultimodalSkillEnabledState({
    runtime_enabled_skills: ["image_vision", "audio_transcribe", "music_generate"],
    effective_enabled_skills_preview: ["image_edit"],
  });

  assert.equal(enabled.image_vision, true);
  assert.equal(enabled.audio_transcribe, true);
  assert.equal(enabled.music_generation, true);
  assert.equal(enabled.image_edit, false);
  assert.equal(enabled.video_generation, false);
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

test("formats context windows compactly", () => {
  assert.equal(formatContextWindow(1_000_000, "en"), "Context: 1M");
  assert.equal(formatContextWindow(32_768, "zh"), "上下文: 32.8K");
});

test("formats provider unsupported labels", () => {
  assert.equal(providerUnsupportedLabel("provider_not_configured", "en"), "Provider not configured");
  assert.equal(providerUnsupportedLabel("model_not_configured", "zh"), "未选择模型");
  assert.equal(providerUnsupportedLabel("unknown", "en"), "Provider unavailable");
});

test("formats LLM test messages from machine keys", () => {
  const en = (zh: string, value: string) => value;
  const zh = (value: string) => value;
  assert.equal(
    formatLlmTestMessage(
      {
        success: true,
        vendor: "minimax",
        model: "MiniMax-M3",
        provider_type: "minimax",
        message_key: "clawd.msg.provider_connection_test_ok",
        message_args: { provider_name: "MiniMax" },
      },
      en,
    ),
    "Connection test passed: MiniMax responded successfully.",
  );
  assert.equal(
    formatLlmTestMessage(
      {
        success: true,
        vendor: "minimax",
        model: "MiniMax-M3",
        provider_type: "minimax",
        message_key: "clawd.msg.provider_connection_test_ok",
        message_args: { provider_name: "MiniMax" },
      },
      zh,
    ),
    "连接测试通过：MiniMax 可正常响应。",
  );
  assert.equal(
    formatLlmTestMessage(
      {
        success: true,
        vendor: "legacy",
        model: "legacy-model",
        provider_type: "legacy",
        message: "legacy message",
      },
      en,
    ),
    "legacy message",
  );
});

test("builds multimodal meta view from structured model fields", () => {
  const view = buildMultimodalMetaView(
    {
      vendor: "minimax",
      model: "MiniMax-Hailuo-02",
      capabilities: ["video.generate"],
      available_models: ["a", "b", "c", "d", "e"],
      capability_family: "video",
      input_modalities: ["text", "image"],
      output_modalities: ["video"],
      async_job_supported: true,
      shared_quota_group: "provider_account:minimax",
      model_list_source: "static_config",
      capability_source: "static_metadata",
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
    "Family: video",
    "Input: text, image",
    "Output: video",
    "Async job supported",
    "Risk: medium",
    "Dry-run supported",
    "Quota/blockers managed by provider",
    "Quota: provider / account:minimax",
    "Model list: static / config",
    "Capability source: static / metadata",
    "Model is not in the available list",
    "Key: mi***ey",
  ]);
});

test("omits empty multimodal meta", () => {
  assert.equal(buildMultimodalMetaView(model(), "en"), null);
});

test("builds model catalog views from structured capability fields", () => {
  const views = buildModelCatalogEntryViews(
    {
      schema_version: 2,
      selected_provider: "minimax",
      selected_model: "MiniMax-M3",
      entries: [
        {
          schema_version: 2,
          provider: "minimax",
          model: "MiniMax-M3",
          models: ["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M2.5", "MiniMax-M2.1", "MiniMax-M2"],
          api_style: "openai_compatible",
          base_url_kind: "minimax_official_openai_compat",
          context_window_tokens: 1_000_000,
          timeout_seconds: 180,
          credential_state: "configured_inline",
          input_modalities: ["text", "image", "video"],
          output_modalities: ["text"],
          supports_text: true,
          supports_image_input: true,
          supports_video_input: true,
          supports_audio_input: false,
          supports_image_understanding: true,
          supports_audio_transcription: true,
          supports_image_generation: true,
          supports_image_edit: true,
          supports_audio_generation: true,
          supports_video_generation: true,
          supports_music_generation: true,
          async_required: true,
          dry_run_supported: true,
          active_text_provider: true,
          config_source: [],
          capability_source: ["https://platform.minimaxi.com/docs/guides/text-generation"],
        },
      ],
    },
    "en",
  );

  assert.equal(views.length, 1);
  assert.equal(views[0].active, true);
  assert.ok(views[0].capabilityBadges.includes("image / input"));
  assert.ok(!views[0].capabilityBadges.includes("audio / input"));
  assert.ok(views[0].metaBadges.includes("Context: 1M"));
  assert.ok(views[0].metaBadges.includes("credential_state=configured / inline"));
  assert.ok(views[0].metaBadges.includes("Input: text, image, video"));
  assert.ok(views[0].metaBadges.includes("Output: text"));
  assert.ok(views[0].metaBadges.includes("async_required=1"));
  assert.ok(views[0].metaBadges.some((badge) => badge.includes("MiniMax-M3")));
});
