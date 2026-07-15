import type { WorkspaceUpdateStatus } from "../types/api";

export type UiLanguage = "zh" | "en";

export interface WorkspaceUpdateNotice {
  tone: "info" | "success" | "error";
  title: string;
  detail: string;
}

export interface WorkspaceUpdateView {
  restarting: boolean;
  running: boolean;
  hasRemoteDiff: boolean;
  knownUpToDate: boolean;
  displayStatus?: string;
  progressPercent: number;
  progressActive: boolean;
  progressLabel: string;
  logPreview: string;
  notice: WorkspaceUpdateNotice | null;
}

function copy(lang: UiLanguage, zh: string, en: string): string {
  return lang === "zh" ? zh : en;
}

export function formatWorkspaceUpdateStep(step: string | null | undefined, lang: UiLanguage): string {
  const labels: Record<string, string> = {
    idle: copy(lang, "空闲", "Idle"),
    starting: copy(lang, "准备更新", "Preparing update"),
    checking_current_version: copy(lang, "检查当前版本", "Checking current version"),
    checking_remote_version: copy(lang, "检查远端版本", "Checking remote version"),
    already_latest: copy(lang, "已经是最新版本", "Already latest"),
    pulling_latest_code: copy(lang, "拉取远端版本", "Pulling remote version"),
    resolving_conflicting_files: copy(lang, "只覆盖冲突文件", "Overwriting conflicts only"),
    skipping_pull_latest_code: copy(lang, "远端无新版本，继续编译", "No remote changes, building"),
    checking_new_version: copy(lang, "确认新版本", "Checking new version"),
    building_workspace: copy(lang, "正在完整编译", "Running full build"),
    building_ui: copy(lang, "正在编译 UI", "Building UI"),
    ui_build_succeeded: copy(lang, "UI 编译完成", "UI build completed"),
    building_clawd: copy(lang, "正在编译 clawd", "Building clawd"),
    downloading_release: copy(lang, "正在下载 Release 包", "Downloading Release package"),
    deploying_release: copy(lang, "正在部署 Release 包", "Deploying Release package"),
    cancel_requested: copy(lang, "正在停止编译", "Stopping build"),
    canceled: copy(lang, "已停止", "Stopped"),
    restarting_clawd: copy(lang, "正在安排重启", "Scheduling restart"),
    restart_scheduled: copy(lang, "已安排重启", "Restart scheduled"),
    clawd_restart_scheduled: copy(lang, "clawd 已安排重启", "clawd restart scheduled"),
    release_restart_scheduled: copy(lang, "Release 已部署，正在重启", "Release deployed, restarting"),
  };
  return labels[step || ""] || step || "--";
}

export function formatWorkspaceUpdateStatus(
  status: string | null | undefined,
  mode: WorkspaceUpdateStatus["mode"] | undefined,
  lang: UiLanguage,
): string {
  if (status === "running") {
    if (mode === "ui_only" || mode === "clawd_only") return copy(lang, "编译中", "Building");
    if (mode === "release_deploy") return copy(lang, "部署中", "Deploying");
    return copy(lang, "更新中", "Updating");
  }
  if (status === "restarting") return copy(lang, "重启中", "Restarting");
  if (status === "up_to_date") return copy(lang, "已是最新", "Up to date");
  if (status === "succeeded") return copy(lang, "已完成", "Completed");
  if (status === "failed") return copy(lang, "失败", "Failed");
  if (status === "canceled") return copy(lang, "已停止", "Stopped");
  return copy(lang, "未运行", "Idle");
}

export function formatWorkspaceUpdateApiError(error: string | null | undefined, lang: UiLanguage): string {
  const code = error?.trim();
  const labels: Record<string, string> = {
    workspace_update_admin_required: copy(lang, "只有管理员可以执行这个操作。", "Only an admin can perform this action."),
    workspace_update_already_running: copy(lang, "更新已经在进行中。", "An update is already running."),
    workspace_update_not_running: copy(lang, "当前没有正在运行的更新。", "No update is currently running."),
  };
  return code ? labels[code] || code : copy(lang, "未知错误", "Unknown error");
}

