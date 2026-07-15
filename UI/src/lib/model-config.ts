import type { ModelCatalogEntry, ModelCatalogResponse, ModelConfigItem, ModelConfigResponse } from "../types/api";

export type UiLanguage = "zh" | "en";

export const MULTIMODAL_KEYS = [
  "image_edit",
  "image_generation",
  "image_vision",
  "audio_synthesize",
  "audio_transcribe",
  "video_generation",
  "music_generation",
] as const;

export type MultimodalKey = (typeof MULTIMODAL_KEYS)[number];
export type MultimodalDraft = Record<string, ModelConfigItem>;

export interface MultimodalMetaView {
  capabilityBadges: string[];
  visibleModels: string[];
  hiddenModelCount: number;
  metaBadges: string[];
}

export interface ModelCatalogEntryView {
  key: string;
  provider: string;
  model: string;
  active: boolean;
  capabilityBadges: string[];
  metaBadges: string[];
}

function copy(lang: UiLanguage, zh: string, en: string): string {
  return lang === "zh" ? zh : en;
}

export function buildMultimodalDraft(config: ModelConfigResponse): MultimodalDraft {
  const draft: MultimodalDraft = {};
  for (const key of MULTIMODAL_KEYS) {
    const item = config[key];
    draft[key] = {
      vendor: item?.vendor ?? "",
      model: item?.model ?? "",
      base_url: item?.base_url ?? "",
      api_key: item?.api_key ?? "",
    };
  }
  return draft;
}

export function buildMultimodalSavePayload(draft: MultimodalDraft): Record<string, ModelConfigItem | undefined> {
  const payload: Record<string, ModelConfigItem | undefined> = {};
  for (const key of MULTIMODAL_KEYS) {
    const item = draft[key];
    if (!item) continue;
    payload[key] = {
      vendor: item.vendor.trim() || item.vendor,
      model: item.model.trim() || item.model,
      base_url: item.base_url?.trim() ?? "",
      api_key: item.api_key?.trim() ?? "",
    };
  }
  return payload;
}

export function updateMultimodalDraftField(
  previous: MultimodalDraft,
  key: MultimodalKey,
  field: keyof ModelConfigItem,
  value: string,
): MultimodalDraft {
  return {
    ...previous,
    [key]: {
      ...(previous[key] ?? { vendor: "", model: "", base_url: "", api_key: "" }),
      [field]: value,
    },
  };
}

export function hasUnsavedMultimodalDraftChanges(
  config: ModelConfigResponse | null | undefined,
  draft: MultimodalDraft,
): boolean {
  if (!config) return false;
  for (const key of MULTIMODAL_KEYS) {
    const saved = config[key];
    const current = draft[key];
    if (!current) continue;
    if ((saved?.vendor ?? "") !== (current.vendor ?? "") || (saved?.model ?? "") !== (current.model ?? "")) return true;
    if ((saved?.base_url ?? "") !== (current.base_url ?? "") || (saved?.api_key ?? "") !== (current.api_key ?? "")) return true;
  }
  return false;
}

export function formatMultimodalToken(token: string): string {
  return token
    .split(/[._-]+/)
    .map((part) => part.trim())
    .filter(Boolean)
    .join(" / ");
}

export function providerUnsupportedLabel(reason: string | null | undefined, lang: UiLanguage): string {
  switch (reason) {
    case "provider_not_configured":
      return copy(lang, "未选择服务商", "Provider not configured");
    case "model_not_configured":
      return copy(lang, "未选择模型", "Model not configured");
    case "model_not_in_available_models":
      return copy(lang, "当前模型不在可选列表", "Model is not in the available list");
    default:
      return copy(lang, "服务商暂不可用", "Provider unavailable");
  }
}

export function formatContextWindow(tokens: number, lang: UiLanguage): string {
  const label = copy(lang, "上下文", "Context");
  if (tokens >= 1_000_000) return `${label}: ${(tokens / 1_000_000).toFixed(tokens % 1_000_000 === 0 ? 0 : 1)}M`;
  if (tokens >= 1_000) return `${label}: ${(tokens / 1_000).toFixed(tokens % 1_000 === 0 ? 0 : 1)}K`;
  return `${label}: ${tokens}`;
}

