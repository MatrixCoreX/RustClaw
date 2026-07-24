import { useMemo, useState } from "react";
import {
  Bitcoin,
  BookOpen,
  Boxes,
  CheckCircle2,
  CloudSun,
  Code2,
  Container,
  Database,
  Download,
  FileSearch,
  Globe2,
  HeartPulse,
  Image,
  Images,
  LineChart,
  ListChecks,
  Loader2,
  MapPin,
  Music2,
  PackagePlus,
  Puzzle,
  RadioTower,
  RefreshCw,
  ScrollText,
  Search,
  Sparkles,
  Trash2,
  Video,
  Volume2,
  Wrench,
  type LucideIcon,
} from "lucide-react";

import { skillDescription, skillRiskLabel, skillRuntimeIssue, type UiLanguage } from "../lib/skill-display";
import { filterSkillStoreItems, skillStoreInstallState } from "../lib/skill-store";
import type { SkillStoreItem, SkillStoreResponse } from "../types/api";
import { SkillRemovalDialog } from "./SkillRemovalDialog";

type Translate = (zh: string, en: string) => string;

const SKILL_ICONS: Record<string, LucideIcon> = {
  browser_web: Globe2,
  crypto: Bitcoin,
  db_basic: Database,
  doc_parse: FileSearch,
  docker_basic: Container,
  health_check: HeartPulse,
  http_basic: RadioTower,
  image_edit: Image,
  image_generate: Sparkles,
  image_vision: Images,
  install_module: PackagePlus,
  invest_copy: BookOpen,
  log_analyze: ScrollText,
  map_merchant: MapPin,
  music_generate: Music2,
  package_manager: Boxes,
  photo_organize: Images,
  stock: LineChart,
  task_control: ListChecks,
  transform: RefreshCw,
  video_generate: Video,
  weather: CloudSun,
  web_search_extract: Search,
  x: Code2,
};

function skillStoreIcon(name: string): LucideIcon {
  if (SKILL_ICONS[name]) return SKILL_ICONS[name];
  if (name.startsWith("audio_")) return Volume2;
  if (name.startsWith("image_")) return Image;
  if (name.startsWith("video_")) return Video;
  if (name.startsWith("music_")) return Music2;
  return Puzzle;
}

export interface SkillStoreCatalogProps {
  lang: UiLanguage;
  t: Translate;
  data: SkillStoreResponse | null;
  loading: boolean;
  error: string | null;
  message: string | null;
  actionName: string | null;
  onRefresh: () => unknown | Promise<unknown>;
  onInstall: (name: string) => unknown | Promise<unknown>;
  onRemove: (name: string, preserveConfig: boolean, preserveData: boolean) => unknown | Promise<unknown>;
}

