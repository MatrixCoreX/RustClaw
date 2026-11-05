import { useEffect, useMemo, useState, type SetStateAction } from "react";

import {
  hasUnsavedLlmDraftChanges,
  hostedRelayDraft,
  initialLlmDraft,
  isLlmConfigured,
  llmVendorSupportsApiFormat,
} from "../lib/llm-config";
import {
  buildMultimodalDraft,
  buildMultimodalSavePayload,
  buildMultimodalSkillEnabledState,
  formatLlmTestMessage,
  hasUnsavedMultimodalDraftChanges,
  MULTIMODAL_SKILL_BY_KEY,
  updateMultimodalDraftField,
  type MultimodalKey,
} from "../lib/model-config";
import { formatUiError } from "../lib/ui-error";
import { readLlmConfig } from "../lib/llm-config-response";
import { readJsonApiResponse } from "../lib/api-response";
import { hasSkillSwitchBaseline, readSkillSwitchAcknowledgement, skillSwitchErrorMessage } from "../lib/skill-switches";
import { PageRequestLifecycle } from "../lib/page-request-lifecycle";
import { useRequestOwner } from "./useRequestOwner";
import type {
  ApiResponse,
  LlmConfigResponse,
  LlmTestResponse,
  ModelCatalogResponse,
  ModelConfigItem,
  ModelConfigResponse,
  SkillsConfigResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseModelConfigRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  requestScope: string;
  onBeforeSaveLlm?: () => void;
}

