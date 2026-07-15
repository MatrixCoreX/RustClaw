import { useEffect, useMemo, useState } from "react";

import {
  hasUnsavedLlmDraftChanges,
  llmVendorSupportsApiFormat,
} from "../lib/llm-config";
import {
  buildMultimodalDraft,
  buildMultimodalSavePayload,
  hasUnsavedMultimodalDraftChanges,
  updateMultimodalDraftField,
  type MultimodalKey,
} from "../lib/model-config";
import type {
  ApiResponse,
  LlmConfigResponse,
  LlmTestResponse,
  ModelCatalogResponse,
  ModelConfigItem,
  ModelConfigResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseModelConfigRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  onBeforeSaveLlm?: () => void;
}

export function useModelConfigRuntime({
  apiFetch,
  t,
  onBeforeSaveLlm,
}: UseModelConfigRuntimeParams) {
  const [llmConfigLoading, setLlmConfigLoading] = useState(false);
  const [llmConfigError, setLlmConfigError] = useState<string | null>(null);
  const [llmConfigData, setLlmConfigData] = useState<LlmConfigResponse | null>(null);
  const [llmDraftVendor, setLlmDraftVendor] = useState("");
  const [llmDraftModel, setLlmDraftModel] = useState("");
  const [llmConfigSaving, setLlmConfigSaving] = useState(false);
  const [llmConfigSaveMessage, setLlmConfigSaveMessage] = useState<string | null>(null);
  const [llmDraftBaseUrl, setLlmDraftBaseUrl] = useState("");
  const [llmDraftApiKey, setLlmDraftApiKey] = useState("");
  const [llmDraftApiFormat, setLlmDraftApiFormat] = useState("openai_compat");
  const [llmTestLoading, setLlmTestLoading] = useState(false);
  const [llmTestMessage, setLlmTestMessage] = useState<string | null>(null);
  const [llmTestError, setLlmTestError] = useState<string | null>(null);
  const [multimodalConfigData, setMultimodalConfigData] = useState<ModelConfigResponse | null>(null);
  const [multimodalConfigLoading, setMultimodalConfigLoading] = useState(false);
  const [multimodalConfigError, setMultimodalConfigError] = useState<string | null>(null);
  const [multimodalDraft, setMultimodalDraft] = useState<Record<string, ModelConfigItem>>({});
  const [multimodalConfigSaving, setMultimodalConfigSaving] = useState(false);
  const [multimodalConfigSaveMessage, setMultimodalConfigSaveMessage] = useState<string | null>(null);
  const [modelsAdvancedOpen, setModelsAdvancedOpen] = useState(false);
  const [modelCatalogData, setModelCatalogData] = useState<ModelCatalogResponse | null>(null);
  const [modelCatalogLoading, setModelCatalogLoading] = useState(false);
  const [modelCatalogError, setModelCatalogError] = useState<string | null>(null);

  const selectedLlmVendorInfo = useMemo(
    () => llmConfigData?.vendors.find((vendor) => vendor.name === llmDraftVendor) ?? null,
    [llmConfigData, llmDraftVendor],
  );

  const hasCustomLlmVendor = useMemo(
    () => (llmConfigData?.vendors ?? []).some((vendor) => vendor.name === "custom"),
    [llmConfigData],
  );

  const hasUnsavedLlmChanges = useMemo(() => {
    return hasUnsavedLlmDraftChanges(
      llmConfigData
        ? {
            selectedVendor: llmConfigData.selected_vendor || "",
            selectedModel: llmConfigData.selected_model || "",
            vendors: llmConfigData.vendors,
            draftVendor: llmDraftVendor,
            draftModel: llmDraftModel,
            draftBaseUrl: llmDraftBaseUrl,
            draftApiKey: llmDraftApiKey,
            draftApiFormat: llmDraftApiFormat,
          }
        : null,
    );
  }, [llmConfigData, llmDraftApiFormat, llmDraftApiKey, llmDraftBaseUrl, llmDraftModel, llmDraftVendor]);

  const llmRestartPending = useMemo(() => {
    if (!llmConfigData) return false;
    const runtimeVendor = llmConfigData.runtime?.vendor?.trim() || "";
    const runtimeModel = llmConfigData.runtime?.model?.trim() || "";
    const savedVendor = llmConfigData.selected_vendor?.trim() || "";
    const savedModel = llmConfigData.selected_model?.trim() || "";
    return llmConfigData.restart_required || runtimeVendor !== savedVendor || runtimeModel !== savedModel;
  }, [llmConfigData]);

  const savedLlmVendorInfo = useMemo(
    () => llmConfigData?.vendors.find((vendor) => vendor.name === llmConfigData.selected_vendor) ?? null,
    [llmConfigData],
  );

  const llmConfigured = useMemo(() => {
    if (!llmConfigData?.selected_vendor || !llmConfigData.selected_model) return false;
    if (!savedLlmVendorInfo) return false;
    return savedLlmVendorInfo.api_key_configured;
  }, [llmConfigData, savedLlmVendorInfo]);

  const llmStepStatus = useMemo<"done" | "attention" | "todo">(() => {
    if (!llmConfigured) return "todo";
    return llmRestartPending ? "attention" : "done";
  }, [llmConfigured, llmRestartPending]);

  const hasUnsavedMultimodalChanges = useMemo(() => {
    return hasUnsavedMultimodalDraftChanges(multimodalConfigData, multimodalDraft);
  }, [multimodalConfigData, multimodalDraft]);

  useEffect(() => {
    setLlmTestMessage(null);
    setLlmTestError(null);
  }, [llmDraftApiFormat, llmDraftApiKey, llmDraftBaseUrl, llmDraftModel, llmDraftVendor]);

  const fetchLlmConfig = async () => {
    setLlmConfigLoading(true);
    setLlmConfigError(null);
    try {
      const res = await apiFetch(`/v1/llm/config`);
      const body = (await res.json()) as ApiResponse<LlmConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `LLM config fetch failed (${res.status})`);
      }
      setLlmConfigData(body.data);
      setLlmDraftVendor(body.data.selected_vendor || "");
      setLlmDraftModel(body.data.selected_model || "");
      const selectedVendor = body.data.vendors.find((vendor) => vendor.name === (body.data.selected_vendor || ""));
      setLlmDraftBaseUrl(selectedVendor?.base_url || "");
      setLlmDraftApiKey(selectedVendor?.api_key || "");
      setLlmDraftApiFormat(llmVendorSupportsApiFormat(selectedVendor?.name) ? (selectedVendor?.api_format || "openai_compat") : "");
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setLlmConfigError(message);
    } finally {
      setLlmConfigLoading(false);
    }
  };

  const saveLlmConfig = async () => {
    setLlmConfigSaving(true);
    setLlmConfigSaveMessage(null);
    setLlmConfigError(null);
    onBeforeSaveLlm?.();
    try {
      const res = await apiFetch(`/v1/llm/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          selected_vendor: llmDraftVendor,
          selected_model: llmDraftModel,
          vendor_base_url: llmDraftBaseUrl,
          vendor_api_key: llmDraftApiKey.trim(),
          vendor_api_format: llmVendorSupportsApiFormat(llmDraftVendor) ? llmDraftApiFormat : undefined,
        }),
      });
      const body = (await res.json()) as ApiResponse<{
        restart_required?: boolean;
      }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `LLM config save failed (${res.status})`);
      }
      setLlmConfigSaveMessage(
        t(
          "大模型设置已保存到 config.toml（需重启 clawd 生效）",
          "LLM settings saved to config.toml (restart clawd to apply)",
        ),
      );
      await fetchLlmConfig();
      await fetchModelCatalog();
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setLlmConfigError(message);
    } finally {
      setLlmConfigSaving(false);
    }
  };

  const testLlmConfig = async () => {
    if (!llmDraftVendor || !llmDraftModel || !llmDraftBaseUrl.trim()) {
      setLlmTestMessage(null);
      setLlmTestError(
        t(
          "请先补齐厂商、模型和 Base URL，再测试连接。",
          "Please fill in vendor, model, and base URL before testing the connection.",
        ),
      );
      return;
    }
    setLlmTestLoading(true);
    setLlmTestMessage(null);
    setLlmTestError(null);
    try {
      const res = await apiFetch(`/v1/llm/test`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          selected_vendor: llmDraftVendor,
          selected_model: llmDraftModel,
          vendor_base_url: llmDraftBaseUrl,
          vendor_api_key: llmDraftApiKey.trim(),
          vendor_api_format: llmVendorSupportsApiFormat(llmDraftVendor) ? llmDraftApiFormat : undefined,
        }),
      });
      const body = (await res.json()) as ApiResponse<LlmTestResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `LLM connection test failed (${res.status})`);
      }
      const message = hasUnsavedLlmChanges
        ? `${body.data.message}${t(
            " 这是页面里的临时草稿；确认没问题后，再点“保存模型设置”。",
            " This used the current draft values; save the settings once you're happy with them.",
          )}`
        : body.data.message;
      setLlmTestMessage(message);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setLlmTestError(message);
    } finally {
      setLlmTestLoading(false);
    }
  };

  const fetchMultimodalConfig = async () => {
    setMultimodalConfigLoading(true);
    setMultimodalConfigError(null);
    try {
      const res = await apiFetch("/v1/admin/model-config");
      const body = (await res.json()) as ApiResponse<ModelConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `model config fetch failed (${res.status})`);
      }
      setMultimodalConfigData(body.data);
      setMultimodalDraft(buildMultimodalDraft(body.data));
    } catch (err) {
      setMultimodalConfigError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setMultimodalConfigLoading(false);
    }
  };

  const fetchModelCatalog = async () => {
    setModelCatalogLoading(true);
    setModelCatalogError(null);
    try {
      const res = await apiFetch("/v1/models/catalog");
      const body = (await res.json()) as ApiResponse<ModelCatalogResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `model catalog fetch failed (${res.status})`);
      }
      setModelCatalogData(body.data);
    } catch (err) {
      setModelCatalogError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setModelCatalogLoading(false);
    }
  };

  const saveMultimodalConfig = async () => {
    setMultimodalConfigSaving(true);
    setMultimodalConfigSaveMessage(null);
    setMultimodalConfigError(null);
    try {
      const payload = buildMultimodalSavePayload(multimodalDraft);
      const res = await apiFetch("/v1/admin/model-config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<{ restart_required?: boolean }>;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `model config save failed (${res.status})`);
      }
      setMultimodalConfigSaveMessage(t("多模态模块配置已保存，需重启 clawd 生效。", "Multimodal config saved. Restart clawd to apply."));
      await fetchMultimodalConfig();
      await fetchModelCatalog();
    } catch (err) {
      setMultimodalConfigError(err instanceof Error ? err.message : t("未知错误", "Unknown error"));
    } finally {
      setMultimodalConfigSaving(false);
    }
  };

  const setMultimodalDraftKey = (key: MultimodalKey, field: keyof ModelConfigItem, value: string) => {
    setMultimodalDraft((prev) => updateMultimodalDraftField(prev, key, field, value));
  };

  const applyLlmVendorDraft = (nextVendor: string) => {
    const vendorInfo = llmConfigData?.vendors.find((vendor) => vendor.name === nextVendor);
    setLlmDraftVendor(nextVendor);
    if (!vendorInfo) {
      setLlmDraftModel("");
      setLlmDraftBaseUrl("");
      setLlmDraftApiKey("");
      setLlmDraftApiFormat("");
      return;
    }
    const nextModel = vendorInfo.default_model || vendorInfo.models[0] || "";
    setLlmDraftModel(nextModel);
    setLlmDraftBaseUrl(vendorInfo.base_url || "");
    setLlmDraftApiKey(vendorInfo.api_key || "");
    setLlmDraftApiFormat(llmVendorSupportsApiFormat(vendorInfo.name) ? (vendorInfo.api_format || "openai_compat") : "");
  };

  const clearLlmConfigError = () => setLlmConfigError(null);

  return {
    llmConfigLoading,
    llmConfigError,
    llmConfigData,
    llmDraftVendor,
    llmDraftModel,
    llmConfigSaving,
    llmConfigSaveMessage,
    llmDraftBaseUrl,
    llmDraftApiKey,
    llmDraftApiFormat,
    llmTestLoading,
    llmTestMessage,
    llmTestError,
    multimodalConfigData,
    multimodalConfigLoading,
    multimodalConfigError,
    multimodalDraft,
    multimodalConfigSaving,
    multimodalConfigSaveMessage,
    modelsAdvancedOpen,
    modelCatalogData,
    modelCatalogLoading,
    modelCatalogError,
    selectedLlmVendorInfo,
    hasCustomLlmVendor,
    hasUnsavedLlmChanges,
    llmRestartPending,
    llmConfigured,
    llmStepStatus,
    hasUnsavedMultimodalChanges,
    setLlmDraftModel,
    setLlmDraftBaseUrl,
    setLlmDraftApiKey,
    setLlmDraftApiFormat,
    setModelsAdvancedOpen,
    fetchLlmConfig,
    saveLlmConfig,
    testLlmConfig,
    fetchMultimodalConfig,
    fetchModelCatalog,
    saveMultimodalConfig,
    setMultimodalDraftKey,
    applyLlmVendorDraft,
    clearLlmConfigError,
  };
}