export function SkillStoreCatalog({
  lang,
  t,
  data,
  loading,
  error,
  message,
  actionName,
  onRefresh,
  onInstall,
  onRemove,
}: SkillStoreCatalogProps) {
  const [query, setQuery] = useState("");
  const [pendingRemoval, setPendingRemoval] = useState<SkillStoreItem | null>(null);
  const items = useMemo(() => {
    return filterSkillStoreItems(data?.items ?? [], query);
  }, [data?.items, query]);
  const mutationRunning = actionName !== null;
  const activeOperation = data?.active_operation ?? null;

  const confirmRemoval = async (preserveConfig: boolean, preserveData: boolean) => {
    if (!pendingRemoval) return;
    const name = pendingRemoval.name;
    setPendingRemoval(null);
    await onRemove(name, preserveConfig, preserveData);
  };

  const renderItem = (item: SkillStoreItem) => {
    const Icon = skillStoreIcon(item.name);
    const runtimeIssue = skillRuntimeIssue(item.skill, lang);
    const actionRunning = actionName === item.name;
    const repairRequired = skillStoreInstallState(item) === "repair_required";
    return (
      <article key={item.name} className="flex min-h-56 flex-col border border-white/10 bg-[#12151f] p-4 shadow-sm rounded-lg">
        <div className="flex items-start justify-between gap-3">
          <span className="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-cyan-400/20 bg-cyan-400/10 text-cyan-100">
            <Icon className="h-5 w-5" aria-hidden="true" />
          </span>
          <span
            className={
              item.installed
                ? "inline-flex items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-[11px] text-emerald-100"
                : "inline-flex items-center gap-1 rounded-full border border-white/15 bg-white/5 px-2 py-1 text-[11px] text-white/55"
            }
          >
            {item.installed ? <CheckCircle2 className="h-3 w-3" /> : null}
            {item.installed
              ? t("已安装", "Installed")
              : repairRequired
                ? t("需要修复", "Repair needed")
                : t("未安装", "Not installed")}
          </span>
        </div>
        <div className="mt-3 min-w-0">
          <h3 className="break-words text-sm font-semibold text-white/90">{item.name}</h3>
          <p className="mt-1 min-h-10 text-xs leading-5 text-white/55">
            {skillDescription(lang, item.description)}
          </p>
        </div>
        <div className="mt-3 flex flex-wrap gap-1.5 text-[10px]">
          <span className="rounded border border-white/10 bg-white/5 px-2 py-1 text-white/45">
            {item.source_kind === "third_party"
              ? t("第三方", "Third party")
              : item.source_kind === "bundled_optional"
                ? t("可选内建", "Optional bundled")
                : t("核心内建", "Core bundled")}
          </span>
          {item.group ? (
            <span className="rounded border border-white/10 bg-white/5 px-2 py-1 text-white/45">{item.group}</span>
          ) : null}
          <span className="rounded border border-white/10 bg-white/5 px-2 py-1 text-white/45">
            {skillRiskLabel(item.skill.risk_level, lang)}
          </span>
        </div>
        {runtimeIssue && item.installed ? (
          <p className="mt-3 text-xs leading-5 text-amber-200/85">{runtimeIssue}</p>
        ) : null}
        {repairRequired ? (
          <p className="mt-3 text-xs leading-5 text-amber-200/85">
            {t(
              "技能设置仍在，但运行文件缺失。修复安装会重新编译并继续使用原有配置。",
              "The skill settings remain, but its runner is missing. Repairing recompiles it and keeps the existing configuration.",
            )}
          </p>
        ) : null}
        <div className="mt-auto pt-4">
          {item.installed ? (
            <button
              type="button"
              onClick={() => setPendingRemoval(item)}
              disabled={mutationRunning}
              className="inline-flex w-full items-center justify-center gap-2 rounded border border-red-500/25 bg-red-500/10 px-3 py-2 text-xs font-medium text-red-100 hover:bg-red-500/15 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {actionRunning ? <Loader2 className="h-4 w-4 animate-spin" /> : <Trash2 className="h-4 w-4" />}
              {actionRunning ? t("正在删除…", "Removing…") : t("删除", "Remove")}
            </button>
          ) : (
            <button
              type="button"
              onClick={() => void onInstall(item.name)}
              disabled={mutationRunning}
              className="theme-accent-btn w-full justify-center px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
            >
              {actionRunning ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : repairRequired ? (
                <Wrench className="h-4 w-4" />
              ) : (
                <Download className="h-4 w-4" />
              )}
              {actionRunning
                ? t("正在安装…", "Installing…")
                : repairRequired
                  ? t("修复安装", "Repair install")
                  : t("安装", "Install")}
            </button>
          )}
        </div>
      </article>
    );
  };

  return (
    <div>
      <div className="flex flex-col gap-3 border-b border-white/10 pb-4 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h2 className="text-base font-semibold text-white">Skill Store</h2>
          <p className="mt-1 text-sm text-white/55">
            {t("安装、删除或重新安装可选技能。", "Install, remove, or reinstall optional skills.")}
          </p>
        </div>
        <div className="flex min-w-0 gap-2 sm:w-auto">
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            className="theme-input min-w-0 sm:w-64"
            placeholder={t("搜索技能", "Search skills")}
            aria-label={t("搜索 Skill Store", "Search Skill Store")}
          />
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={loading}
            className="theme-topbar-btn h-10 w-10 shrink-0 justify-center p-0 disabled:cursor-not-allowed disabled:opacity-50"
            title={t("刷新 Skill Store", "Refresh Skill Store")}
            aria-label={t("刷新 Skill Store", "Refresh Skill Store")}
          >
            {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          </button>
        </div>
      </div>
      {error ? <p className="mt-4 border border-red-500/25 bg-red-500/10 px-3 py-2 text-sm text-red-200 rounded">{error}</p> : null}
      {activeOperation ? (
        <p className="mt-4 flex items-center gap-2 rounded border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-sm text-amber-100">
          <Loader2 className="h-4 w-4 shrink-0 animate-spin" aria-hidden="true" />
          {activeOperation.action === "install"
            ? t(
                `${activeOperation.skill_name} 正在编译并安装，刷新页面后仍会继续显示进度。`,
                `${activeOperation.skill_name} is compiling and installing. Its progress remains visible after a page refresh.`,
              )
            : t(
                `${activeOperation.skill_name} 正在删除，请稍候。`,
                `${activeOperation.skill_name} is being removed. Please wait.`,
              )}
        </p>
      ) : null}
      {message ? <p className="mt-4 border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200 rounded">{message}</p> : null}
      <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-3">{items.map(renderItem)}</div>
      {!loading && items.length === 0 ? (
        <p className="mt-4 border border-white/10 bg-white/5 px-4 py-6 text-center text-sm text-white/50 rounded-lg">
          {t("没有找到匹配的技能。", "No matching skills found.")}
        </p>
      ) : null}
      {pendingRemoval ? (
        <SkillRemovalDialog
          skillName={pendingRemoval.name}
          existingConfigFiles={pendingRemoval.existing_config_files}
          storageKind={pendingRemoval.storage_kind}
          privateDataState={pendingRemoval.private_data_state}
          t={t}
          onCancel={() => setPendingRemoval(null)}
          onConfirm={confirmRemoval}
        />
      ) : null}
    </div>
  );
}
