import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ripemd160 } from "@noble/hashes/legacy.js";
import { base58 } from "@scure/base";

import { validateNniOwnerPrivateKey } from "../lib/nni-owner-public-key";
import { useAssetTransferRuntime } from "./useAssetTransferRuntime";

function encodeTestOwnerPrivateKey(secretKey: Uint8Array): string {
  const suffix = new TextEncoder().encode("K1");
  const payload = new Uint8Array(secretKey.length + suffix.length);
  payload.set(secretKey);
  payload.set(suffix, secretKey.length);
  const checksum = ripemd160(payload).slice(0, 4);
  const encoded = new Uint8Array(secretKey.length + checksum.length);
  encoded.set(secretKey);
  encoded.set(checksum, secretKey.length);
  return base58.encode(encoded);
}

test("asset transfer runtime sends one-time authorization and refreshes after success", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requests: Array<{ path: string; body: Record<string, unknown> }> = [];
  let refreshCount = 0;
  const ownerPrivateKey = encodeTestOwnerPrivateKey(
    Uint8Array.from({ length: 32 }, (_, index) => index + 1),
  );
  const ownerValidation = validateNniOwnerPrivateKey(ownerPrivateKey);
  assert.equal(ownerValidation.ok, true);
  if (!ownerValidation.ok) return;
  let transferRequestCount = 0;
  const apiFetch = async (path: string, init?: RequestInit) => {
    const requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
    requests.push({ path, body: requestBody });
    transferRequestCount += 1;
    if (transferRequestCount === 1) {
      return new Response(JSON.stringify({
        ok: true,
        data: {
          status: "asset_transfer_challenge_created",
          request_id: "0c42e3f7-f5f0-43ff-bc55-ab032daf7eaf",
          task_id: "asset-task-test",
          transfer_id: "asset-transfer-test",
          signing_payload: "asset-transfer-signing-payload",
          from_asset_owner_pubkey: ownerValidation.publicKey,
          node_url: "https://nni.example.test",
        },
      }), { status: 200, headers: { "content-type": "application/json" } });
    }
    return new Response(JSON.stringify({
      ok: true,
      data: {
        schema_version: 1,
        status: "asset_transfer_completed",
        request_id: "0c42e3f7-f5f0-43ff-bc55-ab032daf7eaf",
        idempotent_replay: false,
        transfer: {
          transfer_id: "asset-transfer-test",
          from_asset_owner_pubkey: "sender",
          to_asset_owner_pubkey: "recipient",
          asset: "USD",
          amount_units: "125000000",
          amount: "1.25000000",
          memo: "invoice-7",
          from_balance_after_units: "175000000",
          from_balance_after: "1.75000000",
          to_balance_after_units: "125000000",
          to_balance_after: "1.25000000",
          authorization_mode: "asset_owner",
          created_at_unix: 1_800_000_000,
        },
      },
    }), { status: 200, headers: { "content-type": "application/json" } });
  };
  let runtime: ReturnType<typeof useAssetTransferRuntime> | null = null;
  function Probe() {
    runtime = useAssetTransferRuntime({
      apiFetch,
      t: (zh) => zh,
      onCompleted: () => {
        refreshCount += 1;
      },
    });
    return null;
  }
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.transfer({
      asset: "USD",
      amount: "1.25000000",
      recipientPublicKey: "recipient",
      memo: "invoice-7",
      authorizationMode: "asset_owner",
      ownerPrivateKey,
    });
  });
  assert.equal(requests.length, 2);
  assert.equal(requests[0].body.asset_owner_pubkey, ownerValidation.publicKey);
  assert.equal(requests[1].body.task_id, "asset-task-test");
  assert.equal(typeof requests[1].body.owner_signature, "string");
  for (const request of requests) {
    assert.equal(Object.hasOwn(request.body, "owner_private_key"), false);
  }
  assert.equal(refreshCount, 1);
  assert.equal(runtime!.message, "转账已完成，余额和资产浏览器记录已经更新。");
  assert.equal(runtime!.error, null);
  await act(async () => renderer!.unmount());
});

test("asset transfer runtime renders structured nested errors without exposing tokens", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const apiFetch = async () => new Response(JSON.stringify({
    ok: false,
    error: "nni_asset_transfer_nodes_unavailable",
    data: { attempts: [{ error_code: "nni_asset_transfer_insufficient_aic_balance" }] },
  }), { status: 502, headers: { "content-type": "application/json" } });
  let runtime: ReturnType<typeof useAssetTransferRuntime> | null = null;
  function Probe() {
    runtime = useAssetTransferRuntime({ apiFetch, t: (zh) => zh });
    return null;
  }
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.transfer({
      asset: "AIC",
      amount: "1.00000000",
      recipientPublicKey: "recipient",
      memo: "",
      authorizationMode: "delegated_hardware",
    });
  });
  assert.equal(runtime!.error, "AIC 余额不足。");
  assert.doesNotMatch(runtime!.error ?? "", /nni_asset_transfer/);
  await act(async () => renderer!.unmount());
});

test("asset transfer runtime suppresses concurrent duplicate submissions", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let requestCount = 0;
  let releaseRequest: (() => void) | null = null;
  const blocked = new Promise<void>((resolve) => {
    releaseRequest = resolve;
  });
  const apiFetch = async () => {
    requestCount += 1;
    await blocked;
    return new Response(JSON.stringify({ ok: false, error: "nni_asset_transfer_rate_limited" }), {
      status: 429,
      headers: { "content-type": "application/json" },
    });
  };
  let runtime: ReturnType<typeof useAssetTransferRuntime> | null = null;
  function Probe() {
    runtime = useAssetTransferRuntime({ apiFetch, t: (zh) => zh });
    return null;
  }
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  const input = {
    asset: "AIC" as const,
    amount: "1.00000000",
    recipientPublicKey: "recipient",
    memo: "",
    authorizationMode: "delegated_hardware" as const,
  };
  await act(async () => {
    const first = runtime!.transfer(input);
    const duplicate = runtime!.transfer(input);
    assert.equal(await duplicate, null);
    releaseRequest?.();
    await first;
  });
  assert.equal(requestCount, 1);
  await act(async () => renderer!.unmount());
});
