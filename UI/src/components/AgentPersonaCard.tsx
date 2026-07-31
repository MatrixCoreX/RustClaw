import { useEffect, useMemo, useState } from "react";
import { Loader2, MessageSquare, RefreshCw, Save, UserRound } from "lucide-react";

import type { AgentConfigResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;

export function AgentPersonaCard({
  t,
  config,
  loading,
  saving,
  error,
  message,
  onRefresh,
  onSave,
  onOpenChat,
}: {
  t: Translate;
  config: AgentConfigResponse | null;
  loading: boolean;
  saving: boolean;
  error: string | null;
  message: string | null;
  onRefresh: () => unknown | Promise<unknown>;
  onSave: (agentId: string, profile: string, customPersona: string) => Promise<boolean>;
  onOpenChat?: () => void;
}) {
  const [selectedAgentId, setSelectedAgentId] = useState("");
  const [draftProfile, setDraftProfile] = useState("inherit");
  const [draftCustomPersona, setDraftCustomPersona] = useState("");
  const [dirty, setDirty] = useState(false);
  const agents = config?.agents ?? [];
  const selectedAgent =
    agents.find((agent) => agent.id === selectedAgentId) ?? agents[0] ?? null;

  useEffect(() => {
    if (agents[0] && !agents.some((agent) => agent.id === selectedAgentId)) {
      setSelectedAgentId(agents[0].id);
    }
  }, [agents, selectedAgentId]);

  useEffect(() => {
    if (!selectedAgent || dirty) return;
    setDraftProfile(selectedAgent.saved_profile);
    setDraftCustomPersona(selectedAgent.custom_persona);
  }, [dirty, selectedAgent]);

  const presetById = useMemo(
    () => new Map((config?.preset_catalog ?? []).map((preset) => [preset.id, preset])),
    [config?.preset_catalog],
  );
  const effectivePreset = selectedAgent
    ? presetById.get(selectedAgent.effective_profile)
    : null;
  const savedPreset = selectedAgent ? presetById.get(selectedAgent.saved_profile) : null;
  const customLength = Array.from(draftCustomPersona).length;
  const maxCustomLength = config?.constraints.custom_persona_max_chars ?? 0;
  const canEdit = config?.editable === true;

  const selectAgent = (agentId: string) => {
    const next = agents.find((agent) => agent.id === agentId);
    if (!next) return;
    setSelectedAgentId(next.id);
    setDraftProfile(next.saved_profile);
    setDraftCustomPersona(next.custom_persona);
    setDirty(false);
  };

  const save = async () => {
    if (!selectedAgent || !canEdit || !dirty || customLength > maxCustomLength) return;
    if (await onSave(selectedAgent.id, draftProfile, draftCustomPersona)) setDirty(false);
  };

  return (
    <section className="theme-panel-soft rounded-[22px] border border-white/10 p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex max-w-3xl items-start gap-3">
          <span className="rounded-lg bg-sky-400/10 p-2 text-sky-200">
            <UserRound className="h-5 w-5" />
          </span>
          <div>
            <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">
              {t("聊天风格", "Chat style")}
            </p>
            <h3 className="mt-2 text-base font-semibold text-white">
              {t("选择 Agent 的说话方式", "Choose how the Agent speaks")}
            </h3>
            <p className="mt-2 text-sm leading-6 text-white/65">
              {t(
                "性格只改变聊天语气，不改变它做什么、生成什么或交付什么。",
                "Personality only changes the chat tone. It does not change what the Agent does, creates, or delivers.",
              )}
            </p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          {onOpenChat ? (
            <button type="button" onClick={onOpenChat} className="theme-accent-btn px-3 py-2 text-sm">
              <MessageSquare className="h-4 w-4" />
              {t("开始聊天", "Start chat")}
            </button>
          ) : null}
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={loading || saving}
            className="theme-topbar-btn px-3 py-2 text-sm"
          >
            {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
            {t("刷新", "Refresh")}
          </button>
        </div>
      </div>

      {loading && !config ? (
        <p className="mt-4 text-sm text-white/55">{t("正在读取设置...", "Loading settings...")}</p>
      ) : selectedAgent && config ? (
        <div className="mt-5 grid gap-4 lg:grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)]">
          <div className="space-y-3">
            {agents.length > 1 ? (
              <label className="block text-sm text-white/75">
                <span className="mb-1.5 block text-xs text-white/50">Agent</span>
                <select
                  value={selectedAgent.id}
                  onChange={(event) => selectAgent(event.target.value)}
                  className="w-full rounded-lg border border-white/15 bg-black/25 px-3 py-2 text-white outline-none"
                >
                  {agents.map((agent) => (
                    <option key={agent.id} value={agent.id}>{agent.name}</option>
                  ))}
                </select>
              </label>
            ) : (
              <div className="rounded-lg border border-white/10 bg-black/15 px-3 py-2">
                <p className="text-xs text-white/45">Agent</p>
                <p className="mt-1 text-sm font-medium text-white/90">{selectedAgent.name}</p>
              </div>
            )}
            <div className="grid grid-cols-2 gap-2 text-xs">
              <div className="rounded-lg border border-white/10 bg-black/15 px-3 py-2">
                <p className="text-white/45">{t("已保存", "Saved")}</p>
                <p className="mt-1 text-white/80">
                  {savedPreset ? personaPresetName(savedPreset.name_key, t) : selectedAgent.saved_profile}
                </p>
              </div>
              <div className="rounded-lg border border-white/10 bg-black/15 px-3 py-2">
                <p className="text-white/45">{t("当前生效", "In effect")}</p>
                <p className="mt-1 text-white/80">
                  {effectivePreset ? personaPresetName(effectivePreset.name_key, t) : selectedAgent.effective_profile}
                </p>
              </div>
            </div>
            {dirty ? (
              <p className="rounded-lg border border-amber-300/20 bg-amber-300/[0.06] px-3 py-2 text-xs text-amber-100">
                {t("有尚未保存的修改。", "You have unsaved changes.")}
              </p>
            ) : null}
          </div>

          <div>
            <label className="block text-sm text-white/75">
              <span className="mb-1.5 block text-xs text-white/50">{t("说话方式", "Speaking style")}</span>
              <select
                value={draftProfile}
                disabled={!canEdit || saving}
                onChange={(event) => {
                  setDraftProfile(event.target.value);
                  setDirty(true);
                }}
                className="w-full rounded-lg border border-white/15 bg-black/25 px-3 py-2 text-white outline-none disabled:opacity-60"
              >
                {config.preset_catalog.map((preset) => (
                  <option key={preset.id} value={preset.id}>
                    {personaPresetName(preset.name_key, t)} — {personaPresetDescription(preset.description_key, t)}
                  </option>
                ))}
              </select>
            </label>

            <details className="mt-3 rounded-lg border border-white/10 bg-black/15 p-3" open={draftProfile === "custom"}>
              <summary className="cursor-pointer text-sm text-white/75">
                {t("高级：自定义聊天风格", "Advanced: custom chat style")}
              </summary>
              <label className="mt-3 block">
                <textarea
                  value={draftCustomPersona}
                  disabled={!canEdit || saving}
                  maxLength={maxCustomLength}
                  rows={4}
                  onChange={(event) => {
                    setDraftCustomPersona(event.target.value);
                    setDirty(true);
                  }}
                  placeholder={t("例如：语气温和，先给结论，再简短解释。", "For example: warm tone, conclusion first, then a short explanation.")}
                  className="w-full resize-y rounded-lg border border-white/15 bg-black/25 px-3 py-2 text-sm leading-6 text-white outline-none placeholder:text-white/30 disabled:opacity-60"
                />
                <span className="mt-1 block text-right text-xs text-white/40">
                  {customLength}/{maxCustomLength}
                </span>
                <span className="mt-1 block text-xs leading-5 text-white/45">
                  {t(
                    "只写希望呈现的语气。保存失败时草稿会保留；不确定时可切回“跟随系统”。",
                    "Describe only the tone you want. Your draft is kept if saving fails; choose Follow system if unsure.",
                  )}
                </span>
              </label>
            </details>

            <div className="mt-4 flex flex-wrap items-center gap-3">
              {canEdit ? (
                <button
                  type="button"
                  onClick={() => void save()}
                  disabled={!dirty || saving || customLength > maxCustomLength}
                  className="theme-accent-btn"
                >
                  {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save className="h-4 w-4" />}
                  {saving ? t("保存中", "Saving") : t("保存设置", "Save settings")}
                </button>
              ) : (
                <span className="text-xs text-white/50">{t("仅管理员可修改。", "Only an administrator can make changes.")}</span>
              )}
              <span className="text-xs text-white/45">
                {t("只影响之后新建的任务", "Affects new tasks only")}
              </span>
            </div>
          </div>
        </div>
      ) : (
        <p className="mt-4 text-sm text-white/55">{t("没有可用的 Agent。", "No Agents are available.")}</p>
      )}

      {error ? <p className="mt-3 text-sm text-amber-100">{error}</p> : null}
      {message ? <p className="mt-3 text-sm text-emerald-100">{message}</p> : null}
    </section>
  );
}

function personaPresetName(key: string, t: Translate): string {
  const copy: Record<string, [string, string]> = {
    "agent.persona.inherit.name": ["跟随系统", "Follow system"],
    "agent.persona.executor.name": ["简洁执行", "Concise executor"],
    "agent.persona.companion.name": ["友好陪伴", "Friendly companion"],
    "agent.persona.expert.name": ["专业说明", "Professional expert"],
    "agent.persona.teacher.name": ["耐心讲解", "Patient teacher"],
    "agent.persona.advisor.name": ["稳健建议", "Steady advisor"],
    "agent.persona.reviewer.name": ["审慎复核", "Careful reviewer"],
    "agent.persona.custom.name": ["自定义", "Custom"],
  };
  const value = copy[key];
  return value ? t(value[0], value[1]) : key;
}

function personaPresetDescription(key: string, t: Translate): string {
  const copy: Record<string, [string, string]> = {
    "agent.persona.inherit.description": ["使用系统当前的默认语气。", "Use the system's current default tone."],
    "agent.persona.executor.description": ["直接、克制，优先给出结果。", "Direct and restrained, with the result first."],
    "agent.persona.companion.description": ["更亲切自然，适合日常交流。", "Warm and natural for everyday conversation."],
    "agent.persona.expert.description": ["表达严谨，突出关键依据。", "Precise wording with emphasis on key evidence."],
    "agent.persona.teacher.description": ["循序说明，降低理解门槛。", "Explain progressively and reduce cognitive load."],
    "agent.persona.advisor.description": ["清楚说明取舍与下一步。", "Clarify tradeoffs and the next step."],
    "agent.persona.reviewer.description": ["先指出风险，再给出可执行结论。", "Surface risks before an actionable conclusion."],
    "agent.persona.custom.description": ["只描述希望呈现的聊天语气。", "Describe only the conversational tone you want."],
  };
  const value = copy[key];
  return value ? t(value[0], value[1]) : key;
}
