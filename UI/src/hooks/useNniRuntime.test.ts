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
    if (path === "/v1/nni/device/status") return apiResponse(missingChipStatus());
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
