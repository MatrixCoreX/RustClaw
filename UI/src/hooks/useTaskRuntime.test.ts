import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useTaskRuntime } from "./useTaskRuntime";

test("delivery recovery suppresses same-tick duplicates and discards stale identity responses", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let finish!: (response: Response) => void;
  const requests: Array<{ path: string; body: Record<string, unknown> }> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    requests.push({ path, body: JSON.parse(String(init?.body)) });
    return new Promise<Response>(resolve => { finish = resolve; });
  };
  let runtime!: ReturnType<typeof useTaskRuntime>;
  function Probe({ owner }: { owner: string }) {
    runtime = useTaskRuntime({ apiFetch, t: zh => zh, apiBase: "", uiAuthReady: true,
      currentPage: "dashboard", interactionUserId: 1, interactionChatId: 2,
      activeUserKey: owner, activeIdentityIds: {} });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe, { owner: "one" })); });
  let first!: Promise<void>;
  await act(async () => {
    first = runtime.reconcileTaskDelivery("task-one", "retry_unknown", "a".repeat(64));
    await runtime.reconcileTaskDelivery("task-one", "retry_unknown", "a".repeat(64));
  });
  assert.equal(requests.length, 1);
  assert.equal(requests[0].body.confirmed_duplicate_risk, true);
  assert.match(String(requests[0].body.decision_id), /^[0-9a-f-]{36}$/);
  await act(async () => { renderer.update(React.createElement(Probe, { owner: "two" })); });
  await act(async () => {
    finish(new Response(JSON.stringify({ ok: true, data: { status: "queued" } })));
    await first;
  });
  assert.equal(runtime.taskResult, null);
  assert.equal(runtime.taskControlMessage, null);
  assert.equal(runtime.taskControlError, null);
  assert.equal(runtime.taskControlSubmittingId, null);
  assert.equal(requests.length, 1, "old identity must not fetch or publish a task result");
  await act(async () => { renderer.unmount(); });
});
