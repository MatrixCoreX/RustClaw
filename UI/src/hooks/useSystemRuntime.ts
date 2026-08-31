import { useEffect, useRef, useState } from "react";

import { useUiDialog } from "../components/UiDialogProvider";
import { ApiResponseFormatError, readJsonApiResponse } from "../lib/api-response";
import { sleep } from "../lib/display-format";
import { formatSystemActionError } from "../lib/system-actions";
import { formatUiError } from "../lib/ui-error";
import {
  formatWorkspaceUpdateApiError,
  shouldReloadAfterWorkspaceBuild,
} from "../lib/workspace-update";
import type {
  ApiResponse,
  ConsolePage,
  HealthResponse,
  HostSystemSummary,
  HostDependenciesSnapshot,
  NginxUiStatus,
  PiAppStatusResponse,
  WebdExposureStatus,
  WorkspaceUpdateMode,
  WorkspaceUpdateStatus,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseSystemRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  apiBase: string;
  uiAuthReady: boolean;
  isAdminIdentity: boolean;
  currentPage: ConsolePage;
  fetchHealth: (options?: { silent?: boolean }) => Promise<void>;
  setHealth: (health: HealthResponse) => void;
  setError: (message: string | null) => void;
  clearLlmConfigError: () => void;
  clearSkillsConfigError: () => void;
  fetchLlmConfig: () => unknown | Promise<unknown>;
  fetchMultimodalConfig: () => unknown | Promise<unknown>;
  fetchSkillsConfig: () => unknown | Promise<unknown>;
  fetchSkills: () => unknown | Promise<unknown>;
}

