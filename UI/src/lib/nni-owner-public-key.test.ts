import assert from "node:assert/strict";
import test from "node:test";

import { secp256k1 } from "@noble/curves/secp256k1.js";
import { ripemd160 } from "@noble/hashes/legacy.js";
import { base58 } from "@scure/base";

import {
  assertNniPrivateKeyOperationsAllowed,
  generateNniOwnerKeyPair,
  NNI_INSECURE_HTTP_PRIVATE_KEY_SESSION_KEY,
  nniInsecureHttpPrivateKeyRiskAccepted,
  nniPrivateKeyOperationsAllowed,
  nniPrivateKeyOperationsRequireHttpRiskAcceptance,
  nniOwnerKeyPairBackupFilename,
  normalizeNniOwnerSignature,
  serializeNniOwnerKeyPairBackup,
  setNniInsecureHttpPrivateKeyRiskAccepted,
  signNniOwnerChallenge,
  validateNniOwnerPrivateKey,
  validateNniOwnerPublicKey,
} from "./nni-owner-public-key";

const VALID_OWNER_PUBLIC_KEY = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";

function concatenate(left: Uint8Array, right: Uint8Array): Uint8Array {
  const result = new Uint8Array(left.length + right.length);
  result.set(left);
  result.set(right, left.length);
  return result;
}

function encodeTestPrivateKey(secretKey: Uint8Array): string {
  const checksum = ripemd160(concatenate(secretKey, new TextEncoder().encode("K1"))).slice(0, 4);
  return base58.encode(concatenate(secretKey, checksum));
}

function hexToBytes(value: string): Uint8Array {
  return Uint8Array.from(value.match(/.{2}/g) ?? [], (byte) => Number.parseInt(byte, 16));
}

test("NNI owner public key validation checks the full K1 envelope", () => {
  assert.deepEqual(validateNniOwnerPublicKey(VALID_OWNER_PUBLIC_KEY), {
    ok: true,
    normalized: VALID_OWNER_PUBLIC_KEY,
  });
  assert.deepEqual(validateNniOwnerPublicKey(`PUB_K1_${VALID_OWNER_PUBLIC_KEY}`), {
    ok: false,
    error: "prefix_not_allowed",
  });
  assert.equal(validateNniOwnerPublicKey(`${VALID_OWNER_PUBLIC_KEY.slice(0, -1)}A`).ok, false);
  assert.deepEqual(validateNniOwnerPublicKey(""), { ok: false, error: "required" });
});

test("NNI owner signatures use canonical fixed-width hex", () => {
  assert.equal(normalizeNniOwnerSignature("AB".repeat(64)), "ab".repeat(64));
  assert.equal(normalizeNniOwnerSignature("ab".repeat(63)), null);
  assert.equal(normalizeNniOwnerSignature("zz".repeat(64)), null);
});

test("NNI owner private keys derive their public identity and sign challenges locally", () => {
  const secretKey = Uint8Array.from({ length: 32 }, (_, index) => index + 1);
  const privateKey = encodeTestPrivateKey(secretKey);
  const validation = validateNniOwnerPrivateKey(privateKey);
  assert.equal(validation.ok, true);
  if (!validation.ok) return;

  assert.deepEqual(validateNniOwnerPublicKey(validation.publicKey), {
    ok: true,
    normalized: validation.publicKey,
  });
  const signed = signNniOwnerChallenge(privateKey, "nni-owner-test-challenge");
  assert.equal(signed.publicKey, validation.publicKey);
  assert.equal(signed.signature.length, 128);

  const decodedPublicKey = base58.decode(validation.publicKey).slice(0, 33);
  assert.equal(secp256k1.verify(
    hexToBytes(signed.signature),
    new TextEncoder().encode("nni-owner-test-challenge"),
    decodedPublicKey,
  ), true);
});

test("NNI owner private-key validation rejects wrappers, bad checksums, and zero scalars", () => {
  const secretKey = Uint8Array.from({ length: 32 }, (_, index) => index + 1);
  const privateKey = encodeTestPrivateKey(secretKey);
  assert.deepEqual(validateNniOwnerPrivateKey(`PVT_K1_${privateKey}`), {
    ok: false,
    error: "prefix_not_allowed",
  });
  assert.equal(validateNniOwnerPrivateKey(`${privateKey.slice(0, -1)}A`).ok, false);
  assert.deepEqual(validateNniOwnerPrivateKey(encodeTestPrivateKey(new Uint8Array(32))), {
    ok: false,
    error: "scalar_invalid",
  });
});

test("NNI owner key pairs are generated locally with a valid K1 envelope", () => {
  const generated = generateNniOwnerKeyPair();
  const privateKey = validateNniOwnerPrivateKey(generated.private_key);
  assert.equal(privateKey.ok, true);
  if (!privateKey.ok) return;
  assert.equal(privateKey.publicKey, generated.public_key);
  assert.equal(validateNniOwnerPublicKey(generated.public_key).ok, true);
});

test("NNI owner key-pair backups are versioned JSON files containing the exact generated keys", () => {
  const generated = generateNniOwnerKeyPair();
  const backup = JSON.parse(serializeNniOwnerKeyPairBackup(generated));
  assert.deepEqual(backup, {
    schema_version: 1,
    document_type: "asset_account_key_pair",
    key_type: "K1",
    encoding: "eos_base58_v1",
    public_key: generated.public_key,
    private_key: generated.private_key,
  });
  assert.equal(
    nniOwnerKeyPairBackupFilename(generated),
    `asset-account-keypair-${generated.public_key.slice(0, 12)}.json`,
  );
});

test("NNI private-key operations require HTTPS except on loopback", () => {
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "https:", hostname: "device.example" }), true);
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "http:", hostname: "localhost" }), true);
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "http:", hostname: "127.9.8.7" }), true);
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "http:", hostname: "[::1]" }), true);
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "http:", hostname: "192.168.1.9" }), false);
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "http:", hostname: "device.example" }), false);
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "http:", hostname: "192.168.1.9" }, true), true);
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "file:", hostname: "" }, true), false);
  assert.equal(nniPrivateKeyOperationsAllowed({ protocol: "file:", hostname: "" }), false);
  assert.equal(nniPrivateKeyOperationsRequireHttpRiskAcceptance({ protocol: "http:", hostname: "192.168.1.9" }), true);
  assert.equal(nniPrivateKeyOperationsRequireHttpRiskAcceptance({ protocol: "http:", hostname: "localhost" }), false);
  assert.equal(nniPrivateKeyOperationsRequireHttpRiskAcceptance({ protocol: "https:", hostname: "device.example" }), false);
  assert.throws(
    () => assertNniPrivateKeyOperationsAllowed({ protocol: "http:", hostname: "192.168.1.9" }),
    /nni_private_key_insecure_transport/,
  );
});

test("NNI insecure HTTP risk acceptance is scoped to caller-provided session storage", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
  };

  setNniInsecureHttpPrivateKeyRiskAccepted(false, storage);
  assert.equal(nniInsecureHttpPrivateKeyRiskAccepted(storage), false);
  setNniInsecureHttpPrivateKeyRiskAccepted(true, storage);
  assert.equal(values.get(NNI_INSECURE_HTTP_PRIVATE_KEY_SESSION_KEY), "true");
  assert.equal(nniInsecureHttpPrivateKeyRiskAccepted(storage), true);
  setNniInsecureHttpPrivateKeyRiskAccepted(false, storage);
  assert.equal(nniInsecureHttpPrivateKeyRiskAccepted(storage), false);
});
