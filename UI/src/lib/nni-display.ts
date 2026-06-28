import type { NniDeviceActionResponse, NniDevicePayload } from "../types/api";

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

function copy(lang: UiLanguage, zh: string, en: string): string {
  return lang === "zh" ? zh : en;
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
      "本机公钥必须是白名单合规公钥。请读取并复制本机公钥，确认远程 NNI 服务端白名单已允许该公钥后再重试。",
      "The local public key must be compliant with the whitelist. Read and copy this device public key, confirm the remote NNI server allows it, then retry.",
    );
  }
  if (code === "nni_public_key_whitelist_empty" || code === "public_key_whitelist_empty") {
    return copy(
      lang,
      "本机公钥必须是白名单合规公钥。远程 NNI 服务端还没有配置允许的公钥，请确定你是合法设备以后再重试。",
      "The local public key must be compliant with the whitelist. The remote NNI server has no allowed public keys configured yet; confirm this is an authorized device, then retry.",
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
