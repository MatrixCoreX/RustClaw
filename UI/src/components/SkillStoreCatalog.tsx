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
  Radar,
  RadioTower,
  RefreshCw,
  ScrollText,
  Search,
  Sparkles,
  Trash2,
  Video,
  Volume2,
  Wrench,
  XCircle,
  type LucideIcon,
} from "lucide-react";

import { skillDescription, skillRiskLabel, skillRuntimeIssue, type UiLanguage } from "../lib/skill-display";
import {
  filterSkillStoreItems,
  latestUnresolvedSkillStoreFailure,
  skillStoreInstallState,
} from "../lib/skill-store";
import { formatUiError } from "../lib/ui-error";
import type {
  SkillStoreDependencyResponse,
  SkillStoreDependencyStatus,
  SkillStoreItem,
  SkillStoreResponse,
} from "../types/api";
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
  media_discovery: Radar,
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

const HOST_DEPENDENCY_NAMES: Record<string, readonly [string, string]> = {
  chromium: ["Chromium 浏览器", "Chromium browser"],
  ffmpeg: ["FFmpeg 音视频处理", "FFmpeg media processing"],
  git: ["Git 版本管理", "Git version control"],
  tesseract: ["Tesseract OCR", "Tesseract OCR"],
  tesseract_chi_sim: ["Tesseract 简体中文识别包", "Tesseract Simplified Chinese language data"],
};

const RUNTIME_ASSET_NAMES: Record<string, readonly [string, string]> = {
  modelscope_fsmn_vad: ["FSMN 语音活动检测模型", "FSMN voice activity detection model"],
  modelscope_sensevoice_small: ["SenseVoice Small 语音识别模型", "SenseVoice Small speech recognition model"],
};

function installDependencyLabel(
  token: string,
  labels: Record<string, readonly [string, string]>,
  t: Translate,
): string {
  const label = labels[token];
  return label ? t(label[0], label[1]) : token.replaceAll("_", " ");
}

interface DependencyCheckState {
  loading: boolean;
  data: SkillStoreDependencyResponse | null;
  error: string | null;
}