export function useSystemRuntime({
  apiFetch,
  t,
  apiBase,
  uiAuthReady,
  isAdminIdentity,
  currentPage,
  fetchHealth,
  setHealth,
  setError,
  clearLlmConfigError,
  clearSkillsConfigError,
  fetchLlmConfig,
  fetchMultimodalConfig,
  fetchSkillsConfig,
  fetchSkills,
}: UseSystemRuntimeParams) {
  const { choose: showChoice, confirm: showConfirm } = useUiDialog();
  const workspaceUpdateSilentFailuresRef = useRef(0);
  const workspaceUpdateWasActiveRef = useRef(false);
  const workspaceUpdateActiveModeRef = useRef<WorkspaceUpdateMode | string | undefined>(undefined);
  const workspaceUpdateReloadScheduledRef = useRef(false);
  const [systemRestarting, setSystemRestarting] = useState(false);
  const [systemRestartMessage, setSystemRestartMessage] = useState<string | null>(null);
  const [hostSystemSummary, setHostSystemSummary] = useState<HostSystemSummary | null>(null);
  const [hostSystemLoading, setHostSystemLoading] = useState(false);
  const [hostSystemErrorCode, setHostSystemErrorCode] = useState<string | null>(null);
  const [hostDependencies, setHostDependencies] = useState<HostDependenciesSnapshot | null>(null);
  const [hostDependenciesLoading, setHostDependenciesLoading] = useState(false);
  const [hostDependenciesErrorCode, setHostDependenciesErrorCode] = useState<string | null>(null);
  const [dependencyInstallingId, setDependencyInstallingId] = useState<string | null>(null);
  const [piAppStatus, setPiAppStatus] = useState<PiAppStatusResponse | null>(null);
  const [piAppRestarting, setPiAppRestarting] = useState(false);
  const [piAppRestartMessage, setPiAppRestartMessage] = useState<string | null>(null);
  const [workspaceUpdateStatus, setWorkspaceUpdateStatus] = useState<WorkspaceUpdateStatus | null>(null);
  const [workspaceUpdateLoading, setWorkspaceUpdateLoading] = useState(false);
  const [workspaceUpdateCanceling, setWorkspaceUpdateCanceling] = useState(false);
  const [workspaceUpdateMessage, setWorkspaceUpdateMessage] = useState<string | null>(null);
  const [nginxStatus, setNginxStatus] = useState<NginxUiStatus | null>(null);
  const [nginxStatusLoading, setNginxStatusLoading] = useState(false);
  const [nginxStatusError, setNginxStatusError] = useState<string | null>(null);
  const [webdExposureStatus, setWebdExposureStatus] = useState<WebdExposureStatus | null>(null);
  const [webdExposureLoading, setWebdExposureLoading] = useState(false);
  const [webdExposureUpdating, setWebdExposureUpdating] = useState(false);
  const [webdExposureError, setWebdExposureError] = useState<string | null>(null);
  const [webdExposureMessage, setWebdExposureMessage] = useState<string | null>(null);
  const workspaceUpdateUiLang = (): "zh" | "en" => (t("__zh__", "__en__") === "__zh__" ? "zh" : "en");
  const workspaceUpdateApiErrorMessage = (error: string | null | undefined): string =>
    formatWorkspaceUpdateApiError(error, workspaceUpdateUiLang());

  const fetchHostSystemSummary = async () => {
    setHostSystemLoading(true);
    setHostSystemErrorCode(null);
    try {
      const res = await apiFetch("/v1/system/host-summary");
      const body = (await res.json()) as ApiResponse<HostSystemSummary>;
      if (!res.ok || !body.ok || !body.data) {
        setHostSystemErrorCode(res.status === 401 || res.status === 403 ? "permission_denied" : "unavailable");
        return;
      }
      setHostSystemSummary(body.data);
    } catch {
      setHostSystemErrorCode("disconnected");
    } finally {
      setHostSystemLoading(false);
    }
  };

  const fetchHostDependencies = async (silent = false): Promise<HostDependenciesSnapshot | null> => {
    if (!silent) setHostDependenciesLoading(true);
    setHostDependenciesErrorCode(null);
    try {
      const res = await apiFetch("/v1/system/dependencies");
      const body = (await res.json()) as ApiResponse<HostDependenciesSnapshot>;
      if (!res.ok || !body.ok || !body.data) {
        setHostDependenciesErrorCode(res.status === 401 || res.status === 403 ? "permission_denied" : "unavailable");
        return null;
      }
      setHostDependencies(body.data);
      return body.data;
    } catch {
      setHostDependenciesErrorCode("disconnected");
      return null;
    } finally {
      if (!silent) setHostDependenciesLoading(false);
    }
  };

  const installHostDependency = async (dependencyId: string) => {
    setDependencyInstallingId(dependencyId);
    setHostDependenciesErrorCode(null);
    try {
      const res = await apiFetch("/v1/admin/system-dependencies/install", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ dependency_id: dependencyId }),
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        setHostDependenciesErrorCode(body.error || "install_failed");
        return;
      }
      await fetchHostDependencies(true);
    } catch {
      setHostDependenciesErrorCode("disconnected");
    } finally {
      setDependencyInstallingId(null);
    }
  };

  const fetchWorkspaceUpdateStatus = async (silent = false): Promise<WorkspaceUpdateStatus | null> => {
    if (!silent) {
      setWorkspaceUpdateLoading(true);
      setWorkspaceUpdateMessage(null);
    }
    try {
      const refreshQuery = silent ? "" : "?refresh_release=true";
      const res = await apiFetch(`/v1/admin/workspace-update${refreshQuery}`);
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error ? workspaceUpdateApiErrorMessage(body.error) : `workspace_update_status_http_${res.status}`);
      }
      setWorkspaceUpdateStatus(body.data);
      return body.data;
    } catch (err) {
      if (!silent) {
        const message = formatUiError(err, t, "更新状态暂时无法读取。", "Update status is temporarily unavailable.");
        setWorkspaceUpdateMessage(`${t("查询更新状态失败", "Failed to query update status")}: ${message}`);
      }
      return null;
    } finally {
      if (!silent) {
        setWorkspaceUpdateLoading(false);
      }
    }
  };

  const fetchNginxStatus = async (silent = false): Promise<NginxUiStatus | null> => {
    if (!silent) setNginxStatusLoading(true);
    setNginxStatusError(null);
    try {
      const res = await apiFetch("/v1/admin/nginx");
      const body = await readJsonApiResponse<ApiResponse<NginxUiStatus>>(res);
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nginx_status_http_${res.status}`);
      }
      setNginxStatus(body.data);
      return body.data;
    } catch (err) {
      const message = err instanceof ApiResponseFormatError
        ? err.kind === "html_response"
          ? t(
              "状态接口返回了网页而不是 API 数据。请更新并重启 {product_name}；本地开发页面还应确认 API 指向 webd（默认 8788 端口）。",
              "The status endpoint returned a web page instead of API data. Update and restart {product_name}; for local development, also confirm the API points to webd (port 8788 by default).",
            )
          : t(
              "状态接口返回了无法识别的数据，请更新并重启 {product_name} 后重试。",
              "The status endpoint returned unrecognized data. Update and restart {product_name}, then retry.",
            )
        : formatUiError(
          err,
          t,
          "nginx 状态暂时无法读取，请稍后重试。",
          "The nginx status is temporarily unavailable. Try again shortly.",
        );
      setNginxStatusError(message);
      return null;
    } finally {
      if (!silent) setNginxStatusLoading(false);
    }
  };

  const fetchWebdExposureStatus = async (silent = false): Promise<WebdExposureStatus | null> => {
    if (!silent) setWebdExposureLoading(true);
    setWebdExposureError(null);
    try {
      const res = await apiFetch("/v1/admin/webd-exposure");
      const body = await readJsonApiResponse<ApiResponse<WebdExposureStatus>>(res);
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `webd_exposure_status_http_${res.status}`);
      }
      setWebdExposureStatus(body.data);
      return body.data;
    } catch (err) {
      const message = err instanceof ApiResponseFormatError
        ? t(
            "webd 状态接口返回了无法识别的数据，请更新并重启 {product_name} 后重试。",
            "The webd status endpoint returned unrecognized data. Update and restart {product_name}, then retry.",
          )
        : formatUiError(
          err,
          t,
          "Web 入口状态暂时无法读取，请稍后重试。",
          "The web entry status is temporarily unavailable. Try again shortly.",
        );
      setWebdExposureError(message);
      return null;
    } finally {
      if (!silent) setWebdExposureLoading(false);
    }
  };

  const setWebdExternalAccess = async (externallyAccessible: boolean) => {
    const confirmed = await showConfirm({
      title: externallyAccessible
        ? t("开放 webd 对外端口", "Expose the webd port")
        : t("关闭 webd 对外端口", "Close the public webd port"),
      message: externallyAccessible
        ? t(
            `开放后，局域网或公网可直接连接当前设备的 ${webdExposureStatus?.port ?? 8788} 端口。该入口会绕过 nginx，请确认防火墙和访问网络可信。Web 入口将短暂重启。`,
            `After this is enabled, LAN or internet clients can connect directly to port ${webdExposureStatus?.port ?? 8788} on this device. This path bypasses nginx, so verify the firewall and network trust first. The web entry will restart briefly.`,
          )
        : t(
            `关闭后，webd 只监听 127.0.0.1:${webdExposureStatus?.port ?? 8788}。nginx 页面和 API 仍可使用；当前若通过 IP:${webdExposureStatus?.port ?? 8788} 直连，页面会断开。Web 入口将短暂重启。`,
            `After this is disabled, webd listens only on 127.0.0.1:${webdExposureStatus?.port ?? 8788}. The nginx UI and API remain available; a page connected directly through IP:${webdExposureStatus?.port ?? 8788} will disconnect. The web entry will restart briefly.`,
          ),
      confirmLabel: externallyAccessible ? t("确认开放", "Expose port") : t("确认关闭", "Close port"),
      tone: externallyAccessible ? "danger" : "default",
    });
    if (!confirmed) return;

    setWebdExposureUpdating(true);
    setWebdExposureError(null);
    setWebdExposureMessage(null);
    try {
      const res = await apiFetch("/v1/admin/webd-exposure", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ externally_accessible: externallyAccessible }),
      });
      const body = await readJsonApiResponse<ApiResponse<WebdExposureStatus>>(res);
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `webd_exposure_update_http_${res.status}`);
      }
      setWebdExposureStatus(body.data);
      setWebdExposureMessage(body.data.restart_scheduled
        ? t(
            "访问范围已保存，{product_name} 正在重启。nginx 入口会在服务恢复后继续工作。",
            "The access scope was saved and {product_name} is restarting. The nginx entry will continue working after the service recovers.",
          )
        : t("webd 已经是所选访问范围。", "webd already uses the selected access scope."));
      if (body.data.restart_scheduled) {
        window.setTimeout(() => void fetchWebdExposureStatus(true), 5000);
      }
    } catch (err) {
      const message = formatUiError(
        err,
        t,
        "Web 入口访问范围修改失败，请稍后重试。",
        "The web entry access scope could not be changed. Try again shortly.",
      );
      setWebdExposureError(`${t("修改 webd 访问范围失败", "Failed to update webd access scope")}: ${message}`);
    } finally {
      setWebdExposureUpdating(false);
    }
  };

  const startWorkspaceUpdate = async (mode: WorkspaceUpdateMode = "full") => {
    const modeConfig: Record<WorkspaceUpdateMode, { confirm: string; endpoint: string; started: string }> = {
      full: {
        confirm: t(
          "系统会先检查本地改动：如果只有 configs 目录存在冲突，会临时保存配置、拉取后再恢复；如果源码有本地改动，会停止更新且不会覆盖文件。检查通过后会拉取并完整编译；如果已配置 nginx，还会部署最新 UI，随后重启 {product_name}。确认现在开始吗？",
          "The system checks local changes first. If only configs conflict, it temporarily saves them and restores them after pulling. If source files have local changes, the update stops without overwriting them. After the check passes, it pulls and builds everything, deploys the latest UI when nginx is already configured, and restarts {product_name}. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update",
        started: t(
          "完整编译已开始；完成后会把最新 UI 部署到已配置的 nginx，并重启 {product_name}。",
          "The full build has started. When it finishes, the latest UI will be deployed to the configured nginx site and {product_name} will restart.",
        ),
      },
      full_preserve_nginx: {
        confirm: t(
          "将拉取并完整编译后端、UI、核心工具和本平台技能，但不会安装、升级、重载、复制或修改任何 nginx 内容。完成后会重启 {product_name}。确认继续吗？",
          "Pull and fully build the backend, UI, core tools, and platform skills without installing, upgrading, reloading, copying, or modifying anything in nginx. {product_name} will restart when the build finishes. Continue?",
        ),
        endpoint: "/v1/admin/workspace-update/preserve-nginx",
        started: t(
          "完整编译已开始；nginx 将保持不变，完成后会重启 {product_name}。",
          "The full build has started. nginx will remain unchanged, and {product_name} will restart when the build finishes.",
        ),
      },
      ui_only: {
        confirm: t(
          "只编译并部署 UI，不拉取远端版本，也不重启 clawd。确认现在开始吗？",
          "Build and deploy the UI only. This will not pull the remote version or restart clawd. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update/build-ui",
        started: t("UI 编译已开始，下面会自动刷新进度。", "UI build started. Progress will refresh automatically."),
      },
      clawd_only: {
        confirm: t(
          "只编译 clawd，完成后只重启 clawd；不拉取远端版本，也不编译 UI。确认现在开始吗？",
          "Build clawd only, then restart clawd only. This will not pull the remote version or build the UI. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update/build-clawd",
        started: t("clawd 编译已开始，下面会自动刷新进度。", "clawd build started. Progress will refresh automatically."),
      },
      nginx_enable: {
        confirm: t(
          "将检查 nginx：未安装时自动安装，系统仓库有新版本时自动更新；随后修复 {product_name} Web 入口、启动服务并部署当前 UI。可能需要系统管理员权限，确认继续吗？",
          "Check nginx, install it when missing, and update it when the system repository has a newer version. Then repair the {product_name} web entry, start the service, and deploy the current UI. System administrator privileges may be required. Continue?",
        ),
        endpoint: "/v1/admin/workspace-update/nginx-enable",
        started: t("nginx 检查、修复和 UI 部署任务已开始。", "The nginx check, repair, and UI deployment task has started."),
      },
      nginx_disable: {
        confirm: t(
          "关闭 nginx 会停止并禁用 nginx 服务，同时删除 {product_name} 的 nginx 站点配置和已部署 UI。云服务器或域名入口会立即无法访问，之后需要通过服务器终端或仍可直连的 webd 恢复。确认关闭吗？",
          "Disabling nginx stops and disables the service, then removes the {product_name} nginx site and deployed UI. A cloud server or domain entry will immediately become unreachable; recovery requires server terminal access or a still-reachable direct webd connection. Disable nginx?",
        ),
        endpoint: "/v1/admin/workspace-update/nginx-disable",
        started: t(
          "正在关闭 nginx 并删除已部署 UI。远程入口可能会立即断开。",
          "Disabling nginx and removing the deployed UI. The remote entry may disconnect immediately.",
        ),
      },
      local_https_prepare: {
        confirm: t(
          "为当前局域网地址生成设备 CA 和 HTTPS 证书？这一步不会切换当前页面，也不会启用 HTTPS。",
          "Generate a device CA and HTTPS certificate for the current LAN address? This step does not switch the current page or enable HTTPS.",
        ),
        endpoint: "/v1/admin/workspace-update/local-https-prepare",
        started: t("正在准备设备证书。完成后请先下载并安装 CA。", "Preparing the device certificate. Download and install the CA when this finishes."),
      },
      local_https_enable: {
        confirm: t(
          "确认浏览器已经信任页面显示的设备 CA 后再继续。系统将启用 HTTPS，同时保留 HTTP 入口用于恢复。",
          "Continue only after the browser trusts the device CA shown on this page. HTTPS will be enabled while HTTP remains available for recovery.",
        ),
        endpoint: "/v1/admin/workspace-update/local-https-enable",
        started: t("正在启用 HTTPS。完成后请点击安全地址切换访问。", "Enabling HTTPS. Open the secure address when the operation finishes."),
      },
      local_https_restore: {
        confirm: t(
          "恢复启用 HTTPS 之前的 nginx 配置？证书会保留，稍后可以重新启用。",
          "Restore the nginx configuration from before HTTPS was enabled? Certificates are retained so HTTPS can be enabled again later.",
        ),
        endpoint: "/v1/admin/workspace-update/local-https-restore",
        started: t("正在恢复原来的 HTTP 配置。", "Restoring the previous HTTP configuration."),
      },
      release_deploy: {
        confirm: t(
          "直接下载 GitHub Releases 里适合当前机器的预编译包并部署；会保留 configs、data、logs 和 .pids，完成后重启 clawd。确认现在开始吗？",
          "Download and deploy the prebuilt GitHub Release package for this machine. configs, data, logs, and .pids will be preserved, then clawd will restart. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update/deploy-release",
        started: t("Release 包部署已开始，下面会自动刷新进度。", "Release package deployment started. Progress will refresh automatically."),
      },
      release_restore: {
        confirm: t(
          "将下载适合当前机器的预编译 Release 包，并原子替换当前源码部署。configs、data、logs、.pids 和运行时技能会保留，当前源码树会保存为回滚备份；完成后将不再显示 Git 拉取和本机编译入口。确认恢复为 Release 部署吗？",
          "Download the prebuilt Release package for this machine and atomically replace the current source deployment. configs, data, logs, .pids, and runtime skills are preserved, while the current source tree is retained as a rollback backup. Git pull and local build controls will no longer be shown. Restore the Release deployment?",
        ),
        endpoint: "/v1/admin/workspace-update/restore-release",
        started: t(
          "正在安全恢复为 Release 部署，下面会自动刷新进度。",
          "Safely restoring the Release deployment. Progress will refresh automatically.",
        ),
      },
      source_checkout: {
        confirm: t(
          "切换后会克隆完整源码，保留现有配置、数据、日志和运行二进制，并把当前 Release 安装留作回滚备份。以后将显示 Git 拉取与本机编译功能，更新成本和维护风险也会提高。确认切换吗？",
          "This clones the complete source tree, preserves current configuration, data, logs, and runtime binaries, and keeps the packaged installation as a rollback backup. Git pull and local build controls will then be shown, with higher maintenance cost and risk. Continue?",
        ),
        endpoint: "/v1/admin/workspace-update/enable-source",
        started: t("正在安全切换到源码模式，下面会自动刷新进度。", "Safely switching to source mode. Progress will refresh automatically."),
      },
    };
    let selectedModeKey = mode;
    let confirmed = false;
    if (mode === "full") {
      const choice = await showChoice({
        title: t("选择完整编译方式", "Choose full build behavior"),
        message: t(
          "两种方式都会拉取最新代码并完整编译后端、UI、核心工具和本平台技能，完成后重启 {product_name}。请选择是否同时更新 nginx 中的 UI。",
          "Both choices pull the latest code and fully build the backend, UI, core tools, and platform skills, then restart {product_name}. Choose whether to update the UI hosted by nginx as well.",
        ),
        choices: [
          {
            value: "deploy_nginx",
            label: t("包含 nginx：完整部署最新 UI", "Include nginx: deploy the latest UI"),
            description: t(
              "如果本机已配置 {product_name} nginx 站点，将最新 UI 同步到该站点；未配置时不会新建 nginx。",
              "Sync the latest UI to the existing {product_name} nginx site. This does not create nginx when no site is configured.",
            ),
          },
          {
            value: "preserve_nginx",
            label: t("不动 nginx", "Leave nginx unchanged"),
            description: t(
              "仍然完整编译 UI，但不安装、升级、重载、复制或修改任何 nginx 内容。",
              "Still build the complete UI without installing, upgrading, reloading, copying, or modifying anything in nginx.",
            ),
          },
        ],
      });
      if (!choice) return;
      selectedModeKey = choice === "preserve_nginx" ? "full_preserve_nginx" : "full";
      confirmed = true;
    } else if (mode === "local_https_enable") {
      confirmed = true;
    } else {
      const selectedMode = modeConfig[selectedModeKey];
      confirmed = await showConfirm({
        title: selectedModeKey === "nginx_disable"
          ? t("关闭 nginx Web 入口", "Disable nginx web entry")
          : t("确认系统操作", "Confirm system operation"),
        message: selectedMode.confirm,
        confirmLabel: selectedModeKey === "nginx_disable" ? t("确认关闭", "Disable") : t("继续", "Continue"),
        tone: selectedModeKey === "nginx_disable" ? "danger" : "default",
      });
    }
    if (!confirmed) return;
    const selectedMode = modeConfig[selectedModeKey];
    setWorkspaceUpdateLoading(true);
    setWorkspaceUpdateMessage(null);
    try {
      const requestInit: RequestInit = { method: "POST" };
      if (selectedModeKey === "local_https_enable") {
        requestInit.headers = { "Content-Type": "application/json" };
        requestInit.body = JSON.stringify({
          ca_fingerprint_sha256: nginxStatus?.local_https_ca_fingerprint_sha256 || "",
        });
      }
      const res = await apiFetch(selectedMode.endpoint, requestInit);
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        if (res.status === 409 && body.data) {
          setWorkspaceUpdateStatus(body.data);
          setWorkspaceUpdateMessage(
            t("更新已经在进行中，下面会继续刷新现有进度。", "An update is already running. Existing progress will keep refreshing."),
          );
          return;
        }
        throw new Error(body.error ? workspaceUpdateApiErrorMessage(body.error) : `workspace_update_start_http_${res.status}`);
      }
      setWorkspaceUpdateStatus(body.data);
      setWorkspaceUpdateMessage(selectedMode.started);
    } catch (err) {
      const message = formatUiError(err, t, "系统重启未完成，请查看日志。", "The system restart did not complete. Check the logs.");
      setWorkspaceUpdateMessage(`${t("启动更新失败", "Failed to start update")}: ${message}`);
    } finally {
      setWorkspaceUpdateLoading(false);
    }
  };

  const cancelWorkspaceUpdate = async () => {
    const confirmed = await showConfirm({
      title: t("停止当前操作", "Stop current operation"),
      message: t(
        workspaceUpdateStatus?.mode === "release_deploy" || workspaceUpdateStatus?.mode === "release_restore"
          ? "停止当前部署？已经完成的下载或文件复制不会自动回滚，后续可重新点击下载 Release 部署。"
          : workspaceUpdateStatus?.mode === "nginx_disable"
            ? "停止关闭 nginx？已经停止的服务或已经删除的 UI 不会自动恢复。"
          : workspaceUpdateStatus?.mode === "source_checkout"
            ? "停止切换源码模式？如果尚未完成原子切换，当前 Release 安装会保持不变。"
            : "停止当前编译？已经完成的拉取或文件复制不会自动回滚，后续可重新点击完整编译。",
        workspaceUpdateStatus?.mode === "release_deploy" || workspaceUpdateStatus?.mode === "release_restore"
          ? "Stop the current deployment? Completed download or copy steps will not be rolled back. You can deploy the Release again later."
          : workspaceUpdateStatus?.mode === "nginx_disable"
            ? "Stop disabling nginx? A service already stopped or UI files already removed will not be restored automatically."
          : workspaceUpdateStatus?.mode === "source_checkout"
            ? "Stop switching to source mode? The current Release installation remains unchanged if the atomic switch has not completed."
          : "Stop the current build? Completed pull or copy steps will not be rolled back. You can run Build All again later.",
      ),
      confirmLabel: t("停止", "Stop"),
      tone: "danger",
    });
    if (!confirmed) return;
    setWorkspaceUpdateCanceling(true);
    setWorkspaceUpdateMessage(null);
    try {
      const res = await apiFetch("/v1/admin/workspace-update/cancel", { method: "POST" });
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        if (body.data) setWorkspaceUpdateStatus(body.data);
        throw new Error(body.error ? workspaceUpdateApiErrorMessage(body.error) : `workspace_update_cancel_http_${res.status}`);
      }
      setWorkspaceUpdateStatus(body.data);
      setWorkspaceUpdateMessage(t("已请求停止编译，正在结束当前进程。", "Stop requested. Ending the current build process."));
    } catch (err) {
      const message = formatUiError(err, t, "硬件设备服务重启未完成，请查看日志。", "The hardware-device service restart did not complete. Check the logs.");
      setWorkspaceUpdateMessage(`${t("停止编译失败", "Failed to stop build")}: ${message}`);
    } finally {
      setWorkspaceUpdateCanceling(false);
    }
  };

  const restartSystem = async () => {
    setSystemRestarting(true);
    setSystemRestartMessage(null);
    clearLlmConfigError();
    clearSkillsConfigError();
    let restartAccepted = false;
    try {
      const res = await apiFetch(`/v1/system/restart`, {
        method: "POST",
      });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(formatSystemActionError(body, res.status, t));
      }
      restartAccepted = true;
      setSystemRestartMessage(
        t(
          "已发起重启，页面会短暂断开，稍后会自动恢复。",
          "Restart requested. The page may disconnect briefly and then recover.",
        ),
      );
      await sleep(1800);
      let recovered = false;
      for (let attempt = 0; attempt < 12; attempt += 1) {
        try {
          const probe = await apiFetch(`/v1/health`);
          const body = (await probe.json()) as ApiResponse<HealthResponse>;
          if (probe.ok && body.ok && body.data) {
            recovered = true;
            setHealth(body.data);
            setError(null);
            break;
          }
        } catch {
          // The restart window is expected to cause transient failures while clawd comes back up.
        }
        await sleep(1500);
      }

      if (recovered) {
        await Promise.allSettled([fetchLlmConfig(), fetchMultimodalConfig(), fetchSkillsConfig(), fetchSkills()]);
        setSystemRestartMessage(
          t(
            "{product_name} 已重启完成，当前页面已经恢复。",
            "{product_name} restarted successfully and the page is back online.",
          ),
        );
      } else {
        setSystemRestartMessage(
          t(
            "重启请求已经发出，但暂时还没等到服务恢复。请稍后手动刷新。",
            "Restart was requested, but the service has not recovered yet. Please refresh shortly.",
          ),
        );
      }
      setSystemRestarting(false);
      return recovered;
    } catch (err) {
      const message = formatUiError(err, t, "系统重启未完成，请查看日志。", "The system restart did not complete. Check the logs.");
      setSystemRestartMessage(`${t("重启失败", "Restart failed")}: ${message}`);
      return false;
    } finally {
      if (!restartAccepted) {
        setSystemRestarting(false);
      }
    }
  };

  const fetchPiAppStatus = async () => {
    try {
      const res = await apiFetch(`/v1/pi-app/status`);
      const body = (await res.json()) as ApiResponse<PiAppStatusResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `hardware_device_status_http_${res.status}`);
      }
      setPiAppStatus(body.data);
    } catch {
      setPiAppStatus(null);
    }
  };

  const restartPiApp = async () => {
    setPiAppRestarting(true);
    setPiAppRestartMessage(null);
    try {
      const res = await apiFetch(`/v1/pi-app/restart`, { method: "POST" });
      const body = (await res.json()) as ApiResponse<Record<string, unknown>>;
      if (!res.ok || !body.ok) {
        throw new Error(formatSystemActionError(body, res.status, t));
      }
      setPiAppRestartMessage(t("已发起 Pi App 小程序重启。", "Pi App restart requested."));
    } catch (err) {
      const message = formatUiError(err, t, "硬件设备服务重启未完成，请查看日志。", "The hardware-device service restart did not complete. Check the logs.");
      setPiAppRestartMessage(`${t("Pi App 重启失败", "Pi App restart failed")}: ${message}`);
    } finally {
      setPiAppRestarting(false);
      void fetchPiAppStatus();
    }
  };

  useEffect(() => {
    if (!uiAuthReady || currentPage !== "dashboard") return;
    void fetchHostSystemSummary();
    void fetchHostDependencies();
    if (isAdminIdentity) {
      void fetchWorkspaceUpdateStatus(true);
      void fetchNginxStatus(true);
      void fetchWebdExposureStatus(true);
      void fetchPiAppStatus();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, isAdminIdentity, currentPage]);

  useEffect(() => {
    if (!uiAuthReady || currentPage !== "dashboard") return;
    const hasActiveInstall = hostDependencies?.operations.some(
      (operation) => operation.status === "queued" || operation.status === "running",
    );
    if (!hasActiveInstall) return;
    const interval = window.setInterval(() => void fetchHostDependencies(true), 2500);
    return () => window.clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, currentPage, hostDependencies?.operations]);

  useEffect(() => {
    const status = workspaceUpdateStatus?.status;
    if (
      (
        workspaceUpdateStatus?.mode === "nginx_enable"
        || workspaceUpdateStatus?.mode === "nginx_disable"
        || workspaceUpdateStatus?.mode === "local_https_prepare"
        || workspaceUpdateStatus?.mode === "local_https_enable"
        || workspaceUpdateStatus?.mode === "local_https_restore"
      ) &&
      (status === "succeeded" || status === "failed" || status === "canceled")
    ) {
      void fetchNginxStatus(true);
    }
    if (status === "running" || status === "restarting") {
      workspaceUpdateWasActiveRef.current = true;
      workspaceUpdateActiveModeRef.current = workspaceUpdateStatus?.mode;
      return;
    }
    if (
      workspaceUpdateReloadScheduledRef.current ||
      !shouldReloadAfterWorkspaceBuild(
        workspaceUpdateWasActiveRef.current,
        workspaceUpdateActiveModeRef.current,
        status,
      )
    ) {
      return;
    }
    workspaceUpdateWasActiveRef.current = false;
    workspaceUpdateReloadScheduledRef.current = true;
    window.setTimeout(() => window.location.reload(), 600);
  }, [workspaceUpdateStatus?.mode, workspaceUpdateStatus?.status]);

  useEffect(() => {
    if (!uiAuthReady || !isAdminIdentity) return;
    const status = workspaceUpdateStatus?.status;
    if (status !== "running" && status !== "restarting") return;
    const interval = window.setInterval(async () => {
      const next = await fetchWorkspaceUpdateStatus(true);
      if (!next) {
        workspaceUpdateSilentFailuresRef.current += 1;
        if (status === "restarting" && workspaceUpdateSilentFailuresRef.current >= 3) {
          setWorkspaceUpdateMessage(
            t(
              "{product_name} 可能仍在重启。你可以稍后点击“检查远端版本”确认服务是否恢复。",
              "{product_name} may still be restarting. You can click Check remote shortly to confirm recovery.",
            ),
          );
        }
        return;
      }
      workspaceUpdateSilentFailuresRef.current = 0;
      if (next?.status === "restarting") {
        await sleep(1800);
        await fetchHealth({ silent: true });
      }
    }, 2500);
    return () => window.clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, isAdminIdentity, workspaceUpdateStatus?.status]);

  return {
    systemRestarting,
    systemRestartMessage,
    hostSystemSummary,
    hostSystemLoading,
    hostSystemErrorCode,
    hostDependencies,
    hostDependenciesLoading,
    hostDependenciesErrorCode,
    dependencyInstallingId,
    piAppStatus,
    piAppRestarting,
    piAppRestartMessage,
    workspaceUpdateStatus,
    workspaceUpdateLoading,
    workspaceUpdateCanceling,
    workspaceUpdateMessage,
    nginxStatus,
    nginxStatusLoading,
    nginxStatusError,
    fetchNginxStatus,
    webdExposureStatus,
    webdExposureLoading,
    webdExposureUpdating,
    webdExposureError,
    webdExposureMessage,
    fetchWebdExposureStatus,
    setWebdExternalAccess,
    fetchWorkspaceUpdateStatus,
    startWorkspaceUpdate,
    cancelWorkspaceUpdate,
    restartSystem,
    restartPiApp,
    fetchHostSystemSummary,
    fetchHostDependencies,
    installHostDependency,
  };
}
