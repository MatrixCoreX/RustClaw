import type { NniDeviceActionResponse, NniDevicePayload, NniDeviceStatusResponse } from "../types/api";
import { productCopy } from "./product-identity";

export type UiLanguage = "zh" | "en";

export interface NniPayloadHexField {
  label: string;
  value: string;
  size?: number;
}

export interface NniRuntimeTile {
  delay: number;
  duration: number;
  idleOpacity: number;
}

export type NniSimulationControlMode = "enable" | "disable" | null;

function copy(lang: UiLanguage, zh: string, en: string): string {
  return productCopy(lang === "zh" ? zh : en);
}

export function shortenHex(value?: string | null, head = 16, tail = 16): string {
  const trimmed = value?.trim() ?? "";
  if (!trimmed) return "--";
  if (trimmed.length <= head + tail + 3) return trimmed;
  return `${trimmed.slice(0, head)}...${trimmed.slice(-tail)}`;
}

export function shortNniValue(value?: string | null): string {
  const trimmed = value?.trim() ?? "";
  if (!trimmed) return "--";
  if (trimmed.length <= 22) return trimmed;
  return `${trimmed.slice(0, 10)}...${trimmed.slice(-8)}`;
}

export function nniPayloadHexField(payload?: NniDevicePayload | null): NniPayloadHexField | null {
  if (!payload) return null;
  if (payload.signature) return { label: "signature", value: payload.signature };
  if (payload.pubkey) return { label: "pubkey", value: payload.pubkey };
  if (payload.device_cert_hex) {
    return { label: "device_cert_hex", value: payload.device_cert_hex, size: payload.device_cert_hex_size };
  }
  if (payload.signer_cert_hex) {
    return { label: "signer_cert_hex", value: payload.signer_cert_hex, size: payload.signer_cert_hex_size };
  }
  if (payload.root_cert_hex) {
    return { label: "root_cert_hex", value: payload.root_cert_hex, size: payload.root_cert_hex_size };
  }
  return null;
}

export function nniTimestampSignatureReady(value?: NniDeviceActionResponse | null): boolean {
  const payload = value?.payload;
  return (
    value?.action === "sign_timestamp" &&
    typeof payload?.timestamp === "number" &&
    Number.isFinite(payload.timestamp) &&
    typeof payload.signature === "string" &&
    payload.signature.trim().length > 0
  );
}

export function nniDeviceMessage(
  value: NniDeviceStatusResponse | NniDeviceActionResponse | null | undefined,
  lang: UiLanguage,
  fallback?: string,
): string | null {
  const message = messageForNniKey(value?.message_key, lang);
  return message ?? value?.message ?? fallback ?? null;
}

export function nniDeviceNextStep(
  value: NniDeviceStatusResponse | null | undefined,
  lang: UiLanguage,
): string | null {
  const message = messageForNniKey(value?.next_step_key, lang);
  return message ?? value?.next_step ?? null;
}

export function nniSimulationControlMode(
  status: NniDeviceStatusResponse | null | undefined,
  statusLoading: boolean,
): NniSimulationControlMode {
  if (statusLoading || !status) return null;
  if (status.simulated === true) return "disable";
  if (
    status.signature_chip_present === false &&
    status.status === "signature_chip_missing" &&
    status.simulation_available === true
  ) {
    return "enable";
  }
  return null;
}