export function DependencyList({
  values,
  kind,
  labels,
  emptyLabel,
  check,
  t,
}: {
  values: readonly string[];
  kind: SkillStoreDependencyStatus["kind"];
  labels: Record<string, readonly [string, string]>;
  emptyLabel: string;
  check: DependencyCheckState | null;
  t: Translate;
}) {
  if (values.length === 0) {
    return (
      <span className="inline-flex items-center gap-1.5 text-emerald-200/80">
        <CheckCircle2 className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        {emptyLabel}
      </span>
    );
  }
  const statuses = new Map(
    (check?.data?.dependencies ?? [])
      .filter((dependency) => dependency.kind === kind)
      .map((dependency) => [dependency.id, dependency]),
  );
  return (
    <ul className="space-y-1.5">
      {values.map((dependency) => {
        const status = statuses.get(dependency);
        const installed = status?.installed === true;
        const unknown = Boolean(check?.data) && (!status || status.status_code === "unknown");
        const statusLabel = check?.loading
          ? t("检查中", "Checking")
          : check?.error || unknown
            ? t("无法确认", "Unknown")
            : installed
              ? t("已正确安装", "Installed correctly")
              : check?.data
                ? t("未安装", "Not installed")
                : t("等待检查", "Not checked");
        return (
          <li
            key={dependency}
            title={dependency}
            className="flex items-start gap-2 rounded border border-white/10 bg-white/5 px-2 py-1.5"
          >
            {check?.loading ? (
              <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin text-cyan-200" aria-hidden="true" />
            ) : installed ? (
              <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0 text-emerald-300" aria-hidden="true" />
            ) : (
              <XCircle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-300" aria-hidden="true" />
            )}
            <span className="min-w-0 flex-1 text-white/75">
              {installDependencyLabel(dependency, labels, t)}
              {status?.version ? <span className="ml-1 text-white/40">· {status.version}</span> : null}
            </span>
            <span className={installed ? "shrink-0 text-emerald-200/80" : "shrink-0 text-amber-200/80"}>
              {statusLabel}
            </span>
          </li>
        );
      })}
    </ul>
  );
}

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
  onCheckDependencies: (name: string) => Promise<SkillStoreDependencyResponse>;
  onInstall: (name: string) => unknown | Promise<unknown>;
  onRemove: (name: string, preserveConfig: boolean, preserveData: boolean) => unknown | Promise<unknown>;
  onCancel: (operationId: string) => unknown | Promise<unknown>;
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
  onCheckDependencies,
  onInstall,
  onRemove,
  onCancel,
}: SkillStoreCatalogProps) {
  const [query, setQuery] = useState("");
  const [pendingRemoval, setPendingRemoval] = useState<SkillStoreItem | null>(null);
  const [dependencyChecks, setDependencyChecks] = useState<Record<string, DependencyCheckState>>({});
  const items = useMemo(() => {
    return filterSkillStoreItems(data?.items ?? [], query);
  }, [data?.items, query]);
  const mutationRunning = actionName !== null;
  const activeOperation = data?.active_operation ?? null;
  const recentFailure = latestUnresolvedSkillStoreFailure(data?.recent_operations);

  const operationStageLabel = (stage: string) => {
    const labels: Record<string, readonly [string, string]> = {
      queued: ["等待开始", "Queued"],
      preflight: ["检查运行环境", "Checking prerequisites"],
      dependencies: ["准备独立依赖", "Preparing private dependencies"],
      build: ["准备运行文件", "Preparing runtime files"],
      smoke: ["验证技能协议", "Validating the skill protocol"],
      activate: ["安全启用新版本", "Activating the verified version"],
      configure: ["保存技能设置", "Saving skill settings"],
      remove: ["删除技能运行文件", "Removing skill runtime files"],
      rollback: ["恢复上一版本", "Restoring the previous version"],
    };
    const label = labels[stage];
    return label ? t(label[0], label[1]) : stage;
  };

  const confirmRemoval = async (preserveConfig: boolean, preserveData: boolean) => {
    if (!pendingRemoval) return;
    const name = pendingRemoval.name;
    setPendingRemoval(null);
    await onRemove(name, preserveConfig, preserveData);
  };

  const checkDependencies = async (skillName: string) => {
    setDependencyChecks((current) => ({
      ...current,
      [skillName]: { loading: true, data: current[skillName]?.data ?? null, error: null },
    }));
    try {
      const checked = await onCheckDependencies(skillName);
      setDependencyChecks((current) => ({
        ...current,
        [skillName]: { loading: false, data: checked, error: null },
      }));
    } catch (error) {
      setDependencyChecks((current) => ({
        ...current,
        [skillName]: {
          loading: false,
          data: current[skillName]?.data ?? null,
          error: formatUiError(error, t, "依赖状态检查失败。", "Dependency check failed."),
        },
      }));
    }
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
            {skillDescription(lang, item.description, item.description_zh)}
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
              "技能设置仍在，但运行文件缺失。修复安装会重新验证运行文件，并继续使用原有配置。",
              "The skill settings remain, but its runtime files are missing. Repair verifies them again and keeps the existing configuration.",
            )}
          </p>
        ) : null}
        <details
          className="mt-3 rounded border border-white/10 bg-white/[0.03] px-3 py-2 text-[11px] text-white/50"
          onToggle={(event) => {
            if (event.currentTarget.open) void checkDependencies(item.name);
          }}
        >
          <summary className="cursor-pointer text-white/60">{t("安装信息", "Install details")}</summary>
          <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
            <dt>{t("安装适配器", "Install adapter")}</dt>
            <dd className="break-all text-white/75">{item.build_adapter ?? t("无需构建", "No build")}</dd>
            <dt>{t("支持系统", "Platforms")}</dt>
            <dd className="break-all text-white/75">{item.supported_os?.join(", ") || t("跟随运行环境", "Runtime default")}</dd>
            <dt>{t("可用架构", "Architectures")}</dt>
            <dd className="break-all text-white/75">{item.supported_arch?.join(", ") || t("当前架构", "Current architecture")}</dd>
            <dt>{t("版本", "Version")}</dt>
            <dd className="break-all text-white/75">{item.installed_version ?? item.package_version ?? "—"}</dd>
            <dt>{t("安装联网", "Install network")}</dt>
            <dd className="break-all text-white/75">
              {item.build_network_policy === "approval_required"
                ? t("需要你确认", "Requires your approval")
                : t("不联网", "Offline")}
            </dd>
            <dt>{t("系统工具", "System tools")}</dt>
            <dd>
              <DependencyList
                values={item.host_dependencies ?? []}
                kind="host"
                labels={HOST_DEPENDENCY_NAMES}
                emptyLabel={t("无需额外安装", "No additional installation")}
                check={dependencyChecks[item.name] ?? null}
                t={t}
              />
            </dd>
            <dt>{t("本地模型/资源", "Local models/assets")}</dt>
            <dd>
              <DependencyList
                values={item.runtime_assets ?? []}
                kind="runtime_asset"
                labels={RUNTIME_ASSET_NAMES}
                emptyLabel={t("无需额外下载", "No additional download")}
                check={dependencyChecks[item.name] ?? null}
                t={t}
              />
            </dd>
          </dl>
          {dependencyChecks[item.name]?.error ? (
            <p className="mt-2 text-amber-200/80">{dependencyChecks[item.name].error}</p>
          ) : null}
        </details>
        {!item.installed && item.build_network_policy === "approval_required" ? (
          <p className="mt-3 text-xs leading-5 text-amber-200/85">
            {t(
              "安装会联网获取已声明的依赖或验证远程端点；点击安装后仍会再次确认。",
              "Installation accesses the network for declared dependencies or endpoint validation; you will be asked to confirm.",
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
            {t(
              "安装、删除或重新安装可选技能。当前平台有预编译版本时会直接验证并启用；没有匹配版本时才单独构建这个技能。",
              "Install, remove, or reinstall optional skills. {product_name} verifies and activates a matching platform precompile when available, and builds only this skill when no compatible precompile exists.",
            )}
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
        <div className="mt-4 rounded border border-amber-500/25 bg-amber-500/10 px-3 py-3 text-sm text-amber-100">
          <div className="flex items-center gap-2">
            <Loader2 className="h-4 w-4 shrink-0 animate-spin" aria-hidden="true" />
            <span className="min-w-0 flex-1">
              {activeOperation.skill_name}：{operationStageLabel(activeOperation.stage)}
            </span>
            <button
              type="button"
              onClick={() => void onCancel(activeOperation.operation_id)}
              disabled={activeOperation.cancel_requested}
              className="inline-flex items-center gap-1 rounded border border-amber-200/25 px-2 py-1 text-xs hover:bg-amber-100/10 disabled:cursor-not-allowed disabled:opacity-50"
            >
              <XCircle className="h-3.5 w-3.5" />
              {activeOperation.cancel_requested ? t("正在取消", "Cancelling") : t("取消", "Cancel")}
            </button>
          </div>
          <p className="mt-1 pl-6 text-xs text-amber-100/70">
            {t("可以离开或刷新此页面，任务状态会保留。", "You can leave or refresh this page; the operation state is preserved.")}
          </p>
        </div>
      ) : null}
      {message ? <p className="mt-4 border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200 rounded">{message}</p> : null}
      {recentFailure?.failure?.diagnostic ? (
        <details className="mt-3 rounded border border-white/10 bg-white/[0.03] px-3 py-2 text-xs text-white/55">
          <summary className="cursor-pointer text-white/65">{t("诊断信息", "Diagnostics")}</summary>
          <p className="mt-2 text-white/60">
            {recentFailure.skill_name} · {t("诊断编号", "Diagnostic code")}: {recentFailure.failure.error_code}
          </p>
          <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-black/20 p-2 text-[11px] text-white/55">
            {recentFailure.failure.diagnostic}
          </pre>
        </details>
      ) : null}
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
