import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import { UiDialogProvider } from "../components/UiDialogProvider";
import type { NniConfigResponse, NniDeviceStatusResponse } from "../types/api";
import { useNniRuntime } from "./useNniRuntime";

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
          initial_reward_pool_points: 5000,
          current_reward_pool_units: "500000000000",
          current_reward_pool_points: "5000.00000000",
          distribution: "equal_per_eligible_device",
          halving_epoch_unix: null,
          halving_interval_seconds: 126_144_000,
          halving_era: null,
          rewards_ended: false,
          next_halving_at_unix: null,
        },
        network_rewards: {
          total_distributed_reward_units: "0",
          total_distributed_reward_points: "0.00000000",
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

test("NNI join exposes a structured public-key authorization rejection", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let joinRequest: unknown = null;
  const apiFetch = async (path: string, init?: RequestInit) => {
    if (path === "/v1/nni/device/status?refresh=true") return apiResponse(readyChipStatus());
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
    mounted.runtime().updateNniRemoteNodes("https://node-a.example.test\nhttps://nni.example.test");
  });
  await act(async () => {
    mounted.runtime().updateNniSelectedNodeUrl("https://nni.example.test");
  });
  await act(async () => {
    await mounted.runtime().joinNni();
  });

  assert.equal(mounted.runtime().nniDeviceAuthorizationDenied, true);
  assert.deepEqual(joinRequest, { node_url: "https://nni.example.test" });
  assert.match(mounted.runtime().nniActionError ?? "", /尚未获远程 NNI 服务端允许/);

  await act(async () => {
    mounted.runtime().updateNniRemoteNodes("https://other-nni.example.test");
  });
  assert.equal(mounted.runtime().nniDeviceAuthorizationDenied, false);
  await mounted.unmount();
});