export function buildMultimodalMetaView(item: ModelConfigItem | null | undefined, lang: UiLanguage): MultimodalMetaView | null {
  if (!item) return null;
  const capabilityBadges = (item.capabilities ?? []).map(formatMultimodalToken);
  const modelOptions = (item.available_models ?? []).filter(Boolean);
  const visibleModels = modelOptions.slice(0, 4);
  const metaBadges: string[] = [];
  if (item.capability_family) metaBadges.push(`${copy(lang, "能力族", "Family")}: ${formatMultimodalToken(item.capability_family)}`);
  if ((item.input_modalities ?? []).length > 0) metaBadges.push(`${copy(lang, "输入", "Input")}: ${(item.input_modalities ?? []).map(formatMultimodalToken).join(", ")}`);
  if ((item.output_modalities ?? []).length > 0) metaBadges.push(`${copy(lang, "输出", "Output")}: ${(item.output_modalities ?? []).map(formatMultimodalToken).join(", ")}`);
  if (typeof item.context_window_tokens === "number" && item.context_window_tokens > 0) {
    metaBadges.push(formatContextWindow(item.context_window_tokens, lang));
  }
  if (item.async_job_supported !== undefined && item.async_job_supported !== null) {
    metaBadges.push(item.async_job_supported ? copy(lang, "支持异步任务", "Async job supported") : copy(lang, "同步/短任务", "Sync or short task"));
  }
  if (item.risk_level) metaBadges.push(`${copy(lang, "风险", "Risk")}: ${item.risk_level}`);
  if (item.dry_run_supported !== undefined && item.dry_run_supported !== null) {
    metaBadges.push(item.dry_run_supported ? copy(lang, "支持 dry-run", "Dry-run supported") : copy(lang, "不支持 dry-run", "No dry-run"));
  }
  if (item.external_provider !== undefined && item.external_provider !== null) {
    metaBadges.push(
      item.external_provider
        ? copy(lang, "额度/阻断由外部厂商管理", "Quota/blockers managed by provider")
        : copy(lang, "本地或内置能力", "Local or built-in capability"),
    );
  }
  if (item.shared_quota_group) metaBadges.push(`${copy(lang, "额度组", "Quota")}: ${formatMultimodalToken(item.shared_quota_group)}`);
  if (item.model_list_source) metaBadges.push(`${copy(lang, "模型列表", "Model list")}: ${formatMultimodalToken(item.model_list_source)}`);
  if (item.capability_source) metaBadges.push(`${copy(lang, "能力来源", "Capability source")}: ${formatMultimodalToken(item.capability_source)}`);
  if (item.provider_supported === false) {
    metaBadges.push(providerUnsupportedLabel(item.unsupported_reason, lang));
  }
  if (item.api_key_configured) {
    metaBadges.push(item.api_key_masked ? `${copy(lang, "密钥", "Key")}: ${item.api_key_masked}` : copy(lang, "密钥已配置", "Key configured"));
  }
  if (capabilityBadges.length === 0 && modelOptions.length === 0 && metaBadges.length === 0) return null;
  return {
    capabilityBadges,
    visibleModels,
    hiddenModelCount: Math.max(modelOptions.length - visibleModels.length, 0),
    metaBadges,
  };
}

const CATALOG_CAPABILITY_FIELDS: Array<[keyof ModelCatalogEntry, string]> = [
  ["supports_text", "text"],
  ["supports_image_input", "image_input"],
  ["supports_video_input", "video_input"],
  ["supports_audio_input", "audio_input"],
  ["supports_image_understanding", "image_understanding"],
  ["supports_audio_transcription", "audio_transcription"],
  ["supports_image_generation", "image_generation"],
  ["supports_audio_generation", "audio_generation"],
  ["supports_video_generation", "video_generation"],
  ["supports_music_generation", "music_generation"],
];

export function buildModelCatalogEntryViews(
  catalog: ModelCatalogResponse | null | undefined,
  lang: UiLanguage,
): ModelCatalogEntryView[] {
  return (catalog?.entries ?? []).map((entry) => {
    const capabilityBadges = CATALOG_CAPABILITY_FIELDS.filter(([field]) => entry[field] === true).map(([, token]) =>
      formatMultimodalToken(token),
    );
    const metaBadges = [
      `${copy(lang, "接口", "API")}: ${formatMultimodalToken(entry.api_style)}`,
      `${copy(lang, "地址", "Endpoint")}: ${formatMultimodalToken(entry.base_url_kind)}`,
    ];
    if (typeof entry.context_window_tokens === "number" && entry.context_window_tokens > 0) {
      metaBadges.push(formatContextWindow(entry.context_window_tokens, lang));
    }
    if (typeof entry.timeout_seconds === "number" && entry.timeout_seconds > 0) {
      metaBadges.push(`${copy(lang, "超时", "Timeout")}: ${entry.timeout_seconds}s`);
    }
    if (entry.credential_state) {
      metaBadges.push(`credential_state=${formatMultimodalToken(entry.credential_state)}`);
    }
    if ((entry.input_modalities ?? []).length > 0) {
      metaBadges.push(`${copy(lang, "输入", "Input")}: ${(entry.input_modalities ?? []).map(formatMultimodalToken).join(", ")}`);
    }
    if ((entry.output_modalities ?? []).length > 0) {
      metaBadges.push(`${copy(lang, "输出", "Output")}: ${(entry.output_modalities ?? []).map(formatMultimodalToken).join(", ")}`);
    }
    metaBadges.push(entry.async_required ? "async_required=1" : "async_required=0");
    metaBadges.push(entry.dry_run_supported ? "dry_run_supported=1" : "dry_run_supported=0");
    if (entry.models.length > 0) {
      metaBadges.push(`${copy(lang, "模型池", "Pool")}: ${entry.models.slice(0, 4).join(", ")}${entry.models.length > 4 ? ` +${entry.models.length - 4}` : ""}`);
    }
    return {
      key: `${entry.provider}:${entry.model}`,
      provider: entry.provider,
      model: entry.model,
      active: entry.active_text_provider,
      capabilityBadges,
      metaBadges,
    };
  });
}
