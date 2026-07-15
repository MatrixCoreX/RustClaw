import type { ReactNode } from "react";
import { ChevronDown, Database, Loader2, RefreshCw, Sparkles } from "lucide-react";

import { llmVendorSupportsApiFormat } from "../lib/llm-config";
import type { ModelCatalogEntryView, MultimodalDraft, MultimodalKey } from "../lib/model-config";
import type { LlmConfigResponse, LlmVendorOption, ModelCatalogResponse, ModelConfigItem } from "../types/api";
import { MultimodalConfigSection } from "./MultimodalConfigSection";

type Translate = (zh: string, en: string) => string;
type TranslateSlash = (text: string) => string;

export interface ModelConfigPageProps {
  t: Translate;
  tSlash: TranslateSlash;
  llmConfigData: LlmConfigResponse | null;
  selectedLlmVendorInfo: LlmVendorOption | null;
  hasCustomLlmVendor: boolean;
  llmConfigLoading: boolean;
  llmConfigSaving: boolean;
  llmTestLoading: boolean;
  llmDraftVendor: string;
  llmDraftModel: string;
  llmDraftBaseUrl: string;
  llmDraftApiFormat: string;
  llmDraftApiKey: string;
  llmConfigError: string | null;
  llmConfigSaveMessage: string | null;
  llmTestMessage: string | null;
  llmTestError: string | null;
  hasUnsavedLlmChanges: boolean;
  modelsAdvancedOpen: boolean;
  modelCatalogData: ModelCatalogResponse | null;
  modelCatalogLoading: boolean;
  modelCatalogError: string | null;
  modelCatalogEntryViews: ModelCatalogEntryView[];
  multimodalDraft: MultimodalDraft;
  multimodalConfigLoading: boolean;
  multimodalConfigSaving: boolean;
  multimodalConfigError: string | null;
  multimodalConfigSaveMessage: string | null;
  hasUnsavedMultimodalChanges: boolean;
  onApplyLlmVendorDraft: (value: string) => void;
  onLlmDraftModelChange: (value: string) => void;
  onLlmDraftBaseUrlChange: (value: string) => void;
  onLlmDraftApiFormatChange: (value: string) => void;
  onLlmDraftApiKeyChange: (value: string) => void;
  onTestLlmConfig: () => unknown | Promise<unknown>;
  onSaveLlmConfig: () => unknown | Promise<unknown>;
  onToggleModelsAdvanced: () => void;
  onFetchModelCatalog: () => unknown | Promise<unknown>;
  onFetchMultimodalConfig: () => unknown | Promise<unknown>;
  onSaveMultimodalConfig: () => unknown | Promise<unknown>;
  onMultimodalDraftChange: (key: MultimodalKey, field: keyof ModelConfigItem, value: string) => void;
  renderMultimodalModelMeta: (key: MultimodalKey) => ReactNode;
}