export function formatWorkspaceUpdateNextStep(
  status: WorkspaceUpdateStatus | null | undefined,
  lang: UiLanguage,
): string | null {
  const key = status?.next_step_key?.trim();
  if (!key) return status?.next_step || null;
  const args = status?.next_step_args ?? {};
  const count = numberArg(args, "count");
  const labels: Record<string, string> = {
    "workspace_update.cancel_requested": copy(lang, "正在停止当前编译进程。", "Stopping the current build process."),
    "workspace_update.invalid_git_repo": copy(lang, "请确认 RustClaw 目录是有效 Git 仓库。", "Confirm the RustClaw directory is a valid Git repository."),
    "workspace_update.git_unavailable": copy(lang, "请确认当前用户可以在 RustClaw 目录中运行 git。", "Confirm the current user can run git in the RustClaw directory."),
    "workspace_update.remote_fetch_required_failed": copy(
      lang,
      "更新要求以远端为准；远端检查失败时不会继续编译本地代码。请确认网络、Git remote 和 SSH key 后重试。",
      "Updates require the remote state first. Local code will not be built when the remote check fails. Confirm network, Git remote, and SSH key, then retry.",
    ),
    "workspace_update.upstream_missing": copy(
      lang,
      "未能读取 upstream，无法确认远端目标版本。请确认当前分支已设置 upstream 后重试。",
      "The upstream branch could not be read, so the target remote version cannot be confirmed. Set the upstream branch, then retry.",
    ),
    "workspace_update.pull_conflict_detection_failed": copy(
      lang,
      "拉取失败，且无法可靠识别冲突文件；已保留本地文件，请手动处理后重试。",
      "Pull failed and conflicting files could not be identified reliably. Local files were preserved; resolve manually, then retry.",
    ),
    "workspace_update.pull_failed_no_conflicts": copy(
      lang,
      "拉取失败，但没有发现远端变更与本地未提交文件的直接冲突；已保留本地文件，请手动检查分支是否分叉或权限是否正常。",
      "Pull failed, but no direct conflict was found between remote changes and local uncommitted files. Local files were preserved; check branch divergence or permissions manually.",
    ),
    "workspace_update.conflict_overwrite_failed": copy(
      lang,
      "覆盖冲突文件失败；未冲突的本地文件已保持不动，请手动处理后重试。",
      "Overwriting conflicting files failed. Non-conflicting local files were left unchanged; resolve manually, then retry.",
    ),
    "workspace_update.conflicts_overwritten_retrying_pull": copy(
      lang,
      `已只覆盖 ${count ?? 0} 个冲突路径；其他本地改动和额外文件保持不动，正在重新拉取远端。`,
      `Only ${count ?? 0} conflicting path(s) were overwritten. Other local changes and extra files were left unchanged; pulling remote again.`,
    ),
    "workspace_update.pull_failed_after_conflict_overwrite": copy(
      lang,
      "已覆盖识别到的冲突文件，但重新拉取仍失败；其他本地文件未动，请查看 Git 输出后手动处理。",
      "Detected conflict files were overwritten, but pulling again still failed. Other local files were untouched; inspect Git output and resolve manually.",
    ),
    "workspace_update.pull_failed_preserved": copy(
      lang,
      "拉取远端失败；已保留本地文件，请确认 Git 和网络状态后重试。",
      "Pulling remote failed. Local files were preserved; confirm Git and network state, then retry.",
    ),
    "workspace_update.no_remote_changes_building": copy(
      lang,
      "远端没有新版本；本地文件保持不动，将继续执行完整编译。",
      "No newer remote version was found. Local files remain unchanged; the full build will continue.",
    ),
    "workspace_update.build_logs_refreshing": copy(lang, "正在编译，编译日志会持续刷新。", "Building. Build logs will keep refreshing."),
    "workspace_update.full_build_failed": copy(
      lang,
      "请查看构建日志摘要；修复依赖或编译错误后再重试。",
      "Check the build log summary. Fix dependency or compile errors, then retry.",
    ),
    "workspace_update.full_build_dependency_check": copy(
      lang,
      "请确认服务器依赖完整，并查看构建日志。",
      "Confirm server dependencies are complete and inspect the build logs.",
    ),
    "workspace_update.restart_wait": copy(
      lang,
      "RustClaw 正在重启，请等待 10-20 秒后刷新页面。",
      "RustClaw is restarting. Wait 10-20 seconds, then refresh the page.",
    ),
    "workspace_update.full_restart_failed": copy(
      lang,
      "构建已完成，但自动重启失败。请在服务器上手动重启 clawd。",
      "The build completed, but automatic restart failed. Restart clawd manually on the server.",
    ),
    "workspace_update.ui_build_failed": copy(
      lang,
      "请查看 UI 编译日志；修复依赖或编译错误后再重试。",
      "Check the UI build log. Fix dependency or compile errors, then retry.",
    ),
    "workspace_update.ui_dependency_check": copy(
      lang,
      "请确认 UI 依赖完整，并查看编译日志。",
      "Confirm UI dependencies are complete and inspect the build logs.",
    ),
    "workspace_update.clawd_build_failed": copy(
      lang,
      "请查看 clawd 编译日志；修复 Rust 编译错误后再重试。",
      "Check the clawd build log. Fix Rust compile errors, then retry.",
    ),
    "workspace_update.clawd_dependency_check": copy(
      lang,
      "请确认 Rust 依赖完整，并查看编译日志。",
      "Confirm Rust dependencies are complete and inspect the build logs.",
    ),
    "workspace_update.clawd_restart_failed": copy(
      lang,
      "clawd 构建已完成，但自动重启失败。请在服务器上手动重启 clawd。",
      "The clawd build completed, but automatic restart failed. Restart clawd manually on the server.",
    ),
    "workspace_update.release_deploy_downloading": copy(
      lang,
      "正在下载并部署当前机器对应的 Release 包。",
      "Downloading and deploying the Release package for this machine.",
    ),
    "workspace_update.release_deploy_check_network_or_permissions": copy(
      lang,
      "请查看部署日志；修复网络、GitHub Release 或写入权限问题后再重试。",
      "Check the deployment log. Fix network, GitHub Release, or write-permission issues, then retry.",
    ),
    "workspace_update.release_deploy_restart_scheduled": copy(
      lang,
      "Release 包已部署，RustClaw 正在重启，请等待 10-20 秒后刷新页面。",
      "The Release package was deployed and RustClaw is restarting. Wait 10-20 seconds, then refresh the page.",
    ),
    "workspace_update.release_deploy_restart_failed": copy(
      lang,
      "Release 包已部署，但自动重启失败。请在服务器上手动重启 clawd。",
      "The Release package was deployed, but automatic restart failed. Restart clawd manually on the server.",
    ),
    "workspace_update.canceled": copy(
      lang,
      "编译已停止；可以修复问题后重新编译。",
      "The build stopped. Fix any issues, then build again.",
    ),
  };
  return labels[key] || status?.next_step || key;
}

