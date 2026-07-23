import { AlertCircle, CircleHelp, Database, Loader2, RefreshCw, Trash2, Wrench } from "lucide-react";
import { useState } from "react";

import {
  skillCapabilityLabel,
  skillDescription,
  skillIsolationLabels,
  skillPlannerCapabilityLabel,
  skillRiskLabel,
  skillRuntimeIssue,
  skillUsageExamples,
  type UiLanguage,
} from "../lib/skill-display";
import type { SkillListItem, SkillStoreResponse, SkillsConfigResponse } from "../types/api";
import { SkillRemovalDialog } from "./SkillRemovalDialog";

type Translate = (zh: string, en: string) => string;
type TranslateSlash = (text: string) => string;

export interface SkillSwitchPanelProps {
  lang: UiLanguage;
  t: Translate;
  tSlash: TranslateSlash;
  skillsConfigData: SkillsConfigResponse | null;
  skillsConfigLoading: boolean;
  skillsConfigError: string | null;
  skillSwitchSaving: boolean;
  skillSwitchSaveMessage: string | null;
  hasUnsavedSkillSwitchChanges: boolean;
  managedSkills: string[];
  filteredManagedSkills: string[];
  filteredSkillsTool: string[];
  filteredSkillsBase: string[];
  filteredSkillsImage: string[];
  filteredSkillsAudio: string[];
  filteredSkillsMultimedia: string[];
  filteredSkillsOther: string[];
  normalizedSkillsSearchQuery: string;
  skillsSearchQuery: string;
  skillItemsByName: Map<string, SkillListItem>;
  configuredEnabledSkills: ReadonlySet<string>;
  skillSwitchDraft: Record<string, boolean>;
  recentImportedSkillName: string | null;
  externalSkillNamesSet: ReadonlySet<string>;
  lockedSkillNamesSet: ReadonlySet<string>;
  toolSkillNamesSet: ReadonlySet<string>;
  baseSkillNamesSet: ReadonlySet<string>;
  removableSkillNamesSet: ReadonlySet<string>;
  skillStoreActionName: string | null;
  skillStoreData: SkillStoreResponse | null;
  onFetchSkillsConfig: () => unknown | Promise<unknown>;
  onSaveSkillSwitches: () => unknown | Promise<unknown>;
  onSkillsSearchQueryChange: (value: string) => void;
  onToggleSkillEnabled: (name: string, nextEnabled: boolean) => void;
  onRemoveSkillFromStore: (name: string, preserveConfig: boolean) => unknown | Promise<unknown>;
}

