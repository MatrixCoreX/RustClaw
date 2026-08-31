import { useRef, useState } from "react";

import { useUiDialog } from "../components/UiDialogProvider";
import {
  nniDeviceMessage,
  nniJoinErrorMessage,
  nniJoinRejectsDevicePublicKey,
  parseNniRemoteNodeUrls,
  shortenHex,
  nniTimestampSignatureReady,
  type UiLanguage,
} from "../lib/nni-display";
import { formatNniApiError, formatNniErrorCause } from "../lib/nni-api-error";
import { fetchResilientRead, runCoalescedRead } from "../lib/resilient-read";
import {
  assertNniPrivateKeyOperationsAllowed,
  generateNniOwnerKeyPair as generateLocalNniOwnerKeyPair,
  NNI_PRIVATE_KEY_INSECURE_TRANSPORT_ERROR,
  normalizeNniOwnerSignature,
  signNniOwnerChallenge,
  validateNniOwnerPrivateKey,
  validateNniOwnerPublicKey,
  type NniOwnerKeyPair,
} from "../lib/nni-owner-public-key";
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
  NniNetworkStatsResponse,
  NniOwnerRecoveryChallengeResponse,
  NniOwnerRecoveryResponse,
  NniOwnerUnbindTaskResponse,
  NniOwnerUnbindVerifyResponse,
  NniRewardsResponse,
} from "../types/api";

export const NNI_HEARTBEAT_RECORDS_PAGE_SIZE = 10;
export const NNI_HEARTBEAT_ERRORS_PAGE_SIZE = 10;
export const NNI_REWARDS_PAGE_SIZE = 100;

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

function nniPrivateKeyErrorMessage(cause: unknown, t: Translate): string {
  const code = cause instanceof Error ? cause.message : null;
  if (code === NNI_PRIVATE_KEY_INSECURE_TRANSPORT_ERROR) {
    return t(
      "当前页面使用非本机 HTTP 连接，已禁用私钥操作。请改用 HTTPS，或仅在本机 localhost 页面操作。",
      "Private-key operations are disabled over non-loopback HTTP. Use HTTPS or operate from localhost on this device.",
    );
  }
  if (code === "nni_private_key_secure_random_unavailable") {
    return t(
      "当前浏览器无法提供安全随机数，未生成资产密钥。请更换支持安全上下文的浏览器。",
      "This browser cannot provide secure randomness, so no asset key was generated. Use a browser with a secure context.",
    );
  }
  return formatNniApiError(code, t, t("资产私钥操作失败。", "The asset private-key operation failed."));
}

export interface NniOwnerAuthorizationChallenge {
  mode: "bind";
  taskId: string;
  nodeUrl: string;
  signingPayload: string;
  deviceSignature: string;
  targetOwnerPublicKey: string | null;
  replaceExistingOwner: boolean;
}

export interface UseNniRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
  lang: UiLanguage;
}

