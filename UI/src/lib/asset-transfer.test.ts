import assert from "node:assert/strict";
import test from "node:test";

import { secp256k1 } from "@noble/curves/secp256k1.js";
import { ripemd160 } from "@noble/hashes/legacy.js";
import { base58 } from "@scure/base";

import { validateAssetTransferDraft } from "./asset-transfer";

const KEY_SUFFIX = new TextEncoder().encode("K1");

function ownerPublicKey(seed: number): string {
  const secret = Uint8Array.from({ length: 32 }, (_, index) => (seed + index) % 255 || 1);
  const publicKey = secp256k1.getPublicKey(secret, true);
  const checksumInput = new Uint8Array(publicKey.length + KEY_SUFFIX.length);
  checksumInput.set(publicKey);
  checksumInput.set(KEY_SUFFIX, publicKey.length);
  const checksum = ripemd160(checksumInput).slice(0, 4);
  const encoded = new Uint8Array(publicKey.length + checksum.length);
  encoded.set(publicKey);
  encoded.set(checksum, publicKey.length);
  return base58.encode(encoded);
}

test("asset transfer draft validates K1 accounts and exact eight-decimal units", () => {
  const source = ownerPublicKey(1);
  const recipient = ownerPublicKey(41);
  assert.deepEqual(validateAssetTransferDraft({
    sourcePublicKey: source,
    recipientPublicKey: recipient,
    asset: "AIC",
    amount: "1.25",
    availableBalance: "2.00000000",
  }), {
    ok: true,
    sourcePublicKey: source,
    recipientPublicKey: recipient,
    asset: "AIC",
    amount: "1.25000000",
    amountUnits: 125_000_000n,
  });
});

test("asset transfer draft rejects malformed recipients, self-transfer, precision, and overspend", () => {
  const source = ownerPublicKey(2);
  const recipient = ownerPublicKey(42);
  const common = {
    sourcePublicKey: source,
    recipientPublicKey: recipient,
    asset: "USD" as const,
    availableBalance: "1.00000000",
  };
  assert.deepEqual(validateAssetTransferDraft({
    ...common,
    recipientPublicKey: "invalid",
    amount: "0.1",
  }), { ok: false, error: "recipient_invalid" });
  assert.deepEqual(validateAssetTransferDraft({
    ...common,
    recipientPublicKey: source,
    amount: "0.1",
  }), { ok: false, error: "same_account" });
  assert.deepEqual(validateAssetTransferDraft({ ...common, amount: "0.000000001" }), {
    ok: false,
    error: "amount_invalid",
  });
  assert.deepEqual(validateAssetTransferDraft({ ...common, amount: "1.00000001" }), {
    ok: false,
    error: "insufficient_balance",
  });
});