function messageForNniKey(key: string | null | undefined, lang: UiLanguage): string | null {
  switch (key) {
    case "nni.device_status.helper_missing":
      return copy(lang, "设备签名 helper 未安装，无法检测芯片。", "The device-signing helper is not installed, so the chip cannot be checked.");
    case "nni.device_status.helper_missing.next_step":
      return copy(
        lang,
        "如果本设备需要 NNI 设备签名，请确认 pi_app/signature.py 已随 {product_name} 一起部署。",
        "If this device needs NNI device signing, confirm pi_app/signature.py was deployed with {product_name}.",
      );
    case "nni.device_status.ready":
      return copy(lang, "已检测到芯片，NNI 设备签名可用。", "A chip was detected, and NNI device signing is available.");
    case "nni.device_status.simulated":
      return copy(lang, "软件模拟芯片正在运行，可用于本机协议测试。", "The software-simulated chip is running and can be used for local protocol testing.");
    case "nni.device_status.simulated.next_step":
      return copy(
        lang,
        "这是测试身份，不代表已连接真实芯片，也不具备硬件级密钥保护。远程节点仍可能拒绝未加入白名单的模拟公钥。",
        "This is a test identity, not a real hardware chip, and it has no hardware key protection. Remote nodes may still reject a simulated public key that is not allowlisted.",
      );
    case "nni.device_status.signature_chip_missing":
      return copy(
        lang,
        "未检测到 MatrixAI 芯片。此设备仍可使用 {product_name} 的其他功能。",
        "No MatrixAI chip was detected. This device can still use other {product_name} features.",
      );
    case "nni.device_status.signature_chip_missing.next_step":
      return copy(
        lang,
        "正式参与网络原生智能需要带芯片的 MatrixAI 硬件；若只做本机协议测试，检测确认后可使用模拟芯片。",
        "Production participation in Network Native Intelligence requires MatrixAI hardware with a chip. For local protocol testing only, simulation becomes available after detection is confirmed.",
      );
    case "nni.device_action.completed":
      return copy(lang, "NNI 设备签名操作完成。", "NNI device signing action completed.");
    case "nni.device_action.simulation_enabled":
      return copy(lang, "模拟芯片已启动。", "The simulated chip is now running.");
    case "nni.device_action.simulation_disabled":
      return copy(lang, "模拟芯片已停止。", "The simulated chip has stopped.");
    case "nni.device_action.simulation_failed":
      return copy(
        lang,
        "无法启动模拟芯片。请确认 {product_name} 的 data 目录可写，然后重试。",
        "The simulated chip could not start. Confirm that {product_name} can write to its data directory, then try again.",
      );
    case "nni.device_action.simulation_not_needed":
      return copy(lang, "已检测到真实芯片，无需启用模拟。", "A real chip was detected, so simulation is not needed.");
    case "nni.device_action.signature_chip_missing":
      return copy(lang, "未检测到芯片，无法完成本次 NNI 签名操作。", "No chip was detected, so this NNI signing action cannot be completed.");
    default:
      return null;
  }
}

export function findNniJoinErrorCode(data?: unknown): string | null {
  if (!data || typeof data !== "object") return null;
  const record = data as Record<string, unknown>;
  const directError = typeof record.error === "string" ? record.error : null;
  if (directError) return directError;
  const status = typeof record.status === "string" ? record.status : null;
  if (status === "public_key_not_allowlisted" || status === "public_key_whitelist_empty") return status;
  if (Array.isArray(record.attempts)) {
    for (const attempt of record.attempts) {
      const attemptCode = findNniJoinErrorCode(attempt);
      if (attemptCode) return attemptCode;
    }
  }
  return null;
}

export function parseNniRemoteNodeUrls(value: string): string[] {
  return value
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function nniJoinErrorMessage(
  error: string | undefined,
  data: unknown,
  fallback: string,
  lang: UiLanguage,
): string {
  const code = error || findNniJoinErrorCode(data);
  if (code === "nni_pubkey_not_allowlisted" || code === "nni_public_key_not_allowlisted" || code === "public_key_not_allowlisted") {
    return copy(
      lang,
      "这台设备尚未获远程 NNI 服务端允许。请联系管理员添加设备后再重试。",
      "This device is not allowed by the remote NNI server yet. Ask an administrator to add it, then retry.",
    );
  }
  if (code === "nni_public_key_whitelist_empty" || code === "public_key_whitelist_empty") {
    return copy(
      lang,
      "远程 NNI 服务端尚未允许任何设备。请联系管理员完成设备授权后再重试。",
      "The remote NNI server does not allow any devices yet. Ask an administrator to authorize this device, then retry.",
    );
  }
  return error || fallback;
}

export function nniActionLabel(action: string, lang: UiLanguage): string {
  const labels: Record<string, string> = {
    pubkey: copy(lang, "读取 slot 0 公钥", "Read Slot 0 public key"),
    sign_timestamp: copy(lang, "生成时间戳签名", "Sign current timestamp"),
    sign_challenge: copy(lang, "生成挑战签名", "Sign challenge"),
    tng_device_pubkey: copy(lang, "读取 TNG 设备公钥", "Read TNG device public key"),
    tng_device_cert: copy(lang, "读取设备证书", "Read device certificate"),
    tng_signer_cert: copy(lang, "读取 signer 证书", "Read signer certificate"),
    tng_root_cert: copy(lang, "读取根证书", "Read root certificate"),
  };
  return labels[action] || action;
}

export const NNI_RUNTIME_TILES: NniRuntimeTile[] = Array.from({ length: 32 }, (_, index) => {
  const random = (salt: number) => {
    const value = Math.sin((index + 1) * (salt + 12.9898)) * 43758.5453;
    return value - Math.floor(value);
  };
  return {
    delay: -(random(1) * 2.8),
    duration: 1.1 + random(2) * 1.9,
    idleOpacity: 0.55 + random(3) * 0.25,
  };
});
