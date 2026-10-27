import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { SandboxedAipp } from "../components/AippPage";
import type { AippCatalogItem } from "../types/api";

const app: AippCatalogItem = {
  skill_name: "fixture", package_version: "1.0.0", renderer: "sandbox_bundle_v1",
  data_contract: "capability_bridge_v1", icon: "gallery_vertical_end", default_locale: "en",
  titles: { en: "Fixture" }, descriptions: { en: "Fixture" }, installed: true,
  entrypoint: "index.html", bridge_capabilities: ["fixture.read"], task_channel_scope: "all",
};
const response = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));

for (const mode of ["node-owner", "app-version", "roundtrip"] as const) {
  test(`AiAPP bridge aborts obsolete reads and never posts to a new scope: ${mode}`, async () => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
    const listeners = new Set<(event: MessageEvent) => void>();
    Object.defineProperty(globalThis, "window", { configurable: true, value: {
      addEventListener: (_type: string, listener: (event: MessageEvent) => void) => listeners.add(listener),
      removeEventListener: (_type: string, listener: (event: MessageEvent) => void) => listeners.delete(listener),
      setTimeout, clearTimeout,
    } });
    const replies: unknown[] = [];
    const target = { postMessage: (payload: unknown) => replies.push(payload) };
    const frame = { contentWindow: target };
    const requests: Array<{ path: string; signal?: AbortSignal | null; finish: (response: Response) => void }> = [];
    const api = (path: string, init?: RequestInit) => new Promise<Response>(finish => requests.push({ path, signal: init?.signal, finish }));
    const otherApi = (path: string, init?: RequestInit) => api(path, init);
    const event = () => ({ source: target, origin: "null", data: {
      schema_version: 1, type: "aipp.capability.invoke", request_id: "same-id", capability: "fixture.read", args: {},
    } } as unknown as MessageEvent);
    let root: ReactTestRenderer | undefined;
    try {
      await act(async () => { root = create(<SandboxedAipp requestScope="first" app={app} lang="en" apiFetch={api} />, { createNodeMock: () => frame }); });
      const oldListener = [...listeners][0];
      await act(async () => { oldListener(event()); });
      assert.equal(requests.length, 1);
      await act(async () => { root!.update(<SandboxedAipp requestScope="first" app={app} lang="en" apiFetch={otherApi} />); });
      assert.equal(requests[0].signal?.aborted, false, "ordinary renders must not abort requests when apiFetch has a fresh function identity");
      await act(async () => { root!.update(<SandboxedAipp requestScope="second" app={mode === "app-version" ? { ...app, package_version: "2.0.0" } : app} lang="en" apiFetch={otherApi} />); });
      if (mode === "roundtrip") await act(async () => { root!.update(<SandboxedAipp requestScope="first" app={app} lang="en" apiFetch={api} />); });
      assert.equal(requests[0].signal?.aborted, true);
      await act(async () => { oldListener(event()); });
      assert.equal(requests.length, 1, "removed listener cannot submit tasks");
      await act(async () => { [...listeners][0](event()); });
      assert.equal(requests.length, 2, "new scope owns its in-flight request ID set");
      await act(async () => { requests[0].finish(response({ task_id: "old-private-task" })); });
      assert.equal(requests.length, 2, "obsolete response must not start polling");
      assert.deepEqual(replies, []);
      await act(async () => { requests[1].finish(response({ task_id: "current-task" })); });
      assert.equal(requests.length, 3);
      await act(async () => { requests[2].finish(response({ task_id: "current-task", status: "succeeded", result_json: { value: "current" } })); });
      assert.equal(replies.length, 1);
      assert.equal((replies[0] as { data: { task_id: string } }).data.task_id, "current-task");
    } finally {
      await act(async () => root?.unmount());
      if (previous) Object.defineProperty(globalThis, "window", previous);
      else Reflect.deleteProperty(globalThis, "window");
    }
  });
}
