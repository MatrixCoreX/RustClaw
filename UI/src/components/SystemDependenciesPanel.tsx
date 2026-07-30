import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Download,
  Loader2,
  PackageCheck,
  RefreshCw,
  Wrench,
} from "lucide-react";

import type {
  DependencyInstallOperation,
  HostDependenciesSnapshot,
  HostDependencyStatus,
} from "../types/api";
import { useUiDialog } from "./UiDialogProvider";

type Translate = (zh: string, en: string) => string;

interface SystemDependenciesPanelProps {
  t: Translate;
  snapshot: HostDependenciesSnapshot | null;
  loading: boolean;
  errorCode: string | null;
  isAdmin: boolean;
  installingId: string | null;
  onRefresh: () => unknown | Promise<unknown>;
  onInstall: (dependencyId: string) => unknown | Promise<unknown>;
}

const DEPENDENCY_NAMES: Record<string, [string, string]> = {
  bash: ["Bash 脚本环境", "Bash scripting environment"],
  tar: ["归档解包工具", "Archive extraction tool"],
  git: ["Git 版本管理", "Git version control"],
  curl: ["网络下载工具", "Network transfer tool"],
  python3: ["Python 运行环境", "Python runtime"],
  sandbox_backend: ["安全执行沙箱", "Secure execution sandbox"],
  process_tools: ["系统进程工具", "System process tools"],
  rustc: ["Rust 编译器", "Rust compiler"],
  cargo: ["Cargo 构建工具", "Cargo build tool"],
  clang: ["Clang 编译器", "Clang compiler"],
  libclang: ["LLVM / libclang 工具链", "LLVM / libclang toolchain"],
  cmake: ["CMake 构建工具", "CMake build tool"],
  pkg_config: ["原生依赖查询工具", "Native dependency lookup"],
  protoc: ["Protobuf 编译器", "Protobuf compiler"],
  make: ["Make 构建工具", "Make build tool"],
  node: ["Node.js 运行环境", "Node.js runtime"],
  npm: ["NPM 包管理器", "NPM package manager"],
  npx: ["NPX 包执行器", "NPX package runner"],
  browser_playwright: ["浏览器自动化组件", "Browser automation package"],
  go: ["Go 开发工具链", "Go development toolchain"],
  ripgrep: ["快速文件搜索", "Fast file search"],
  zip: ["ZIP 打包工具", "ZIP archive writer"],
  unzip: ["ZIP 解包工具", "ZIP archive reader"],
  pdf_tools: ["PDF 文本工具", "PDF text tools"],
  lsof: ["端口与进程诊断", "Port and process diagnostics"],
  ffmpeg: ["音视频处理", "Audio and video processing"],
  docker: ["Docker 容器", "Docker containers"],
  chromium: ["浏览器自动化引擎", "Browser automation engine"],
  libreoffice: ["Office 文档渲染", "Office document rendering"],
  nginx: ["Web 服务器", "Web server"],
  rsync: ["部署文件同步", "Deployment file sync"],
};

const CAPABILITY_NAMES: Record<string, [string, string]> = {
  workspace: ["工作区操作", "Workspace operations"],
  system_update: ["系统更新", "System updates"],
  http_basic: ["网页请求", "Web requests"],
  runtime_scripts: ["运行脚本", "Runtime scripts"],
  nni: ["NNI", "NNI"],
  source_build: ["源码编译", "Source builds"],
  skill_store: ["Skill Store", "Skill Store"],
  native_bindings: ["原生组件", "Native components"],
  agent_tools: ["Agent 内置工具", "Built-in agent tools"],
  skill_runtime: ["技能运行时", "Skill runtime"],
  ui_build: ["UI 编译", "UI builds"],
  fs_search: ["文件搜索", "File search"],
  code_index: ["代码索引", "Code indexing"],
  install_module: ["模块安装", "Module installation"],
  archive_basic: ["归档工具", "Archive tools"],
  doc_parse: ["文档解析", "Document parsing"],
  process_basic: ["进程工具", "Process tools"],
  health_check: ["健康检查", "Health checks"],
  audio_transcribe: ["语音转写", "Audio transcription"],
  video_generate: ["视频生成", "Video generation"],
  music_generate: ["音乐生成", "Music generation"],
  docker_basic: ["Docker 工具", "Docker tools"],
  browser_web: ["浏览器工具", "Browser tools"],
  office_workspace: ["Office 工具", "Office tools"],
  web_entry: ["Web 入口", "Web entry"],
  deployment: ["部署", "Deployment"],
};

const CATEGORY_NAMES: Record<string, [string, string]> = {
  runtime: ["运行环境", "Runtime"],
  build: ["源码构建", "Source build"],
  tool: ["内置工具", "Built-in tool"],
  skill: ["技能依赖", "Skill dependency"],
  optional: ["可选组件", "Optional component"],
};

