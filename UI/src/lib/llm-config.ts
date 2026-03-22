export interface LlmVendorSnapshot {
  name: string;
  base_url: string;
  api_key?: string;
}

export interface LlmDirtyStateInput {
  selectedVendor: string;
  selectedModel: string;
  vendors: LlmVendorSnapshot[];
  draftVendor: string;
  draftModel: string;
  draftBaseUrl: string;
  draftApiKey: string;
}

export function hasUnsavedLlmDraftChanges(input: LlmDirtyStateInput | null | undefined): boolean {
  if (!input) return false;
  const savedDraftVendor = input.vendors.find((vendor) => vendor.name === input.draftVendor) ?? null;
  const savedSelectedVendor = input.vendors.find((vendor) => vendor.name === input.selectedVendor) ?? null;
  const savedVendor = savedDraftVendor ?? savedSelectedVendor;

  return (
    input.draftVendor.trim() !== input.selectedVendor.trim() ||
    input.draftModel.trim() !== input.selectedModel.trim() ||
    input.draftBaseUrl.trim() !== (savedVendor?.base_url || "").trim() ||
    input.draftApiKey !== (savedVendor?.api_key || "")
  );
}