export function formatWorkspaceUpdateTime(ts: number | null | undefined, lang: UiLanguage): string {
  if (!ts) return "--";
  return new Date(ts * 1000).toLocaleString(lang === "zh" ? "zh-CN" : "en-US", {
    hour12: false,
  });
}

function numberArg(args: Record<string, unknown>, key: string): number | null {
  const value = args[key];
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

function workspaceUpdateProgressPercent(status: WorkspaceUpdateStatus | null | undefined, running: boolean): number {
  if (!status) return 0;
  if (status.status === "up_to_date") return 100;
  if (status.status === "succeeded") return 100;
  if (status.status === "failed" || status.status === "canceled") return 100;
  if (status.status === "restarting" || status.step === "restart_scheduled") return 100;
  const stepProgress: Record<string, number> = {
    idle: 0,
    starting: 5,
    checking_current_version: 12,
    checking_remote_version: 22,
    pulling_latest_code: 38,
    resolving_conflicting_files: 48,
    skipping_pull_latest_code: 52,
    checking_new_version: 58,
    building_workspace: 82,
    building_ui: 82,
    building_clawd: 82,
    downloading_release: 35,
    deploying_release: 78,
    cancel_requested: 92,
    restarting_clawd: 96,
  };
  return stepProgress[status.step] ?? (running ? 50 : 0);
}

function workspaceUpdateProgressLabel(status: WorkspaceUpdateStatus | null | undefined, running: boolean, lang: UiLanguage): string {
  if (running && status?.step === "building_workspace") {
    return copy(lang, "编译中，实际耗时取决于设备性能。", "Building; duration depends on device performance.");
  }
  if (running && status?.step === "building_ui") {
    return copy(lang, "UI 编译中，完成后会部署到 nginx。", "Building the UI; it will deploy to nginx when finished.");
  }
  if (running && status?.step === "building_clawd") {
    return copy(lang, "clawd 编译中，完成后会安排 clawd 重启。", "Building clawd; clawd will restart when finished.");
  }
  if (running && status?.step === "downloading_release") {
    return copy(lang, "正在下载当前机器对应的 GitHub Release 包。", "Downloading the GitHub Release package for this machine.");
  }
  if (running && status?.mode === "release_deploy") {
    return copy(lang, "Release 包部署中，完成后会保留配置并重启 clawd。", "Deploying the Release package; configs will be preserved and clawd will restart.");
  }
  return formatWorkspaceUpdateStep(status?.step, lang);
}

function workspaceUpdateLogPreview(status: WorkspaceUpdateStatus | null | undefined, lang: UiLanguage): string {
  const stdoutPreview = status?.stdout_tail?.trim() || "";
  const stderrPreview = status?.stderr_tail?.trim() || "";
  return [
    stdoutPreview ? `${copy(lang, "构建输出", "Build output")}\n${stdoutPreview}` : "",
    stderrPreview
      ? `${copy(lang, "构建日志（stderr，不一定是错误）", "Build log (stderr, not necessarily errors)")}\n${stderrPreview}`
      : "",
  ]
    .filter(Boolean)
    .join("\n\n");
}

function workspaceUpdateErrorNotice(
  status: WorkspaceUpdateStatus,
  lang: UiLanguage,
): { title: string; detail: string } {
  const error = status.error?.trim() || "";
  if (error === "git path list output is too large to process safely") {
    return {
      title: copy(lang, "Git 文件列表过大，无法安全处理", "Git path list is too large to process safely"),
      detail: copy(
        lang,
        "本地变更或未跟踪文件太多，自动更新无法安全判断哪些路径可以覆盖。请先清理无关构建产物、临时文件或手动处理 Git 状态，然后再重试。",
        "There are too many local changes or untracked files for the updater to safely decide which paths can be overwritten. Clean unrelated build artifacts or temporary files, or resolve the Git state manually, then retry.",
      ),
    };
  }
  return {
    title: error || copy(lang, "更新失败", "Update failed"),
    detail: copy(
      lang,
      status.mode === "release_deploy"
        ? "请查看下方日志摘要；修复网络、GitHub Release 或写入权限问题后再重试。"
        : "请查看下方日志摘要；修复 Git、网络或编译问题后再重试。",
      status.mode === "release_deploy"
        ? "Check the log summary below, then fix network, GitHub Release, or write-permission issues and retry."
        : "Check the log summary below, then fix Git, network, or build issues and retry.",
    ),
  };
}

function workspaceUpdateNotice(
  status: WorkspaceUpdateStatus | null | undefined,
  displayStatus: string | undefined,
  restarting: boolean,
  lang: UiLanguage,
): WorkspaceUpdateNotice | null {
  if (!status) return null;
  const nextStep = formatWorkspaceUpdateNextStep(status, lang);
  if (status.status === "canceled") {
    return {
      tone: "info",
      title: copy(lang, "编译已停止。", "Build stopped."),
      detail: nextStep ?? copy(
        lang,
        "当前编译进程已结束；如果需要继续，请修复问题后重新点击完整编译。",
        "The current build process has ended. Fix any issues and run Build All again when ready.",
      ),
    };
  }
  if (status.status === "failed" || status.error) {
    const errorNotice = workspaceUpdateErrorNotice(status, lang);
    return {
      tone: "error",
      title: errorNotice.title,
      detail: nextStep ?? errorNotice.detail,
    };
  }
  if (restarting) {
    return {
      tone: "success",
      title: copy(
        lang,
        status.mode === "release_deploy" ? "Release 包已部署，RustClaw 正在重启。" : "构建已完成，RustClaw 正在重启。",
        status.mode === "release_deploy" ? "Release package deployed and RustClaw is restarting." : "Build completed and RustClaw is restarting.",
      ),
      detail: nextStep ?? copy(
        lang,
        "请等待 10-20 秒；如果页面没有自动恢复，可以稍后点击“检查远端版本”。",
        "Wait 10-20 seconds. If the page does not recover automatically, click Check remote shortly.",
      ),
    };
  }
  if (status.status === "running") {
    return {
      tone: "info",
      title: formatWorkspaceUpdateStep(status.step, lang),
      detail: nextStep ?? copy(
        lang,
        status.mode === "release_deploy"
          ? "Release 部署正在进行，日志会在下方持续刷新。"
          : "更新流程正在进行，编译日志会在下方持续刷新。",
        status.mode === "release_deploy"
          ? "Release deployment is running. Logs will keep refreshing below."
          : "The update is running. Build logs will keep refreshing below.",
      ),
    };
  }
  if (displayStatus === "up_to_date") {
    return {
      tone: "success",
      title: copy(lang, "远端已经是最新版本。", "The remote version is up to date."),
      detail: copy(
        lang,
        "如需重新应用当前本地环境，仍可点击完整编译。",
        "Use Build All if you need to re-apply the current local environment.",
      ),
    };
  }
  return null;
}

export function buildWorkspaceUpdateView(status: WorkspaceUpdateStatus | null | undefined, lang: UiLanguage): WorkspaceUpdateView {
  const restarting = status?.status === "restarting";
  const running = status?.status === "running" || restarting;
  const hasRemoteDiff = Boolean(status?.old_commit) && Boolean(status?.remote_commit) && status?.old_commit !== status?.remote_commit;
  const knownUpToDate =
    Boolean(status?.old_commit) &&
    Boolean(status?.remote_commit) &&
    status?.old_commit === status?.remote_commit &&
    (status?.status === "idle" || status?.status === "up_to_date");
  const displayStatus = knownUpToDate ? "up_to_date" : status?.status;
  const progressPercent = workspaceUpdateProgressPercent(status, running);
  return {
    restarting,
    running,
    hasRemoteDiff,
    knownUpToDate,
    displayStatus,
    progressPercent,
    progressActive: running && progressPercent > 0 && progressPercent < 100,
    progressLabel: workspaceUpdateProgressLabel(status, running, lang),
    logPreview: workspaceUpdateLogPreview(status, lang),
    notice: workspaceUpdateNotice(status, displayStatus, restarting, lang),
  };
}
