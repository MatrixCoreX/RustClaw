import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider } from "../components/UiDialogProvider";
import { generateNniOwnerKeyPair } from "../lib/nni-owner-public-key";
import { useNniRuntime } from "./useNniRuntime";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const owner = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";
const nodeA = "https://node-a.example.test";
const nodeB = "https://node-b.example.test";
const config = { remote_nodes: [nodeA, nodeB], selected_node_url: nodeA,
  asset_service_node_url: nodeA, bancor_service_node_url: nodeA, joined: false };
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
function json(data: unknown) { return new Response(JSON.stringify({ ok: true, data })); }
const challenge = { challenge: "challenge", node_url: nodeA, task_id: "binding-1", owner_signature_required: true };
async function mount(apiFetch: (path: string, init?: RequestInit) => Promise<Response>) {
  let runtime!: ReturnType<typeof useNniRuntime>;
  function Probe({ scope = "local" }: { scope?: string }) {
    runtime = useNniRuntime({ apiFetch, t: (zh) => zh, lang: "zh", requestScope: scope });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(UiDialogProvider, null, React.createElement(Probe))); });
  return {
    get: () => runtime,
    scope: async (scope: string) => {
      await act(async () => renderer.update(React.createElement(UiDialogProvider, null, React.createElement(Probe, { scope }))));
    },
    close: async () => { await act(async () => renderer.unmount()); },
  };
}

test("cancelled binding cannot sign a late challenge or restore its dialog", async () => {
  const pending = deferred<Response>();
  let signs = 0;
  const mounted = await mount(async (path) => {
    if (path === "/v1/nni/config") return json(config);
    if (path.includes("/device/status")) return json({ signature_chip_present: true });
    if (path === "/v1/nni/join/request") return pending.promise;
    signs++;
    return json({ payload: { signature: "ab".repeat(64) } });
  });
  await act(async () => { await mounted.get().fetchNniConfig(); await mounted.get().fetchNniDeviceStatus(); });
  let binding!: Promise<unknown>;
  await act(async () => { binding = mounted.get().startNniCustomOwnerAuthorization(owner); });
  await act(async () => { mounted.get().cancelNniOwnerAuthorization(); });
  await act(async () => { pending.resolve(json(challenge)); assert.equal(await binding, null); });
  assert.equal(signs, 0);
  assert.equal(mounted.get().nniOwnerAuthorizationChallenge, null);
  assert.equal(mounted.get().nniOwnerActionLoading, null);
  assert.equal(mounted.get().nniActionMessage, null);
  await mounted.close();
});