export function useModelConfigRuntime({
  apiFetch,
  t,
  requestScope,
  onBeforeSaveLlm,
}: UseModelConfigRuntimeParams) {
  const isCurrent = useRequestOwner(requestScope);
  const owner = useMemo(() => ({
    llmReads: new PageRequestLifecycle(), llmTests: new PageRequestLifecycle(), multimodalReads: new PageRequestLifecycle(), catalogReads: new PageRequestLifecycle(),
    llmSaving: false, multimodalSaving: false, switchSaving: false,
    llmDirty: false, llmEdit: 0, multimodalDirty: false, multimodalEdit: 0,
  }), [isCurrent]);
  const renderedLlmEdit = owner.llmEdit;
  const renderedMultimodalEdit = owner.multimodalEdit;
  const [llmConfigLoading, setLlmConfigLoading] = useState(false);
  const [llmConfigError, setLlmConfigError] = useState<string | null>(null);
  const [llmConfigData, setLlmConfigData] = useState<LlmConfigResponse | null>(null);
  const [llmDraftVendor, setLlmDraftVendor] = useState("");
  const [llmDraftModel, setLlmDraftModel] = useState("");
  const [llmConfigSaving, setLlmConfigSaving] = useState(false);
  const [llmConfigSaveMessage, setLlmConfigSaveMessage] = useState<string | null>(null);
  const [llmDraftBaseUrl, setLlmDraftBaseUrl] = useState("");
  const [llmDraftApiFormat, setLlmDraftApiFormat] = useState("openai_compat");
  const [llmDraftApiKey, setLlmDraftApiKey] = useState("");
  const [llmTestLoading, setLlmTestLoading] = useState(false);
  const [llmTestMessage, setLlmTestMessage] = useState<string | null>(null);
  const [llmTestError, setLlmTestError] = useState<string | null>(null);
  const [multimodalConfigData, setMultimodalConfigData] = useState<ModelConfigResponse | null>(null);
  const [multimodalConfigLoading, setMultimodalConfigLoading] = useState(false);
  const [multimodalConfigError, setMultimodalConfigError] = useState<string | null>(null);
  const [multimodalDraft, setMultimodalDraft] = useState<Record<string, ModelConfigItem>>({});
  const [multimodalConfigSaving, setMultimodalConfigSaving] = useState(false);
  const [multimodalConfigSaveMessage, setMultimodalConfigSaveMessage] = useState<string | null>(null);
  const [multimodalSkillEnabled, setMultimodalSkillEnabled] = useState<Record<string, boolean>>({});
  const [multimodalSkillSwitchSaving, setMultimodalSkillSwitchSaving] = useState<MultimodalKey | null>(null);
  const [multimodalSkillSwitchMessage, setMultimodalSkillSwitchMessage] = useState<string | null>(null);
  const [modelsAdvancedOpen, setModelsAdvancedOpen] = useState(false);
  const [modelCatalogData, setModelCatalogData] = useState<ModelCatalogResponse | null>(null);
  const [modelCatalogLoading, setModelCatalogLoading] = useState(false);
  const [modelCatalogError, setModelCatalogError] = useState<string | null>(null);

  useEffect(() => {
    setLlmConfigData(null); setLlmConfigLoading(false); setLlmConfigError(null);
    setLlmConfigSaving(false); setLlmConfigSaveMessage(null);
    setLlmDraftVendor(""); setLlmDraftModel(""); setLlmDraftBaseUrl(""); setLlmDraftApiFormat("openai_compat"); setLlmDraftApiKey("");
    setLlmTestLoading(false); setLlmTestError(null); setLlmTestMessage(null);
    setMultimodalConfigData(null); setMultimodalDraft({}); setMultimodalConfigLoading(false); setMultimodalConfigError(null);
    setMultimodalConfigSaving(false); setMultimodalConfigSaveMessage(null); setMultimodalSkillEnabled({});
    setMultimodalSkillSwitchSaving(null); setMultimodalSkillSwitchMessage(null); setModelsAdvancedOpen(false);
    setModelCatalogData(null); setModelCatalogLoading(false); setModelCatalogError(null);
  }, [owner]);

  const editLlm = () => {
    owner.llmEdit += 1; owner.llmDirty = true; owner.llmTests.invalidate(); setLlmTestLoading(false);
  };

  const selectedLlmVendorInfo = useMemo(
    () => llmConfigData?.vendors.find((vendor) => vendor.name === llmDraftVendor) ?? null,
    [llmConfigData, llmDraftVendor],
  );

  const hasUnsavedLlmChanges = useMemo(() => {
    return llmDraftApiKey.trim().length > 0 || hasUnsavedLlmDraftChanges(
      llmConfigData
        ? {
            selectedVendor: llmConfigData.selected_vendor || "",
            selectedModel: llmConfigData.selected_model || "",
            vendors: llmConfigData.vendors,
            draftVendor: llmDraftVendor,
            draftModel: llmDraftModel,
            draftBaseUrl: llmDraftBaseUrl,
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

  const llmConfigured = useMemo(() => {
    if (!llmConfigData) return false;
    return isLlmConfigured({
      selectedVendor: llmConfigData.selected_vendor,
      selectedModel: llmConfigData.selected_model,
      vendors: llmConfigData.vendors,
      runtime: llmConfigData.runtime,
    });
  }, [llmConfigData]);

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

  const loadLlmConfig = async (duringSave = false) => {
    if (!isCurrent() || (owner.llmSaving && !duringSave)) return;
    const request = owner.llmReads.begin(false)!;
    const ownsRead = () => isCurrent() && owner.llmReads.isCurrent(request) && (!owner.llmSaving || duringSave);
    setLlmConfigLoading(true);
    setLlmConfigError(null);
    try {
      const res = await apiFetch(`/v1/llm/config`);
      const body = await readJsonApiResponse<ApiResponse<unknown> | null>(res);
      if (!ownsRead()) return;
      if (!res.ok || !body?.ok || !body.data) {
        throw new Error(typeof body?.error === "string" ? body.error : `llm_config_fetch_http_${res.status}`);
      }
      const config = readLlmConfig(body.data);
      const draft = initialLlmDraft({
        selectedVendor: config.selected_vendor,
        selectedModel: config.selected_model,
        vendors: config.vendors,
        hostedRelay: config.hosted_relay,
        runtime: config.runtime,
      });
      setLlmConfigData(config);
      if (!owner.llmDirty) {
        owner.llmTests.invalidate(); setLlmTestLoading(false);
        setLlmDraftVendor(draft.vendor); setLlmDraftModel(draft.model);
        setLlmDraftBaseUrl(draft.baseUrl); setLlmDraftApiFormat(draft.apiFormat); setLlmDraftApiKey("");
      }
    } catch (err) {
      if (!ownsRead()) return;
      const message = formatUiError(err, t, "大模型设置暂时无法读取。", "LLM settings are temporarily unavailable.");
      setLlmConfigError(message);
    } finally {
      if (ownsRead() && owner.llmReads.finish(request)) setLlmConfigLoading(false);
    }
  };
  const fetchLlmConfig = () => loadLlmConfig();

  const saveLlmConfig = async () => {
    if (!isCurrent() || owner.llmSaving) return;
    owner.llmSaving = true; owner.llmReads.invalidate(); owner.llmTests.invalidate(); owner.catalogReads.invalidate();
    setLlmTestLoading(false);
    setLlmConfigLoading(false); setModelCatalogLoading(false);
    setLlmConfigSaving(true);
    setLlmConfigSaveMessage(null);
    setLlmConfigError(null);
    try {
      onBeforeSaveLlm?.();
      const res = await apiFetch(`/v1/llm/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          selected_vendor: llmDraftVendor,
          selected_model: llmDraftModel,
          vendor_base_url: llmDraftBaseUrl,
          vendor_api_format: llmVendorSupportsApiFormat(llmDraftVendor) ? llmDraftApiFormat : undefined,
          vendor_api_key: llmDraftApiKey.trim() || undefined,
        }),
      });
      const body = (await res.json()) as ApiResponse<{
        restart_required?: boolean;
      }>;
      if (!isCurrent()) return;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `llm_config_save_http_${res.status}`);
      }
      setLlmConfigSaveMessage(
        t(
          "模型设置已保存。填写的 API Key 已单独保存到本机环境变量文件；请按页面提示重启服务。",
          "Model settings saved. Any entered API key was saved separately in the local environment file. Restart the service if prompted.",
        ),
      );
      if (owner.llmEdit === renderedLlmEdit) { owner.llmDirty = false; setLlmDraftApiKey(""); }
      await loadLlmConfig(true);
      await loadModelCatalog(true);
    } catch (err) {
      if (!isCurrent()) return;
      const message = formatUiError(err, t, "大模型设置保存失败，请稍后重试。", "LLM settings could not be saved. Try again shortly.");
      setLlmConfigError(message);
    } finally {
      owner.llmSaving = false;
      if (isCurrent()) setLlmConfigSaving(false);
    }
  };

  const testLlmConfig = async () => {
    if (!isCurrent()) return;
    const request = owner.llmTests.begin(false)!;
    const ownsRequest = () => isCurrent() && owner.llmTests.isCurrent(request);
    if (!llmDraftVendor || !llmDraftModel || !llmDraftBaseUrl.trim()) {
      owner.llmTests.finish(request); setLlmTestLoading(false);
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
          vendor_api_format: llmVendorSupportsApiFormat(llmDraftVendor) ? llmDraftApiFormat : undefined,
          vendor_api_key: llmDraftApiKey.trim() || undefined,
        }),
      });
      const body = (await res.json()) as ApiResponse<LlmTestResponse>;
      if (!ownsRequest()) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `llm_connection_test_http_${res.status}`);
      }
      const responseMessage = formatLlmTestMessage(body.data, t);
      const message = hasUnsavedLlmChanges
        ? `${responseMessage}${t(
            " 这是页面里的临时草稿；确认没问题后，再点“保存模型设置”。",
            " This used the current draft values; save the settings once you're happy with them.",
          )}`
        : responseMessage;
      setLlmTestMessage(message);
    } catch (err) {
      if (!ownsRequest()) return;
      const message = formatUiError(err, t, "大模型连接测试失败，请检查设置和网络。", "The LLM connection test failed. Check the settings and network.");
      setLlmTestError(message);
    } finally {
      if (ownsRequest() && owner.llmTests.finish(request)) setLlmTestLoading(false);
    }
  };

  const loadMultimodalConfig = async (duringSave = false) => {
    if (!isCurrent() || ((owner.multimodalSaving || owner.switchSaving) && !duringSave)) return;
    const request = owner.multimodalReads.begin(false)!;
    const ownsRead = () => isCurrent() && owner.multimodalReads.isCurrent(request)
      && (!(owner.multimodalSaving || owner.switchSaving) || duringSave);
    setMultimodalConfigLoading(true);
    setMultimodalConfigError(null);
    try {
      const [modelRes, skillsRes] = await Promise.all([
        apiFetch("/v1/admin/model-config"),
        apiFetch("/v1/skills/config"),
      ]);
      const modelBody = (await modelRes.json()) as ApiResponse<ModelConfigResponse>;
      const skillsBody = (await skillsRes.json()) as ApiResponse<SkillsConfigResponse>;
      if (!ownsRead()) return;
      if (!modelRes.ok || !modelBody.ok || !modelBody.data) {
        throw new Error(modelBody.error || `model_config_fetch_http_${modelRes.status}`);
      }
      if (!skillsRes.ok || !skillsBody.ok || !skillsBody.data || !hasSkillSwitchBaseline(skillsBody.data)) {
        throw new Error(skillsBody.error || `skill_config_fetch_http_${skillsRes.status}`);
      }
      setMultimodalConfigData(modelBody.data);
      if (!owner.multimodalDirty) setMultimodalDraft(buildMultimodalDraft(modelBody.data));
      setMultimodalSkillEnabled(buildMultimodalSkillEnabledState(skillsBody.data));
    } catch (err) {
      if (ownsRead()) setMultimodalConfigError(formatUiError(err, t, "多模态设置暂时无法读取。", "Multimodal settings are temporarily unavailable."));
    } finally {
      if (ownsRead() && owner.multimodalReads.finish(request)) setMultimodalConfigLoading(false);
    }
  };
  const fetchMultimodalConfig = () => loadMultimodalConfig();

  const loadModelCatalog = async (duringSave = false) => {
    if (!isCurrent() || ((owner.llmSaving || owner.multimodalSaving) && !duringSave)) return;
    const request = owner.catalogReads.begin(false)!;
    const ownsRead = () => isCurrent() && owner.catalogReads.isCurrent(request);
    setModelCatalogLoading(true);
    setModelCatalogError(null);
    try {
      const res = await apiFetch("/v1/models/catalog");
      const body = (await res.json()) as ApiResponse<ModelCatalogResponse>;
      if (!ownsRead()) return;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `model_catalog_fetch_http_${res.status}`);
      }
      setModelCatalogData(body.data);
    } catch (err) {
      if (ownsRead()) setModelCatalogError(formatUiError(err, t, "模型列表暂时无法读取。", "The model catalog is temporarily unavailable."));
    } finally {
      if (ownsRead() && owner.catalogReads.finish(request)) setModelCatalogLoading(false);
    }
  };
  const fetchModelCatalog = () => loadModelCatalog();

  const saveMultimodalConfig = async () => {
    if (!isCurrent() || owner.multimodalSaving) return;
    owner.multimodalSaving = true; owner.multimodalReads.invalidate(); owner.catalogReads.invalidate();
    setMultimodalConfigLoading(false); setModelCatalogLoading(false);
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
      if (!isCurrent()) return;
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `model_config_save_http_${res.status}`);
      }
      setMultimodalConfigSaveMessage(t("多模态模块配置已保存，需重启 clawd 生效。", "Multimodal config saved. Restart clawd to apply."));
      if (owner.multimodalEdit === renderedMultimodalEdit) owner.multimodalDirty = false;
      await loadMultimodalConfig(true);
      await loadModelCatalog(true);
    } catch (err) {
      if (isCurrent()) setMultimodalConfigError(formatUiError(err, t, "多模态设置保存失败，请稍后重试。", "Multimodal settings could not be saved. Try again shortly."));
    } finally {
      owner.multimodalSaving = false;
      if (isCurrent()) setMultimodalConfigSaving(false);
    }
  };

  const setMultimodalSkillEnabledNow = async (key: MultimodalKey, enabled: boolean) => {
    if (!isCurrent() || owner.switchSaving) return;
    owner.switchSaving = true; owner.multimodalReads.invalidate(); setMultimodalConfigLoading(false);
    const skillName = MULTIMODAL_SKILL_BY_KEY[key];
    setMultimodalSkillSwitchSaving(key);
    setMultimodalSkillSwitchMessage(null);
    setMultimodalConfigError(null);
    try {
      const currentRes = await apiFetch("/v1/skills/config");
      const currentBody = (await currentRes.json()) as ApiResponse<SkillsConfigResponse>;
      if (!isCurrent()) return;
      if (!currentRes.ok || !currentBody.ok || !currentBody.data || !hasSkillSwitchBaseline(currentBody.data)) {
        throw new Error(currentBody.error || `skill_config_fetch_http_${currentRes.status}`);
      }
      const skillSwitches = { [skillName]: enabled };
      const saveRes = await apiFetch("/v1/skills/config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ skill_switches: skillSwitches, expected_revision: currentBody.data.revision }),
      });
      const saveBody = (await saveRes.json()) as ApiResponse<unknown>;
      if (!isCurrent()) return;
      if (!saveRes.ok || !saveBody.ok) {
        throw new Error(saveBody.error || `skill_config_save_http_${saveRes.status}`);
      }
      const ack = readSkillSwitchAcknowledgement(saveBody.data, skillSwitches);
      setMultimodalSkillEnabled((previous) => ({ ...previous, [key]: enabled }));
      setMultimodalSkillSwitchMessage(
        ack.restart_required
          ? t("开关修改已保存，重启服务后生效。", "The switch change was saved and will apply after a service restart.")
          : enabled
          ? t("这个多模态技能已开启，现在可以接收新调用。", "This multimodal skill is enabled and can receive new calls.")
          : t("这个多模态技能已关闭，已有模型设置会保留。", "This multimodal skill is disabled; its model settings are preserved."),
      );
    } catch (err) {
      if (isCurrent()) setMultimodalConfigError(skillSwitchErrorMessage(err, t));
    } finally {
      owner.switchSaving = false;
      if (isCurrent()) setMultimodalSkillSwitchSaving(null);
    }
  };

  const setMultimodalDraftKey = (key: MultimodalKey, field: keyof ModelConfigItem, value: string) => {
    if (!isCurrent()) return;
    owner.multimodalEdit += 1; owner.multimodalDirty = true;
    setMultimodalDraft((prev) => updateMultimodalDraftField(prev, key, field, value));
  };

  const applyLlmVendorDraft = (nextVendor: string) => {
    if (!isCurrent()) return;
    editLlm();
    const vendorInfo = llmConfigData?.vendors.find((vendor) => vendor.name === nextVendor);
    setLlmDraftVendor(nextVendor);
    setLlmDraftApiKey("");
    if (!vendorInfo) {
      setLlmDraftModel("");
      setLlmDraftBaseUrl("");
      setLlmDraftApiFormat("");
      return;
    }
    const nextModel = vendorInfo.default_model || vendorInfo.models[0] || "";
    setLlmDraftModel(nextModel);
    setLlmDraftBaseUrl(vendorInfo.base_url || "");
    setLlmDraftApiFormat(llmVendorSupportsApiFormat(vendorInfo.name) ? (vendorInfo.api_format || "openai_compat") : "");
  };

  const applyHostedRelayDraft = () => {
    if (!isCurrent()) return;
    const preset = llmConfigData?.hosted_relay;
    if (!preset) return;
    editLlm();
    const draft = hostedRelayDraft(preset);
    setLlmDraftVendor(draft.vendor);
    setLlmDraftModel(draft.model);
    setLlmDraftBaseUrl(draft.baseUrl);
    setLlmDraftApiFormat(draft.apiFormat);
    setLlmDraftApiKey("");
  };

  const clearLlmConfigError = () => { if (isCurrent()) setLlmConfigError(null); };

  return {
    llmConfigLoading,
    llmConfigError,
    llmConfigData,
    llmDraftVendor,
    llmDraftModel,
    llmConfigSaving,
    llmConfigSaveMessage,
    llmDraftBaseUrl,
    llmDraftApiFormat,
    llmDraftApiKey,
    llmTestLoading,
    llmTestMessage,
    llmTestError,
    multimodalConfigData,
    multimodalConfigLoading,
    multimodalConfigError,
    multimodalDraft,
    multimodalConfigSaving,
    multimodalConfigSaveMessage,
    multimodalSkillEnabled,
    multimodalSkillSwitchSaving,
    multimodalSkillSwitchMessage,
    modelsAdvancedOpen,
    modelCatalogData,
    modelCatalogLoading,
    modelCatalogError,
    selectedLlmVendorInfo,
    hasUnsavedLlmChanges,
    llmRestartPending,
    llmConfigured,
    llmStepStatus,
    hasUnsavedMultimodalChanges,
    setLlmDraftModel: (value: string) => { if (isCurrent()) { editLlm(); setLlmDraftModel(value); } },
    setLlmDraftBaseUrl: (value: string) => { if (isCurrent()) { editLlm(); setLlmDraftBaseUrl(value); } },
    setLlmDraftApiFormat: (value: string) => { if (isCurrent()) { editLlm(); setLlmDraftApiFormat(value); } },
    setLlmDraftApiKey: (value: string) => { if (isCurrent()) { editLlm(); setLlmDraftApiKey(value); } },
    setModelsAdvancedOpen: (value: SetStateAction<boolean>) => { if (isCurrent()) setModelsAdvancedOpen(value); },
    fetchLlmConfig,
    saveLlmConfig,
    testLlmConfig,
    fetchMultimodalConfig,
    fetchModelCatalog,
    saveMultimodalConfig,
    setMultimodalSkillEnabledNow,
    setMultimodalDraftKey,
    applyLlmVendorDraft,
    applyHostedRelayDraft,
    clearLlmConfigError,
  };
}