function localizedToken(t: Translate, token: string, labels: Record<string, [string, string]>): string {
  const label = labels[token];
  return label ? t(label[0], label[1]) : token;
}

function dependencyErrorLabel(t: Translate, code: string | null): string {
  switch (code) {
    case "permission_denied":
      return t("当前账号无权检查系统依赖。", "This account cannot inspect system dependencies.");
    case "disconnected":
      return t("暂时无法连接 {product_name}。", "{product_name} is temporarily unreachable.");
    case "package_manager_unavailable":
      return t("未检测到受支持的系统包管理器，请手动安装依赖。", "No supported system package manager was detected. Install the dependency manually.");
    case "dependency_install_unsupported":
      return t("当前平台不支持自动安装这一项，请按系统方式手动配置。", "Automatic installation is not supported for this dependency on the current platform.");
    case "dependency_install_admin_required":
      return t("只有管理员可以安装系统依赖。", "Only administrators can install system dependencies.");
    case "dependency_already_installed":
      return t("该依赖已经可用，请刷新检查结果。", "This dependency is already available. Refresh the check result.");
    case "install_failed":
      return t("无法启动安装，请刷新状态后重试。", "The install could not be started. Refresh and try again.");
    default:
      return t("依赖状态暂不可用，请稍后刷新。", "Dependency status is unavailable. Refresh shortly.");
  }
}

function operationErrorLabel(t: Translate, operation: DependencyInstallOperation | undefined): string | null {
  if (!operation || operation.status !== "failed") return null;
  switch (operation.error_code) {
    case "dependency_still_missing":
      return t("安装命令已结束，但仍未检测到该依赖。", "The install command finished, but the dependency is still missing.");
    case "package_manager_launch_failed":
      return t("无法启动系统包管理器。", "The system package manager could not be started.");
    default:
      return t("安装失败，可展开日志查看系统返回。", "Installation failed. Expand the log for system output.");
  }
}

