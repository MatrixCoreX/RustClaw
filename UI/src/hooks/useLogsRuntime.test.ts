import assert from "node:assert/strict";
import test from "node:test";
import React, { createRef } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import { useLogsRuntime, type UseLogsRuntimeParams } from "./useLogsRuntime";

type PendingRequest = {
  path: string;
  resolve: (response: Response) => void;
  reject: (error: Error) => void;
};

async function harness() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      setInterval(callback: () => void) { timers.set(++nextTimer, callback); return nextTimer; },
      clearInterval(id: number) { timers.delete(id); },
    },
  });
  const document = {
    visibilityState: "hidden",
    addEventListener() {},
    removeEventListener() {},
  };
  Object.defineProperty(globalThis, "document", { configurable: true, value: document });
  const pending: PendingRequest[] = [];
  let props: UseLogsRuntimeParams = {
    apiFetch: (path) => new Promise<Response>((resolve, reject) => pending.push({ path, resolve, reject })),
    t: (zh) => zh,
    apiBase: "http://localhost:8788",
    currentPage: "logs",
    pollingSeconds: 10,
    uiAuthReady: true,
    logContainerRef: createRef<HTMLPreElement>(),
  };
  let current!: ReturnType<typeof useLogsRuntime>;
  function Probe() { current = useLogsRuntime(props); return null; }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe)); });
  const respond = async (index: number, data: unknown) => {
    await act(async () => {
      pending[index].resolve(new Response(JSON.stringify({ ok: true, data, error: null }), {
        headers: { "Content-Type": "application/json" },
      }));
    });
  };
  return {
    get current() { return current; },
    pending,
    async update(change: Partial<UseLogsRuntimeParams>) {
      props = { ...props, ...change };
      await act(async () => { renderer.update(React.createElement(Probe)); });
    },
    async poll() {
      document.visibilityState = "visible";
      await act(async () => { for (const callback of timers.values()) callback(); });
      document.visibilityState = "hidden";
    },
    respond,
    async startContent(file = "runtime.log") {
      await act(async () => { current.setSelectedLogFile(file); });
      const listIndex = pending.length;
      await act(async () => { void current.refreshLogs(); });
      await respond(listIndex, { files: [file], total: 1 });
      return pending.length - 1;
    },
    async dispose() {
      await act(async () => { renderer.unmount(); });
      for (const [key, descriptor] of [["window", previousWindow], ["document", previousDocument]] as const) {
        if (descriptor) Object.defineProperty(globalThis, key, descriptor);
        else Reflect.deleteProperty(globalThis, key);
      }
    },
  };
}

test("a delayed manual request cannot overwrite a newly selected log", async () => {
  const h = await harness();
  try {
    const first = await h.startContent("first.log");
    const second = await h.startContent("second.log");
    await h.respond(second, { text: "second content" });
    await h.respond(first, { text: "obsolete content" });
    assert.equal(h.current.logText, "second content");
    assert.equal(h.current.selectedLogFile, "second.log");
    assert.equal(h.current.logLoading, false);
  } finally { await h.dispose(); }
});

test("obsolete errors cannot replace the current log state", async () => {
  const h = await harness();
  try {
    const first = await h.startContent("first.log");
    const second = await h.startContent("second.log");
    await h.respond(second, { text: "current" });
    await act(async () => { h.pending[first].reject(new Error("obsolete request failed")); });
    assert.equal(h.current.logError, null);
    assert.equal(h.current.logText, "current");
  } finally { await h.dispose(); }
});

test("line-count changes invalidate an older manual content request", async () => {
  const h = await harness();
  try {
    const first = await h.startContent();
    await act(async () => { h.current.setLogTailLines(400); });
    const second = await h.startContent();
    await h.respond(second, { text: "400 lines" });
    await h.respond(first, { text: "200 lines" });
    assert.equal(h.current.logText, "400 lines");
  } finally { await h.dispose(); }
});

test("log-list responses apply only to the latest endpoint", async () => {
  const h = await harness();
  try {
    await act(async () => { void h.current.refreshLogs(); });
    await h.update({ apiBase: "http://localhost:9876" });
    await act(async () => { void h.current.refreshLogs(); });
    await h.respond(1, { files: ["new.log"], total: 2, has_more: false });
    await h.respond(2, { text: "current" });
    await h.respond(0, { files: ["old.log"], total: 2, has_more: true, next_cursor: "obsolete" });
    assert.deepEqual(h.current.logFiles, ["new.log"]);
    assert.equal(h.current.selectedLogFile, "new.log");
    assert.equal(h.current.logFilesHasNext, false);
    assert.equal(h.current.logFilesLoading, false);
  } finally { await h.dispose(); }
});

test("leaving the page invalidates a pending manual refresh without starting its content fetch", async () => {
  const h = await harness();
  try {
    await act(async () => { void h.current.refreshLogs(); });
    await h.update({ currentPage: "home" });
    await h.respond(0, { files: ["obsolete.log"], total: 1 });
    assert.equal(h.pending.length, 1);
    assert.deepEqual(h.current.logFiles, []);
    assert.equal(h.current.logFilesLoading, false);
  } finally { await h.dispose(); }
});

test("endpoint changes reject stale content and release loading", async () => {
  const h = await harness();
  try {
    const first = await h.startContent();
    await h.update({ apiBase: "http://localhost:9876" });
    await h.respond(first, { text: "from previous endpoint" });
    assert.equal(h.current.logText, "");
    assert.equal(h.current.logLoading, false);
  } finally { await h.dispose(); }
});

test("polls share the current manual request and old file selections are not reused", async () => {
  const h = await harness();
  try {
    const first = await h.startContent("first.log");
    await h.poll();
    assert.equal(h.pending.filter((request) => request.path.includes("/logs/latest?")).length, 1);
    await h.respond(h.pending.length - 1, { files: ["first.log", "second.log"], total: 2 });
    await act(async () => { h.current.setSelectedLogFile("second.log"); });
    const latest = await h.startContent("first.log");
    assert.equal(h.pending.filter((request) => request.path.includes("/logs/latest?")).length, 2);
    await h.respond(latest, { text: "latest first.log" });
    await h.respond(first, { text: "old first.log" });
    assert.equal(h.current.logText, "latest first.log");
  } finally { await h.dispose(); }
});

test("an obsolete completion cannot clear the current loading indicator", async () => {
  const h = await harness();
  try {
    const first = await h.startContent("first.log");
    const second = await h.startContent("second.log");
    await h.respond(first, { text: "obsolete" });
    assert.equal(h.current.logLoading, true);
    assert.equal(h.current.logText, "");
    await h.respond(second, { text: "current" });
    assert.equal(h.current.logLoading, false);
    assert.equal(h.current.logText, "current");
  } finally { await h.dispose(); }
});

test("changing endpoint or losing authentication clears previously loaded log data", async () => {
  const h = await harness();
  try {
    await h.respond(await h.startContent(), { text: "previous endpoint data" });
    await h.update({ apiBase: "http://localhost:9876" });
    assert.equal(h.current.logText, "");
    assert.deepEqual(h.current.logFiles, []);
    await h.respond(await h.startContent(), { text: "authenticated data" });
    await h.update({ uiAuthReady: false });
    assert.equal(h.current.logText, "");
    assert.deepEqual(h.current.logFiles, []);
  } finally { await h.dispose(); }
});
