import { secp256k1 } from "@noble/curves/secp256k1.js";
import { ripemd160 } from "@noble/hashes/legacy.js";
import { base58 } from "@scure/base";

const OWNER_PUBLIC_KEY_BYTES = 33;
const OWNER_PRIVATE_KEY_BYTES = 32;
const OWNER_CHECKSUM_BYTES = 4;
const OWNER_KEY_SUFFIX = new TextEncoder().encode("K1");

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
