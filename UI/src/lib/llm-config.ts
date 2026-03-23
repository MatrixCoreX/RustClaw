export interface LlmVendorSnapshot {
  name: string;
  base_url: string;
  api_key?: string;
  api_format?: string;
}

export interface LlmDirtyStateInput {
  selectedVendor: string;
  selectedModel: string;
  vendors: LlmVendorSnapshot[];
  draftVendor: string;
  draftModel: string;
  draftBaseUrl: string;
  draftApiKey: string;
  draftApiFormat: string;
}

function normalizeMinimaxApiFormat(value: string | null | undefined): string {
  const trimmed = (value || "").trim();
  if (trimmed === "anthropic" || trimmed === "anthropic_claude") {
    return "anthropic_claude";
  }
  return "openai_compat";
}

export function hasUnsavedLlmDraftChanges(input: LlmDirtyStateInput | null | undefined): boolean {
  if (!input) return false;
  const savedDraftVendor = input.vendors.find((vendor) => vendor.name === input.draftVendor) ?? null;
  const savedSelectedVendor = input.vendors.find((vendor) => vendor.name === input.selectedVendor) ?? null;
  const savedVendor = savedDraftVendor ?? savedSelectedVendor;
  const shouldCompareApiFormat = (savedVendor?.name || input.draftVendor || input.selectedVendor).trim() === "minimax";

  return (
    input.draftVendor.trim() !== input.selectedVendor.trim() ||
    input.draftModel.trim() !== input.selectedModel.trim() ||
    input.draftBaseUrl.trim() !== (savedVendor?.base_url || "").trim() ||
    input.draftApiKey !== (savedVendor?.api_key || "") ||
    (shouldCompareApiFormat &&
      normalizeMinimaxApiFormat(input.draftApiFormat) !== normalizeMinimaxApiFormat(savedVendor?.api_format))
  );
}
