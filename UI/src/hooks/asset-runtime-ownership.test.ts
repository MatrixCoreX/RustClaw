import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ripemd160 } from "@noble/hashes/legacy.js";
import { base58 } from "@scure/base";
import { useAssetTransferHistoryRuntime } from "./useAssetTransferHistoryRuntime";
import { useAssetTransferRuntime, type AssetTransferInput } from "./useAssetTransferRuntime";
import { validateNniOwnerPrivateKey } from "../lib/nni-owner-public-key";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const t = (zh: string) => zh;
const input: AssetTransferInput = {
  asset: "AIC", amount: "1", recipientPublicKey: "recipient", memo: "",
  authorizationMode: "delegated_hardware",
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
function json(data: unknown) {
  return new Response(JSON.stringify({ ok: true, data }), { status: 200 });
}
function history(label: string) {
  return json({
    schema_version: 1, owner_pubkey: "owner", page: 1, per_page: 100,
    source_filter: "all", direction_filter: "all", transactions: [], marker: label,
  });
}
function completed() {
  return json({ status: "asset_transfer_completed", transfer: { transfer_id: "transfer-1" } });
}

test("asset history drops old cache and rejects A -> B -> A late responses", async () => {
  const pending = deferred<Response>();
  let calls = 0;
  const apiFetch = async () => ++calls === 1 ? pending.promise : history(`call-${calls}`);
  let runtime!: ReturnType<typeof useAssetTransferHistoryRuntime>;
  function Probe({ scope }: { scope: string }) {
    runtime = useAssetTransferHistoryRuntime({ apiFetch, t, requestScope: scope });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe, { scope: "A" })); });
  let first!: Promise<unknown>;
  await act(async () => { first = runtime.load("owner"); });
  await act(async () => { renderer.update(React.createElement(Probe, { scope: "B" })); });
  await act(async () => { await runtime.load("owner"); });
  await act(async () => { renderer.update(React.createElement(Probe, { scope: "A" })); });
  assert.equal(runtime.history, null);
  await act(async () => { pending.resolve(history("old-A")); });
  assert.equal(await first, null);
  await act(async () => { await runtime.load("owner"); });
  assert.equal(calls, 3);
  assert.equal((runtime.history as unknown as { marker: string }).marker, "call-3");
  await act(async () => { renderer.unmount(); });
});

test("asset history invalidates already cached records when endpoint or session changes", async () => {
  let calls = 0;
  const apiFetch = async () => history(`call-${++calls}`);
  let runtime!: ReturnType<typeof useAssetTransferHistoryRuntime>;
  function Probe({ scope }: { scope: string }) {
    runtime = useAssetTransferHistoryRuntime({ apiFetch, t, requestScope: scope });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe, { scope: "session-A-node-1" })); });
  await act(async () => { await runtime.load("owner"); });
  await act(async () => { renderer.update(React.createElement(Probe, { scope: "session-B-node-2" })); });
  assert.equal(runtime.history, null);
  await act(async () => { await runtime.load("owner"); });
  assert.equal(calls, 2);
  await act(async () => { renderer.update(React.createElement(Probe, { scope: "" })); });
  await act(async () => { assert.equal(await runtime.load("owner"), null); });
  assert.equal(calls, 2);
  assert.equal(runtime.history, null);
  await act(async () => { renderer.unmount(); });
});

test("asset history rejects stale JSON parsing completion and unmounted callbacks", async () => {
  const body = deferred<unknown>();
  const apiFetch = async () => ({ ok: true, json: () => body.promise }) as Response;
  let runtime!: ReturnType<typeof useAssetTransferHistoryRuntime>;
  function Probe() {
    runtime = useAssetTransferHistoryRuntime({ apiFetch, t, requestScope: "A" });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe)); });
  let pending!: Promise<unknown>;
  await act(async () => { pending = runtime.load("owner"); });
  await act(async () => { renderer.unmount(); });
  body.resolve(await history("old").json());
  assert.equal(await pending, null);
});

test("asset transfer does not sign an old challenge after the session changes", async () => {
  const secret = Uint8Array.from({ length: 32 }, (_, index) => index + 1);
  const suffix = new TextEncoder().encode("K1");
  const payload = new Uint8Array(secret.length + suffix.length);
  payload.set(secret); payload.set(suffix, secret.length);
  const encoded = new Uint8Array(secret.length + 4);
  encoded.set(secret); encoded.set(ripemd160(payload).slice(0, 4), secret.length);
  const key = base58.encode(encoded);
  const owner = validateNniOwnerPrivateKey(key);
  assert.equal(owner.ok, true);
  if (!owner.ok) return;
  const pending = deferred<Response>();
  let calls = 0;
  let refreshes = 0;
  const apiFetch = async () => { calls++; return pending.promise; };
  let runtime!: ReturnType<typeof useAssetTransferRuntime>;
  function Probe({ scope }: { scope: string }) {
    runtime = useAssetTransferRuntime({ apiFetch, t, requestScope: scope, onCompleted: () => { refreshes++; } });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe, { scope: "A" })); });
  let transfer!: Promise<unknown>;
  await act(async () => { transfer = runtime.transfer({ ...input, authorizationMode: "asset_owner", ownerPrivateKey: key }); });
  await act(async () => { renderer.update(React.createElement(Probe, { scope: "B" })); });
  await act(async () => {
    pending.resolve(json({ signing_payload: "challenge", from_asset_owner_pubkey: owner.publicKey }));
    assert.equal(await transfer, null);
  });
  assert.equal(calls, 1);
  assert.equal(refreshes, 0);
  assert.equal(runtime.error, null);
  assert.equal(runtime.lastTransfer, null);
  await act(async () => { renderer.unmount(); });
});

test("old transfer finalization cannot unlock a new session's pending transfer", async () => {
  const first = deferred<Response>();
  const second = deferred<Response>();
  let calls = 0;
  let refreshes = 0;
  const apiFetch = async () => ++calls === 1 ? first.promise : second.promise;
  let runtime!: ReturnType<typeof useAssetTransferRuntime>;
  function Probe({ scope }: { scope: string }) {
    runtime = useAssetTransferRuntime({ apiFetch, t, requestScope: scope, onCompleted: () => { refreshes++; } });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe, { scope: "A" })); });
  let a!: Promise<unknown>;
  let b!: Promise<unknown>;
  await act(async () => { a = runtime.transfer(input); });
  await act(async () => { renderer.update(React.createElement(Probe, { scope: "B" })); });
  await act(async () => { b = runtime.transfer(input); });
  assert.equal(calls, 2);
  await act(async () => { first.resolve(completed()); assert.equal(await a, null); });
  assert.equal(runtime.loading, true);
  assert.equal(runtime.lastTransfer, null);
  await act(async () => { assert.equal(await runtime.transfer(input), null); });
  assert.equal(calls, 2);
  await act(async () => { second.resolve(completed()); await b; });
  assert.equal(runtime.loading, false);
  assert.equal(refreshes, 1);
  await act(async () => { renderer.unmount(); });
});
