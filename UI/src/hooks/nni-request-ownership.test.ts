import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider } from "../components/UiDialogProvider";
import { useNniRuntime } from "./useNniRuntime";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const t = (zh: string) => zh;
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
function json(data: unknown) {
  return new Response(JSON.stringify({ ok: true, data }), { status: 200 });
}
const data = {
  joined: true, remote_nodes: ["https://node.example.test"],
  signature_chip_present: true, records: [{ marker: "record" }], page: 1,
};

for (const [method, field, loading, empty] of [
  ["fetchNniConfig", "nniRemoteNodes", "nniConfigLoading", ""],
  ["fetchNniDeviceStatus", "nniStatus", "nniStatusLoading", null],
  ["fetchNniNetworkStats", "nniNetworkStats", "nniNetworkStatsLoading", null],
  ["fetchNniRewards", "nniRewards", "nniRewardsLoading", null],
  ["fetchNniHeartbeatRecords", "nniHeartbeatRecords", "nniHeartbeatRecordsLoading", []],
  ["fetchNniHeartbeatErrors", "nniHeartbeatErrors", "nniHeartbeatErrorsLoading", []],
] as const) {
  test(`NNI ${method} invalidates coalesced reads across A -> B -> A`, async () => {
    const old = deferred<Response>();
    const fresh = deferred<Response>();
    let calls = 0;
    const apiFetch = async () => ++calls === 1 ? old.promise : fresh.promise;
    let runtime!: ReturnType<typeof useNniRuntime>;
    function Probe({ scope }: { scope: string }) {
      runtime = useNniRuntime({ apiFetch, t, lang: "zh", requestScope: scope });
      return null;
    }
    const view = (scope: string) => React.createElement(UiDialogProvider, null, React.createElement(Probe, { scope }));
    let renderer!: ReactTestRenderer;
    await act(async () => { renderer = create(view("A")); });
    let a!: Promise<unknown>;
    let b!: Promise<unknown>;
    await act(async () => { a = runtime[method](); });
    await act(async () => { renderer.update(view("B")); });
    await act(async () => { renderer.update(view("A")); });
    await act(async () => { b = runtime[method](); });
    assert.equal(calls, 2);
    await act(async () => { old.resolve(json(data)); await a; });
    assert.deepEqual(runtime[field], empty);
    assert.equal(runtime[loading], true);
    await act(async () => { fresh.resolve(json(data)); await b; });
    assert.notDeepEqual(runtime[field], empty);
    await act(async () => { renderer.update(view("")); });
    assert.deepEqual(runtime[field], empty);
    await act(async () => { await runtime[method](); });
    assert.equal(calls, 2);
    await act(async () => { renderer.unmount(); });
  });
}

test("NNI late join cannot enable heartbeat in a different session", async () => {
  const pending = deferred<Response>();
  const requests: string[] = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    requests.push(path);
    if (init?.method === "POST") return pending.promise;
    if (path === "/v1/nni/config") return json({ ...data, asset_owner_pubkey: "owner" });
    return json(data);
  };
  let runtime!: ReturnType<typeof useNniRuntime>;
  function Probe({ scope }: { scope: string }) {
    runtime = useNniRuntime({ apiFetch, t, lang: "zh", requestScope: scope });
    return null;
  }
  const view = (scope: string) => React.createElement(UiDialogProvider, null, React.createElement(Probe, { scope }));
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(view("A")); });
  await act(async () => { await runtime.fetchNniConfig(); await runtime.fetchNniDeviceStatus(); });
  let join!: Promise<unknown>;
  await act(async () => { join = runtime.joinNni(); });
  await act(async () => { renderer.update(view("B")); });
  await act(async () => { pending.resolve(json({ ...data, asset_owner_pubkey: "old-owner" })); await join; });
  assert.equal(runtime.nniJoined, false);
  assert.equal(runtime.nniAssetOwnerPubkey, null);
  assert.equal(runtime.nniActionMessage, null);
  assert.equal(runtime.nniActionError, null);
  assert.equal(requests.some((path) => path.includes("/rewards?") || path.includes("/records?")), false);
  await act(async () => { renderer.unmount(); });
});

test("NNI old config reads cannot overwrite a successful local config write", async () => {
  const old = deferred<Response>();
  const apiFetch = async (_path: string, init?: RequestInit) => init?.method === "POST"
    ? json({ ...data, joined: true }) : old.promise;
  let runtime!: ReturnType<typeof useNniRuntime>;
  function Probe() { runtime = useNniRuntime({ apiFetch, t, lang: "zh" }); return null; }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(UiDialogProvider, null, React.createElement(Probe))); });
  let read!: Promise<unknown>;
  await act(async () => { read = runtime.fetchNniConfig(); });
  await act(async () => { await runtime.setNniJoinedPersisted(true); });
  await act(async () => { old.resolve(json({ ...data, joined: false })); await read; });
  assert.equal(runtime.nniJoined, true);
  await act(async () => { renderer.unmount(); });
});

test("NNI reward pagination ignores an earlier selected page", async () => {
  const first = deferred<Response>();
  const second = deferred<Response>();
  const apiFetch = async (path: string) => path.includes("page=2") ? second.promise : first.promise;
  let runtime!: ReturnType<typeof useNniRuntime>;
  function Probe() { runtime = useNniRuntime({ apiFetch, t, lang: "zh" }); return null; }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(UiDialogProvider, null, React.createElement(Probe))); });
  let a!: Promise<unknown>;
  let b!: Promise<unknown>;
  await act(async () => { a = runtime.fetchNniRewards(1); b = runtime.fetchNniRewards(2); });
  await act(async () => { second.resolve(json({ page: 2 })); await b; });
  await act(async () => { first.resolve(json({ page: 1 })); await a; });
  assert.equal(runtime.nniRewards?.page, 2);
  await act(async () => { renderer.unmount(); });
});

test("NNI remote reads use confirmed nodes rather than unsaved node drafts", async () => {
  const pending = deferred<Response>();
  const nodeA = "https://node-a.example.test";
  const nodeB = "https://node-b.example.test";
  let selectedNode = nodeA;
  const apiFetch = async (path: string) => path === "/v1/nni/config"
    ? json({ ...data, remote_nodes: [nodeA, nodeB], selected_node_url: selectedNode })
    : pending.promise;
  let runtime!: ReturnType<typeof useNniRuntime>;
  function Probe() { runtime = useNniRuntime({ apiFetch, t, lang: "zh" }); return null; }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(UiDialogProvider, null, React.createElement(Probe))); });
  await act(async () => { await runtime.fetchNniConfig(); });
  let read!: Promise<unknown>;
  await act(async () => { read = runtime.fetchNniNetworkStats(); });
  await act(async () => { runtime.updateNniSelectedNodeUrl(nodeB); });
  assert.equal(runtime.nniSelectedNodeUrl, nodeB);
  assert.equal(runtime.nniActiveNodeUrl, nodeA);
  selectedNode = nodeB;
  await act(async () => { await runtime.fetchNniConfig(); });
  await act(async () => { pending.resolve(json({ active_devices: 7 })); await read; });
  assert.equal(runtime.nniActiveNodeUrl, nodeB);
  assert.equal(runtime.nniNetworkStats, null);
  assert.equal(runtime.nniNetworkStatsLoading, false);
  await act(async () => { renderer.unmount(); });
});
