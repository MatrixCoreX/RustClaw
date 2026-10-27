import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useTaskRuntime, type UseTaskRuntimeParams } from "./useTaskRuntime";

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

function reply(data: unknown): Response {
  return new Response(JSON.stringify({ ok: true, data }));
}

async function taskProbe(apiFetch: UseTaskRuntimeParams["apiFetch"]) {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let runtime!: ReturnType<typeof useTaskRuntime>;
  const defaults = { owner: "one", apiBase: "node-one", ready: true,
    page: "dashboard" as UseTaskRuntimeParams["currentPage"] };
  function Probe({ owner, apiBase, ready, page }: typeof defaults) {
    runtime = useTaskRuntime({ apiFetch, t: zh => zh, apiBase, uiAuthReady: ready,
      currentPage: page, interactionUserId: 1, interactionChatId: 2,
      activeUserKey: owner, activeIdentityIds: {} });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe, defaults)); });
  return {
    get runtime() { return runtime; },
    async update(props: Partial<typeof defaults>) {
      await act(async () => { renderer.update(React.createElement(Probe, { ...defaults, ...props })); });
    },
    async close() { await act(async () => { renderer.unmount(); }); },
  };
}

function pendingApi() {
  const calls: Array<{ path: string; finish: (data: unknown) => void; fail: (error: Error) => void }> = [];
  return {
    calls,
    apiFetch: (path: string) => new Promise<Response>((resolve, reject) => {
      calls.push({ path, finish: data => resolve(reply(data)), fail: reject });
    }),
  };
}

test("newest task selection and teaching page win over slow older requests", async t => {
  const api = pendingApi();
  const probe = await taskProbe(api.apiFetch);
  t.after(() => probe.close());
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = probe.runtime.viewTask("first"); });
  await act(async () => { second = probe.runtime.viewTask("second"); });
  await act(async () => {
    api.calls[1].finish({ task_id: "second", status: "succeeded" });
    await second;
  });
  await act(async () => {
    api.calls[0].finish({ task_id: "first", status: "succeeded" });
    await first;
  });
  assert.equal(probe.runtime.taskResult?.task_id, "second");
  await act(async () => { first = probe.runtime.queryTaskLlmDebug("second", 0); });
  await act(async () => { second = probe.runtime.queryTaskLlmDebug("second", 20); });
  await act(async () => {
    api.calls[3].finish({ task_id: "second", pagination: { offset: 20 } });
    await second;
    api.calls[2].finish({ task_id: "second", pagination: { offset: 0 } });
    await first;
  });
  assert.equal(probe.runtime.taskLlmDebug?.pagination?.offset, 20);
});

test("a late task-detail error cannot clear the newer task or its loading state", async t => {
  const api = pendingApi();
  const probe = await taskProbe(api.apiFetch);
  t.after(() => probe.close());
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = probe.runtime.viewTask("first"); });
  await act(async () => { second = probe.runtime.viewTask("second"); });
  await act(async () => { api.calls[0].fail(new Error("late_failure")); await first; });
  assert.equal(probe.runtime.taskLoading, true);
  assert.equal(probe.runtime.taskError, null);
  await act(async () => { api.calls[1].finish({ task_id: "second", status: "succeeded" }); await second; });
  assert.equal(probe.runtime.taskLoading, false);
  assert.equal(probe.runtime.taskResult?.task_id, "second");
});

test("active-task pages cannot be replaced by an older slow page", async t => {
  const api = pendingApi();
  const probe = await taskProbe(api.apiFetch);
  t.after(() => probe.close());
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = probe.runtime.fetchActiveTasks(false, 0); });
  await act(async () => { second = probe.runtime.fetchActiveTasks(false, 20); });
  await act(async () => {
    api.calls[1].finish({ tasks: [{ task_id: "second-page" }], total: 30, offset: 20 }); await second;
    api.calls[0].finish({ tasks: [{ task_id: "first-page" }], total: 30, offset: 0 }); await first;
  });
  assert.equal(probe.runtime.activeTasksOffset, 20);
  assert.equal(probe.runtime.activeTasks[0]?.task_id, "second-page");
});

for (const change of ["owner", "api", "logout", "owner-roundtrip"] as const) {
  test(`identity/API change discards in-flight task, teaching, list and grant responses: ${change}`, async t => {
    const api = pendingApi();
    const probe = await taskProbe(api.apiFetch);
    t.after(() => probe.close());
    const work: Promise<unknown>[] = [];
    await act(async () => {
      work.push(probe.runtime.viewTask("private-task"));
      work.push(probe.runtime.queryTaskLlmDebug("private-task"));
      work.push(probe.runtime.fetchActiveTasks());
      work.push(probe.runtime.fetchTaskHistory());
      work.push(probe.runtime.fetchApprovalScopeGrants());
    });
    if (change === "owner" || change === "owner-roundtrip") await probe.update({ owner: "two" });
    if (change === "owner-roundtrip") await probe.update({ owner: "one" });
    if (change === "api") await probe.update({ apiBase: "node-two" });
    if (change === "logout") await probe.update({ ready: false });
    await act(async () => {
      api.calls[0].finish({ task_id: "private-task", status: "succeeded" });
      api.calls[1].finish({ task_id: "private-task", records: ["private-trace"] });
      api.calls[2].finish({ tasks: [{ task_id: "private-task" }] });
      api.calls[3].finish({ tasks: [{ task_id: "private-task" }] });
      api.calls[4].finish({ grants: [{ grant_id: "private-grant" }] });
      await Promise.all(work);
    });
    assert.equal(probe.runtime.taskResult, null);
    assert.equal(probe.runtime.taskLlmDebug, null);
    assert.deepEqual(probe.runtime.activeTasks, []);
    assert.deepEqual(probe.runtime.taskHistory, []);
    assert.deepEqual(probe.runtime.approvalScopeGrants, []);
    assert.equal(probe.runtime.taskLoading, false);
    assert.equal(probe.runtime.taskLlmDebugLoading, false);
    assert.equal(probe.runtime.activeTasksLoading, false);
    assert.equal(probe.runtime.approvalScopeGrantsLoading, false);
  });
}

