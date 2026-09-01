import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ripemd160 } from "@noble/hashes/legacy.js";
import { base58 } from "@scure/base";

import { UiDialogProvider } from "../components/UiDialogProvider";
import { validateNniOwnerPrivateKey } from "../lib/nni-owner-public-key";
import type { NniConfigResponse, NniDeviceStatusResponse } from "../types/api";
import { useNniRuntime } from "./useNniRuntime";

function concatenate(left: Uint8Array, right: Uint8Array): Uint8Array {
  const result = new Uint8Array(left.length + right.length);
  result.set(left);
  result.set(right, left.length);
  return result;
}

function encodeTestOwnerPrivateKey(secretKey: Uint8Array): string {
  const checksum = ripemd160(concatenate(secretKey, new TextEncoder().encode("K1"))).slice(0, 4);
  return base58.encode(concatenate(secretKey, checksum));
}

function apiResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify({ ok: status < 400, data, error: status < 400 ? null : "request_failed" }), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function joinedConfig(): NniConfigResponse {
  return {
    remote_nodes: ["https://nni.example.test"],
    joined: true,
    heartbeat_interval_seconds: 600,
    heartbeat_request_count: 4,
    heartbeat_network_retry_limit: 3,
    last_heartbeat_at_ts: 1_800_000_000,
    last_heartbeat_network_failures: 0,
    config_path: "/runtime/data/nni/runtime-config.json",
  };
}

function missingChipStatus(): NniDeviceStatusResponse {
  return {
    nni_available: true,
    status: "signature_chip_missing",
    helper_available: true,
    signature_chip_present: false,
    simulated: false,
    device_kind: "unavailable",
    simulation_available: true,
    message_key: "nni.device_status.signature_chip_missing",
    next_step_key: "nni.device_status.signature_chip_missing.next_step",
    pubkey: null,
    pubkey_preview: null,
    pubkey_fingerprint: null,
    meta: null,
  };
}

function readyChipStatus(): NniDeviceStatusResponse {
  return {
    nni_available: true,
    status: "ready",
    helper_available: true,
    signature_chip_present: true,
    simulated: false,
    device_kind: "hardware",
    simulation_available: false,
    message_key: "nni.device_status.ready",
    pubkey: "ab".repeat(64),
    pubkey_preview: "abab...abab",
    pubkey_fingerprint: "fingerprint",
    meta: null,
  };
}

async function mountRuntime(apiFetch: (path: string, init?: RequestInit) => Promise<Response>) {
  let runtime: ReturnType<typeof useNniRuntime> | null = null;
  function Probe() {
    runtime = useNniRuntime({ apiFetch, t: (zh) => zh, lang: "zh" });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(UiDialogProvider, null, React.createElement(Probe)));
  });
  return {
    runtime: () => runtime!,
    unmount: async () => {
      await act(async () => renderer!.unmount());
    },
  };
}

test("NNI device detection failure does not clear a previously joined runtime", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requests: Array<{ path: string; method: string }> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    requests.push({ path, method: init?.method ?? "GET" });
    if (path === "/v1/nni/config") return apiResponse(joinedConfig());
    if (path === "/v1/nni/device/status?refresh=true") return apiResponse(missingChipStatus());
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => {
    await mounted.runtime().fetchNniConfig(true);
    await mounted.runtime().fetchNniDeviceStatus(true);
  });

  assert.equal(mounted.runtime().nniJoined, true);
  assert.equal(mounted.runtime().nniStatus?.signature_chip_present, false);
  assert.equal(requests.filter((request) => request.method === "POST").length, 0);
  await mounted.unmount();
});

