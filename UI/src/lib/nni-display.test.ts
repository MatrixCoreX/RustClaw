import test from "node:test";
import assert from "node:assert/strict";

import {
  NNI_RUNTIME_TILES,
  findNniJoinErrorCode,
  nniActionLabel,
  nniJoinErrorMessage,
  nniPayloadHexField,
  parseNniRemoteNodeUrls,
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

test("formats NNI join errors from structured codes", () => {
  assert.match(
    nniJoinErrorMessage(undefined, { status: "public_key_not_allowlisted" }, "fallback", "en"),
    /whitelist/,
  );
  assert.match(
    nniJoinErrorMessage("nni_public_key_whitelist_empty", null, "fallback", "zh"),
    /白名单/,
  );
  assert.equal(nniJoinErrorMessage(undefined, null, "fallback", "en"), "fallback");
});

test("parses remote NNI node urls", () => {
  assert.deepEqual(parseNniRemoteNodeUrls("https://a\n https://b,https://c "), ["https://a", "https://b", "https://c"]);
});

test("formats NNI action labels", () => {
  assert.equal(nniActionLabel("pubkey", "en"), "Read Slot 0 public key");
  assert.equal(nniActionLabel("sign_timestamp", "zh"), "生成时间戳签名");
  assert.equal(nniActionLabel("custom_action", "en"), "custom_action");
});

test("builds deterministic runtime tiles", () => {
  assert.equal(NNI_RUNTIME_TILES.length, 32);
  assert.ok(NNI_RUNTIME_TILES.every((tile) => tile.duration >= 1.1 && tile.duration <= 3.0));
});