function DependencyRow({
  t,
  dependency,
  operation,
  isAdmin,
  starting,
  onInstall,
}: {
  t: Translate;
  dependency: HostDependencyStatus;
  operation: DependencyInstallOperation | undefined;
  isAdmin: boolean;
  starting: boolean;
  onInstall: (dependencyId: string) => unknown | Promise<unknown>;
}) {
  const { confirm } = useUiDialog();
  const active = operation?.status === "queued" || operation?.status === "running";
  const operationError = operationErrorLabel(t, operation);
  const capabilityLabel = dependency.used_by
    .map((token) => localizedToken(t, token, CAPABILITY_NAMES))
    .join(t("、", ", "));

  const install = async () => {
    const accepted = await confirm({
      title: t("安装系统依赖", "Install system dependency"),
      message: t(
        `将使用 ${dependency.package_manager ?? "系统包管理器"} 安装“${localizedToken(t, dependency.id, DEPENDENCY_NAMES)}”。安装可能需要几分钟，期间可以离开本页面。`,
        `{product_name} will use ${dependency.package_manager ?? "the system package manager"} to install “${localizedToken(t, dependency.id, DEPENDENCY_NAMES)}”. This may take several minutes, and you may leave this page while it runs.`,
      ),
      confirmLabel: t("开始安装", "Install"),
    });
    if (accepted) await onInstall(dependency.id);
  };

  return (
    <div className="border-t border-white/8 py-3 first:border-t-0">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            {dependency.installed ? (
              <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-300" />
            ) : (
              <AlertTriangle className={`h-4 w-4 shrink-0 ${dependency.required ? "text-rose-300" : "text-amber-300"}`} />
            )}
            <p className="text-sm font-medium text-white/92">
              {localizedToken(t, dependency.id, DEPENDENCY_NAMES)}
            </p>
            {dependency.required ? (
              <span className="theme-status-pill text-[10px]">{t("系统必需", "Required")}</span>
            ) : (
              <span className="theme-status-pill text-[10px]">
                {localizedToken(t, dependency.category, CATEGORY_NAMES)}
              </span>
            )}
          </div>
          <p className="mt-1.5 break-words text-xs leading-5 text-white/52">
            {dependency.installed
              ? dependency.version || t("已检测到", "Detected")
              : t("未检测到", "Not detected")}
          </p>
          <p className="mt-1 text-xs leading-5 text-white/42">
            {t("用于：", "Used by: ")}{capabilityLabel || t("系统功能", "System features")}
          </p>
          {operationError ? <p className="mt-2 text-xs text-rose-200">{operationError}</p> : null}
          {operation?.status === "succeeded" ? (
            <p className="mt-2 text-xs text-emerald-200">{t("安装完成，状态已重新检查。", "Installed and rechecked.")}</p>
          ) : null}
          {operation?.log_tail && operation.status === "failed" ? (
            <details className="mt-2 text-xs text-white/55">
              <summary className="cursor-pointer">{t("查看安装日志", "View install log")}</summary>
              <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-black/20 p-3 text-[11px] leading-5">
                {operation.log_tail}
              </pre>
            </details>
          ) : null}
        </div>
        {!dependency.installed && isAdmin && dependency.installable ? (
          <button
            type="button"
            onClick={() => void install()}
            disabled={starting || active}
            className="theme-secondary-btn px-3 py-2 text-xs"
          >
            {starting || active ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
            {active ? t("安装中", "Installing") : t("安装", "Install")}
          </button>
        ) : null}
        {!dependency.installed && !dependency.installable ? (
          <span className="text-xs text-white/40">{t("需手动配置", "Manual setup")}</span>
        ) : null}
      </div>
    </div>
  );
}

export function SystemDependenciesPanel({
  t,
  snapshot,
  loading,
  errorCode,
  isAdmin,
  installingId,
  onRefresh,
  onInstall,
}: SystemDependenciesPanelProps) {
  const [expanded, setExpanded] = useState(() => (snapshot?.summary.missing_required ?? 0) > 0);
  useEffect(() => {
    if ((snapshot?.summary.missing_required ?? 0) > 0) setExpanded(true);
  }, [snapshot?.summary.missing_required]);

  const operationsByDependency = useMemo(() => {
    const operations = [...(snapshot?.operations ?? [])].sort(
      (left, right) => (right.started_ts ?? 0) - (left.started_ts ?? 0),
    );
    return new Map(operations.map((operation) => [operation.dependency_id, operation]));
  }, [snapshot?.operations]);
  const dependencies = useMemo(
    () => [...(snapshot?.dependencies ?? [])].sort((left, right) => Number(left.installed) - Number(right.installed)),
    [snapshot?.dependencies],
  );

  return (
    <section className="theme-panel-soft px-4 py-4 sm:px-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <span className="rounded-lg bg-cyan-300/10 p-2 text-cyan-200">
            <PackageCheck className="h-5 w-5" />
          </span>
          <div>
            <p className="theme-kicker text-[10px] uppercase">{t("运行环境", "Environment")}</p>
            <h3 className="mt-1.5 text-base font-semibold text-white">{t("系统依赖检查", "System dependency check")}</h3>
            <p className="mt-1.5 text-xs leading-5 text-white/52">
              {snapshot
                ? t(
                    `已检测 ${snapshot.summary.total} 项，${snapshot.summary.installed} 项可用；系统必需缺失 ${snapshot.summary.missing_required} 项，可选能力缺失 ${snapshot.summary.missing_optional} 项。`,
                    `${snapshot.summary.total} checked and ${snapshot.summary.installed} available; ${snapshot.summary.missing_required} required and ${snapshot.summary.missing_optional} optional dependencies are missing.`,
                  )
                : t("检查 {product_name}、内置工具和技能所需的本机依赖。", "Check local dependencies used by {product_name}, built-in tools, and skills.")}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={loading}
            className="theme-icon-btn"
            title={t("重新检查", "Check again")}
            aria-label={t("重新检查", "Check again")}
          >
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          </button>
          <button
            type="button"
            onClick={() => setExpanded((value) => !value)}
            className="theme-secondary-btn px-3 py-2 text-xs"
            aria-expanded={expanded}
          >
            {expanded ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            {expanded ? t("收起", "Collapse") : t("查看详情", "View details")}
          </button>
        </div>
      </div>

      {errorCode ? (
        <div className="mt-4 flex items-start gap-2 rounded-lg border border-amber-300/20 bg-amber-300/[0.06] px-3 py-2 text-xs text-amber-100/85">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <span>{dependencyErrorLabel(t, errorCode)}</span>
        </div>
      ) : null}

      {expanded && snapshot ? (
        <div className="mt-4 border-t border-white/8 pt-3">
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-white/45">
            <span>{t("缺失项优先显示", "Missing items are shown first")}</span>
            <span>{t("包管理器：", "Package manager: ")}{snapshot.package_manager ?? t("未检测到", "Not detected")}</span>
          </div>
          {dependencies.map((dependency) => (
            <DependencyRow
              key={dependency.id}
              t={t}
              dependency={dependency}
              operation={operationsByDependency.get(dependency.id)}
              isAdmin={isAdmin}
              starting={installingId === dependency.id}
              onInstall={onInstall}
            />
          ))}
          {!isAdmin ? (
            <div className="mt-3 flex items-start gap-2 rounded-lg bg-white/[0.035] px-3 py-2 text-xs leading-5 text-white/52">
              <Wrench className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{t("管理员可以在这里安装支持自动处理的缺失依赖。", "Administrators can install supported missing dependencies here.")}</span>
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
