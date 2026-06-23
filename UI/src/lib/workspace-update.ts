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

export function formatWorkspaceUpdateTime(ts: number | null | undefined, lang: UiLanguage): string {
  if (!ts) return "--";
  return new Date(ts * 1000).toLocaleString(lang === "zh" ? "zh-CN" : "en-US", {
    hour12: false,
  });
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

function workspaceUpdateNotice(
  status: WorkspaceUpdateStatus | null | undefined,
  displayStatus: string | undefined,
  restarting: boolean,
  lang: UiLanguage,
): WorkspaceUpdateNotice | null {
  if (!status) return null;
  if (status.status === "canceled") {
    return {
      tone: "info",
      title: copy(lang, "编译已停止。", "Build stopped."),
      detail: copy(
        lang,
        "当前编译进程已结束；如果需要继续，请修复问题后重新点击完整编译。",
        "The current build process has ended. Fix any issues and run Build All again when ready.",
      ),
    };
  }
  if (status.status === "failed" || status.error) {
    return {
      tone: "error",
      title: status.error || copy(lang, "更新失败", "Update failed"),
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
  if (restarting) {
    return {
      tone: "success",
      title: copy(
        lang,
        status.mode === "release_deploy" ? "Release 包已部署，RustClaw 正在重启。" : "构建已完成，RustClaw 正在重启。",
        status.mode === "release_deploy" ? "Release package deployed and RustClaw is restarting." : "Build completed and RustClaw is restarting.",
      ),
      detail: copy(
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
      detail: copy(
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
