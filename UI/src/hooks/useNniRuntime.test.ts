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

test("NNI join exposes a structured public-key authorization rejection", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const apiFetch = async (path: string, init?: RequestInit) => {
    if (path === "/v1/nni/device/status") return apiResponse(readyChipStatus());
    if (path === "/v1/nni/join/request") {
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
    mounted.runtime().updateNniRemoteNodes("https://nni.example.test");
  });
  await act(async () => {
    await mounted.runtime().joinNni();
  });

  assert.equal(mounted.runtime().nniDeviceAuthorizationDenied, true);
  assert.match(mounted.runtime().nniActionError ?? "", /尚未获远程 NNI 服务端允许/);

  await act(async () => {
    mounted.runtime().updateNniRemoteNodes("https://other-nni.example.test");
  });
  assert.equal(mounted.runtime().nniDeviceAuthorizationDenied, false);
  await mounted.unmount();
});
