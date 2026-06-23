import test from "node:test";
import assert from "node:assert/strict";

import {
  NNI_RUNTIME_TILES,
  findNniJoinErrorCode,
  nniPayloadHexField,
  shortenHex,
  shortNniValue,
} from "./nni-display.ts";

test("shortens long hex values with stable head and tail", () => {
  assert.equal(shortenHex("abcdef0123456789", 4, 4), "abcd...6789");
  assert.equal(shortenHex("", 4, 4), "--");
});

test("shortens NNI identifiers for compact rows", () => {
  assert.equal(shortNniValue("node-1234567890-abcdefghi"), "node-12345...bcdefghi");
  assert.equal(shortNniValue("short"), "short");
});

test("selects the first available NNI payload hex field", () => {
  assert.deepEqual(
    nniPayloadHexField({
      device_cert_hex: "device-cert",
      device_cert_hex_size: 11,
      signer_cert_hex: "signer-cert",
      root_cert_hex: "root-cert",
    }),
    { label: "device_cert_hex", value: "device-cert", size: 11 },
  );
  assert.deepEqual(nniPayloadHexField({ pubkey: "pubkey-value" }), {
    label: "pubkey",
    value: "pubkey-value",
  });
});

test("finds nested NNI join error codes", () => {
  assert.equal(
    findNniJoinErrorCode({
      attempts: [
        { status: "pending" },
        { attempts: [{ status: "public_key_whitelist_empty" }] },
      ],
    }),
    "public_key_whitelist_empty",
  );
});

test("builds deterministic runtime tiles", () => {
  assert.equal(NNI_RUNTIME_TILES.length, 32);
  assert.ok(NNI_RUNTIME_TILES.every((tile) => tile.duration >= 1.1 && tile.duration <= 3.0));
});
