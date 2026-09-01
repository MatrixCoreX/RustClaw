import { secp256k1 } from "@noble/curves/secp256k1.js";
import { ripemd160 } from "@noble/hashes/legacy.js";
import { base58 } from "@scure/base";

const OWNER_PUBLIC_KEY_BYTES = 33;
const OWNER_PRIVATE_KEY_BYTES = 32;
const OWNER_CHECKSUM_BYTES = 4;
const OWNER_KEY_SUFFIX = new TextEncoder().encode("K1");
const OWNER_RANDOM_SEED_BYTES = 48;

export const NNI_PRIVATE_KEY_INSECURE_TRANSPORT_ERROR = "nni_private_key_insecure_transport";

export interface NniOwnerKeyPair {
  key_type: "K1";
  encoding: "eos_base58_v1";
  public_key: string;
  private_key: string;
}

export interface NniOwnerKeyPairBackup {
  schema_version: 1;
  document_type: "asset_account_key_pair";
  key_type: NniOwnerKeyPair["key_type"];
  encoding: NniOwnerKeyPair["encoding"];
  public_key: string;
  private_key: string;
}

export interface NniPrivateKeyLocation {
  protocol: string;
  hostname: string;
}

export type NniOwnerPublicKeyError =
  | "required"
  | "prefix_not_allowed"
  | "encoding_invalid"
  | "length_invalid"
  | "checksum_invalid"
  | "curve_point_invalid";

export type NniOwnerPublicKeyValidation =
  | { ok: true; normalized: string }
  | { ok: false; error: NniOwnerPublicKeyError };

export type NniOwnerPrivateKeyError =
  | "required"
  | "prefix_not_allowed"
  | "encoding_invalid"
  | "length_invalid"
  | "checksum_invalid"
  | "scalar_invalid";

export type NniOwnerPrivateKeyValidation =
  | { ok: true; normalized: string; publicKey: string }
  | { ok: false; error: NniOwnerPrivateKeyError };

export interface NniOwnerChallengeSignature {
  publicKey: string;
  signature: string;
}

function concatenate(left: Uint8Array, right: Uint8Array): Uint8Array {
  const combined = new Uint8Array(left.length + right.length);
  combined.set(left);
  combined.set(right, left.length);
  return combined;
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function ownerChecksum(payload: Uint8Array): Uint8Array {
  return ripemd160(concatenate(payload, OWNER_KEY_SUFFIX)).slice(0, OWNER_CHECKSUM_BYTES);
}

function encodeOwnerPublicKey(publicKey: Uint8Array): string {
  return base58.encode(concatenate(publicKey, ownerChecksum(publicKey)));
}

function encodeOwnerPrivateKey(secretKey: Uint8Array): string {
  return base58.encode(concatenate(secretKey, ownerChecksum(secretKey)));
}

function browserLocation(): NniPrivateKeyLocation | null {
  return typeof window === "undefined"
    ? null
    : { protocol: window.location.protocol, hostname: window.location.hostname };
}

function isIpv4Loopback(hostname: string): boolean {
  const parts = hostname.split(".");
  return parts.length === 4
    && parts[0] === "127"
    && parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255);
}

export function nniPrivateKeyOperationsAllowed(
  location: NniPrivateKeyLocation | null = browserLocation(),
): boolean {
  // Non-browser callers cannot collect a key through a web transport. Browser callers must
  // use TLS unless they are on the loopback secure-context exception.
  if (!location) return true;
  if (location.protocol === "https:") return true;
  if (location.protocol !== "http:") return false;
  const hostname = location.hostname.trim().toLowerCase().replace(/^\[|\]$/g, "");
  return hostname === "localhost" || hostname === "::1" || isIpv4Loopback(hostname);
}

export function assertNniPrivateKeyOperationsAllowed(
  location: NniPrivateKeyLocation | null = browserLocation(),
): void {
  if (!nniPrivateKeyOperationsAllowed(location)) {
    throw new Error(NNI_PRIVATE_KEY_INSECURE_TRANSPORT_ERROR);
  }
}

export function generateNniOwnerKeyPair(): NniOwnerKeyPair {
  assertNniPrivateKeyOperationsAllowed();
  if (!globalThis.crypto?.getRandomValues) {
    throw new Error("nni_private_key_secure_random_unavailable");
  }
  const seed = globalThis.crypto.getRandomValues(new Uint8Array(OWNER_RANDOM_SEED_BYTES));
  const secretKey = secp256k1.utils.randomSecretKey(seed);
  seed.fill(0);
  try {
    return {
      key_type: "K1",
      encoding: "eos_base58_v1",
      public_key: encodeOwnerPublicKey(secp256k1.getPublicKey(secretKey, true)),
      private_key: encodeOwnerPrivateKey(secretKey),
    };
  } finally {
    secretKey.fill(0);
  }
}

