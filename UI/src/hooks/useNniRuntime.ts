import { useState } from "react";

import {
  nniJoinErrorMessage,
  parseNniRemoteNodeUrls,
  shortenHex,
  type UiLanguage,
} from "../lib/nni-display";
import type {
  ApiResponse,
  NniConfigResponse,
  NniDeviceActionResponse,
  NniDeviceStatusResponse,
  NniHeartbeatErrorRecord,
  NniHeartbeatErrorsResponse,
  NniHeartbeatRecord,
  NniHeartbeatRecordsResponse,
  NniJoinTaskResponse,
  NniJoinVerifyResponse,
} from "../types/api";

export const NNI_HEARTBEAT_RECORDS_PAGE_SIZE = 10;
export const NNI_HEARTBEAT_ERRORS_PAGE_SIZE = 10;

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseNniRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  lang: UiLanguage;
}

export function useNniRuntime({ apiFetch, t, lang }: UseNniRuntimeParams) {
  const [nniStatus, setNniStatus] = useState<NniDeviceStatusResponse | null>(null);
  const [nniStatusLoading, setNniStatusLoading] = useState(false);
  const [nniStatusError, setNniStatusError] = useState<string | null>(null);
  const [nniActionLoading, setNniActionLoading] = useState<string | null>(null);
  const [nniActionResult, setNniActionResult] = useState<NniDeviceActionResponse | null>(null);
  const [nniActionError, setNniActionError] = useState<string | null>(null);
  const [nniActionMessage, setNniActionMessage] = useState<string | null>(null);
  const [nniJoined, setNniJoined] = useState(false);
  const [nniRemoteNodes, setNniRemoteNodes] = useState("");
  const [nniHeartbeatRequestCount, setNniHeartbeatRequestCount] = useState(0);
  const [nniHeartbeatRetryLimit, setNniHeartbeatRetryLimit] = useState(3);
  const [nniLastHeartbeatAtTs, setNniLastHeartbeatAtTs] = useState<number | null>(null);
  const [nniLastHeartbeatNetworkFailures, setNniLastHeartbeatNetworkFailures] = useState(0);
  const [nniHeartbeatRecords, setNniHeartbeatRecords] = useState<NniHeartbeatRecord[]>([]);
  const [nniHeartbeatRecordsPage, setNniHeartbeatRecordsPage] = useState(1);
  const [nniHeartbeatRecordsTotal, setNniHeartbeatRecordsTotal] = useState(0);
  const [nniHeartbeatRecordsTotalPages, setNniHeartbeatRecordsTotalPages] = useState(1);
  const [nniHeartbeatRecordsLoading, setNniHeartbeatRecordsLoading] = useState(false);
  const [nniHeartbeatRecordsClearing, setNniHeartbeatRecordsClearing] = useState(false);
  const [nniHeartbeatRecordsError, setNniHeartbeatRecordsError] = useState<string | null>(null);
  const [nniHeartbeatRecordsMessage, setNniHeartbeatRecordsMessage] = useState<string | null>(null);
  const [nniHeartbeatErrors, setNniHeartbeatErrors] = useState<NniHeartbeatErrorRecord[]>([]);
  const [nniHeartbeatErrorsPage, setNniHeartbeatErrorsPage] = useState(1);
  const [nniHeartbeatErrorsTotal, setNniHeartbeatErrorsTotal] = useState(0);
  const [nniHeartbeatErrorsTotalPages, setNniHeartbeatErrorsTotalPages] = useState(1);
  const [nniHeartbeatErrorsLoading, setNniHeartbeatErrorsLoading] = useState(false);
  const [nniHeartbeatErrorsClearing, setNniHeartbeatErrorsClearing] = useState(false);
  const [nniHeartbeatErrorsError, setNniHeartbeatErrorsError] = useState<string | null>(null);
  const [nniHeartbeatErrorsMessage, setNniHeartbeatErrorsMessage] = useState<string | null>(null);
  const [nniConfigLoading, setNniConfigLoading] = useState(false);
  const [nniConfigSaving, setNniConfigSaving] = useState(false);
  const [nniConfigError, setNniConfigError] = useState<string | null>(null);
  const [nniConfigMessage, setNniConfigMessage] = useState<string | null>(null);

  const nniRemoteNodeUrls = () => parseNniRemoteNodeUrls(nniRemoteNodes);

  const applyNniConfigResponse = (config: NniConfigResponse) => {
    setNniJoined(config.joined);
    setNniRemoteNodes(config.remote_nodes.join("\n"));
    setNniHeartbeatRequestCount(config.heartbeat_request_count ?? 0);
    setNniHeartbeatRetryLimit(config.heartbeat_network_retry_limit ?? 3);
    setNniLastHeartbeatAtTs(config.last_heartbeat_at_ts ?? null);
    setNniLastHeartbeatNetworkFailures(config.last_heartbeat_network_failures ?? 0);
  };

  const setNniJoinedPersisted = async (joined: boolean, options?: { persistRemoteNodes?: boolean }) => {
    setNniJoined(joined);
    try {
      const payload: { joined: boolean; remote_nodes?: string[] } = { joined };
      if (options?.persistRemoteNodes) {
        payload.remote_nodes = nniRemoteNodeUrls();
      }
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI config update failed (${res.status})`);
      }
      applyNniConfigResponse(body.data);
      setNniConfigError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniConfigError(message);
    }
  };

  const fetchNniDeviceStatus = async (silent = false) => {
    if (!silent) {
      setNniStatusLoading(true);
      setNniStatusError(null);
    }
    try {
      const res = await apiFetch(`/v1/nni/device/status`);
      const body = (await res.json()) as ApiResponse<NniDeviceStatusResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI 状态获取失败 (${res.status})`);
      }
      setNniStatus(body.data);
      setNniStatusError(null);
      if (!body.data.signature_chip_present) {
        await setNniJoinedPersisted(false);
      }
      return body.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniStatusError(message);
      return null;
    } finally {
      if (!silent) {
        setNniStatusLoading(false);
      }
    }
  };

  const runNniDeviceAction = async (action: string, options?: { challenge?: string }) => {
    setNniActionLoading(action);
    setNniActionError(null);
    setNniActionMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/device/action`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action, challenge: options?.challenge }),
      });
      const body = (await res.json()) as ApiResponse<NniDeviceActionResponse>;
      if (!res.ok || !body.ok || !body.data) {
        const actionData = body.data;
        if (actionData?.signature_chip_present === false) {
          setNniStatus((prev) =>
            prev
              ? {
                  ...prev,
                  signature_chip_present: false,
                  status: "signature_chip_missing",
                  message: t(
                    "未检测到设备签名芯片。此设备仍可使用 RustClaw，NNI 的设备签名能力暂不可用。",
                    "No device signature chip was detected. RustClaw can still run, but NNI device signing is unavailable.",
                  ),
                }
              : prev,
          );
        }
        throw new Error(body.error || `NNI 操作失败 (${res.status})`);
      }
      setNniActionResult(body.data);
      setNniActionMessage(body.data.message || t("NNI 操作已完成。", "NNI action completed."));
      if (body.data.payload?.pubkey) {
        setNniStatus((prev) =>
          prev
            ? {
                ...prev,
                signature_chip_present: true,
                status: "ready",
                pubkey: body.data.payload?.pubkey,
                pubkey_preview: shortenHex(body.data.payload?.pubkey, 12, 12),
              }
            : prev,
        );
      }
      return body.data;
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniActionError(message);
      await setNniJoinedPersisted(false);
      return null;
    } finally {
      setNniActionLoading(null);
    }
  };

  const requestNniJoinTask = async (): Promise<NniJoinTaskResponse | null> => {
    const nodeUrls = nniRemoteNodeUrls();
    if (nodeUrls.length === 0) {
      throw new Error(t("请先填写至少一个远程 NNI 节点地址。", "Enter at least one remote NNI node URL first."));
    }
    const res = await apiFetch(`/v1/nni/join/request`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ node_urls: nodeUrls }),
    });
    const body = (await res.json()) as ApiResponse<NniJoinTaskResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(nniJoinErrorMessage(body.error, body.data, `NNI join request failed (${res.status})`, lang));
    }
    return body.data;
  };

  const verifyNniJoinTask = async (taskId: string, nodeUrl: string, signature: string): Promise<NniJoinVerifyResponse | null> => {
    const res = await apiFetch(`/v1/nni/join/verify`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ task_id: taskId, node_url: nodeUrl, signature }),
    });
    const body = (await res.json()) as ApiResponse<NniJoinVerifyResponse>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(nniJoinErrorMessage(body.error, body.data, `NNI join verify failed (${res.status})`, lang));
    }
    return body.data;
  };

  const fetchNniConfig = async (silent = false) => {
    if (!silent) setNniConfigLoading(true);
    setNniConfigError(null);
    try {
      const res = await apiFetch(`/v1/nni/config`);
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI config load failed (${res.status})`);
      }
      applyNniConfigResponse(body.data);
      if (!silent) setNniConfigMessage(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniConfigError(message);
    } finally {
      if (!silent) setNniConfigLoading(false);
    }
  };

  const fetchNniHeartbeatRecords = async (page = nniHeartbeatRecordsPage, silent = false) => {
    const safePage = Math.max(1, page);
    if (!silent) {
      setNniHeartbeatRecordsLoading(true);
      setNniHeartbeatRecordsError(null);
      setNniHeartbeatRecordsMessage(null);
    }
    try {
      const params = new URLSearchParams({
        page: String(safePage),
        per_page: String(NNI_HEARTBEAT_RECORDS_PAGE_SIZE),
      });
      const res = await apiFetch(`/v1/nni/records?${params.toString()}`);
      const body = (await res.json()) as ApiResponse<NniHeartbeatRecordsResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI request records load failed (${res.status})`);
      }
      setNniHeartbeatRecords(body.data.records ?? []);
      setNniHeartbeatRecordsPage(body.data.page || safePage);
      setNniHeartbeatRecordsTotal(body.data.total ?? 0);
      setNniHeartbeatRecordsTotalPages(Math.max(1, body.data.total_pages ?? 1));
      setNniHeartbeatRecordsError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      if (!silent) setNniHeartbeatRecordsError(message);
    } finally {
      if (!silent) setNniHeartbeatRecordsLoading(false);
    }
  };

  const clearNniHeartbeatRecords = async () => {
    const confirmed = window.confirm(
      t(
        "确定清理本机 NNI 请求记录吗？这只会清理本机保存的加入和心跳历史，不会修改远程 NNI 服务端记录。",
        "Clear local NNI request records? This only clears Join and Heartbeat history saved on this device and will not change remote NNI server records.",
      ),
    );
    if (!confirmed) return;
    setNniHeartbeatRecordsClearing(true);
    setNniHeartbeatRecordsError(null);
    setNniHeartbeatRecordsMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/records/clear`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });
      const rawText = await res.text();
      let body: ApiResponse<{ deleted_records?: number }>;
      try {
        body = JSON.parse(rawText) as ApiResponse<{ deleted_records?: number }>;
      } catch {
        throw new Error(t("NNI 请求记录清理接口返回了非 JSON 内容。", "The NNI request records clear endpoint returned non-JSON content."));
      }
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `NNI request records clear failed (${res.status})`);
      }
      const deletedRecords = body.data?.deleted_records ?? 0;
      setNniHeartbeatRecords([]);
      setNniHeartbeatRecordsPage(1);
      setNniHeartbeatRecordsTotal(0);
      setNniHeartbeatRecordsTotalPages(1);
      setNniHeartbeatRecordsMessage(
        t(
          `已清理 ${deletedRecords} 条本机 NNI 请求记录。`,
          `${deletedRecords} local NNI request records cleared.`,
        ),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniHeartbeatRecordsError(message);
    } finally {
      setNniHeartbeatRecordsClearing(false);
    }
  };

  const fetchNniHeartbeatErrors = async (page = nniHeartbeatErrorsPage, silent = false) => {
    const safePage = Math.max(1, page);
    if (!silent) {
      setNniHeartbeatErrorsLoading(true);
      setNniHeartbeatErrorsError(null);
      setNniHeartbeatErrorsMessage(null);
    }
    try {
      const params = new URLSearchParams({
        page: String(safePage),
        per_page: String(NNI_HEARTBEAT_ERRORS_PAGE_SIZE),
      });
      const res = await apiFetch(`/v1/nni/heartbeat/errors?${params.toString()}`);
      const rawText = await res.text();
      let body: ApiResponse<NniHeartbeatErrorsResponse>;
      try {
        body = JSON.parse(rawText) as ApiResponse<NniHeartbeatErrorsResponse>;
      } catch {
        const trimmed = rawText.trim().toLowerCase();
        if (trimmed.startsWith("<!doctype") || trimmed.startsWith("<html")) {
          throw new Error(
            t(
              "后端心跳错误接口还不可用，通常是 clawd 还没更新或正在重启。请等待编译重启完成后再刷新。",
              "The backend heartbeat error endpoint is not available yet. clawd is usually still updating or restarting; refresh after the build restart completes.",
            ),
          );
        }
        throw new Error(t("NNI 心跳错误接口返回了非 JSON 内容。", "The NNI heartbeat error endpoint returned non-JSON content."));
      }
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI heartbeat errors load failed (${res.status})`);
      }
      setNniHeartbeatErrors(body.data.records ?? []);
      setNniHeartbeatErrorsPage(body.data.page || safePage);
      setNniHeartbeatErrorsTotal(body.data.total ?? 0);
      setNniHeartbeatErrorsTotalPages(Math.max(1, body.data.total_pages ?? 1));
      setNniHeartbeatErrorsError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      if (!silent) setNniHeartbeatErrorsError(message);
    } finally {
      if (!silent) setNniHeartbeatErrorsLoading(false);
    }
  };

  const clearNniHeartbeatErrors = async () => {
    const confirmed = window.confirm(
      t(
        "确定清理本机心跳错误记录吗？这只会清理本机页面里的错误历史，不会修改远程 NNI 服务端请求记录。",
        "Clear local heartbeat error history? This only clears the local error history shown here and will not change remote NNI server request records.",
      ),
    );
    if (!confirmed) return;
    setNniHeartbeatErrorsClearing(true);
    setNniHeartbeatErrorsError(null);
    setNniHeartbeatErrorsMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/heartbeat/errors/clear`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });
      const rawText = await res.text();
      let body: ApiResponse<{ deleted_records?: number }>;
      try {
        body = JSON.parse(rawText) as ApiResponse<{ deleted_records?: number }>;
      } catch {
        throw new Error(t("NNI 心跳错误清理接口返回了非 JSON 内容。", "The NNI heartbeat error clear endpoint returned non-JSON content."));
      }
      if (!res.ok || !body.ok) {
        throw new Error(body.error || `NNI heartbeat errors clear failed (${res.status})`);
      }
      const deletedRecords = body.data?.deleted_records ?? 0;
      setNniHeartbeatErrors([]);
      setNniHeartbeatErrorsPage(1);
      setNniHeartbeatErrorsTotal(0);
      setNniHeartbeatErrorsTotalPages(1);
      setNniHeartbeatErrorsMessage(
        t(
          `已清理 ${deletedRecords} 条本机心跳错误记录。`,
          `${deletedRecords} local heartbeat error records cleared.`,
        ),
      );
      await fetchNniConfig(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniHeartbeatErrorsError(message);
    } finally {
      setNniHeartbeatErrorsClearing(false);
    }
  };

  const saveNniConfig = async () => {
    setNniConfigSaving(true);
    setNniConfigError(null);
    setNniConfigMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ remote_nodes: nniRemoteNodeUrls() }),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `NNI config save failed (${res.status})`);
      }
      applyNniConfigResponse(body.data);
      setNniConfigMessage(t("远程 NNI 节点已保存到配置文件。", "Remote NNI nodes were saved to the config file."));
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniConfigError(message);
    } finally {
      setNniConfigSaving(false);
    }
  };

  const testJoinNni = async () => {
    const status = nniStatus ?? (await fetchNniDeviceStatus(false));
    if (!status?.signature_chip_present) {
      setNniActionError(
        status?.message ||
          t(
            "未检测到设备签名芯片，暂时不能执行 NNI 测试加入。",
            "No device signature chip was detected, so this device cannot run the NNI test join yet.",
          ),
      );
      await setNniJoinedPersisted(false);
      return;
    }
    const result = await runNniDeviceAction("sign_timestamp");
    if (result?.payload?.signature) {
      setNniActionMessage(
        t(
          "测试签名已完成：本机已生成时间戳签名。只有点击加入并通过服务端验签后，才会开启运行状态。",
          "Test signature completed: this device generated a timestamp signature. The runtime starts only after Join passes server verification.",
        ),
      );
    }
  };

  const joinNni = async () => {
    setNniActionLoading("join_nni");
    setNniActionError(null);
    setNniActionMessage(null);
    const status = nniStatus ?? (await fetchNniDeviceStatus(false));
    if (!status?.signature_chip_present) {
      setNniActionError(
        status?.message ||
          t(
            "未检测到设备签名芯片，暂时不能加入需要设备签名的 NNI。",
            "No device signature chip was detected, so this device cannot join signed NNI yet.",
          ),
      );
      await setNniJoinedPersisted(false);
      setNniActionLoading(null);
      return;
    }
    try {
      const task = await requestNniJoinTask();
      if (!task?.challenge) {
        throw new Error("nni_join_challenge_missing");
      }
      const signatureResult = await runNniDeviceAction("sign_challenge", { challenge: task.challenge });
      const signature = signatureResult?.payload?.signature;
      if (!signature) {
        throw new Error("nni_join_signature_missing");
      }
      setNniActionLoading("join_nni");
      const verified = await verifyNniJoinTask(task.task_id, task.node_url, signature);
      if (!verified?.joined || !verified.compliant) {
        throw new Error("nni_join_verify_rejected");
      }
      await setNniJoinedPersisted(true, { persistRemoteNodes: true });
      setNniActionMessage(
        t(
          "设备签名已通过服务端验证，NNI 已开始运行。",
          "The device signature was verified by the server, and NNI is now running.",
        ),
      );
      await fetchNniHeartbeatRecords(1, true);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setNniActionError(message);
      await setNniJoinedPersisted(false);
    } finally {
      setNniActionLoading(null);
    }
  };

  const updateNniRemoteNodes = (value: string) => {
    setNniRemoteNodes(value);
    setNniConfigMessage(null);
    setNniConfigError(null);
  };

  return {
    nniStatus,
    nniStatusLoading,
    nniStatusError,
    nniActionLoading,
    nniActionResult,
    nniActionError,
    nniActionMessage,
    nniJoined,
    nniRemoteNodes,
    nniRemoteNodeCount: nniRemoteNodeUrls().length,
    nniHeartbeatRequestCount,
    nniHeartbeatRetryLimit,
    nniLastHeartbeatAtTs,
    nniLastHeartbeatNetworkFailures,
    nniHeartbeatRecords,
    nniHeartbeatRecordsPage,
    nniHeartbeatRecordsTotal,
    nniHeartbeatRecordsTotalPages,
    nniHeartbeatRecordsLoading,
    nniHeartbeatRecordsClearing,
    nniHeartbeatRecordsError,
    nniHeartbeatRecordsMessage,
    nniHeartbeatErrors,
    nniHeartbeatErrorsPage,
    nniHeartbeatErrorsTotal,
    nniHeartbeatErrorsTotalPages,
    nniHeartbeatErrorsLoading,
    nniHeartbeatErrorsClearing,
    nniHeartbeatErrorsError,
    nniHeartbeatErrorsMessage,
    nniConfigLoading,
    nniConfigSaving,
    nniConfigError,
    nniConfigMessage,
    setNniActionMessage,
    setNniActionError,
    fetchNniDeviceStatus,
    setNniJoinedPersisted,
    joinNni,
    testJoinNni,
    fetchNniConfig,
    saveNniConfig,
    updateNniRemoteNodes,
    fetchNniHeartbeatRecords,
    clearNniHeartbeatRecords,
    fetchNniHeartbeatErrors,
    clearNniHeartbeatErrors,
    runNniDeviceAction,
  };
}