export function useNniRuntime({ apiFetch, t, lang }: UseNniRuntimeParams) {
  const { confirm: showConfirm } = useUiDialog();
  const readRequestsRef = useRef(new Map<string, Promise<unknown>>());
  const [nniStatus, setNniStatus] = useState<NniDeviceStatusResponse | null>(null);
  const [nniStatusLoading, setNniStatusLoading] = useState(false);
  const [nniStatusError, setNniStatusError] = useState<string | null>(null);
  const [nniActionLoading, setNniActionLoading] = useState<string | null>(null);
  const [nniActionResult, setNniActionResult] = useState<NniDeviceActionResponse | null>(null);
  const [nniActionError, setNniActionError] = useState<string | null>(null);
  const [nniActionMessage, setNniActionMessage] = useState<string | null>(null);
  const [nniDeviceAuthorizationDenied, setNniDeviceAuthorizationDenied] = useState(false);
  const [nniJoined, setNniJoined] = useState(false);
  const [nniAssetOwnerPubkey, setNniAssetOwnerPubkey] = useState<string | null>(null);
  const [nniOwnerKeyPair, setNniOwnerKeyPair] = useState<NniOwnerKeyPair | null>(null);
  const [nniOwnerActionLoading, setNniOwnerActionLoading] = useState<"generate" | "recover" | "custom" | "unbind" | null>(null);
  const [nniOwnerAuthorizationChallenge, setNniOwnerAuthorizationChallenge] =
    useState<NniOwnerAuthorizationChallenge | null>(null);
  const [nniRemoteNodes, setNniRemoteNodes] = useState("");
  const [nniSelectedNodeUrl, setNniSelectedNodeUrl] = useState("");
  const [nniBancorServiceNodeUrl, setNniBancorServiceNodeUrl] = useState("");
  const [nniBancorServiceNodeSaving, setNniBancorServiceNodeSaving] = useState(false);
  const [nniBancorServiceNodeError, setNniBancorServiceNodeError] = useState<string | null>(null);
  const [nniAssetServiceNodeUrl, setNniAssetServiceNodeUrl] = useState("");
  const [nniAssetServiceNodeSaving, setNniAssetServiceNodeSaving] = useState(false);
  const [nniAssetServiceNodeError, setNniAssetServiceNodeError] = useState<string | null>(null);
  const [nniHeartbeatIntervalSeconds, setNniHeartbeatIntervalSeconds] = useState<number | null>(null);
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
  const [nniRewards, setNniRewards] = useState<NniRewardsResponse | null>(null);
  const [nniRewardsLoading, setNniRewardsLoading] = useState(false);
  const [nniRewardsError, setNniRewardsError] = useState<string | null>(null);
  const [nniNetworkStats, setNniNetworkStats] = useState<NniNetworkStatsResponse | null>(null);
  const [nniNetworkStatsLoading, setNniNetworkStatsLoading] = useState(false);
  const [nniNetworkStatsError, setNniNetworkStatsError] = useState<string | null>(null);
  const [nniConfigLoading, setNniConfigLoading] = useState(false);
  const [nniConfigSaving, setNniConfigSaving] = useState(false);
  const [nniConfigError, setNniConfigError] = useState<string | null>(null);
  const [nniConfigMessage, setNniConfigMessage] = useState<string | null>(null);

  const nniRemoteNodeUrls = () => parseNniRemoteNodeUrls(nniRemoteNodes);
  const selectedNniNodeUrl = () => {
    const nodeUrls = nniRemoteNodeUrls();
    return nodeUrls.includes(nniSelectedNodeUrl) ? nniSelectedNodeUrl : nodeUrls[0] ?? "";
  };
  const selectedNniAssetServiceNodeUrl = () => {
    const nodeUrls = nniRemoteNodeUrls();
    if (nodeUrls.includes(nniAssetServiceNodeUrl)) return nniAssetServiceNodeUrl;
    return selectedNniNodeUrl();
  };
  const selectedNniBancorServiceNodeUrl = () => {
    const nodeUrls = nniRemoteNodeUrls();
    if (nodeUrls.includes(nniBancorServiceNodeUrl)) return nniBancorServiceNodeUrl;
    return selectedNniNodeUrl();
  };

  const applyNniConfigResponse = (config: NniConfigResponse) => {
    setNniJoined(config.joined);
    setNniAssetOwnerPubkey(config.asset_owner_pubkey ?? null);
    setNniRemoteNodes(config.remote_nodes.join("\n"));
    setNniSelectedNodeUrl(config.selected_node_url ?? config.remote_nodes[0] ?? "");
    setNniBancorServiceNodeUrl(
      config.bancor_service_node_url
      ?? config.selected_node_url
      ?? config.remote_nodes[0]
      ?? "",
    );
    setNniAssetServiceNodeUrl(
      config.asset_service_node_url
      ?? config.selected_node_url
      ?? config.remote_nodes[0]
      ?? "",
    );
    setNniHeartbeatIntervalSeconds(config.heartbeat_interval_seconds ?? null);
    setNniHeartbeatRequestCount(config.heartbeat_request_count ?? 0);
    setNniHeartbeatRetryLimit(config.heartbeat_network_retry_limit ?? 3);
    setNniLastHeartbeatAtTs(config.last_heartbeat_at_ts ?? null);
    setNniLastHeartbeatNetworkFailures(config.last_heartbeat_network_failures ?? 0);
  };

  const setNniJoinedPersisted = async (joined: boolean, options?: { persistRemoteNodes?: boolean }) => {
    setNniJoined(joined);
    try {
      const payload: {
        joined: boolean;
        remote_nodes?: string[];
        selected_node_url?: string;
        bancor_service_node_url?: string;
        asset_service_node_url?: string;
      } = { joined };
      if (options?.persistRemoteNodes) {
        payload.remote_nodes = nniRemoteNodeUrls();
        payload.selected_node_url = selectedNniNodeUrl();
        payload.bancor_service_node_url = selectedNniBancorServiceNodeUrl();
        payload.asset_service_node_url = selectedNniAssetServiceNodeUrl();
      }
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_config_update_http_${res.status}`);
      }
      applyNniConfigResponse(body.data);
      setNniConfigError(null);
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("NNI 配置更新失败。", "NNI configuration update failed."));
      setNniConfigError(message);
    }
  };

  const readNniDeviceStatus = (silent: boolean, forceRefresh: boolean) => runCoalescedRead(
    readRequestsRef.current,
    forceRefresh ? "device-status-refresh" : "device-status",
    async () => {
    if (!silent) {
      setNniStatusLoading(true);
      setNniStatusError(null);
    }
    try {
      const path = forceRefresh ? `/v1/nni/device/status?refresh=true` : `/v1/nni/device/status`;
      const res = await fetchResilientRead(apiFetch, path);
      const body = (await res.json()) as ApiResponse<NniDeviceStatusResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_status_fetch_http_${res.status}`);
      }
      setNniStatus(body.data);
      setNniStatusError(null);
      return body.data;
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("NNI 状态暂时无法读取。", "NNI status is temporarily unavailable."));
      if (!silent) setNniStatusError(message);
      return null;
    } finally {
      if (!silent) {
        setNniStatusLoading(false);
      }
    }
    },
  );

  const fetchNniDeviceStatus = (silent = false) => readNniDeviceStatus(silent, true);

  const ensureNniDeviceStatus = (silent = false) => (
    nniStatus ? Promise.resolve(nniStatus) : readNniDeviceStatus(silent, false)
  );

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
                  simulated: false,
                  device_kind: "unavailable",
                  status: "signature_chip_missing",
                  message_key: "nni.device_status.signature_chip_missing",
                  next_step_key: "nni.device_status.signature_chip_missing.next_step",
                }
              : prev,
          );
          throw new Error(
            nniDeviceMessage(actionData, lang) ||
              t(
                "未检测到 MatrixAI 芯片，无法完成本次操作。",
                "No MatrixAI chip was detected, so this action cannot be completed.",
              ),
          );
        }
        const actionMessage = nniDeviceMessage(actionData, lang);
        if (actionMessage) throw new Error(actionMessage);
        throw new Error(body.error || `nni_action_http_${res.status}`);
      }
      setNniActionResult(body.data);
      setNniActionMessage(nniDeviceMessage(body.data, lang, t("NNI 操作已完成。", "NNI action completed.")));
      if (body.data.action === "simulation_disable") {
        setNniStatus((prev) =>
          prev
            ? {
                ...prev,
                signature_chip_present: false,
                simulated: false,
                device_kind: "unavailable",
                simulation_available: true,
                status: "signature_chip_missing",
                message_key: "nni.device_status.signature_chip_missing",
                next_step_key: "nni.device_status.signature_chip_missing.next_step",
                pubkey: null,
                pubkey_preview: null,
                pubkey_fingerprint: null,
                meta: null,
              }
            : prev,
        );
      }
      if (body.data.payload?.pubkey) {
        setNniStatus((prev) =>
          prev
            ? {
                ...prev,
                signature_chip_present: true,
                simulated: body.data.simulated ?? prev.simulated,
                device_kind: body.data.device_kind ?? prev.device_kind,
                status: body.data.simulated ? "simulated" : "ready",
                pubkey: body.data.payload?.pubkey,
                pubkey_preview: shortenHex(body.data.payload?.pubkey, 12, 12),
              }
            : prev,
        );
      }
      return body.data;
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("NNI 操作未完成。", "The NNI operation did not complete."));
      setNniActionError(message);
      return null;
    } finally {
      setNniActionLoading(null);
    }
  };

  const setNniDeviceSimulation = async (enabled: boolean) => {
    const result = await runNniDeviceAction(enabled ? "simulation_enable" : "simulation_disable");
    if (!result) return null;
    return fetchNniDeviceStatus(false);
  };

  const requestNniJoinTask = async (
    assetOwnerPubkey?: string | null,
    replaceExistingOwner = false,
  ): Promise<NniJoinTaskResponse | null> => {
    const nodeUrl = selectedNniNodeUrl();
    if (!nodeUrl) {
      throw new Error(t("请先填写至少一个远程 NNI 节点地址。", "Enter at least one remote NNI node URL first."));
    }
    const res = await apiFetch(`/v1/nni/join/request`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        node_url: nodeUrl,
        asset_owner_pubkey: assetOwnerPubkey || undefined,
        replace_existing_owner: replaceExistingOwner,
      }),
    });
    const body = (await res.json()) as ApiResponse<NniJoinTaskResponse>;
    if (!res.ok || !body.ok || !body.data) {
      if (nniJoinRejectsDevicePublicKey(body.error, body.data)) {
        setNniDeviceAuthorizationDenied(true);
      }
      throw new Error(nniJoinErrorMessage(body.error, body.data, t("NNI 加入请求未完成。", "The NNI join request did not complete."), lang));
    }
    return body.data;
  };

  const verifyNniJoinTask = async (
    taskId: string,
    nodeUrl: string,
    signature: string,
    options?: {
      ownerSignature?: string;
      replaceExistingOwner?: boolean;
    },
  ): Promise<NniJoinVerifyResponse | null> => {
    const res = await apiFetch(`/v1/nni/join/verify`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        task_id: taskId,
        node_url: nodeUrl,
        signature,
        owner_signature: options?.ownerSignature,
        replace_existing_owner: options?.replaceExistingOwner === true,
      }),
    });
    const body = (await res.json()) as ApiResponse<NniJoinVerifyResponse>;
    if (!res.ok || !body.ok || !body.data) {
      if (nniJoinRejectsDevicePublicKey(body.error, body.data)) {
        setNniDeviceAuthorizationDenied(true);
      }
      throw new Error(nniJoinErrorMessage(body.error, body.data, t("NNI 加入验证未完成。", "The NNI join verification did not complete."), lang));
    }
    return body.data;
  };

  const generateNniOwnerKeyPair = async () => {
    setNniOwnerActionLoading("generate");
    setNniActionError(null);
    setNniActionMessage(null);
    try {
      const keyPair = generateLocalNniOwnerKeyPair();
      setNniOwnerKeyPair(keyPair);
      setNniActionMessage(t(
        "资产密钥已生成。请立即抄写私钥；页面刷新后无法找回。",
        "The asset key was generated. Copy the private key now; it cannot be recovered after refresh.",
      ));
      return keyPair;
    } catch (err) {
      setNniActionError(nniPrivateKeyErrorMessage(err, t));
      return null;
    } finally {
      setNniOwnerActionLoading(null);
    }
  };

  const clearNniOwnerKeyPair = () => setNniOwnerKeyPair(null);

  const recoverNniOwner = async (ownerPrivateKey: string) => {
    const nodeUrl = selectedNniNodeUrl();
    if (!nodeUrl) {
      setNniActionError(t("请先填写至少一个远程 NNI 节点地址。", "Enter at least one remote NNI node URL first."));
      return null;
    }
    setNniOwnerActionLoading("recover");
    setNniActionError(null);
    setNniActionMessage(null);
    try {
      assertNniPrivateKeyOperationsAllowed();
      const validation = validateNniOwnerPrivateKey(ownerPrivateKey);
      if (!validation.ok) {
        throw new Error("nni_owner_private_key_invalid");
      }
      const challengeResponse = await apiFetch(`/v1/nni/owner/recover`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          node_url: nodeUrl,
          asset_owner_pubkey: validation.publicKey,
        }),
      });
      const challengeBody = (await challengeResponse.json()) as ApiResponse<NniOwnerRecoveryChallengeResponse>;
      if (!challengeResponse.ok || !challengeBody.ok || !challengeBody.data) {
        throw new Error(nniJoinErrorMessage(
          challengeBody.error,
          challengeBody.data,
          `NNI recovery failed (${challengeResponse.status})`,
          lang,
        ));
      }
      const challenge = challengeBody.data;
      const signed = signNniOwnerChallenge(validation.normalized, challenge.signing_payload);
      if (signed.publicKey !== validation.publicKey || challenge.asset_owner_pubkey !== validation.publicKey) {
        throw new Error("nni_owner_recovery_identity_changed");
      }
      const verifyResponse = await apiFetch(`/v1/nni/owner/recover`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          node_url: nodeUrl,
          asset_owner_pubkey: validation.publicKey,
          task_id: challenge.task_id,
          device_signature: challenge.device_signature,
          owner_signature: signed.signature,
        }),
      });
      const body = (await verifyResponse.json()) as ApiResponse<NniOwnerRecoveryResponse>;
      if (!verifyResponse.ok || !body.ok || !body.data) {
        throw new Error(nniJoinErrorMessage(
          body.error,
          body.data,
          `NNI recovery failed (${verifyResponse.status})`,
          lang,
        ));
      }
      setNniAssetOwnerPubkey(body.data.asset_owner_pubkey);
      setNniOwnerKeyPair(null);
      await setNniJoinedPersisted(true, { persistRemoteNodes: true });
      setNniActionMessage(t(
        "资产账户已恢复到当前设备，旧设备授权已撤销。",
        "The asset account was recovered to this device and the old device authorization was revoked.",
      ));
      return body.data;
    } catch (err) {
      setNniActionError(nniPrivateKeyErrorMessage(err, t));
      return null;
    } finally {
      setNniOwnerActionLoading(null);
    }
  };

  const startNniCustomOwnerAuthorization = async (ownerPublicKey: string) => {
    const validation = validateNniOwnerPublicKey(ownerPublicKey);
    if (!validation.ok) {
      setNniActionError(t("资产公钥格式无效。", "The asset public key format is invalid."));
      return null;
    }
    const replaceExistingOwner = Boolean(
      nniAssetOwnerPubkey && nniAssetOwnerPubkey !== validation.normalized,
    );
    if (nniAssetOwnerPubkey === validation.normalized) {
      setNniActionError(t("当前设备已经绑定这个资产公钥。", "This device already uses this asset public key."));
      return null;
    }
    const status = nniStatus ?? (await fetchNniDeviceStatus(false));
    if (!status?.signature_chip_present) {
      setNniActionError(t(
        "当前设备没有可用的签名芯片，不能修改资产授权。",
        "This device has no available signing chip, so asset authorization cannot be changed.",
      ));
      return null;
    }
    setNniOwnerActionLoading("custom");
    setNniActionError(null);
    setNniActionMessage(null);
    try {
      const task = await requestNniJoinTask(validation.normalized, replaceExistingOwner);
      if (!task?.challenge) throw new Error("nni_join_challenge_missing");
      const signatureResult = await runNniDeviceAction("sign_challenge", {
        challenge: task.challenge,
      });
      const deviceSignature = signatureResult?.payload?.signature;
      if (!deviceSignature) throw new Error("nni_join_signature_missing");
      if (task.owner_signature_required !== true) {
        throw new Error("nni_target_owner_signature_requirement_missing");
      }
      const challenge: NniOwnerAuthorizationChallenge = {
        mode: "bind",
        taskId: task.task_id,
        nodeUrl: task.node_url,
        signingPayload: task.challenge,
        deviceSignature,
        targetOwnerPublicKey: validation.normalized,
        replaceExistingOwner,
      };
      setNniOwnerAuthorizationChallenge(challenge);
      setNniActionMessage(t(
        "设备签名已完成。请使用目标资产密钥签名下方数据。",
        "The device signature is ready. Sign the payload with the target asset key.",
      ));
      return challenge;
    } catch (err) {
      setNniActionError(formatNniErrorCause(err, t, t("资产绑定请求失败。", "Asset binding request failed.")));
      return null;
    } finally {
      setNniOwnerActionLoading(null);
    }
  };

  const startNniOwnerUnbind = async () => {
    if (!nniAssetOwnerPubkey) {
      setNniActionError(t("当前设备没有已绑定的资产公钥。", "This device has no bound asset public key."));
      return null;
    }
    const status = nniStatus ?? (await fetchNniDeviceStatus(false));
    if (!status?.signature_chip_present) {
      setNniActionError(t(
        "当前设备没有可用的签名芯片，不能解绑资产密钥。",
        "This device has no available signing chip, so the asset key cannot be unbound.",
      ));
      return null;
    }
    const confirmed = await showConfirm({
      title: t("解绑资产密钥", "Unbind asset key"),
      message: t(
        "解绑只使用当前硬件设备签名，不需要资产私钥；它只撤销当前设备，不影响同一资产账户下的其他设备。能控制设备签名的人可以执行解绑，并把当前设备未来的奖励改绑到其能签名的新账户。完成后当前设备会停止 NNI 心跳，重新绑定后才能继续获得奖励。",
        "Unbinding uses only this hardware device signature and does not require the asset private key. It revokes only this device and leaves other devices on the same asset account unchanged. Anyone controlling the device signer can unbind it and redirect this device's future rewards to a new asset account they can sign for. NNI heartbeats stop until this device is bound again.",
      ),
      confirmLabel: t("继续解绑", "Continue"),
      tone: "danger",
    });
    if (!confirmed) return null;
    const nodeUrl = selectedNniNodeUrl();
    if (!nodeUrl) {
      setNniActionError(t("请先选择 NNI 节点。", "Select an NNI node first."));
      return null;
    }
    setNniOwnerActionLoading("unbind");
    setNniActionError(null);
    setNniActionMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/owner/unbind/request`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ node_url: nodeUrl }),
      });
      const body = (await res.json()) as ApiResponse<NniOwnerUnbindTaskResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(nniJoinErrorMessage(body.error, body.data, t("资产解绑请求未完成。", "The asset unbind request did not complete."), lang));
      }
      const signatureResult = await runNniDeviceAction("sign_challenge", {
        challenge: body.data.signing_payload,
      });
      const deviceSignature = signatureResult?.payload?.signature;
      if (!deviceSignature) throw new Error("nni_asset_unbind_device_signature_missing");
      const verifyRes = await apiFetch(`/v1/nni/owner/unbind/verify`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          task_id: body.data.task_id,
          node_url: body.data.node_url,
          device_signature: deviceSignature,
        }),
      });
      const verifyBody = (await verifyRes.json()) as ApiResponse<NniOwnerUnbindVerifyResponse>;
      if (!verifyRes.ok || !verifyBody.ok || !verifyBody.data) {
        throw new Error(nniJoinErrorMessage(
          verifyBody.error,
          verifyBody.data,
          `NNI unbind verify failed (${verifyRes.status})`,
          lang,
        ));
      }
      setNniAssetOwnerPubkey(null);
      setNniJoined(false);
      setNniActionMessage(t(
        "设备签名已验证，当前设备已经解绑资产密钥。",
        "The device signature was verified and the asset key was unbound from this device.",
      ));
      return verifyBody.data;
    } catch (err) {
      setNniActionError(formatNniErrorCause(err, t, t("资产解绑请求失败。", "Asset unbind request failed.")));
      return null;
    } finally {
      setNniOwnerActionLoading(null);
    }
  };

  const verifyNniOwnerAuthorization = async (
    challenge: NniOwnerAuthorizationChallenge,
    ownerSignatureInput: string,
  ) => {
    const ownerSignature = normalizeNniOwnerSignature(ownerSignatureInput);
    if (!ownerSignature) {
      setNniActionError(t("资产签名必须是 128 位十六进制。", "The asset signature must be 128 hexadecimal characters."));
      return null;
    }
    setNniOwnerActionLoading("custom");
    setNniActionError(null);
    try {
      const verified = await verifyNniJoinTask(
        challenge.taskId,
        challenge.nodeUrl,
        challenge.deviceSignature,
        {
          ownerSignature,
          replaceExistingOwner: challenge.replaceExistingOwner,
        },
      );
      if (!verified?.joined || !verified.compliant) throw new Error("nni_join_verify_rejected");
      await setNniJoinedPersisted(true, { persistRemoteNodes: true });
      setNniAssetOwnerPubkey(verified.asset_owner_pubkey ?? challenge.targetOwnerPublicKey);
      setNniOwnerKeyPair(null);
      setNniOwnerAuthorizationChallenge(null);
      setNniActionMessage(t(
        challenge.replaceExistingOwner
          ? "目标资产签名已验证，资产公钥已更换。"
          : "目标资产签名已验证，资产账户已绑定。",
        challenge.replaceExistingOwner
          ? "The target asset signature was verified and the asset public key was replaced."
          : "The target asset signature was verified and the asset account was bound.",
      ));
      return verified;
    } catch (err) {
      setNniActionError(formatNniErrorCause(err, t, t("资产授权失败。", "Asset authorization failed.")));
      return null;
    } finally {
      setNniOwnerActionLoading(null);
    }
  };

  const completeNniOwnerAuthorization = async (ownerSignatureInput: string) => {
    const challenge = nniOwnerAuthorizationChallenge;
    if (!challenge) {
      setNniActionError(t("当前没有待签名的资产操作。", "There is no pending asset authorization."));
      return null;
    }
    return verifyNniOwnerAuthorization(challenge, ownerSignatureInput);
  };

  const authorizeNniOwnerWithPrivateKey = async (ownerPrivateKeyInput: string) => {
    try {
      assertNniPrivateKeyOperationsAllowed();
    } catch (err) {
      setNniActionError(nniPrivateKeyErrorMessage(err, t));
      return null;
    }
    const validation = validateNniOwnerPrivateKey(ownerPrivateKeyInput);
    if (!validation.ok) {
      setNniActionError(t(
        "资产私钥无效。请检查 Base58 编码、K1 校验和及密钥内容。",
        "The asset private key is invalid. Check its Base58 encoding, K1 checksum, and key data.",
      ));
      return null;
    }

    const challenge = await startNniCustomOwnerAuthorization(validation.publicKey);
    if (!challenge) return null;
    try {
      const signed = signNniOwnerChallenge(validation.normalized, challenge.signingPayload);
      if (signed.publicKey !== challenge.targetOwnerPublicKey) {
        throw new Error("nni_owner_private_key_target_mismatch");
      }
      return await verifyNniOwnerAuthorization(challenge, signed.signature);
    } catch (err) {
      setNniActionError(nniPrivateKeyErrorMessage(err, t));
      return null;
    }
  };

  const cancelNniOwnerAuthorization = () => {
    setNniOwnerAuthorizationChallenge(null);
    setNniActionError(null);
  };

  const fetchNniConfig = (silent = false) => runCoalescedRead(
    readRequestsRef.current,
    "config",
    async () => {
    if (!silent) setNniConfigLoading(true);
    if (!silent) setNniConfigError(null);
    try {
      const res = await fetchResilientRead(apiFetch, `/v1/nni/config`);
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_config_load_http_${res.status}`);
      }
      applyNniConfigResponse(body.data);
      if (!silent) setNniConfigMessage(null);
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("NNI 配置暂时无法读取。", "NNI configuration is temporarily unavailable."));
      if (!silent) setNniConfigError(message);
    } finally {
      if (!silent) setNniConfigLoading(false);
    }
    },
  );

  const fetchNniHeartbeatRecords = (page = nniHeartbeatRecordsPage, silent = false) => {
    const safePage = Math.max(1, page);
    return runCoalescedRead(readRequestsRef.current, `heartbeat-records:${safePage}`, async () => {
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
      const res = await fetchResilientRead(apiFetch, `/v1/nni/records?${params.toString()}`);
      const body = (await res.json()) as ApiResponse<NniHeartbeatRecordsResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_request_records_load_http_${res.status}`);
      }
      setNniHeartbeatRecords(body.data.records ?? []);
      setNniHeartbeatRecordsPage(body.data.page || safePage);
      setNniHeartbeatRecordsTotal(body.data.total ?? 0);
      setNniHeartbeatRecordsTotalPages(Math.max(1, body.data.total_pages ?? 1));
      setNniHeartbeatRecordsError(null);
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("NNI 请求记录暂时无法读取。", "NNI request history is temporarily unavailable."));
      if (!silent) setNniHeartbeatRecordsError(message);
    } finally {
      if (!silent) setNniHeartbeatRecordsLoading(false);
    }
    });
  };

  const clearNniHeartbeatRecords = async () => {
    const confirmed = await showConfirm({
      title: t("清理 NNI 请求记录", "Clear NNI request records"),
      message: t(
        "确定清理本机 NNI 请求记录吗？这只会清理本机保存的加入和心跳历史，不会修改远程 NNI 服务端记录。",
        "Clear local NNI request records? This only clears Join and Heartbeat history saved on this device and will not change remote NNI server records.",
      ),
      confirmLabel: t("清理", "Clear"),
      tone: "danger",
    });
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
        throw new Error(body.error || `nni_request_records_clear_http_${res.status}`);
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
      const message = formatNniErrorCause(err, t, t("NNI 请求记录清理失败。", "NNI request history could not be cleared."));
      setNniHeartbeatRecordsError(message);
    } finally {
      setNniHeartbeatRecordsClearing(false);
    }
  };

  const fetchNniHeartbeatErrors = (page = nniHeartbeatErrorsPage, silent = false) => {
    const safePage = Math.max(1, page);
    return runCoalescedRead(readRequestsRef.current, `heartbeat-errors:${safePage}`, async () => {
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
      const res = await fetchResilientRead(apiFetch, `/v1/nni/heartbeat/errors?${params.toString()}`);
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
        throw new Error(body.error || `nni_heartbeat_errors_load_http_${res.status}`);
      }
      setNniHeartbeatErrors(body.data.records ?? []);
      setNniHeartbeatErrorsPage(body.data.page || safePage);
      setNniHeartbeatErrorsTotal(body.data.total ?? 0);
      setNniHeartbeatErrorsTotalPages(Math.max(1, body.data.total_pages ?? 1));
      setNniHeartbeatErrorsError(null);
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("NNI 心跳错误暂时无法读取。", "NNI heartbeat errors are temporarily unavailable."));
      if (!silent) setNniHeartbeatErrorsError(message);
    } finally {
      if (!silent) setNniHeartbeatErrorsLoading(false);
    }
    });
  };

  const clearNniHeartbeatErrors = async () => {
    const confirmed = await showConfirm({
      title: t("清理心跳错误", "Clear heartbeat errors"),
      message: t(
        "确定清理本机心跳错误记录吗？这只会清理本机页面里的错误历史，不会修改远程 NNI 服务端请求记录。",
        "Clear local heartbeat error history? This only clears the local error history shown here and will not change remote NNI server request records.",
      ),
      confirmLabel: t("清理", "Clear"),
      tone: "danger",
    });
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
        throw new Error(body.error || `nni_heartbeat_errors_clear_http_${res.status}`);
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
      const message = formatNniErrorCause(err, t, t("NNI 心跳错误清理失败。", "NNI heartbeat errors could not be cleared."));
      setNniHeartbeatErrorsError(message);
    } finally {
      setNniHeartbeatErrorsClearing(false);
    }
  };

  const fetchNniRewards = (page = nniRewards?.page ?? 1, silent = false) => {
    const safePage = Math.max(1, page);
    return runCoalescedRead(readRequestsRef.current, `rewards:${safePage}`, async () => {
    if (!silent) {
      setNniRewardsLoading(true);
      setNniRewardsError(null);
    }
    try {
      const params = new URLSearchParams({
        page: String(safePage),
        per_page: String(NNI_REWARDS_PAGE_SIZE),
      });
      const res = await fetchResilientRead(apiFetch, `/v1/nni/rewards?${params.toString()}`);
      const body = (await res.json()) as ApiResponse<NniRewardsResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_reward_records_load_http_${res.status}`);
      }
      setNniRewards(body.data);
      setNniRewardsError(null);
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("NNI 奖励记录暂时无法读取。", "NNI reward history is temporarily unavailable."));
      if (!silent) setNniRewardsError(message);
    } finally {
      if (!silent) setNniRewardsLoading(false);
    }
    });
  };

  const fetchNniNetworkStats = (silent = false) => runCoalescedRead(
    readRequestsRef.current,
    "network-stats",
    async () => {
      if (!silent) {
        setNniNetworkStatsLoading(true);
        setNniNetworkStatsError(null);
      }
      try {
        const res = await fetchResilientRead(apiFetch, "/v1/nni/network-stats");
        const body = (await res.json()) as ApiResponse<NniNetworkStatsResponse>;
        if (!res.ok || !body.ok || !body.data) {
          throw new Error(body.error || `nni_network_stats_load_http_${res.status}`);
        }
        setNniNetworkStats(body.data);
        setNniNetworkStatsError(null);
      } catch (err) {
        const message = formatNniErrorCause(err, t, t("NNI 网络状态暂时无法读取。", "NNI network status is temporarily unavailable."));
        if (!silent) setNniNetworkStatsError(message);
      } finally {
        if (!silent) setNniNetworkStatsLoading(false);
      }
    },
  );

  const saveNniConfig = async () => {
    setNniConfigSaving(true);
    setNniConfigError(null);
    setNniConfigMessage(null);
    try {
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          remote_nodes: nniRemoteNodeUrls(),
          selected_node_url: selectedNniNodeUrl(),
          bancor_service_node_url: selectedNniBancorServiceNodeUrl(),
          asset_service_node_url: selectedNniAssetServiceNodeUrl(),
        }),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_config_save_http_${res.status}`);
      }
      applyNniConfigResponse(body.data);
      setNniConfigMessage(
        t(
          "远程 NNI 节点已保存到独立运行数据，不会修改主配置文件。",
          "Remote NNI nodes were saved to independent runtime data without changing the main config file.",
        ),
      );
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("NNI 配置保存失败。", "NNI configuration could not be saved."));
      setNniConfigError(message);
    } finally {
      setNniConfigSaving(false);
    }
  };

  const testJoinNni = async () => {
    const result = await runNniDeviceAction("sign_timestamp");
    if (nniTimestampSignatureReady(result)) {
      setNniActionMessage(
        t(
          "测试签名已完成：本机已生成时间戳签名。只有点击加入并通过服务端验签后，才会开启运行状态。",
          "Test signature completed: this device generated a timestamp signature. The runtime starts only after Join passes server verification.",
        ),
      );
    }
    return result;
  };

  const joinNni = async () => {
    setNniActionLoading("join_nni");
    setNniActionError(null);
    setNniActionMessage(null);
    setNniDeviceAuthorizationDenied(false);
    const status = nniStatus ?? (await fetchNniDeviceStatus(false));
    if (!status?.signature_chip_present) {
      setNniActionError(
        nniDeviceMessage(status, lang) ||
          t(
            "未检测到芯片，暂时不能加入需要设备签名的 NNI。",
            "No chip was detected, so this device cannot join signed NNI yet.",
          ),
      );
      await setNniJoinedPersisted(false);
      setNniActionLoading(null);
      return;
    }
    try {
      const ownerKeyPair = nniOwnerKeyPair;
      if (!nniAssetOwnerPubkey && !ownerKeyPair) {
        await generateNniOwnerKeyPair();
        return;
      }
      if (!nniAssetOwnerPubkey && ownerKeyPair) {
        await authorizeNniOwnerWithPrivateKey(ownerKeyPair.private_key);
        return;
      }
      const task = await requestNniJoinTask(nniAssetOwnerPubkey);
      if (!task?.challenge) {
        throw new Error("nni_join_challenge_missing");
      }
      if (task.owner_signature_required) {
        throw new Error(t(
          "远程节点要求资产密钥签名，请通过资产账户的重新绑定流程完成授权。",
          "The remote node requires an asset-key signature. Complete authorization through the asset-account rebind flow.",
        ));
      }
      const signatureResult = await runNniDeviceAction("sign_challenge", { challenge: task.challenge });
      const signature = signatureResult?.payload?.signature;
      if (!signature) {
        throw new Error("nni_join_signature_missing");
      }
      setNniActionLoading("join_nni");
      const verified = await verifyNniJoinTask(
        task.task_id,
        task.node_url,
        signature,
      );
      if (!verified?.joined || !verified.compliant) {
        throw new Error("nni_join_verify_rejected");
      }
      await setNniJoinedPersisted(true, { persistRemoteNodes: true });
      setNniAssetOwnerPubkey(verified.asset_owner_pubkey ?? task.asset_owner_pubkey ?? null);
      setNniOwnerKeyPair(null);
      setNniDeviceAuthorizationDenied(false);
      setNniActionMessage(
        t(
          "设备签名已通过服务端验证，NNI 已开始运行。",
          "The device signature was verified by the server, and NNI is now running.",
        ),
      );
      await fetchNniHeartbeatRecords(1, true);
      await fetchNniRewards(1, true);
    } catch (err) {
      const message = formatNniErrorCause(err, t, t("加入 NNI 未完成。", "Joining NNI did not complete."));
      setNniActionError(message);
      await setNniJoinedPersisted(false);
    } finally {
      setNniActionLoading(null);
    }
  };

  const updateNniRemoteNodes = (value: string) => {
    setNniRemoteNodes(value);
    const nodeUrls = parseNniRemoteNodeUrls(value);
    if (!nodeUrls.includes(nniSelectedNodeUrl)) {
      setNniSelectedNodeUrl(nodeUrls[0] ?? "");
    }
    if (!nodeUrls.includes(nniAssetServiceNodeUrl)) {
      setNniAssetServiceNodeUrl(
        nodeUrls.includes(nniSelectedNodeUrl) ? nniSelectedNodeUrl : nodeUrls[0] ?? "",
      );
    }
    if (!nodeUrls.includes(nniBancorServiceNodeUrl)) {
      setNniBancorServiceNodeUrl(
        nodeUrls.includes(nniSelectedNodeUrl) ? nniSelectedNodeUrl : nodeUrls[0] ?? "",
      );
    }
    setNniDeviceAuthorizationDenied(false);
    setNniConfigMessage(null);
    setNniConfigError(null);
  };

  const updateNniSelectedNodeUrl = (value: string) => {
    if (!nniRemoteNodeUrls().includes(value)) return;
    setNniSelectedNodeUrl(value);
    setNniDeviceAuthorizationDenied(false);
    setNniConfigMessage(null);
    setNniConfigError(null);
  };

  const updateNniAssetServiceNodeUrl = async (value: string): Promise<boolean> => {
    if (!nniRemoteNodeUrls().includes(value)) {
      setNniAssetServiceNodeError(t("请选择已配置的节点。", "Select a configured node."));
      return false;
    }
    if (value === selectedNniAssetServiceNodeUrl()) return true;
    setNniAssetServiceNodeSaving(true);
    setNniAssetServiceNodeError(null);
    try {
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ asset_service_node_url: value }),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_asset_service_node_update_http_${res.status}`);
      }
      applyNniConfigResponse(body.data);
      return true;
    } catch (err) {
      setNniAssetServiceNodeError(formatNniErrorCause(
        err,
        t,
        t("资产服务节点切换失败。", "Asset service node switch failed."),
      ));
      return false;
    } finally {
      setNniAssetServiceNodeSaving(false);
    }
  };

  const updateNniBancorServiceNodeUrl = async (value: string): Promise<boolean> => {
    if (!nniRemoteNodeUrls().includes(value)) {
      setNniBancorServiceNodeError(t("请选择已配置的节点。", "Select a configured node."));
      return false;
    }
    if (value === selectedNniBancorServiceNodeUrl()) return true;
    setNniBancorServiceNodeSaving(true);
    setNniBancorServiceNodeError(null);
    try {
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ bancor_service_node_url: value }),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_bancor_service_node_update_http_${res.status}`);
      }
      applyNniConfigResponse(body.data);
      return true;
    } catch (err) {
      setNniBancorServiceNodeError(formatNniErrorCause(
        err,
        t,
        t("BANCOR 节点切换失败。", "BANCOR node switch failed."),
      ));
      return false;
    } finally {
      setNniBancorServiceNodeSaving(false);
    }
  };

  const addNniFinancialServiceNodeUrl = async (
    service: "assets" | "bancor",
    value: string,
  ): Promise<boolean> => {
    const currentNodes = nniRemoteNodeUrls();
    if (currentNodes.includes(value)) {
      return service === "assets"
        ? updateNniAssetServiceNodeUrl(value)
        : updateNniBancorServiceNodeUrl(value);
    }
    const setSaving = service === "assets"
      ? setNniAssetServiceNodeSaving
      : setNniBancorServiceNodeSaving;
    const setError = service === "assets"
      ? setNniAssetServiceNodeError
      : setNniBancorServiceNodeError;
    setSaving(true);
    setError(null);
    try {
      const selectedHeartbeatNode = selectedNniNodeUrl();
      const payload: {
        remote_nodes: string[];
        selected_node_url?: string;
        bancor_service_node_url: string;
        asset_service_node_url: string;
      } = {
        remote_nodes: [...currentNodes, value],
        bancor_service_node_url: service === "bancor"
          ? value
          : selectedNniBancorServiceNodeUrl(),
        asset_service_node_url: service === "assets"
          ? value
          : selectedNniAssetServiceNodeUrl(),
      };
      if (selectedHeartbeatNode) payload.selected_node_url = selectedHeartbeatNode;
      const res = await apiFetch(`/v1/nni/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = (await res.json()) as ApiResponse<NniConfigResponse>;
      if (!res.ok || !body.ok || !body.data) {
        throw new Error(body.error || `nni_financial_service_node_update_http_${res.status}`);
      }
      applyNniConfigResponse(body.data);
      return true;
    } catch (err) {
      setError(formatNniErrorCause(
        err,
        t,
        t("自定义节点添加失败。", "Failed to add the custom node."),
      ));
      return false;
    } finally {
      setSaving(false);
    }
  };

  const addNniAssetServiceNodeUrl = (value: string) =>
    addNniFinancialServiceNodeUrl("assets", value);
  const addNniBancorServiceNodeUrl = (value: string) =>
    addNniFinancialServiceNodeUrl("bancor", value);

  return {
    nniStatus,
    nniStatusLoading,
    nniStatusError,
    nniActionLoading,
    nniActionResult,
    nniActionError,
    nniActionMessage,
    nniDeviceAuthorizationDenied,
    nniJoined,
    nniAssetOwnerPubkey,
    nniOwnerKeyPair,
    nniOwnerActionLoading,
    nniOwnerAuthorizationChallenge,
    nniRemoteNodes,
    nniRemoteNodeUrls: nniRemoteNodeUrls(),
    nniSelectedNodeUrl: selectedNniNodeUrl(),
    nniBancorServiceNodeUrl: selectedNniBancorServiceNodeUrl(),
    nniBancorServiceNodeSaving,
    nniBancorServiceNodeError,
    nniAssetServiceNodeUrl: selectedNniAssetServiceNodeUrl(),
    nniAssetServiceNodeSaving,
    nniAssetServiceNodeError,
    nniRemoteNodeCount: nniRemoteNodeUrls().length,
    nniHeartbeatIntervalSeconds,
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
    nniRewards,
    nniRewardsLoading,
    nniRewardsError,
    nniNetworkStats,
    nniNetworkStatsLoading,
    nniNetworkStatsError,
    nniConfigLoading,
    nniConfigSaving,
    nniConfigError,
    nniConfigMessage,
    setNniActionMessage,
    setNniActionError,
    fetchNniDeviceStatus,
    ensureNniDeviceStatus,
    setNniJoinedPersisted,
    joinNni,
    generateNniOwnerKeyPair,
    clearNniOwnerKeyPair,
    recoverNniOwner,
    startNniCustomOwnerAuthorization,
    authorizeNniOwnerWithPrivateKey,
    startNniOwnerUnbind,
    completeNniOwnerAuthorization,
    cancelNniOwnerAuthorization,
    testJoinNni,
    fetchNniConfig,
    saveNniConfig,
    updateNniRemoteNodes,
    updateNniSelectedNodeUrl,
    updateNniBancorServiceNodeUrl,
    updateNniAssetServiceNodeUrl,
    addNniBancorServiceNodeUrl,
    addNniAssetServiceNodeUrl,
    fetchNniHeartbeatRecords,
    clearNniHeartbeatRecords,
    fetchNniHeartbeatErrors,
    clearNniHeartbeatErrors,
    fetchNniRewards,
    fetchNniNetworkStats,
    runNniDeviceAction,
    setNniDeviceSimulation,
  };
}