export function serializeNniOwnerKeyPairBackup(keyPair: NniOwnerKeyPair): string {
  const backup: NniOwnerKeyPairBackup = {
    schema_version: 1,
    document_type: "asset_account_key_pair",
    key_type: keyPair.key_type,
    encoding: keyPair.encoding,
    public_key: keyPair.public_key,
    private_key: keyPair.private_key,
  };
  return `${JSON.stringify(backup, null, 2)}\n`;
}

export function nniOwnerKeyPairBackupFilename(keyPair: NniOwnerKeyPair): string {
  const publicKeyPrefix = keyPair.public_key.slice(0, 12);
  return `asset-account-keypair-${publicKeyPrefix}.json`;
}

function decodeOwnerPrivateKey(value: string):
  | { ok: true; normalized: string; secretKey: Uint8Array }
  | { ok: false; error: NniOwnerPrivateKeyError } {
  const normalized = value.trim();
  if (!normalized) return { ok: false, error: "required" };
  if (/^(EOS|PUB_|PVT_)/.test(normalized)) {
    return { ok: false, error: "prefix_not_allowed" };
  }

  let decoded: Uint8Array;
  try {
    decoded = base58.decode(normalized);
  } catch {
    return { ok: false, error: "encoding_invalid" };
  }
  if (decoded.length !== OWNER_PRIVATE_KEY_BYTES + OWNER_CHECKSUM_BYTES) {
    return { ok: false, error: "length_invalid" };
  }

  const secretKey = decoded.slice(0, OWNER_PRIVATE_KEY_BYTES);
  const checksum = decoded.slice(OWNER_PRIVATE_KEY_BYTES);
  if (!equalBytes(checksum, ownerChecksum(secretKey)) || base58.encode(decoded) !== normalized) {
    secretKey.fill(0);
    return { ok: false, error: "checksum_invalid" };
  }
  if (!secp256k1.utils.isValidSecretKey(secretKey)) {
    secretKey.fill(0);
    return { ok: false, error: "scalar_invalid" };
  }
  return { ok: true, normalized, secretKey };
}

function bytesToHex(value: Uint8Array): string {
  return Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function validateNniOwnerPublicKey(value: string): NniOwnerPublicKeyValidation {
  const normalized = value.trim();
  if (!normalized) return { ok: false, error: "required" };
  if (/^(EOS|PUB_|PVT_)/.test(normalized)) {
    return { ok: false, error: "prefix_not_allowed" };
  }

  let decoded: Uint8Array;
  try {
    decoded = base58.decode(normalized);
  } catch {
    return { ok: false, error: "encoding_invalid" };
  }
  if (decoded.length !== OWNER_PUBLIC_KEY_BYTES + OWNER_CHECKSUM_BYTES) {
    return { ok: false, error: "length_invalid" };
  }

  const payload = decoded.slice(0, OWNER_PUBLIC_KEY_BYTES);
  const checksum = decoded.slice(OWNER_PUBLIC_KEY_BYTES);
  const expectedChecksum = ownerChecksum(payload);
  if (!equalBytes(checksum, expectedChecksum) || base58.encode(decoded) !== normalized) {
    return { ok: false, error: "checksum_invalid" };
  }
  try {
    secp256k1.Point.fromBytes(payload);
  } catch {
    return { ok: false, error: "curve_point_invalid" };
  }
  return { ok: true, normalized };
}

export function validateNniOwnerPrivateKey(value: string): NniOwnerPrivateKeyValidation {
  const decoded = decodeOwnerPrivateKey(value);
  if (decoded.ok === false) return { ok: false, error: decoded.error };
  try {
    return {
      ok: true,
      normalized: decoded.normalized,
      publicKey: encodeOwnerPublicKey(secp256k1.getPublicKey(decoded.secretKey, true)),
    };
  } finally {
    decoded.secretKey.fill(0);
  }
}

export function signNniOwnerChallenge(
  privateKey: string,
  signingPayload: string,
): NniOwnerChallengeSignature {
  const decoded = decodeOwnerPrivateKey(privateKey);
  if (decoded.ok === false) throw new Error(`nni_owner_private_key_${decoded.error}`);
  try {
    const publicKey = encodeOwnerPublicKey(secp256k1.getPublicKey(decoded.secretKey, true));
    const signature = secp256k1.sign(
      new TextEncoder().encode(signingPayload),
      decoded.secretKey,
      { format: "compact" },
    );
    return { publicKey, signature: bytesToHex(signature) };
  } finally {
    decoded.secretKey.fill(0);
  }
}

export function normalizeNniOwnerSignature(value: string): string | null {
  const normalized = value.trim();
  return /^[0-9a-fA-F]{128}$/.test(normalized) ? normalized.toLowerCase() : null;
}