test("BANCOR and asset nodes change independently from the NNI heartbeat node", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let config: NniConfigResponse = {
    ...joinedConfig(),
    remote_nodes: [
      "https://node-a.example.test",
      "https://node-b.example.test",
      "https://node-c.example.test",
    ],
    selected_node_url: "https://node-a.example.test",
    bancor_service_node_url: "https://node-a.example.test",
    asset_service_node_url: "https://node-a.example.test",
  };
  const apiFetch = async (path: string, init?: RequestInit) => {
    assert.equal(path, "/v1/nni/config");
    if (init?.method === "POST") {
      const payload = JSON.parse(String(init.body)) as {
        bancor_service_node_url?: string;
        asset_service_node_url?: string;
      };
      config = { ...config, ...payload };
    }
    return apiResponse(config);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => {
    await mounted.runtime().fetchNniConfig(true);
  });
  await act(async () => {
    assert.equal(
      await mounted.runtime().updateNniAssetServiceNodeUrl("https://node-b.example.test"),
      true,
    );
  });
  await act(async () => {
    assert.equal(
      await mounted.runtime().updateNniBancorServiceNodeUrl("https://node-c.example.test"),
      true,
    );
  });

  assert.equal(mounted.runtime().nniSelectedNodeUrl, "https://node-a.example.test");
  assert.equal(mounted.runtime().nniBancorServiceNodeUrl, "https://node-c.example.test");
  assert.equal(mounted.runtime().nniAssetServiceNodeUrl, "https://node-b.example.test");
  await mounted.unmount();
});

test("custom financial nodes are appended without changing the active heartbeat node", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let config: NniConfigResponse = {
    ...joinedConfig(),
    remote_nodes: ["https://node-a.example.test", "https://node-b.example.test"],
    selected_node_url: "https://node-a.example.test",
    bancor_service_node_url: "https://node-a.example.test",
    asset_service_node_url: "https://node-b.example.test",
  };
  const payloads: Array<Record<string, unknown>> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    assert.equal(path, "/v1/nni/config");
    if (init?.method === "POST") {
      const payload = JSON.parse(String(init.body)) as Record<string, unknown>;
      payloads.push(payload);
      config = { ...config, ...payload } as NniConfigResponse;
    }
    return apiResponse(config);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => mounted.runtime().fetchNniConfig(true));
  await act(async () => {
    assert.equal(
      await mounted.runtime().addNniBancorServiceNodeUrl("https://node-c.example.test"),
      true,
    );
  });
  await act(async () => {
    assert.equal(
      await mounted.runtime().addNniAssetServiceNodeUrl("https://node-d.example.test"),
      true,
    );
  });

  assert.equal(mounted.runtime().nniSelectedNodeUrl, "https://node-a.example.test");
  assert.equal(mounted.runtime().nniBancorServiceNodeUrl, "https://node-c.example.test");
  assert.equal(mounted.runtime().nniAssetServiceNodeUrl, "https://node-d.example.test");
  assert.deepEqual(mounted.runtime().nniRemoteNodeUrls, [
    "https://node-a.example.test",
    "https://node-b.example.test",
    "https://node-c.example.test",
    "https://node-d.example.test",
  ]);
  assert.equal(payloads[0]?.selected_node_url, "https://node-a.example.test");
  assert.equal(payloads[1]?.bancor_service_node_url, "https://node-c.example.test");
  await mounted.unmount();
});

test("NNI page entry reuses the device status until an explicit refresh", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let statusReads = 0;
  const apiFetch = async (path: string) => {
    if (path === "/v1/nni/device/status" || path === "/v1/nni/device/status?refresh=true") {
      statusReads += 1;
      return apiResponse(readyChipStatus());
    }
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => {
    await mounted.runtime().ensureNniDeviceStatus();
  });
  await act(async () => {
    await mounted.runtime().ensureNniDeviceStatus(true);
  });
  assert.equal(statusReads, 1);

  await act(async () => {
    await mounted.runtime().fetchNniDeviceStatus();
  });
  assert.equal(statusReads, 2);
  await mounted.unmount();
});

