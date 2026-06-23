import type { NniDevicePayload } from "../types/api";

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