test("task controls and approvals work on LAN HTTP without crypto.randomUUID", async t => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "crypto");
  const getRandomValues = globalThis.crypto.getRandomValues.bind(globalThis.crypto);
  Object.defineProperty(globalThis, "crypto", { configurable: true, value: { getRandomValues } });
  t.after(() => { if (previous) Object.defineProperty(globalThis, "crypto", previous); });
  const keys: string[] = [];
  const probe = await taskProbe(async (path, init) => {
    const body = init?.body ? JSON.parse(String(init.body)) : {};
    if (body.idempotency_key) keys.push(body.idempotency_key);
    return reply(path === "/v1/tasks/active" ? { tasks: [] } : { status: "accepted" });
  });
  t.after(() => probe.close());
  for (const control of ["steer", "pause", "resume"] as const) {
    await act(async () => { await probe.runtime.controlTaskById(control, "task", "updated direction"); });
    assert.equal(probe.runtime.taskControlError, null);
  }
  await act(async () => { await probe.runtime.controlSubagentById("stop", "parent", "child"); });
  assert.equal(probe.runtime.taskControlError, null);
  await act(async () => { await probe.runtime.decideTaskApprovalById("task", "approval", "approve_once"); });
  assert.equal(probe.runtime.taskControlError, null);
  assert.equal(keys.length, 5);
  assert.equal(new Set(keys).size, 5);
  for (const key of keys) assert.match(key, /[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});

test("an old live stream final query cannot replace the selected historical task", async t => {
  let finishLive!: (response: Response) => void;
  const probe = await taskProbe(async path => {
    if (path.includes("/events?")) return new Response(`data: ${JSON.stringify({
      schema_version: 1, task_id: "live", seq: 1, event_kind: "task_final", payload: { status: "succeeded" },
    })}\n\n`, { headers: { "Content-Type": "text/event-stream" } });
    if (path === "/v1/tasks/live") return new Promise<Response>(resolve => { finishLive = resolve; });
    return reply({ task_id: "history", status: "succeeded" });
  });
  t.after(() => probe.close());
  await act(async () => { probe.runtime.markTaskSubmitted("live"); });
  assert.equal(typeof finishLive, "function");
  await act(async () => { await probe.runtime.viewTask("history"); });
  await act(async () => {
    finishLive(reply({ task_id: "live", status: "succeeded" }));
    await new Promise(resolve => setImmediate(resolve));
  });
  assert.equal(probe.runtime.taskResult?.task_id, "history");
  assert.equal(probe.runtime.trackingTaskId, null);
});

test("stale mutation completions and old event handlers cannot publish into a new identity", async t => {
  const api = pendingApi();
  const probe = await taskProbe(api.apiFetch);
  t.after(() => probe.close());
  const oldControl = probe.runtime.controlTaskById;
  let first!: Promise<void>;
  await act(async () => { first = oldControl("pause", "old"); });
  await probe.update({ owner: "two" });
  await act(async () => {
    api.calls[0].finish({ status: "paused" }); await first;
    await oldControl("pause", "old");
  });
  assert.equal(api.calls.length, 1);
  assert.equal(probe.runtime.taskControlMessage, null);
  assert.equal(probe.runtime.taskControlError, null);
  assert.equal(probe.runtime.taskControlSubmittingId, null);
});

test("slow periodic refreshes do not overlap or starve task and grant results", async t => {
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const oldDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  let tick!: () => void;
  Object.defineProperty(globalThis, "window", { configurable: true, value: {
    setInterval: (callback: () => void) => { tick = callback; return 1; }, clearInterval: () => {},
  } });
  Object.defineProperty(globalThis, "document", { configurable: true, value: {
    visibilityState: "visible", addEventListener: () => {}, removeEventListener: () => {},
  } });
  const api = pendingApi();
  const probe = await taskProbe(api.apiFetch);
  t.after(async () => {
    await probe.close();
    if (oldWindow) Object.defineProperty(globalThis, "window", oldWindow); else Reflect.deleteProperty(globalThis, "window");
    if (oldDocument) Object.defineProperty(globalThis, "document", oldDocument); else Reflect.deleteProperty(globalThis, "document");
  });
  await probe.update({ page: "tasks" });
  assert.equal(api.calls.length, 2);
  await act(async () => { tick(); tick(); tick(); });
  assert.equal(api.calls.length, 2, "a slow request must not be invalidated by each polling tick");
  await act(async () => {
    api.calls[0].finish({ tasks: [{ task_id: "slow-task" }] });
    api.calls[1].finish({ grants: [] });
    await new Promise(resolve => setImmediate(resolve));
  });
  assert.equal(probe.runtime.activeTasks[0]?.task_id, "slow-task");
  await act(async () => { tick(); });
  assert.equal(api.calls.length, 4, "polling resumes after the prior refresh finishes");
  await act(async () => { api.calls[2].finish({ tasks: [] }); api.calls[3].finish({ grants: [] }); });
});