test("public NNI network stats load without joining or invoking the device signer", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requests: string[] = [];
  const apiFetch = async (path: string) => {
    requests.push(path);
    if (path === "/v1/nni/network-stats") {
      return apiResponse({
        schema_version: 1,
        status: "heartbeat_network_stats",
        network_devices: {
          registered_device_count: 153,
          active_device_count: 0,
          active_period_start_unix: null,
          active_period_end_unix: null,
          first_heartbeat_unix: null,
          window_seconds: 600,
        },
        reward_policy: {
          phase: "active",
          accepting_reward_heartbeats: true,
          reward_start_time_unix: 1_800_000_000,
          starts_in_seconds: 0,
          first_settlement_at_unix: 1_800_000_600,
          interval_seconds: 600,
          initial_reward_pool_aic: 5000,
          current_reward_pool_units: "500000000000",
          current_reward_pool_aic: "5000.00000000",
          distribution: "equal_per_eligible_device",
          halving_epoch_unix: null,
          halving_interval_seconds: 126_144_000,
          halving_era: null,
          rewards_ended: false,
          next_halving_at_unix: null,
        },
        network_rewards: {
          total_distributed_reward_units: "0",
          total_distributed_reward_aic: "0.00000000",
          settled_period_count: 0,
          first_period_start_unix: null,
          latest_period_end_unix: null,
        },
      });
    }
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => {
    await mounted.runtime().fetchNniNetworkStats();
  });

  assert.equal(mounted.runtime().nniJoined, false);
  assert.equal(mounted.runtime().nniNetworkStats?.network_devices.registered_device_count, 153);
  assert.deepEqual(requests, ["/v1/nni/network-stats"]);
  await mounted.unmount();
});

test("NNI device action failure does not persist an implicit leave", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requests: Array<{ path: string; method: string }> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    requests.push({ path, method: init?.method ?? "GET" });
    if (path === "/v1/nni/config") return apiResponse(joinedConfig());
    if (path === "/v1/nni/device/action") throw new Error("signature helper timed out");
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => {
    await mounted.runtime().fetchNniConfig(true);
    await mounted.runtime().runNniDeviceAction("pubkey");
  });

  assert.equal(mounted.runtime().nniJoined, true);
  assert.match(mounted.runtime().nniActionError ?? "", /timed out/);
  assert.equal(requests.filter((request) => request.method === "POST" && request.path === "/v1/nni/config").length, 0);
  await mounted.unmount();
});

test("NNI initial binding exposes a structured public-key authorization rejection", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const assetOwnerPubkey = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";
  let joinRequest: unknown = null;
  const apiFetch = async (path: string, init?: RequestInit) => {
    if (path === "/v1/nni/device/status?refresh=true") return apiResponse(readyChipStatus());
    if (path === "/v1/nni/config" && !init?.method) {
      return apiResponse({ ...joinedConfig(), joined: false, asset_owner_pubkey: null });
    }
    if (path === "/v1/nni/join/request") {
      joinRequest = JSON.parse(String(init?.body));
      return new Response(
        JSON.stringify({
          ok: false,
          error: "nni_remote_nodes_unavailable",
          data: {
            status: "remote_nodes_unavailable",
            attempts: [{ error: "nni_pubkey_not_allowlisted", data: { status: "public_key_not_allowlisted" } }],
          },
        }),
        { status: 502, headers: { "content-type": "application/json" } },
      );
    }
    if (path === "/v1/nni/config" && init?.method === "POST") {
      return apiResponse({ ...joinedConfig(), joined: false });
    }
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => {
    await mounted.runtime().fetchNniConfig();
  });
  await act(async () => {
    mounted.runtime().updateNniRemoteNodes("https://node-a.example.test\nhttps://nni.example.test");
  });
  await act(async () => {
    mounted.runtime().updateNniSelectedNodeUrl("https://nni.example.test");
  });
  await act(async () => {
    await mounted.runtime().startNniCustomOwnerAuthorization(assetOwnerPubkey);
  });

  assert.equal(mounted.runtime().nniDeviceAuthorizationDenied, true);
  assert.deepEqual(joinRequest, {
    node_url: "https://nni.example.test",
    asset_owner_pubkey: assetOwnerPubkey,
    replace_existing_owner: false,
  });
  assert.match(mounted.runtime().nniActionError ?? "", /尚未获远程 NNI 服务端允许/);

  await act(async () => {
    mounted.runtime().updateNniRemoteNodes("https://other-nni.example.test");
  });
  assert.equal(mounted.runtime().nniDeviceAuthorizationDenied, false);
  await mounted.unmount();
});

