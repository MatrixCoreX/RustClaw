import { useEffect, useRef, useState } from "react";

import { sleep } from "../lib/display-format";
import type {
  ApiResponse,
  HealthResponse,
  PiAppStatusResponse,
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
  const workspaceUpdateSilentFailuresRef = useRef(0);
  const [systemRestarting, setSystemRestarting] = useState(false);
  const [systemRestartMessage, setSystemRestartMessage] = useState<string | null>(null);
  const [piAppStatus, setPiAppStatus] = useState<PiAppStatusResponse | null>(null);
  const [piAppRestarting, setPiAppRestarting] = useState(false);
  const [piAppRestartMessage, setPiAppRestartMessage] = useState<string | null>(null);
  const [workspaceUpdateStatus, setWorkspaceUpdateStatus] = useState<WorkspaceUpdateStatus | null>(null);
  const [workspaceUpdateLoading, setWorkspaceUpdateLoading] = useState(false);
  const [workspaceUpdateCanceling, setWorkspaceUpdateCanceling] = useState(false);
  const [workspaceUpdateMessage, setWorkspaceUpdateMessage] = useState<string | null>(null);

  const fetchWorkspaceUpdateStatus = async (silent = false): Promise<WorkspaceUpdateStatus | null> => {
    if (!silent) {
      setWorkspaceUpdateLoading(true);
      setWorkspaceUpdateMessage(null);
    }
    try {
      const res = await apiFetch("/v1/admin/workspace-update");
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `workspace update status failed (${res.status})`);
      }
      setWorkspaceUpdateStatus(body.data);
      return body.data;
    } catch (err) {
      if (!silent) {
        const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
        setWorkspaceUpdateMessage(`${t("查询更新状态失败", "Failed to query update status")}: ${message}`);
      }
      return null;
    } finally {
      if (!silent) {
        setWorkspaceUpdateLoading(false);
      }
    }
  };

  const startWorkspaceUpdate = async (mode: WorkspaceUpdateMode = "full") => {
    const modeConfig: Record<WorkspaceUpdateMode, { confirm: string; endpoint: string; started: string }> = {
      full: {
        confirm: t(
          "系统会先正常拉取远端版本；如果拉取被本地冲突文件阻挡，只覆盖这些冲突文件，其他本地改动和额外文件保持不动。随后会完整编译并重启 clawd。确认现在开始吗？",
          "The system will pull the remote version first. If local conflicting files block the pull, only those conflict files will be overwritten; other local changes and extra files are left untouched. It will then run a full build and restart clawd. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update",
        started: t("更新已开始，下面会自动刷新进度。", "Update started. Progress will refresh automatically."),
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
      release_deploy: {
        confirm: t(
          "直接下载 GitHub Releases 里适合当前机器的预编译包并部署；会保留 configs、data、logs 和 .pids，完成后重启 clawd。确认现在开始吗？",
          "Download and deploy the prebuilt GitHub Release package for this machine. configs, data, logs, and .pids will be preserved, then clawd will restart. Start now?",
        ),
        endpoint: "/v1/admin/workspace-update/deploy-release",
        started: t("Release 包部署已开始，下面会自动刷新进度。", "Release package deployment started. Progress will refresh automatically."),
      },
    };
    const selectedMode = modeConfig[mode];
    const confirmed = window.confirm(selectedMode.confirm);
    if (!confirmed) return;
    setWorkspaceUpdateLoading(true);
    setWorkspaceUpdateMessage(null);
    try {
      const res = await apiFetch(selectedMode.endpoint, { method: "POST" });
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        if (res.status === 409 && body.data) {
          setWorkspaceUpdateStatus(body.data);
          setWorkspaceUpdateMessage(
            t("更新已经在进行中，下面会继续刷新现有进度。", "An update is already running. Existing progress will keep refreshing."),
          );
          return;
        }
        throw new Error(body.error || `workspace update start failed (${res.status})`);
      }
      setWorkspaceUpdateStatus(body.data);
      setWorkspaceUpdateMessage(selectedMode.started);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setWorkspaceUpdateMessage(`${t("启动更新失败", "Failed to start update")}: ${message}`);
    } finally {
      setWorkspaceUpdateLoading(false);
    }
  };

  const cancelWorkspaceUpdate = async () => {
    const confirmed = window.confirm(
      t(
        workspaceUpdateStatus?.mode === "release_deploy"
          ? "停止当前部署？已经完成的下载或文件复制不会自动回滚，后续可重新点击下载 Release 部署。"
          : "停止当前编译？已经完成的拉取或文件复制不会自动回滚，后续可重新点击完整编译。",
        workspaceUpdateStatus?.mode === "release_deploy"
          ? "Stop the current deployment? Completed download or copy steps will not be rolled back. You can deploy the Release again later."
          : "Stop the current build? Completed pull or copy steps will not be rolled back. You can run Build All again later.",
      ),
    );
    if (!confirmed) return;
    setWorkspaceUpdateCanceling(true);
    setWorkspaceUpdateMessage(null);
    try {
      const res = await apiFetch("/v1/admin/workspace-update/cancel", { method: "POST" });
      const body = (await res.json()) as ApiResponse<WorkspaceUpdateStatus>;
      if (!res.ok || !body.ok || !body.data) {
        if (body.data) setWorkspaceUpdateStatus(body.data);
        throw new Error(body.error || `workspace update cancel failed (${res.status})`);
      }
      setWorkspaceUpdateStatus(body.data);
      setWorkspaceUpdateMessage(t("已请求停止编译，正在结束当前进程。", "Stop requested. Ending the current build process."));
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
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
        throw new Error(body.error || `system restart failed (${res.status})`);
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
            "RustClaw 已重启完成，当前页面已经恢复。",
            "RustClaw restarted successfully and the page is back online.",
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
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
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
        throw new Error(body.error || `Pi App status failed (${res.status})`);
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
        throw new Error(body.error || `Pi App restart failed (${res.status})`);
      }
      setPiAppRestartMessage(t("已发起 Pi App 小程序重启。", "Pi App restart requested."));
    } catch (err) {
      const message = err instanceof Error ? err.message : t("未知错误", "Unknown error");
      setPiAppRestartMessage(`${t("Pi App 重启失败", "Pi App restart failed")}: ${message}`);
    } finally {
      setPiAppRestarting(false);
      void fetchPiAppStatus();
    }
  };

  useEffect(() => {
    if (!uiAuthReady || !isAdminIdentity) return;
    void fetchWorkspaceUpdateStatus(true);
    void fetchPiAppStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiBase, uiAuthReady, isAdminIdentity]);

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
              "RustClaw 可能仍在重启。你可以稍后点击“检查远端版本”确认服务是否恢复。",
              "RustClaw may still be restarting. You can click Check remote shortly to confirm recovery.",
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
    piAppStatus,
    piAppRestarting,
    piAppRestartMessage,
    workspaceUpdateStatus,
    workspaceUpdateLoading,
    workspaceUpdateCanceling,
    workspaceUpdateMessage,
    fetchWorkspaceUpdateStatus,
    startWorkspaceUpdate,
    cancelWorkspaceUpdate,
    restartSystem,
    restartPiApp,
  };
}
