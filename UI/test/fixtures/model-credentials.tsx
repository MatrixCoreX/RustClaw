import React, { useEffect } from "react";
import { createRoot } from "react-dom/client";
import { ModelConfigPage } from "../../src/components/ModelConfigPage";
import { useModelConfigRuntime } from "../../src/hooks/useModelConfigRuntime";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
const zh = params.get("lang") === "zh";
const t = (cn: string, en: string) => zh ? cn : en;
document.documentElement.dataset.theme = params.get("theme") || "light";
const apiFetch = (path: string, init?: RequestInit) => fetch(path, init);
function Fixture() {
  const runtime = useModelConfigRuntime({ apiFetch, t });
  useEffect(() => { void runtime.fetchLlmConfig(); }, []);
  return <main className="theme-shell mx-auto max-w-5xl p-4"><ModelConfigPage
    {...runtime} t={t} tSlash={text => text.split(" / ")[zh ? 0 : 1] || text}
    modelCatalogEntryViews={[]} canManageMultimodalSkills={true}
    onApplyLlmVendorDraft={runtime.applyLlmVendorDraft}
    onApplyHostedRelayDraft={runtime.applyHostedRelayDraft}
    onLlmDraftModelChange={runtime.setLlmDraftModel}
    onLlmDraftBaseUrlChange={runtime.setLlmDraftBaseUrl}
    onLlmDraftApiFormatChange={runtime.setLlmDraftApiFormat}
    onLlmDraftApiKeyChange={runtime.setLlmDraftApiKey}
    onTestLlmConfig={runtime.testLlmConfig} onSaveLlmConfig={runtime.saveLlmConfig}
    onToggleModelsAdvanced={() => runtime.setModelsAdvancedOpen(!runtime.modelsAdvancedOpen)}
    onFetchModelCatalog={runtime.fetchModelCatalog}
    onFetchMultimodalConfig={runtime.fetchMultimodalConfig}
    onSaveMultimodalConfig={runtime.saveMultimodalConfig}
    onMultimodalDraftChange={runtime.setMultimodalDraftKey}
    onMultimodalSkillEnabledChange={runtime.setMultimodalSkillEnabledNow}
    renderMultimodalModelMeta={() => null}
  /></main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