test("NNI restores the server-side asset binding when local state is empty", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const existingOwner = "7tkEuc2r5gBxejbtDsEn72rJHHcst3t9bmHJxZNEXZo9Nz9CxB";
  const requestedOwner = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";
  const apiFetch = async (path: string, init?: RequestInit) => {
    if (path === "/v1/nni/config") {
      return apiResponse({ ...joinedConfig(), joined: false, asset_owner_pubkey: null });
    }
    if (path === "/v1/nni/device/status?refresh=true") return apiResponse(readyChipStatus());
    if (path === "/v1/nni/join/request") {
      return new Response(JSON.stringify({
        ok: false,
        error: "nni_asset_device_already_bound",
        data: {
          status: "asset_owner_conflict",
          asset_owner_pubkey: existingOwner,
          local_binding_restored: true,
          joined: false,
        },
      }), { status: 409, headers: { "content-type": "application/json" } });
    }
    throw new Error(`unexpected request: ${path} ${init?.method ?? "GET"}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => mounted.runtime().fetchNniConfig());
  await act(async () => {
    await mounted.runtime().startNniCustomOwnerAuthorization(requestedOwner);
  });

  assert.equal(mounted.runtime().nniAssetOwnerPubkey, existingOwner);
  assert.equal(mounted.runtime().nniJoined, false);
  assert.equal(mounted.runtime().nniOwnerAuthorizationChallenge, null);
  assert.match(mounted.runtime().nniActionError ?? "", /已恢复本机绑定显示/);
  await mounted.unmount();
});

test("NNI starts heartbeats for an existing binding without creating another join challenge", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const assetOwnerPubkey = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";
  const requests: Array<{ path: string; body: Record<string, unknown> | null }> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : null;
    requests.push({ path, body });
    if (path === "/v1/nni/config" && !init?.method) {
      return apiResponse({ ...joinedConfig(), joined: false, asset_owner_pubkey: assetOwnerPubkey });
    }
    if (path === "/v1/nni/device/status?refresh=true") return apiResponse(readyChipStatus());
    if (path === "/v1/nni/config" && init?.method === "POST") {
      return apiResponse({ ...joinedConfig(), joined: true, asset_owner_pubkey: assetOwnerPubkey });
    }
    if (path.startsWith("/v1/nni/records?")) {
      return apiResponse({ records: [], page: 1, per_page: 10, total: 0, total_pages: 1 });
    }
    if (path.startsWith("/v1/nni/rewards?")) {
      return apiResponse({ records: [], page: 1, per_page: 10, total: 0, total_pages: 1 });
    }
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => mounted.runtime().fetchNniConfig());
  assert.equal(mounted.runtime().nniJoined, false);
  await act(async () => mounted.runtime().joinNni());

  const configRequest = requests.find((request) => request.path === "/v1/nni/config" && request.body);
  assert.equal(configRequest?.body?.joined, true);
  assert.equal(requests.some((request) => request.path === "/v1/nni/join/request"), false);
  assert.equal(requests.some((request) => request.path === "/v1/nni/join/verify"), false);
  assert.equal(mounted.runtime().nniJoined, true);
  await mounted.unmount();
});

test("NNI asset owner generation is local and keeps the private key in transient UI state only", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requests: Array<{ path: string; body: unknown }> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    requests.push({
      path,
      body: init?.body ? JSON.parse(String(init.body)) : null,
    });
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => {
    await mounted.runtime().generateNniOwnerKeyPair();
  });

  const generated = mounted.runtime().nniOwnerKeyPair;
  assert.ok(generated);
  const validation = validateNniOwnerPrivateKey(generated.private_key);
  assert.equal(validation.ok, true);
  assert.equal(validation.ok ? validation.publicKey : null, generated.public_key);
  assert.deepEqual(requests, []);
  await act(async () => mounted.runtime().clearNniOwnerKeyPair());
  assert.equal(mounted.runtime().nniOwnerKeyPair, null);
  await mounted.unmount();
});

test("NNI recovery signs in the browser and never sends the owner private key", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const ownerPrivateKey = encodeTestOwnerPrivateKey(
    Uint8Array.from({ length: 32 }, (_, index) => index + 1),
  );
  const validation = validateNniOwnerPrivateKey(ownerPrivateKey);
  assert.equal(validation.ok, true);
  if (!validation.ok) return;
  const assetOwnerPubkey = validation.publicKey;
  const signingPayload = JSON.stringify({
    action: "rotate_asset_device",
    task_id: "recovery-task-1",
    asset_owner_pubkey: assetOwnerPubkey,
  });
  const requests: Array<{ path: string; body: Record<string, unknown> | null }> = [];
  let recoveryRequestCount = 0;
  const apiFetch = async (path: string, init?: RequestInit) => {
    const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : null;
    requests.push({ path, body });
    if (path === "/v1/nni/owner/recover") {
      recoveryRequestCount += 1;
      if (recoveryRequestCount === 1) {
        return apiResponse({
          schema_version: 1,
          status: "asset_recovery_challenge_created",
          task_id: "recovery-task-1",
          signing_payload: signingPayload,
          device_signature: "ab".repeat(64),
          asset_owner_pubkey: assetOwnerPubkey,
        });
      }
      return apiResponse({
        schema_version: 1,
        status: "asset_device_rotated",
        asset_owner_pubkey: assetOwnerPubkey,
        device_pubkey: "ab".repeat(64),
        authorization_epoch: 2,
        authorization_status: "active",
      });
    }
    if (path === "/v1/nni/config" && init?.method === "POST") {
      return apiResponse({
        ...joinedConfig(),
        joined: false,
        asset_owner_pubkey: assetOwnerPubkey,
      });
    }
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => {
    mounted.runtime().updateNniRemoteNodes("https://nni.example.test");
  });
  await act(async () => {
    await mounted.runtime().recoverNniOwner(ownerPrivateKey);
  });

  assert.equal(mounted.runtime().nniAssetOwnerPubkey, assetOwnerPubkey);
  assert.equal(mounted.runtime().nniOwnerKeyPair, null);
  assert.equal(requests[0].path, "/v1/nni/owner/recover");
  assert.equal(requests[0].body?.asset_owner_pubkey, assetOwnerPubkey);
  assert.equal(requests[1].path, "/v1/nni/owner/recover");
  assert.equal(requests[1].body?.task_id, "recovery-task-1");
  assert.equal(typeof requests[1].body?.owner_signature, "string");
  assert.equal(requests[2].path, "/v1/nni/config");
  assert.equal(requests[2].body?.joined, false);
  assert.equal(mounted.runtime().nniJoined, false);
  for (const request of requests) {
    assert.equal(Object.hasOwn(request.body ?? {}, "owner_private_key"), false);
    assert.equal(Object.hasOwn(request.body ?? {}, "private_key"), false);
  }
  await mounted.unmount();
});

test("NNI custom owner binding forwards external signatures without private key material", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const assetOwnerPubkey = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";
  const ownerSignature = "AB".repeat(64);
  const deviceSignature = "cd".repeat(64);
  const requests: Array<{ path: string; body: Record<string, unknown> | null }> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : null;
    requests.push({ path, body });
    if (path === "/v1/nni/device/status?refresh=true") return apiResponse(readyChipStatus());
    if (path === "/v1/nni/join/request") {
      return apiResponse({
        status: "challenge_created",
        task_id: "join-custom-owner",
        challenge: "canonical-owner-challenge",
        device_pubkey: "ab".repeat(64),
        node_url: "https://nni.example.test",
        expires_at_ts: 1_900_000_000,
        request_interval_seconds: 60,
        asset_owner_pubkey: assetOwnerPubkey,
        owner_signature_required: true,
      });
    }
    if (path === "/v1/nni/device/action") {
      return apiResponse({
        action: "sign_challenge",
        signature_chip_present: true,
        payload: { signature: deviceSignature },
      });
    }
    if (path === "/v1/nni/join/verify") {
      return apiResponse({
        status: "joined",
        task_id: "join-custom-owner",
        device_pubkey: "ab".repeat(64),
        node_url: "https://nni.example.test",
        compliant: true,
        joined: true,
        verified_at_ts: 1_800_000_000,
        next_allowed_ts: 1_800_000_060,
        asset_owner_pubkey: assetOwnerPubkey,
      });
    }
    if (path === "/v1/nni/config" && init?.method === "POST") {
      return apiResponse({ ...joinedConfig(), joined: false, asset_owner_pubkey: assetOwnerPubkey });
    }
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => mounted.runtime().updateNniRemoteNodes("https://nni.example.test"));
  await act(async () => {
    await mounted.runtime().startNniCustomOwnerAuthorization(assetOwnerPubkey);
  });
  assert.equal(mounted.runtime().nniOwnerAuthorizationChallenge?.mode, "bind");
  await act(async () => {
    await mounted.runtime().completeNniOwnerAuthorization(ownerSignature);
  });

  const joinRequest = requests.find((request) => request.path === "/v1/nni/join/request");
  assert.deepEqual(joinRequest?.body, {
    node_url: "https://nni.example.test",
    asset_owner_pubkey: assetOwnerPubkey,
    replace_existing_owner: false,
  });
  const verifyRequest = requests.find((request) => request.path === "/v1/nni/join/verify");
  assert.equal(verifyRequest?.body?.owner_signature, ownerSignature.toLowerCase());
  assert.equal(verifyRequest?.body?.signature, deviceSignature);
  assert.equal(verifyRequest?.body?.replace_existing_owner, false);
  assert.equal(Object.hasOwn(verifyRequest?.body ?? {}, "owner_private_key"), false);
  assert.equal(Object.hasOwn(verifyRequest?.body ?? {}, "private_key"), false);
  const configRequest = requests.find((request) => request.path === "/v1/nni/config");
  assert.equal(configRequest?.body?.joined, false);
  assert.equal(mounted.runtime().nniJoined, false);
  assert.equal(mounted.runtime().nniAssetOwnerPubkey, assetOwnerPubkey);
  assert.equal(mounted.runtime().nniOwnerAuthorizationChallenge, null);
  await mounted.unmount();
});

test("NNI convenient binding signs in the browser and never sends private key material", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const ownerPrivateKey = encodeTestOwnerPrivateKey(
    Uint8Array.from({ length: 32 }, (_, index) => index + 1),
  );
  const ownerValidation = validateNniOwnerPrivateKey(ownerPrivateKey);
  assert.equal(ownerValidation.ok, true);
  if (!ownerValidation.ok) return;
  const requests: Array<{ path: string; body: Record<string, unknown> | null }> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : null;
    requests.push({ path, body });
    if (path === "/v1/nni/device/status?refresh=true") return apiResponse(readyChipStatus());
    if (path === "/v1/nni/join/request") {
      return apiResponse({
        status: "challenge_created",
        task_id: "join-local-owner",
        challenge: "canonical-local-owner-challenge",
        device_pubkey: "ab".repeat(64),
        node_url: "https://nni.example.test",
        expires_at_ts: 1_900_000_000,
        request_interval_seconds: 60,
        asset_owner_pubkey: ownerValidation.publicKey,
        owner_signature_required: true,
      });
    }
    if (path === "/v1/nni/device/action") {
      return apiResponse({
        action: "sign_challenge",
        signature_chip_present: true,
        payload: { signature: "cd".repeat(64) },
      });
    }
    if (path === "/v1/nni/join/verify") {
      return apiResponse({
        status: "joined",
        task_id: "join-local-owner",
        device_pubkey: "ab".repeat(64),
        node_url: "https://nni.example.test",
        compliant: true,
        joined: true,
        verified_at_ts: 1_800_000_000,
        next_allowed_ts: 1_800_000_060,
        asset_owner_pubkey: ownerValidation.publicKey,
      });
    }
    if (path === "/v1/nni/config" && init?.method === "POST") {
      return apiResponse({ ...joinedConfig(), joined: false, asset_owner_pubkey: ownerValidation.publicKey });
    }
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => mounted.runtime().updateNniRemoteNodes("https://nni.example.test"));
  await act(async () => {
    await mounted.runtime().authorizeNniOwnerWithPrivateKey(ownerPrivateKey);
  });

  const request = requests.find((item) => item.path === "/v1/nni/join/request");
  assert.equal(request?.body?.asset_owner_pubkey, ownerValidation.publicKey);
  const verify = requests.find((item) => item.path === "/v1/nni/join/verify");
  assert.match(String(verify?.body?.owner_signature), /^[0-9a-f]{128}$/);
  assert.equal(Object.hasOwn(verify?.body ?? {}, "owner_private_key"), false);
  assert.equal(Object.hasOwn(verify?.body ?? {}, "private_key"), false);
  assert.equal(JSON.stringify(requests).includes(ownerPrivateKey), false);
  const configRequest = requests.find((item) => item.path === "/v1/nni/config");
  assert.equal(configRequest?.body?.joined, false);
  assert.equal(mounted.runtime().nniJoined, false);
  assert.equal(mounted.runtime().nniAssetOwnerPubkey, ownerValidation.publicKey);
  assert.equal(mounted.runtime().nniOwnerAuthorizationChallenge, null);
  await mounted.unmount();
});

test("NNI owner replacement requires the hardware and target-owner signatures", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const previousOwner = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";
  const targetOwner = "6PhSs6H49U1Lb6vz9GDtUF9RjtpFpkS6Rxm94LumQrnCziKzkb";
  const targetOwnerSignature = "78".repeat(64);
  const requests: Array<{ path: string; body: Record<string, unknown> | null }> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : null;
    requests.push({ path, body });
    if (path === "/v1/nni/config" && !init?.method) {
      return apiResponse({ ...joinedConfig(), asset_owner_pubkey: previousOwner });
    }
    if (path === "/v1/nni/device/status?refresh=true") return apiResponse(readyChipStatus());
    if (path === "/v1/nni/join/request") {
      return apiResponse({
        status: "challenge_created",
        task_id: "replace-owner",
        challenge: "replace-owner-payload",
        device_pubkey: "ab".repeat(64),
        node_url: "https://nni.example.test",
        expires_at_ts: 1_900_000_000,
        request_interval_seconds: 60,
        asset_owner_pubkey: targetOwner,
        owner_signature_required: true,
        previous_owner_signature_required: false,
        previous_asset_owner_pubkey: previousOwner,
      });
    }
    if (path === "/v1/nni/device/action") {
      return apiResponse({
        action: "sign_challenge",
        signature_chip_present: true,
        payload: { signature: "56".repeat(64) },
      });
    }
    if (path === "/v1/nni/join/verify") {
      return apiResponse({
        status: "joined",
        task_id: "replace-owner",
        device_pubkey: "ab".repeat(64),
        node_url: "https://nni.example.test",
        compliant: true,
        joined: true,
        verified_at_ts: 1_800_000_000,
        next_allowed_ts: 1_800_000_060,
        asset_owner_pubkey: targetOwner,
      });
    }
    if (path === "/v1/nni/config" && init?.method === "POST") {
      return apiResponse({ ...joinedConfig(), joined: false, asset_owner_pubkey: targetOwner });
    }
    throw new Error(`unexpected request: ${path}`);
  };
  const mounted = await mountRuntime(apiFetch);

  await act(async () => mounted.runtime().fetchNniConfig());
  await act(async () => {
    await mounted.runtime().startNniCustomOwnerAuthorization(targetOwner);
  });
  assert.equal(mounted.runtime().nniOwnerAuthorizationChallenge?.replaceExistingOwner, true);
  assert.equal(
    mounted.runtime().nniOwnerAuthorizationChallenge?.targetOwnerPublicKey,
    targetOwner,
  );
  await act(async () => {
    await mounted.runtime().completeNniOwnerAuthorization(targetOwnerSignature);
  });

  const request = requests.find((item) => item.path === "/v1/nni/join/request");
  assert.equal(request?.body?.replace_existing_owner, true);
  assert.equal(request?.body?.asset_owner_pubkey, targetOwner);
  const verify = requests.find((item) => item.path === "/v1/nni/join/verify");
  assert.equal(verify?.body?.owner_signature, targetOwnerSignature);
  assert.equal(Object.hasOwn(verify?.body ?? {}, "previous_owner_signature"), false);
  assert.equal(verify?.body?.replace_existing_owner, true);
  assert.equal(Object.hasOwn(verify?.body ?? {}, "owner_private_key"), false);
  const configRequest = requests.find((item) => item.path === "/v1/nni/config" && item.body);
  assert.equal(configRequest?.body?.joined, false);
  assert.equal(mounted.runtime().nniJoined, false);
  assert.equal(mounted.runtime().nniAssetOwnerPubkey, targetOwner);
  await mounted.unmount();
});