export function ModelConfigPage({
  t,
  tSlash,
  llmConfigData,
  selectedLlmVendorInfo,
  hasCustomLlmVendor,
  llmConfigLoading,
  llmConfigSaving,
  llmTestLoading,
  llmDraftVendor,
  llmDraftModel,
  llmDraftBaseUrl,
  llmDraftApiFormat,
  llmDraftApiKey,
  llmConfigError,
  llmConfigSaveMessage,
  llmTestMessage,
  llmTestError,
  hasUnsavedLlmChanges,
  modelsAdvancedOpen,
  modelCatalogData,
  modelCatalogLoading,
  modelCatalogError,
  modelCatalogEntryViews,
  multimodalDraft,
  multimodalConfigLoading,
  multimodalConfigSaving,
  multimodalConfigError,
  multimodalConfigSaveMessage,
  hasUnsavedMultimodalChanges,
  onApplyLlmVendorDraft,
  onLlmDraftModelChange,
  onLlmDraftBaseUrlChange,
  onLlmDraftApiFormatChange,
  onLlmDraftApiKeyChange,
  onTestLlmConfig,
  onSaveLlmConfig,
  onToggleModelsAdvanced,
  onFetchModelCatalog,
  onFetchMultimodalConfig,
  onSaveMultimodalConfig,
  onMultimodalDraftChange,
  renderMultimodalModelMeta,
}: ModelConfigPageProps) {
  const supportsApiFormat = llmVendorSupportsApiFormat(selectedLlmVendorInfo?.name);

  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
      <div className="mb-5">
        <div className="rounded-2xl border border-white/10 bg-black/20 p-5">
          <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("第一步", "Step one")}</p>
          <h3 className="mt-2 text-xl font-semibold tracking-tight">
            {t("先把主模型配好，后面的微信和 Telegram 才能真正工作。", "Configure the main model first so WeChat and Telegram can actually work afterward.")}
          </h3>
          <p className="mt-3 max-w-2xl text-sm leading-7 text-white/70">
            {t(
              "这里只处理 RustClaw 的主大模型。第一次使用时，先选厂商、模型、接口地址和 API Key，保存后如果提示需要重启，就再重启一次。",
              "This section only handles RustClaw's main LLM. For first-time setup, choose the vendor, model, endpoint, and API key. After saving, restart if the page tells you to.",
            )}
          </p>
        </div>
      </div>

      <div className="mb-5 rounded-2xl border border-white/10 bg-black/20 p-4">
        <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
          <h3 className="text-base font-semibold">{t("大模型设置", "LLM Settings")}</h3>
          <div className="flex items-center gap-2">
            {hasCustomLlmVendor ? (
              <button
                type="button"
                onClick={() => onApplyLlmVendorDraft("custom")}
                disabled={llmConfigLoading}
                className="theme-secondary-btn px-3 py-2 text-xs"
              >
                <Sparkles className="h-3.5 w-3.5" />
                {t("自定义模型", "Custom model")}
              </button>
            ) : null}
            <button
              type="button"
              onClick={() => void onTestLlmConfig()}
              disabled={llmTestLoading || llmConfigLoading || !llmDraftVendor || !llmDraftModel || !llmDraftBaseUrl.trim()}
              className="theme-secondary-btn px-3 py-2 text-xs"
            >
              {llmTestLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
              {t("测试连接", "Test Connection")}
            </button>
            <button
              type="button"
              onClick={() => void onSaveLlmConfig()}
              disabled={llmConfigSaving || llmConfigLoading || !hasUnsavedLlmChanges || !llmDraftVendor || !llmDraftModel || !llmDraftBaseUrl.trim()}
              className="theme-accent-btn px-3 py-2 text-xs"
            >
              {llmConfigSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Database className="h-3.5 w-3.5" />}
              {tSlash("保存模型设置 / Save LLM Settings")}
            </button>
          </div>
        </div>

        <div className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <label className="block space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">{t("模型厂商", "Vendor")}</span>
              <select
                className="theme-input"
                value={llmDraftVendor}
                onChange={(event) => onApplyLlmVendorDraft(event.target.value)}
              >
                <option value="">{t("请选择厂商", "Select a vendor")}</option>
                {(llmConfigData?.vendors ?? []).map((vendor) => (
                  <option key={vendor.name} value={vendor.name}>
                    {vendor.name === "custom"
                      ? t("custom（自定义）", "custom (Custom)")
                      : vendor.name === "mimo"
                        ? "mimo (Xiaomi MiMo)"
                        : vendor.name}
                  </option>
                ))}
              </select>
            </label>

            <label className="block space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">{t("具体模型", "Model")}</span>
              <input
                className="theme-input"
                value={llmDraftModel}
                onChange={(event) => onLlmDraftModelChange(event.target.value)}
                list={selectedLlmVendorInfo ? `llm-models-${selectedLlmVendorInfo.name}` : undefined}
                disabled={!selectedLlmVendorInfo}
                placeholder={selectedLlmVendorInfo ? t("输入模型名", "Enter model name") : t("先选厂商", "Choose a vendor first")}
              />
              {selectedLlmVendorInfo ? (
                <datalist id={`llm-models-${selectedLlmVendorInfo.name}`}>
                  {(selectedLlmVendorInfo.models ?? []).map((model) => (
                    <option key={model} value={model} />
                  ))}
                </datalist>
              ) : null}
              {selectedLlmVendorInfo?.name === "custom" ? (
                <p className="text-xs text-white/45">{t("自定义厂商下可直接填写任意模型名。", "With the custom vendor, you can enter any model name directly.")}</p>
              ) : null}
            </label>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <label className="block space-y-2">
              <span className="text-xs uppercase tracking-widest text-white/50">Base URL</span>
              <input
                className="theme-input"
                value={llmDraftBaseUrl}
                onChange={(event) => onLlmDraftBaseUrlChange(event.target.value)}
                placeholder="https://api.openai.com/v1"
                disabled={!selectedLlmVendorInfo}
              />
            </label>

            {supportsApiFormat ? (
              <label className="block space-y-2">
                <span className="text-xs uppercase tracking-widest text-white/50">{t("接口协议", "Protocol")}</span>
                <select
                  className="theme-input"
                  value={llmDraftApiFormat || "openai_compat"}
                  onChange={(event) => onLlmDraftApiFormatChange(event.target.value)}
                >
                  <option value="openai_compat">{t("OpenAI（默认）", "OpenAI (Default)")}</option>
                  <option value="anthropic_claude">{t("Anthropic", "Anthropic")}</option>
                </select>
              </label>
            ) : (
              <label className="block space-y-2">
                <span className="text-xs uppercase tracking-widest text-white/50">API Key</span>
                <input
                  type="text"
                  className="theme-input"
                  value={llmDraftApiKey}
                  onChange={(event) => onLlmDraftApiKeyChange(event.target.value)}
                  placeholder="sk-..."
                  autoComplete="off"
                  disabled={!selectedLlmVendorInfo}
                />
              </label>
            )}
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            {supportsApiFormat ? (
              <label className="block space-y-2">
                <span className="text-xs uppercase tracking-widest text-white/50">API Key</span>
                <input
                  type="text"
                  className="theme-input"
                  value={llmDraftApiKey}
                  onChange={(event) => onLlmDraftApiKeyChange(event.target.value)}
                  placeholder="sk-..."
                  autoComplete="off"
                  disabled={!selectedLlmVendorInfo}
                />
              </label>
            ) : null}

            {supportsApiFormat ? <div /> : null}
          </div>

          {llmConfigError ? (
            <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
              {tSlash("模型配置读取/保存失败 / LLM config read/save failed")}: {llmConfigError}
            </p>
          ) : null}
          {llmConfigSaveMessage ? (
            <p className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
              {llmConfigSaveMessage}
            </p>
          ) : null}
          {llmTestMessage ? (
            <p className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
              {llmTestMessage}
            </p>
          ) : null}
          {llmTestError ? (
            <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
              {llmTestError}
            </p>
          ) : null}
          {hasUnsavedLlmChanges ? (
            <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
              {t("你有未保存的大模型变更，请点击“保存模型设置”。", "You have unsaved LLM changes. Click \"Save LLM Settings\".")}
            </p>
          ) : null}
        </div>
      </div>

      <div className="mb-5 rounded-2xl border border-white/10 bg-black/20 p-4">
        <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold">{t("模型能力目录", "Model Capability Catalog")}</h3>
            <p className="mt-1 text-sm text-white/55">
              {t("这里展示运行时从配置读取到的模型、能力和长尾任务边界。", "This shows runtime model, capability, and long-tail task boundaries read from configuration.")}
            </p>
          </div>
          <button
            type="button"
            onClick={() => void onFetchModelCatalog()}
            disabled={modelCatalogLoading}
            className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
          >
            {modelCatalogLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            {t("刷新", "Refresh")}
          </button>
        </div>

        {modelCatalogError ? (
          <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{modelCatalogError}</p>
        ) : null}

        <div className="grid gap-3 lg:grid-cols-2">
          {modelCatalogEntryViews.map((entry) => (
            <div key={entry.key} className="rounded-xl border border-white/10 bg-white/[0.03] p-3">
              <div className="flex flex-wrap items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="truncate text-sm font-semibold text-white">{entry.provider}</p>
                  <p className="mt-1 break-all text-xs text-white/55">{entry.model}</p>
                </div>
                {entry.active ? (
                  <span className="rounded-md border border-emerald-400/30 bg-emerald-500/10 px-2 py-1 text-[11px] text-emerald-100">
                    active_text_provider
                  </span>
                ) : null}
              </div>
              <div className="mt-3 flex flex-wrap gap-1.5">
                {entry.capabilityBadges.map((badge) => (
                  <span key={`${entry.key}-cap-${badge}`} className="rounded-md border border-sky-400/25 bg-sky-500/10 px-2 py-1 text-[11px] text-sky-100/85">
                    {badge}
                  </span>
                ))}
              </div>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {entry.metaBadges.map((badge) => (
                  <span key={`${entry.key}-meta-${badge}`} className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-[11px] text-white/60">
                    {badge}
                  </span>
                ))}
              </div>
            </div>
          ))}
        </div>

        {!modelCatalogLoading && modelCatalogEntryViews.length === 0 ? (
          <p className="rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-sm text-white/55">
            {t("暂未读取到模型能力目录。", "No model capability catalog is available yet.")}
          </p>
        ) : null}

        {modelCatalogData ? (
          <details className="mt-3 rounded-xl border border-white/10 bg-black/20 p-3">
            <summary className="cursor-pointer text-xs font-medium text-white/65">raw_model_catalog_json</summary>
            <pre className="mt-3 max-h-72 overflow-auto rounded-lg bg-black/30 p-3 text-[11px] leading-relaxed text-white/70">
              {JSON.stringify(modelCatalogData, null, 2)}
            </pre>
          </details>
        ) : null}
      </div>

      <div className="mt-6 space-y-6">
        <div className="rounded-2xl border border-white/10 bg-black/20 p-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h3 className="text-base font-semibold">{t("多模态模块", "Multimodal Modules")}</h3>
              <p className="mt-2 text-sm text-white/55">
                {t("以下是图像、声音、视频和音乐模块。第一次使用可以先不配置，等主模型和机器人接入跑通后再补。", "These image, audio, video, and music modules are advanced settings. You can skip them on the first run and come back after the main model and bot setup are working.")}
              </p>
            </div>
            <button
              type="button"
              onClick={onToggleModelsAdvanced}
              className="theme-topbar-btn px-3 py-2 text-xs font-medium"
            >
              <ChevronDown className={`h-3.5 w-3.5 transition-transform ${modelsAdvancedOpen ? "rotate-180" : ""}`} />
              {modelsAdvancedOpen ? t("收起多模态模块", "Hide multimodal modules") : t("展开多模态模块", "Show multimodal modules")}
            </button>
          </div>

          {modelsAdvancedOpen ? (
            <div className="mt-5 space-y-6 border-t border-white/10 pt-5">
              <div className="flex flex-wrap items-center justify-end gap-2">
                <button
                  type="button"
                  onClick={() => void onFetchMultimodalConfig()}
                  disabled={multimodalConfigLoading}
                  className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {multimodalConfigLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                  {t("刷新", "Refresh")}
                </button>
                <button
                  type="button"
                  onClick={() => void onSaveMultimodalConfig()}
                  disabled={multimodalConfigSaving || multimodalConfigLoading || !hasUnsavedMultimodalChanges}
                  className="theme-accent-btn px-3 py-2 text-xs"
                >
                  {multimodalConfigSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Database className="h-3.5 w-3.5" />}
                  {t("保存多模态配置", "Save Multimodal Config")}
                </button>
              </div>

              {multimodalConfigError ? (
                <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{multimodalConfigError}</p>
              ) : null}
              {multimodalConfigSaveMessage ? (
                <p className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">{multimodalConfigSaveMessage}</p>
              ) : null}
              {hasUnsavedMultimodalChanges ? (
                <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                  {t("你有未保存的多模态配置变更。", "You have unsaved multimodal config changes.")}
                </p>
              ) : null}

              <MultimodalConfigSection
                title={t("图像模块", "Image Modules")}
                description={t("图像编辑、文生图、图像理解可分别配置厂商、模型及该厂商的 API 地址与密钥（写入 configs/image.toml）。", "Configure vendor, model, base URL and API key per image module. Saved to configs/image.toml.")}
                entries={[
                  { key: "image_edit", label: t("图像编辑", "Image Edit") },
                  { key: "image_generation", label: t("文生图", "Image Generate") },
                  { key: "image_vision", label: t("图像理解", "Image Vision") },
                ]}
                draft={multimodalDraft}
                labels={{
                  vendor: t("厂商", "Vendor"),
                  model: t("模型", "Model"),
                  apiUrl: t("API 地址 (base_url)", "API URL (base_url)"),
                  apiKey: "API Key",
                }}
                onDraftChange={onMultimodalDraftChange}
                renderMeta={renderMultimodalModelMeta}
              />

              <MultimodalConfigSection
                title={t("声音模块", "Audio Modules")}
                description={t("语音合成、语音转写可分别配置厂商、模型及该厂商的 API 地址与密钥（写入 configs/audio.toml）。", "Configure vendor, model, base URL and API key per audio module. Saved to configs/audio.toml.")}
                entries={[
                  { key: "audio_synthesize", label: t("语音合成", "Audio TTS") },
                  { key: "audio_transcribe", label: t("语音转写", "Audio STT") },
                ]}
                draft={multimodalDraft}
                labels={{
                  vendor: t("厂商", "Vendor"),
                  model: t("模型", "Model"),
                  apiUrl: t("API 地址 (base_url)", "API URL (base_url)"),
                  apiKey: "API Key",
                }}
                onDraftChange={onMultimodalDraftChange}
                renderMeta={renderMultimodalModelMeta}
              />

              <MultimodalConfigSection
                title={t("视频模块", "Video Modules")}
                description={t("视频生成可配置厂商、模型及该厂商的 API 地址与密钥（写入 configs/video.toml）。", "Configure vendor, model, base URL and API key for video generation. Saved to configs/video.toml.")}
                entries={[{ key: "video_generation", label: t("视频生成", "Video Generate") }]}
                draft={multimodalDraft}
                labels={{
                  vendor: t("厂商", "Vendor"),
                  model: t("模型", "Model"),
                  apiUrl: t("API 地址 (base_url)", "API URL (base_url)"),
                  apiKey: "API Key",
                }}
                onDraftChange={onMultimodalDraftChange}
                renderMeta={renderMultimodalModelMeta}
              />

              <MultimodalConfigSection
                title={t("音乐模块", "Music Modules")}
                description={t("音乐生成可配置厂商、模型及该厂商的 API 地址与密钥（写入 configs/music.toml）。", "Configure vendor, model, base URL and API key for music generation. Saved to configs/music.toml.")}
                entries={[{ key: "music_generation", label: t("音乐生成", "Music Generate") }]}
                draft={multimodalDraft}
                labels={{
                  vendor: t("厂商", "Vendor"),
                  model: t("模型", "Model"),
                  apiUrl: t("API 地址 (base_url)", "API URL (base_url)"),
                  apiKey: "API Key",
                }}
                onDraftChange={onMultimodalDraftChange}
                renderMeta={renderMultimodalModelMeta}
              />
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
}
