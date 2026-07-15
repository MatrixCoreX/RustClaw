import test from "node:test";
import assert from "node:assert/strict";

import {
  NNI_RUNTIME_TILES,
  findNniJoinErrorCode,
  nniActionLabel,
  nniDeviceMessage,
  nniDeviceNextStep,
  nniJoinErrorMessage,
  nniPayloadHexField,
  nniTimestampSignatureReady,
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

test("recognizes completed timestamp signatures for NNI test join", () => {
  assert.equal(
    nniTimestampSignatureReady({
      action: "sign_timestamp",
      signature_chip_present: true,
      message: "ok",
      payload: { timestamp: 1_800_000_000, signature: "ab".repeat(64) },
    }),
    true,
  );
  assert.equal(
    nniTimestampSignatureReady({
      action: "sign_timestamp",
      signature_chip_present: true,
      message: "missing timestamp",
      payload: { signature: "ab".repeat(64) },
    }),
    false,
  );
  assert.equal(
    nniTimestampSignatureReady({
      action: "sign_challenge",
      signature_chip_present: true,
      message: "different action",
      payload: { timestamp: 1_800_000_000, signature: "ab".repeat(64) },
    }),
    false,
  );
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

test("renders NNI device messages from machine keys", () => {
  assert.equal(
    nniDeviceMessage(
      {
        nni_available: true,
        helper_available: true,
        signature_chip_present: true,
        status: "ready",
        message_key: "nni.device_status.ready",
      },
      "en",
    ),
    "A device signature chip was detected, and NNI device signing is available.",
  );
  assert.equal(
    nniDeviceNextStep(
      {
        nni_available: true,
        helper_available: true,
        signature_chip_present: false,
        status: "signature_chip_missing",
        next_step_key: "nni.device_status.signature_chip_missing.next_step",
      },
      "zh",
    ),
    "如果这是无签名芯片设备，可以忽略本页签名操作；如果应当有芯片，请检查 I2C 接线、地址和 cryptoauthlib 环境。",
  );
  assert.equal(
    nniDeviceMessage(
      {
        action: "sign_timestamp",
        signature_chip_present: true,
        message_key: "nni.device_action.completed",
      },
      "zh",
    ),
    "NNI 设备签名操作完成。",
  );
});

test("keeps legacy NNI message fields as display fallback", () => {
  assert.equal(
    nniDeviceMessage(
      {
        nni_available: true,
        helper_available: false,
        signature_chip_present: false,
        status: "helper_missing",
        message: "legacy status message",
      },
      "en",
      "fallback status",
    ),
    "legacy status message",
  );
  assert.equal(
    nniDeviceNextStep(
      {
        nni_available: true,
        helper_available: false,
        signature_chip_present: false,
        status: "helper_missing",
        next_step: "legacy next step",
      },
      "en",
    ),
    "legacy next step",
  );
  assert.equal(nniDeviceMessage(null, "en", "fallback message"), "fallback message");
});

test("builds deterministic runtime tiles", () => {
  assert.equal(NNI_RUNTIME_TILES.length, 32);
  assert.ok(NNI_RUNTIME_TILES.every((tile) => tile.duration >= 1.1 && tile.duration <= 3.0));
});
