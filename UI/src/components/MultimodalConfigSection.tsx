import type { ReactNode } from "react";

import type { ModelConfigItem } from "../types/api";
import type { MultimodalDraft, MultimodalKey } from "../lib/model-config";

export interface MultimodalConfigEntry {
  key: MultimodalKey;
  label: string;
}

export interface MultimodalConfigSectionLabels {
  vendor: string;
  model: string;
  apiUrl: string;
  apiKey: string;
}

export interface MultimodalConfigSectionProps {
  title: string;
  description: string;
  entries: MultimodalConfigEntry[];
  draft: MultimodalDraft;
  labels: MultimodalConfigSectionLabels;
  onDraftChange: (key: MultimodalKey, field: keyof ModelConfigItem, value: string) => void;
  renderMeta: (key: MultimodalKey) => ReactNode;
}

export function MultimodalConfigSection({
  title,
  description,
  entries,
  draft,
  labels,
  onDraftChange,
  renderMeta,
}: MultimodalConfigSectionProps) {
  return (
    <div className="rounded-2xl border border-white/10 bg-black/20 p-4">
      <h4 className="mb-3 text-sm font-medium text-white/90">{title}</h4>
      <p className="mb-4 text-xs text-white/50">{description}</p>
      <div className="space-y-4">
        {entries.map(({ key, label }) => (
          <div key={key} className="space-y-2 rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
            <div className="flex flex-wrap items-center gap-3">
              <span className="w-24 shrink-0 text-xs font-medium text-white/80">{label}</span>
              <input
                className="theme-input w-28 shrink-0 text-xs"
                placeholder={labels.vendor}
                value={draft[key]?.vendor ?? ""}
                onChange={(event) => onDraftChange(key, "vendor", event.target.value)}
              />
              <input
                className="theme-input min-w-[140px] flex-1 text-xs"
                placeholder={labels.model}
                value={draft[key]?.model ?? ""}
                onChange={(event) => onDraftChange(key, "model", event.target.value)}
              />
            </div>
            <div className="flex flex-wrap items-center gap-2 pl-[7.5rem]">
              <input
                className="theme-input min-w-[200px] flex-1 text-xs"
                placeholder={labels.apiUrl}
                value={draft[key]?.base_url ?? ""}
                onChange={(event) => onDraftChange(key, "base_url", event.target.value)}
              />
              <input
                className="theme-input min-w-[160px] flex-1 text-xs"
                type="password"
                placeholder={labels.apiKey}
                value={draft[key]?.api_key ?? ""}
                onChange={(event) => onDraftChange(key, "api_key", event.target.value)}
              />
            </div>
            {renderMeta(key)}
          </div>
        ))}
      </div>
    </div>
  );
}