export function SkillSwitchPanel({
  lang,
  t,
  tSlash,
  skillsConfigData,
  skillsConfigLoading,
  skillsConfigError,
  skillSwitchSaving,
  skillSwitchSaveMessage,
  hasUnsavedSkillSwitchChanges,
  managedSkills,
  filteredManagedSkills,
  filteredSkillsTool,
  filteredSkillsBase,
  filteredSkillsImage,
  filteredSkillsAudio,
  filteredSkillsMultimedia,
  filteredSkillsOther,
  normalizedSkillsSearchQuery,
  skillsSearchQuery,
  skillItemsByName,
  configuredEnabledSkills,
  skillSwitchDraft,
  recentImportedSkillName,
  externalSkillNamesSet,
  lockedSkillNamesSet,
  toolSkillNamesSet,
  baseSkillNamesSet,
  removableSkillNamesSet,
  skillStoreActionName,
  skillStoreData,
  onFetchSkillsConfig,
  onSaveSkillSwitches,
  onSkillsSearchQueryChange,
  onToggleSkillEnabled,
  onRemoveSkillFromStore,
}: SkillSwitchPanelProps) {
  const [pendingRemovalName, setPendingRemovalName] = useState<string | null>(null);
  const storeMutationRunning = skillStoreActionName !== null;
  const pendingRemovalItem = skillStoreData?.items.find((item) => item.name === pendingRemovalName);
  const confirmRemoval = async (preserveConfig: boolean) => {
    if (!pendingRemovalName) return;
    const name = pendingRemovalName;
    setPendingRemovalName(null);
    await onRemoveSkillFromStore(name, preserveConfig);
  };
  const renderSkillRow = (name: string) => {
    const skillItem = skillItemsByName.get(name);
    const runtimeIssue = skillRuntimeIssue(skillItem, lang);
    const visiblePlannerCapabilities = (skillItem?.planner_capabilities ?? []).slice(0, 3);
    const visibleCapabilities = (skillItem?.capabilities ?? []).slice(0, 3);
    const visibleIsolationLabels = skillIsolationLabels(skillItem, lang).slice(0, 2);
    const configuredEnabled = configuredEnabledSkills.has(name);
    const persistedSwitchValue = skillsConfigData?.skill_switches?.[name];
    const draftSwitchValue = skillSwitchDraft[name];
    const pendingApply = persistedSwitchValue !== draftSwitchValue;
    const isRecentImport = recentImportedSkillName === name;
    const isExternalSkill = externalSkillNamesSet.has(name);
    const isLockedSkill = lockedSkillNamesSet.has(name);
    const isToolSkill = toolSkillNamesSet.has(name);
    const canRemove = removableSkillNamesSet.has(name);
    const isRemoving = skillStoreActionName === name;
    const usageExamples = skillUsageExamples(skillItem ?? { name }, lang);
    const usageExamplesId = `skill-usage-examples-${name}`;
    const statusMeta = [
      isToolSkill ? t("系统工具", "Tool") : null,
      baseSkillNamesSet.has(name) && !isToolSkill ? t("系统基础能力", "Core capability") : null,
      skillItem?.fixed_on ? t("固定开启", "Always on") : null,
      skillItem?.initial_core ? t("初始可见", "Initially visible") : null,
      skillItem?.deferred ? t("按需加载", "Loaded on demand") : null,
      isExternalSkill ? t("外部导入", "Imported") : null,
      skillItem?.group ? `${t("分组", "Group")}: ${skillItem.group}` : null,
    ].filter(Boolean) as string[];
    const setupMeta = [
      skillItem?.required_bins?.length
        ? `${t("必需工具", "Required tools")}: ${skillItem.required_bins.join(", ")}`
        : null,
      skillItem?.optional_bins?.length
        ? `${t("可选工具", "Optional tools")}: ${skillItem.optional_bins.join(", ")}`
        : null,
      skillItem?.config_files?.length
        ? `${t("配置入口", "Configuration")}: ${skillItem.config_files.join(", ")}`
        : null,
      skillItem?.supported_os?.length
        ? `${t("支持系统", "Supported systems")}: ${skillItem.supported_os.join(", ")}`
        : null,
      ...(skillItem?.platform_notes ?? []),
    ].filter(Boolean) as string[];

    return (
      <label
        id={`skill-row-${name}`}
        key={name}
        className={
          isRecentImport
            ? "group relative flex flex-col gap-2 rounded-lg border border-sky-400/40 bg-sky-500/10 px-2.5 py-2 text-xs shadow-[0_0_0_1px_rgba(56,189,248,0.18)] hover:z-40 focus-within:z-40 sm:flex-row sm:items-center sm:justify-between"
            : "group relative flex flex-col gap-2 rounded-lg border border-white/10 bg-[#12151f] px-2.5 py-2 text-xs hover:z-40 focus-within:z-40 sm:flex-row sm:items-center sm:justify-between"
        }
      >
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0 items-center gap-1.5">
            <span className="block min-w-0 truncate text-sm text-white/90">{name}</span>
            <button
              type="button"
              aria-describedby={usageExamplesId}
              aria-label={t("查看自然语言调用示例", "View natural-language examples")}
              className="inline-flex shrink-0 cursor-help text-white/35 outline-none hover:text-white/70 focus:text-white/70"
            >
              <CircleHelp className="h-3.5 w-3.5" />
            </button>
          </span>
          <span className="mt-0.5 block break-words text-[11px] leading-4 text-white/50">
            {skillDescription(lang, skillItem?.description)}
          </span>
          {statusMeta.length > 0 ? (
            <span className="mt-1 block text-[10px] leading-4 text-white/35">{statusMeta.join(" · ")}</span>
          ) : null}
          <span className="mt-1 flex flex-wrap gap-1">
            <span className="rounded border border-white/10 bg-white/5 px-1.5 py-0.5 text-[10px] text-white/45">
              {skillRiskLabel(skillItem?.risk_level, lang)}
            </span>
            {skillItem?.requires_confirmation ? (
              <span className="rounded border border-amber-500/25 bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-100">
                {t("操作前确认", "Confirms first")}
              </span>
            ) : null}
            {skillItem?.side_effect ? (
              <span className="rounded border border-sky-500/25 bg-sky-500/10 px-1.5 py-0.5 text-[10px] text-sky-100">
                {t("会改变状态", "Changes state")}
              </span>
            ) : null}
            {visibleIsolationLabels.map((label) => (
              <span
                key={`isolation-${label}`}
                className="rounded border border-violet-500/20 bg-violet-500/10 px-1.5 py-0.5 text-[10px] text-violet-100"
              >
                {label}
              </span>
            ))}
            {visiblePlannerCapabilities.map((capability) => (
              <span
                key={`planner-${capability}`}
                className="rounded border border-cyan-500/20 bg-cyan-500/10 px-1.5 py-0.5 text-[10px] text-cyan-100"
              >
                {skillPlannerCapabilityLabel(capability, lang)}
              </span>
            ))}
            {visibleCapabilities.map((capability) => (
              <span key={`runtime-${capability}`} className="rounded border border-white/10 bg-white/5 px-1.5 py-0.5 text-[10px] text-white/45">
                {skillCapabilityLabel(capability, lang)}
              </span>
            ))}
          </span>
          {runtimeIssue ? (
            <span className="mt-1 flex items-start gap-1 text-[10px] leading-4 text-amber-200/90">
              <AlertCircle className="mt-0.5 h-3 w-3 shrink-0" />
              <span>{runtimeIssue}</span>
            </span>
          ) : skillItem?.missing_optional_bins?.length ? (
            <span className="mt-1 block text-[10px] leading-4 text-white/35">
              {t("可选工具未找到", "Optional tools missing")}: {skillItem.missing_optional_bins.join(", ")}
            </span>
          ) : null}
        </span>
        <span className="mt-1 flex shrink-0 flex-wrap items-center gap-1.5 sm:mt-0">
          {skillItem?.runtime_available === false ? (
            <span className="inline-flex items-center gap-1 rounded-full border border-amber-500/35 bg-amber-500/12 px-2 py-0.5 text-[10px] font-medium text-amber-200">
              <Wrench className="h-3 w-3" />
              {t("需配置", "Needs setup")}
            </span>
          ) : null}
          <span
            className={
              configuredEnabled
                ? "inline-flex items-center gap-1 rounded-full border border-emerald-500/35 bg-emerald-500/12 px-2 py-0.5 text-[10px] font-medium text-emerald-200"
                : "inline-flex items-center gap-1 rounded-full border border-amber-500/35 bg-amber-500/12 px-2 py-0.5 text-[10px] font-medium text-amber-200"
            }
          >
            <span
              className={
                configuredEnabled ? "h-1 w-1 rounded-full bg-emerald-300" : "h-1 w-1 rounded-full bg-amber-300"
              }
            />
            {configuredEnabled ? t("已开启", "On") : t("已关闭", "Off")}
          </span>
          {pendingApply ? (
            <span className="text-[10px] text-amber-200/85">
              {t("保存后生效", "After save")}
            </span>
          ) : null}
          <button
            type="button"
            onClick={() => onToggleSkillEnabled(name, !configuredEnabled)}
            disabled={isLockedSkill || storeMutationRunning}
            className={
              isLockedSkill || storeMutationRunning
                ? `cursor-not-allowed rounded border px-1.5 py-0.5 text-[10px] ${
                    isLockedSkill
                      ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-100/80"
                      : "border-white/10 bg-white/5 text-white/35"
                  }`
                : "rounded border border-white/20 bg-white/5 px-1.5 py-0.5 text-[10px] text-white/80 hover:bg-white/10"
            }
            title={
              storeMutationRunning && !isLockedSkill
                ? t("技能列表正在更新，请稍候。", "The skill list is updating. Please wait.")
                : isLockedSkill
                ? isToolSkill
                  ? t("这是底层工具能力，UI 中不能关闭。", "This is a low-level tool capability and cannot be disabled in the UI.")
                  : t("这是系统基础能力，UI 中不能关闭。", "This is a core system capability and cannot be disabled in the UI.")
                : configuredEnabled
                  ? t("先设为关闭，保存后才会真正关闭", "Choose Disable first. It only turns off after you save.")
                  : t("先设为开启，保存后才会真正开启", "Choose Enable first. It only turns on after you save.")
            }
          >
            {isLockedSkill ? t("固定", "Fixed") : configuredEnabled ? t("关", "Off") : isRecentImport ? t("启用", "Enable") : t("开", "On")}
          </button>
          {canRemove ? (
            <button
              type="button"
              onClick={() => setPendingRemovalName(name)}
              disabled={storeMutationRunning}
              className="inline-flex items-center gap-1 rounded border border-red-500/25 bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-100 hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
              title={t("从工具/技能中删除，可在 Skill Store 重新安装", "Remove from Tools/Skills; reinstall from Skill Store")}
            >
              {isRemoving ? <Loader2 className="h-3 w-3 animate-spin" /> : <Trash2 className="h-3 w-3" />}
              {t("删除", "Remove")}
            </button>
          ) : null}
        </span>
        <span
          id={usageExamplesId}
          role="tooltip"
          className="pointer-events-none invisible absolute left-2 right-2 top-full z-50 mt-1 block border border-white/15 bg-[#181b25] px-3 py-2.5 text-left opacity-0 shadow-xl transition-opacity group-hover:visible group-hover:opacity-100 group-focus-within:visible group-focus-within:opacity-100"
        >
          <span className="block text-[11px] font-semibold text-white/85">
            {t("自然语言调用示例", "Natural-language examples")}
          </span>
          <span className="mt-1.5 block space-y-1 text-[11px] leading-4 text-white/65">
            {usageExamples.map((example, index) => (
              <span key={`${index}-${example}`} className="block before:mr-1.5 before:content-['•']">
                {example}
              </span>
            ))}
          </span>
          {setupMeta.length > 0 ? (
            <>
              <span className="mt-2 block border-t border-white/10 pt-2 text-[11px] font-semibold text-white/85">
                {t("安装与运行要求", "Setup and runtime requirements")}
              </span>
              <span className="mt-1 block space-y-1 text-[11px] leading-4 text-white/55">
                {setupMeta.map((entry, index) => (
                  <span key={`${index}-${entry}`} className="block before:mr-1.5 before:content-['•']">
                    {entry}
                  </span>
                ))}
              </span>
            </>
          ) : null}
        </span>
      </label>
    );
  };

  const renderSkillGroup = (title: string, filteredList: string[]) => {
    if (filteredList.length === 0) return null;
    return (
      <div key={title} className="space-y-2">
        <h6 className="text-xs font-semibold uppercase tracking-wider text-white/60">{title}</h6>
        <div className="grid gap-1.5 sm:grid-cols-2 xl:grid-cols-3">{filteredList.map(renderSkillRow)}</div>
      </div>
    );
  };

  return (
    <>
    <div className="rounded-xl border border-white/10 bg-black/20 p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <h4 className="text-sm font-semibold">{t("工具/技能开关", "Tools/Skills Switches")}</h4>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void onFetchSkillsConfig()}
            disabled={skillsConfigLoading || storeMutationRunning}
            className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
          >
            {skillsConfigLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            {tSlash("刷新配置 / Refresh Config")}
          </button>
          <button
            type="button"
            onClick={() => void onSaveSkillSwitches()}
            disabled={skillSwitchSaving || skillsConfigLoading || storeMutationRunning || !hasUnsavedSkillSwitchChanges}
            className="theme-accent-btn px-3 py-2 text-xs"
          >
            {skillSwitchSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Database className="h-3.5 w-3.5" />}
            {tSlash("保存开关 / Save Switches")}
          </button>
        </div>
      </div>
      {hasUnsavedSkillSwitchChanges ? (
        <p className="mt-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
          {t("你有未保存的技能开关变更，请点击“保存开关”。", "You have unsaved skill switch changes. Click \"Save Switches\".")}
        </p>
      ) : null}
      {skillsConfigError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {tSlash("配置读取/保存失败 / Config read/save failed")}: {skillsConfigError}
        </p>
      ) : null}
      {skillSwitchSaveMessage ? (
        <p className="mt-3 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
          {skillSwitchSaveMessage}
        </p>
      ) : null}
      <p className="mt-3 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs text-white/65">
        {t(
          "工具能力固定开启；技能按图片、语音、基础能力与其它分组展示。按钮只是先选择；点击“保存开关”后会提示重启，确认后系统会自动帮你重启并生效。",
          "Tool capabilities stay always on. Skills are grouped by image, audio, core capabilities, and others. Buttons only stage your choice; after Save Switches you will be prompted to restart.",
        )}
      </p>

      <div className="mt-4 space-y-4">
        <div className="rounded-xl border border-white/10 bg-[#12151f] px-4 py-3">
          <div className="flex items-center justify-between gap-3">
            <h5 className="text-sm font-semibold text-white">{t("工具/技能分组", "Tools/Skills by group")}</h5>
            <span className="theme-meta-pill !rounded-xl !px-2.5 !py-1 text-[11px]">
              {filteredManagedSkills.length}/{managedSkills.length}
            </span>
          </div>
          <p className="mt-1 text-xs leading-5 text-white/50">
            {t(
              "工具固定开启；图片、语音、基础能力与其它技能可以按需管理。新导入的技能会出现在对应分组。",
              "Tools stay always on; image, audio, core capabilities, and other skills can be managed as needed. Newly imported skills appear in the matching group.",
            )}
          </p>
          <label className="mt-3 block space-y-2">
            <span className="text-[10px] uppercase tracking-widest text-white/45">
              {t("按名称查找技能", "Find a skill by name")}
            </span>
            <input
              className="theme-input"
              value={skillsSearchQuery}
              onChange={(event) => onSkillsSearchQueryChange(event.target.value)}
              placeholder={t("例如 crypto、image、binance", "For example crypto, image, or binance")}
            />
          </label>
        </div>
        <div className="space-y-4">
          {renderSkillGroup(t("固定开启的工具", "Always-on tools"), filteredSkillsTool)}
          {renderSkillGroup(t("固定开启的基础技能", "Always-on core skills"), filteredSkillsBase)}
          {renderSkillGroup(t("图片技能", "Image skills"), filteredSkillsImage)}
          {renderSkillGroup(t("语音技能", "Voice / Audio skills"), filteredSkillsAudio)}
          {renderSkillGroup(t("多媒体技能", "Multimedia skills"), filteredSkillsMultimedia)}
          {renderSkillGroup(t("其他", "Others"), filteredSkillsOther)}
        </div>
        {normalizedSkillsSearchQuery &&
          filteredSkillsTool.length === 0 &&
          filteredSkillsImage.length === 0 &&
          filteredSkillsAudio.length === 0 &&
          filteredSkillsMultimedia.length === 0 &&
          filteredSkillsBase.length === 0 &&
          filteredSkillsOther.length === 0 ? (
          <div className="rounded-xl border border-white/10 bg-white/5 px-4 py-3 text-sm text-white/60">
            {t("没有找到匹配的技能。可以试试更短的关键词，比如 crypto、image、audio。", "No matching skills found. Try a shorter keyword like crypto, image, or audio.")}
          </div>
        ) : null}
        {managedSkills.length === 0 ? (
          <span className="text-xs text-white/50">{skillsConfigLoading ? tSlash("加载中... / Loading...") : "--"}</span>
        ) : null}
      </div>
    </div>
    {pendingRemovalName ? (
      <SkillRemovalDialog
        skillName={pendingRemovalName}
        existingConfigFiles={pendingRemovalItem?.existing_config_files}
        t={t}
        onCancel={() => setPendingRemovalName(null)}
        onConfirm={confirmRemoval}
      />
    ) : null}
    </>
  );
}
