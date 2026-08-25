export interface LlmVendorSnapshot {
  name: string;
  base_url: string;
  api_format?: string;
}

export interface HostedRelaySnapshot {
  vendor: string;
  model: string;
  base_url: string;
  api_format: string;
}

export interface LlmDraftSnapshot {
  vendor: string;
  model: string;
  baseUrl: string;
  apiFormat: string;
}

export interface InitialLlmDraftInput {
  selectedVendor: string;
  selectedModel: string;
  vendors: ReadonlyArray<LlmVendorSnapshot & {
    default_model?: string;
    models?: string[];
    api_key_configured?: boolean;
  }>;
  hostedRelay?: HostedRelaySnapshot | null;
  runtime?: {
    vendor: string;
    model: string;
  } | null;
}

export const HOSTED_RELAY_SELECTION = "__hosted_relay__";

export interface LlmDirtyStateInput {
  selectedVendor: string;
  selectedModel: string;
  vendors: LlmVendorSnapshot[];
  draftVendor: string;
  draftModel: string;
  draftBaseUrl: string;
  draftApiFormat: string;
}

export interface LlmConfiguredStateInput {
  selectedVendor: string;
  selectedModel: string;
  vendors: ReadonlyArray<{
    name: string;
    api_key_configured: boolean;
  }>;
  runtime?: {
    vendor: string;
    model: string;
  } | null;
}

export function llmVendorSupportsApiFormat(vendor: string | null | undefined): boolean {
  const normalized = (vendor || "").trim();
  return normalized === "minimax" || normalized === "mimo";
}

export function hostedRelayDraft(preset: HostedRelaySnapshot): LlmDraftSnapshot {
  return {
    vendor: preset.vendor,
    model: preset.model,
    baseUrl: preset.base_url,
    apiFormat: preset.api_format,
  };
}

export function isHostedRelayDraft(
  preset: HostedRelaySnapshot | null | undefined,
  draft: LlmDraftSnapshot,
): boolean {
  if (!preset) return false;
  return (
    draft.vendor === preset.vendor
    && draft.model === preset.model
    && draft.baseUrl === preset.base_url
    && normalizeLlmApiFormat(draft.apiFormat) === normalizeLlmApiFormat(preset.api_format)
  );
}

export function initialLlmDraft(input: InitialLlmDraftInput): LlmDraftSnapshot {
  const selectedVendor = input.vendors.find((vendor) => vendor.name === input.selectedVendor);
  const activeRuntime = Boolean(input.runtime?.vendor.trim() && input.runtime?.model.trim());
  const selectedProviderReady = Boolean(
    input.selectedVendor.trim()
    && input.selectedModel.trim()
    && (selectedVendor?.api_key_configured || activeRuntime),
  );
  if (!selectedProviderReady && input.hostedRelay) {
    return hostedRelayDraft(input.hostedRelay);
  }
  return {
    vendor: input.selectedVendor,
    model: input.selectedModel,
    baseUrl: selectedVendor?.base_url || "",
    apiFormat: llmVendorSupportsApiFormat(selectedVendor?.name)
      ? (selectedVendor?.api_format || "openai_compat")
      : "",
  };
}

function normalizeLlmApiFormat(value: string | null | undefined): string {
  const trimmed = (value || "").trim();
  if (trimmed === "anthropic" || trimmed === "anthropic_claude") {
    return "anthropic_claude";
  }
  return "openai_compat";
}

export function isLlmConfigured(input: LlmConfiguredStateInput | null | undefined): boolean {
  if (!input) return false;
  const runtimeVendor = input.runtime?.vendor.trim() || "";
  const runtimeModel = input.runtime?.model.trim() || "";
  if (runtimeVendor && runtimeModel) return true;

  const selectedVendor = input.selectedVendor.trim();
  const selectedModel = input.selectedModel.trim();
  if (!selectedVendor || !selectedModel) return false;
  return input.vendors.some(
    (vendor) => vendor.name === selectedVendor && vendor.api_key_configured,
  );
}

export function hasUnsavedLlmDraftChanges(input: LlmDirtyStateInput | null | undefined): boolean {
  if (!input) return false;
  const savedDraftVendor = input.vendors.find((vendor) => vendor.name === input.draftVendor) ?? null;
  const savedSelectedVendor = input.vendors.find((vendor) => vendor.name === input.selectedVendor) ?? null;
  const savedVendor = savedDraftVendor ?? savedSelectedVendor;
  const shouldCompareApiFormat = llmVendorSupportsApiFormat(
    savedVendor?.name || input.draftVendor || input.selectedVendor,
  );

  return (
    input.draftVendor.trim() !== input.selectedVendor.trim() ||
    input.draftModel.trim() !== input.selectedModel.trim() ||
    input.draftBaseUrl.trim() !== (savedVendor?.base_url || "").trim() ||
    (shouldCompareApiFormat &&
      normalizeLlmApiFormat(input.draftApiFormat) !== normalizeLlmApiFormat(savedVendor?.api_format))
  );
}
