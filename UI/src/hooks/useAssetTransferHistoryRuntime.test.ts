import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import { useAssetTransferHistoryRuntime } from "./useAssetTransferHistoryRuntime";

function historyResponse(
  ownerPublicKey: string,
  page = 1,
  source = "all",
  direction = "all",
): Response {
  return new Response(JSON.stringify({
    ok: true,
    data: {
      schema_version: 1,
      status: "asset_transfer_history",
      owner_pubkey: ownerPublicKey,
      page,
      per_page: 100,
      total_transactions: 0,
      total_pages: 1,
      source_filter: source,
      direction_filter: direction,
      transactions: [],
    },
  }), { status: 200, headers: { "content-type": "application/json" } });
}

test("asset transfer history runtime encodes the selected public key", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const paths: string[] = [];
  const apiFetch = async (path: string) => {
    paths.push(path);
    return historyResponse("owner/+ key");
  };
  let runtime: ReturnType<typeof useAssetTransferHistoryRuntime> | null = null;
  function Probe() {
    runtime = useAssetTransferHistoryRuntime({ apiFetch, t: (zh) => zh });
    return null;
  }
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.load("owner/+ key");
  });

  assert.deepEqual(paths, [
    "/v1/nni/assets/transfers?owner_pubkey=owner%2F%2B+key&limit=100&page=1&source=all&direction=all",
  ]);
  assert.equal(runtime!.history?.owner_pubkey, "owner/+ key");
  assert.equal(runtime!.error, null);
  await act(async () => renderer!.unmount());
});

test("asset transfer history caches ten display pages per one hundred record batch", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const paths: string[] = [];
  const apiFetch = async (path: string) => {
    paths.push(path);
    const page = path.includes("page=2") ? 2 : 1;
    return historyResponse("owner", page);
  };
  let runtime: ReturnType<typeof useAssetTransferHistoryRuntime> | null = null;
  function Probe() {
    runtime = useAssetTransferHistoryRuntime({ apiFetch, t: (zh) => zh });
    return null;
  }
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.load("owner", { displayPage: 1 });
    await runtime!.load("owner", { displayPage: 10 });
    await runtime!.load("owner", { displayPage: 11 });
  });

  assert.equal(paths.length, 2);
  assert.match(paths[0], /page=1/);
  assert.match(paths[1], /page=2/);
  assert.equal(runtime!.history?.page, 2);
  await act(async () => renderer!.unmount());
});

test("asset transfer history requests another batch when filters change", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const paths: string[] = [];
  const apiFetch = async (path: string) => {
    paths.push(path);
    return historyResponse("owner", 1, "trade", "incoming");
  };
  let runtime: ReturnType<typeof useAssetTransferHistoryRuntime> | null = null;
  function Probe() {
    runtime = useAssetTransferHistoryRuntime({ apiFetch, t: (zh) => zh });
    return null;
  }
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.load("owner", { source: "trade", direction: "incoming" });
  });

  assert.deepEqual(paths, [
    "/v1/nni/assets/transfers?owner_pubkey=owner&limit=100&page=1&source=trade&direction=incoming",
  ]);
  assert.equal(runtime!.history?.source_filter, "trade");
  await act(async () => renderer!.unmount());
});

test("asset transfer history runtime ignores a stale account response", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let resolveFirst: ((response: Response) => void) | null = null;
  let resolveSecond: ((response: Response) => void) | null = null;
  const firstResponse = new Promise<Response>((resolve) => {
    resolveFirst = resolve;
  });
  const secondResponse = new Promise<Response>((resolve) => {
    resolveSecond = resolve;
  });
  const apiFetch = (path: string) => path.includes("first") ? firstResponse : secondResponse;
  let runtime: ReturnType<typeof useAssetTransferHistoryRuntime> | null = null;
  function Probe() {
    runtime = useAssetTransferHistoryRuntime({ apiFetch, t: (zh) => zh });
    return null;
  }
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });

  let firstLoad: Promise<unknown>;
  let secondLoad: Promise<unknown>;
  await act(async () => {
    firstLoad = runtime!.load("first");
    secondLoad = runtime!.load("second");
  });
  await act(async () => {
    resolveSecond?.(historyResponse("second"));
    await secondLoad!;
  });
  await act(async () => {
    resolveFirst?.(historyResponse("first"));
    await firstLoad!;
  });

  assert.equal(runtime!.history?.owner_pubkey, "second");
  assert.equal(runtime!.loading, false);
  await act(async () => renderer!.unmount());
});
