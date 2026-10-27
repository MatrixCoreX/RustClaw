import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useAssetTransferRuntime, type AssetTransferInput } from "./useAssetTransferRuntime";
import { useBancorRuntime } from "./useBancorRuntime";

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
function json(data: unknown) { return Response.json({ ok: true, data }); }
function completed() { return json({ status: "asset_transfer_completed", transfer: { transfer_id: "committed" } }); }

for (const failure of ["rejected", "null", "false"] as const) {
  test(`confirmed transfer stays successful when its refresh is ${failure}`, async () => {
    let transfers = 0;
    let runtime!: ReturnType<typeof useAssetTransferRuntime>;
    function Probe() {
      runtime = useAssetTransferRuntime({
        apiFetch: async () => { transfers++; return completed(); }, t,
        onCompleted: async () => {
          if (failure === "rejected") throw new Error("fixture_read_failed");
          return failure === "null" ? null : false;
        },
      });
      return null;
    }
    let renderer!: ReactTestRenderer;
    await act(async () => { renderer = create(React.createElement(Probe)); });
    await act(async () => { assert.ok(await runtime.transfer(input)); });
    assert.equal(transfers, 1);
    assert.equal(runtime.lastTransfer?.transfer.transfer_id, "committed");
    assert.equal(runtime.error, null);
    assert.equal(runtime.message, "转账已完成，余额暂未刷新，请稍后刷新资产。");
    assert.equal(runtime.loading, false);
    await act(async () => renderer.unmount());
  });
}

test("transfer follow-up account refresh does not reuse a pre-transfer balance request", async () => {
  const old = deferred<Response>();
  let reads = 0;
  let transfers = 0;
  const apiFetch = async (path: string) => {
    if (path.endsWith("/transfer")) { transfers++; return completed(); }
    return ++reads === 1 ? old.promise : json({ page: 1, marker: "new-balance" });
  };
  let account!: ReturnType<typeof useBancorRuntime>;
  let transfer!: ReturnType<typeof useAssetTransferRuntime>;
  function Probe() {
    account = useBancorRuntime({ apiFetch, t, cacheScope: "asset-mutation", overviewRoutePrefix: "/v1/nni/assets" });
    transfer = useAssetTransferRuntime({ apiFetch, t, onCompleted: () => account.refreshAccountAfterMutation() });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe)); });
  let pending!: Promise<unknown>;
  let submitting!: Promise<unknown>;
  await act(async () => { pending = account.fetchAccount(); });
  await act(async () => { submitting = transfer.transfer(input); });
  assert.equal(reads, 2);
  await act(async () => {
    old.resolve(json({ page: 1, marker: "old-balance" }));
    assert.equal(await pending, null);
    assert.ok(await submitting);
  });
  assert.equal((account.account as unknown as { marker: string }).marker, "new-balance");
  assert.equal(transfers, 1);
  await act(async () => renderer.unmount());
});