test("late verification after cancellation does not dispatch follow-up config writes", async () => {
  const pending = deferred<Response>();
  let configWrites = 0;
  let verifications = 0;
  const mounted = await mount(async (path, init) => {
    if (path === "/v1/nni/config") {
      if (init?.method === "POST") configWrites++;
      return json(config);
    }
    if (path.includes("/device/status")) return json({ signature_chip_present: true });
    if (path === "/v1/nni/join/request") return json(challenge);
    if (path === "/v1/nni/join/verify") { verifications++; return pending.promise; }
    return json({ payload: { signature: "ab".repeat(64) } });
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  await act(async () => { assert.ok(await mounted.get().startNniCustomOwnerAuthorization(owner)); });
  let verification!: Promise<unknown>;
  await act(async () => { verification = mounted.get().completeNniOwnerAuthorization("ab".repeat(64)); });
  assert.equal(verifications, 1);
  await act(async () => { mounted.get().cancelNniOwnerAuthorization(); });
  await act(async () => { pending.resolve(json({ joined: true, compliant: true, asset_owner_pubkey: owner })); assert.equal(await verification, null); });
  assert.equal(configWrites, 0);
  assert.equal(mounted.get().nniOwnerAuthorizationChallenge, null);
  assert.equal(mounted.get().nniActionMessage, null);
  await mounted.close();
});

test("cancellation during device signing suppresses its late result and verification", async () => {
  const pending = deferred<Response>();
  const requests: string[] = [];
  const mounted = await mount(async (path) => {
    requests.push(path);
    if (path === "/v1/nni/config") return json(config);
    if (path.includes("/device/status")) return json({ signature_chip_present: true });
    if (path === "/v1/nni/join/request") return json(challenge);
    if (path === "/v1/nni/device/action") return pending.promise;
    throw new Error(`unexpected request: ${path}`);
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  let binding!: Promise<unknown>;
  await act(async () => { binding = mounted.get().startNniCustomOwnerAuthorization(owner); });
  assert.ok(requests.includes("/v1/nni/device/action"));
  await act(async () => { mounted.get().cancelNniOwnerAuthorization(); });
  await act(async () => { pending.resolve(json({ payload: { signature: "ab".repeat(64) } })); assert.equal(await binding, null); });
  assert.equal(mounted.get().nniActionResult, null);
  assert.equal(mounted.get().nniActionMessage, null);
  assert.equal(mounted.get().nniActionLoading, null);
  assert.equal(mounted.get().nniOwnerActionLoading, null);
  assert.equal(mounted.get().nniOwnerAuthorizationChallenge, null);
  assert.equal(requests.includes("/v1/nni/join/verify"), false);
  await mounted.close();
});

test("replaced authorization keeps its own loading state when the older request finishes", async () => {
  const older = deferred<Response>();
  const newer = deferred<Response>();
  let joins = 0;
  const mounted = await mount(async (path) => {
    if (path === "/v1/nni/config") return json(config);
    if (path.includes("/device/status")) return json({ signature_chip_present: true });
    if (path === "/v1/nni/join/request") return ++joins === 1 ? older.promise : newer.promise;
    return json({ payload: { signature: "ab".repeat(64) } });
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = mounted.get().startNniCustomOwnerAuthorization(owner); });
  await act(async () => { second = mounted.get().startNniCustomOwnerAuthorization(owner); });
  assert.equal(joins, 2);
  await act(async () => { older.resolve(json(challenge)); assert.equal(await first, null); });
  assert.equal(mounted.get().nniOwnerActionLoading, "custom");
  await act(async () => { newer.resolve(json({ ...challenge, task_id: "newer" })); assert.ok(await second); });
  assert.equal(mounted.get().nniOwnerAuthorizationChallenge?.taskId, "newer");
  await mounted.close();
});

test("repeated confirmation dispatches only one verification for the active challenge", async () => {
  const pending = deferred<Response>();
  let verifications = 0;
  let savedJoined: unknown;
  const mounted = await mount(async (path, init) => {
    if (path === "/v1/nni/config") {
      if (init?.method === "POST") savedJoined = JSON.parse(String(init.body)).joined;
      return json({ ...config, asset_owner_pubkey: savedJoined === false ? owner : null });
    }
    if (path.includes("/device/status")) return json({ signature_chip_present: true });
    if (path === "/v1/nni/join/request") return json(challenge);
    if (path === "/v1/nni/join/verify") { verifications++; return pending.promise; }
    return json({ payload: { signature: "ab".repeat(64) } });
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  await act(async () => { assert.ok(await mounted.get().startNniCustomOwnerAuthorization(owner)); });
  let first!: Promise<unknown>;
  await act(async () => {
    first = mounted.get().completeNniOwnerAuthorization("ab".repeat(64));
    assert.equal(await mounted.get().completeNniOwnerAuthorization("ab".repeat(64)), null);
  });
  assert.equal(verifications, 1);
  await act(async () => { pending.resolve(json({ joined: true, compliant: true, asset_owner_pubkey: owner })); assert.ok(await first); });
  assert.equal(savedJoined, false);
  assert.equal(mounted.get().nniJoined, false);
  assert.equal(mounted.get().nniAssetOwnerPubkey, owner);
  assert.equal(mounted.get().nniOwnerAuthorizationChallenge, null);
  await mounted.close();
});

test("config writes serialize through JSON parsing before applying another acknowledgement", async () => {
  const parsed = deferred<unknown>();
  let writes = 0;
  const mounted = await mount(async (_path, init) => {
    if (init?.method !== "POST") return json(config);
    writes++;
    if (writes === 1) return { ok: true, json: () => parsed.promise } as Response;
    return json({ ...config, asset_service_node_url: nodeB, bancor_service_node_url: nodeB });
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => {
    first = mounted.get().updateNniAssetServiceNodeUrl(nodeB);
    second = mounted.get().updateNniBancorServiceNodeUrl(nodeB);
  });
  assert.equal(writes, 1);
  await act(async () => {
    parsed.resolve({ ok: true, data: { ...config, asset_service_node_url: nodeB } });
    await first; await second;
  });
  assert.equal(mounted.get().nniAssetServiceNodeUrl, nodeB);
  assert.equal(mounted.get().nniBancorServiceNodeUrl, nodeB);
  await mounted.close();
});

test("a failed config write does not poison the next queued write", async () => {
  const failed = deferred<Response>();
  let writes = 0;
  const mounted = await mount(async (_path, init) => {
    if (init?.method !== "POST") return json(config);
    return ++writes === 1 ? failed.promise : json({ ...config, bancor_service_node_url: nodeB });
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = mounted.get().updateNniAssetServiceNodeUrl(nodeB); second = mounted.get().updateNniBancorServiceNodeUrl(nodeB); });
  await act(async () => { failed.resolve(new Response(JSON.stringify({ ok: false, error: "fixture_rejected" }), { status: 400 })); assert.equal(await first, false); assert.equal(await second, true); });
  assert.equal(writes, 2);
  assert.equal(mounted.get().nniBancorServiceNodeUrl, nodeB);
  await mounted.close();
});

test("adding custom nodes concurrently preserves both nodes and independent service choices", async () => {
  const pending = deferred<Response>();
  const nodeC = "https://node-c.example.test";
  const nodeD = "https://node-d.example.test";
  let current = { ...config };
  let writes = 0;
  const mounted = await mount(async (_path, init) => {
    if (init?.method !== "POST") return json(current);
    current = { ...current, ...JSON.parse(String(init.body)) };
    return ++writes === 1 ? pending.promise : json(current);
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => {
    first = mounted.get().addNniAssetServiceNodeUrl(nodeC);
    second = mounted.get().addNniBancorServiceNodeUrl(nodeD);
  });
  await act(async () => { pending.resolve(json(current)); assert.equal(await first, true); assert.equal(await second, true); });
  assert.deepEqual(mounted.get().nniRemoteNodeUrls, [nodeA, nodeB, nodeC, nodeD]);
  assert.equal(mounted.get().nniAssetServiceNodeUrl, nodeC);
  assert.equal(mounted.get().nniBancorServiceNodeUrl, nodeD);
  await mounted.close();
});

test("owner changes prevent queued configuration dispatch and late acknowledgements", async () => {
  const pending = deferred<Response>();
  let writes = 0;
  const mounted = await mount(async (_path, init) => {
    if (init?.method !== "POST") return json(config);
    writes++;
    return pending.promise;
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = mounted.get().updateNniAssetServiceNodeUrl(nodeB); second = mounted.get().updateNniBancorServiceNodeUrl(nodeB); });
  assert.equal(writes, 1);
  await mounted.scope("other-session");
  await act(async () => { pending.resolve(json({ ...config, asset_service_node_url: nodeB })); assert.equal(await first, false); assert.equal(await second, false); });
  assert.equal(writes, 1);
  assert.equal(mounted.get().nniRemoteNodeUrls.length, 0);
  assert.equal(mounted.get().nniAssetServiceNodeUrl, "");
  await mounted.close();
});

for (const service of ["assets", "bancor"] as const) {
  test(`${service} selection back to the original node waits for both saves`, async () => {
    const firstResponse = deferred<Response>();
    const secondResponse = deferred<Response>();
    const field = service === "assets" ? "asset_service_node_url" : "bancor_service_node_url";
    let writes = 0;
    const mounted = await mount(async (_path, init) => {
      if (init?.method !== "POST") return json(config);
      return ++writes === 1 ? firstResponse.promise : secondResponse.promise;
    });
    await act(async () => { await mounted.get().fetchNniConfig(); });
    const update = () => service === "assets"
      ? mounted.get().updateNniAssetServiceNodeUrl : mounted.get().updateNniBancorServiceNodeUrl;
    const saving = () => service === "assets"
      ? mounted.get().nniAssetServiceNodeSaving : mounted.get().nniBancorServiceNodeSaving;
    let first!: Promise<unknown>;
    let second!: Promise<unknown>;
    await act(async () => { first = update()(nodeB); second = update()(nodeA); });
    await act(async () => { firstResponse.resolve(json({ ...config, [field]: nodeB })); assert.equal(await first, true); });
    assert.equal(writes, 2);
    assert.equal(saving(), true);
    await act(async () => { secondResponse.resolve(json(config)); assert.equal(await second, true); });
    assert.equal(saving(), false);
    assert.equal(service === "assets" ? mounted.get().nniAssetServiceNodeUrl : mounted.get().nniBancorServiceNodeUrl, nodeA);
    await mounted.close();
  });
}

test("heartbeat configuration does not overwrite a pending service node selection", async () => {
  const pending = deferred<Response>();
  let current = { ...config };
  let writes = 0;
  const mounted = await mount(async (_path, init) => {
    if (init?.method !== "POST") return json(current);
    current = { ...current, ...JSON.parse(String(init.body)) };
    return ++writes === 1 ? pending.promise : json(current);
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = mounted.get().updateNniAssetServiceNodeUrl(nodeB); second = mounted.get().setNniJoinedPersisted(true, { persistRemoteNodes: true }); });
  await act(async () => { pending.resolve(json(current)); assert.equal(await first, true); assert.equal(await second, true); });
  assert.equal(mounted.get().nniAssetServiceNodeUrl, nodeB);
  assert.equal(mounted.get().nniJoined, true);
  await mounted.close();
});

test("invalid configuration JSON releases the queue and reads wait until all writes finish", async () => {
  const parsed = deferred<unknown>();
  let reads = 0;
  let writes = 0;
  const mounted = await mount(async (_path, init) => {
    if (init?.method !== "POST") { reads++; return json(config); }
    if (++writes === 1) return { ok: true, json: () => parsed.promise.then(() => { throw new SyntaxError("fixture_json"); }) } as Response;
    return json({ ...config, bancor_service_node_url: nodeB });
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = mounted.get().updateNniAssetServiceNodeUrl(nodeB); second = mounted.get().updateNniBancorServiceNodeUrl(nodeB); });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  assert.equal(reads, 1);
  assert.equal(writes, 1);
  await act(async () => { parsed.resolve(null); assert.equal(await first, false); assert.equal(await second, true); });
  assert.equal(writes, 2);
  assert.equal(mounted.get().nniBancorServiceNodeUrl, nodeB);
  await act(async () => { await mounted.get().fetchNniConfig(); });
  assert.equal(reads, 2);
  await mounted.close();
});

test("cancelled authorization skips a queued follow-up without rolling back sent verification", async () => {
  const pending = deferred<Response>();
  let writes = 0;
  let verified = false;
  const mounted = await mount(async (path, init) => {
    if (path === "/v1/nni/config") {
      if (init?.method !== "POST") return json(config);
      writes++;
      return pending.promise;
    }
    if (path.includes("/device/status")) return json({ signature_chip_present: true });
    if (path === "/v1/nni/join/request") return json(challenge);
    if (path === "/v1/nni/join/verify") {
      verified = true;
      return json({ joined: true, compliant: true, asset_owner_pubkey: owner });
    }
    return json({ payload: { signature: "ab".repeat(64) } });
  });
  await act(async () => { await mounted.get().fetchNniConfig(); });
  await act(async () => { assert.ok(await mounted.get().startNniCustomOwnerAuthorization(owner)); });
  let saved!: Promise<unknown>;
  let authorization!: Promise<unknown>;
  await act(async () => {
    saved = mounted.get().updateNniAssetServiceNodeUrl(nodeB);
    authorization = mounted.get().completeNniOwnerAuthorization("ab".repeat(64));
  });
  assert.equal(verified, true);
  await act(async () => { mounted.get().cancelNniOwnerAuthorization(); });
  await act(async () => {
    pending.resolve(json({ ...config, asset_service_node_url: nodeB, asset_owner_pubkey: owner }));
    assert.equal(await saved, true);
    assert.equal(await authorization, null);
  });
  assert.equal(writes, 1);
  assert.equal(mounted.get().nniAssetOwnerPubkey, owner);
  assert.equal(mounted.get().nniOwnerAuthorizationChallenge, null);
  assert.equal(mounted.get().nniActionMessage, null);
  await mounted.close();
});

for (const method of ["authorizeNniOwnerWithPrivateKey", "recoverNniOwner"] as const) {
  test(`${method} cannot restore a stale node snapshot after a slow challenge`, async () => {
    const pair = generateNniOwnerKeyPair();
    const pending = deferred<Response>();
    let current = { ...config, asset_owner_pubkey: null as string | null };
    let recoverRequests = 0;
    const mounted = await mount(async (path, init) => {
      if (path === "/v1/nni/config") {
        if (init?.method === "POST") current = { ...current, ...JSON.parse(String(init.body)) };
        return json(current);
      }
      if (path.includes("/device/status")) return json({ signature_chip_present: true });
      if (path === "/v1/nni/join/request") return pending.promise;
      if (path === "/v1/nni/owner/recover" && ++recoverRequests === 1) return pending.promise;
      if (path === "/v1/nni/join/verify" || path === "/v1/nni/owner/recover") {
        current = { ...current, asset_owner_pubkey: pair.public_key };
        return json({ joined: true, compliant: true, asset_owner_pubkey: pair.public_key });
      }
      return json({ payload: { signature: "ab".repeat(64) } });
    });
    await act(async () => { await mounted.get().fetchNniConfig(); });
    let authorization!: Promise<unknown>;
    await act(async () => { authorization = mounted.get()[method](pair.private_key); });
    await act(async () => { assert.equal(await mounted.get().updateNniAssetServiceNodeUrl(nodeB), true); });
    await act(async () => {
      pending.resolve(json(method === "recoverNniOwner"
        ? { task_id: "recovery", signing_payload: "fixture", device_signature: "ab".repeat(64), asset_owner_pubkey: pair.public_key }
        : challenge));
      assert.ok(await authorization);
    });
    assert.equal(mounted.get().nniAssetServiceNodeUrl, nodeB);
    assert.equal(mounted.get().nniAssetOwnerPubkey, pair.public_key);
    assert.equal(mounted.get().nniJoined, false);
    await mounted.close();
  });
}
