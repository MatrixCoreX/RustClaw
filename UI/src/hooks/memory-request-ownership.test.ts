import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { act } from "react";
import ts from "typescript";
import { assertOwner, backend, begin, data, deferred, filters, json, mount, page, settings } from "./memory-runtime.fixture";

test("memory active-owner refresh keeps the complete existing read contract", async () => {
  const host = await mount(backend(() => "A"));
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    assertOwner(host.runtime, "A");
    assert.equal(host.runtime.memoryVectorStatus?.state, "ready");
    assert.equal(host.runtime.memoryError, null);
    assert.equal(host.runtime.memoryLoading, false);
  } finally { await host.close(); }
});

test("memory denied confirmation keeps the existing no-write behavior", async () => {
  const writes: string[] = [];
  const host = await mount(async (path, init) => {
    if (init?.method === "POST") writes.push(path);
    return json(data(path, "A"));
  }, "A", async () => false);
  try {
    await act(async () => { await host.runtime.deleteMemoryItemWithRevision("fixture", 1); });
    assert.deepEqual(writes, []);
  } finally { await host.close(); }
});

test("memory clears all cached private state and filters when the request owner changes", async () => {
  const host = await mount(backend(() => "A"));
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); await host.runtime.fetchMemoryPage(filters("private")); });
    await host.scope("B");
    assert.equal(host.runtime.memoryOverview, null);
    assert.equal(host.runtime.memorySettings, null);
    assert.equal(host.runtime.memoryPage, null);
    assert.equal(host.runtime.memoryRemoteDisclosure, null);
    assert.equal(host.runtime.memoryVectorStatus, null);
    assert.deepEqual(host.runtime.memoryPreferences, []);
    assert.deepEqual(host.runtime.memoryFacts, []);
    assert.deepEqual(host.runtime.memoryRecent, []);
    assert.deepEqual(host.runtime.memoryFilters, filters());
  } finally { await host.close(); }
});

for (const returnToA of [false, true]) {
  test(`memory rejects an old complete refresh after ${returnToA ? "A-B-A" : "A-B"} ownership change`, async () => {
    const old = deferred<Response>();
    let owner = "A";
    let first = true;
    const host = await mount(async (path) => {
      if (path === "/v1/memory" && first) { first = false; return old.promise; }
      return json(data(path, owner));
    });
    try {
      const task = await begin(() => host.runtime.fetchMemoryData());
      owner = "B"; await host.scope("B");
      await act(async () => { await host.runtime.fetchMemoryData(); });
      if (returnToA) {
        owner = "fresh-A"; await host.scope("A");
        await act(async () => { await host.runtime.fetchMemoryData(); });
      }
      await act(async () => { old.resolve(json(data("/v1/memory", "A"))); await task.pending; });
      assertOwner(host.runtime, owner);
    } finally { await host.close(); }
  });
}

test("memory late JSON parsing cannot replace the new owner's state", async () => {
  const body = deferred<unknown>();
  let owner = "A";
  let first = true;
  const host = await mount(async (path) => {
    if (path === "/v1/memory/settings" && first) {
      first = false; return { ok: true, json: () => body.promise } as Response;
    }
    return json(data(path, owner));
  });
  try {
    const task = await begin(() => host.runtime.fetchMemoryData());
    owner = "B"; await host.scope("B");
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { body.resolve({ ok: true, data: settings("A") }); await task.pending; });
    assertOwner(host.runtime, "B");
    assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});

test("memory logout and retained callbacks cannot start new HTTP work", async () => {
  let calls = 0;
  const host = await mount(async (path) => { calls++; return json(data(path, "A")); });
  const old = host.runtime;
  try {
    await host.scope("");
    await act(async () => { await host.runtime.fetchMemoryData(); await old.fetchMemoryPage(filters()); });
    assert.equal(calls, 0);
  } finally { await host.close(); }
});

test("memory unmounted retained callbacks cannot start reads or mutations", async () => {
  let calls = 0;
  const host = await mount(async (path) => { calls++; return json(data(path, "A")); });
  const old = host.runtime;
  await host.close();
  await old.fetchMemoryData();
  await old.correctMemoryItem("fixture", 1, "replacement");
  assert.equal(calls, 0);
});

test("memory previous-owner read rejection cannot expose an error in the new owner's page", async () => {
  const failure = deferred<Response>();
  let owner = "A";
  const host = await mount(async (path) => path.includes("search=old") ? failure.promise : json(data(path, owner)));
  try {
    const old = await begin(() => host.runtime.fetchMemoryPage(filters("old")));
    owner = "B"; await host.scope("B");
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { failure.reject(new Error("old private fixture error")); await old.pending; });
    assertOwner(host.runtime, "B");
    assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});

test("memory pagination keeps the latest same-owner result and search intent", async () => {
  const first = deferred<Response>();
  const host = await mount(async (path) => path.includes("search=old") ? first.promise : json(page("fresh", 2)));
  try {
    const old = await begin(() => host.runtime.fetchMemoryPage(filters("old")));
    await act(async () => { await host.runtime.fetchMemoryPage(filters("fresh"), 2); });
    await act(async () => { first.resolve(json(page("old"))); await old.pending; });
    assert.equal(host.runtime.memoryPage?.items[0]?.content, "fresh");
    assert.equal(host.runtime.memoryPage?.page, 2);
    assert.equal(host.runtime.memoryFilters.search, "fresh");
  } finally { await host.close(); }
});

test("memory old read failure cannot unlock a newer pending page", async () => {
  const old = deferred<Response>();
  const fresh = deferred<Response>();
  let calls = 0;
  const host = await mount(async () => ++calls === 1 ? old.promise : fresh.promise);
  try {
    const a = await begin(() => host.runtime.fetchMemoryPage(filters("old")));
    const b = await begin(() => host.runtime.fetchMemoryPage(filters("fresh")));
    await act(async () => { old.reject(new Error("old fixture failure")); await a.pending; });
    assert.equal(host.runtime.memoryLoading, true);
    assert.equal(host.runtime.memoryError, null);
    await act(async () => { fresh.resolve(json(page("fresh"))); await b.pending; });
    assert.equal(host.runtime.memoryLoading, false);
  } finally { await host.close(); }
});

test("memory old overview bundle cannot overwrite a newer filtered page", async () => {
  const old = deferred<Response>();
  const host = await mount(async (path) => path === "/v1/memory" ? old.promise : json(data(path, "A")));
  try {
    const refresh = await begin(() => host.runtime.fetchMemoryData());
    await act(async () => { await host.runtime.fetchMemoryPage(filters("fresh"), 2); });
    await act(async () => { old.resolve(json(data("/v1/memory", "A"))); await refresh.pending; });
    assert.equal(host.runtime.memoryPage?.page, 2);
    assert.equal(host.runtime.memoryFilters.search, "fresh");
  } finally { await host.close(); }
});

test("memory late settings scope read cannot restore an earlier selection", async () => {
  const old = deferred<Response>();
  const host = await mount(async (path) => path.includes("scope=principal")
    ? old.promise : json(settings("A", 2, "conversation-fresh")));
  try {
    const task = await begin(() => host.runtime.selectMemorySettingScope("principal"));
    await act(async () => { await host.runtime.selectMemorySettingScope("conversation", "conversation-fresh"); });
    await act(async () => { old.resolve(json(settings("A"))); await task.pending; });
    assert.equal(host.runtime.memorySettingScope, "conversation");
    assert.equal(host.runtime.memorySettings?.conversation_id, "conversation-fresh");
  } finally { await host.close(); }
});

test("memory settings save invalidates an already-running older refresh", async () => {
  const old = deferred<Response>();
  let reads = 0;
  const host = await mount(async (path, init) => {
    if (init?.method === "POST") return json({ ...settings("A", 2), use_memory: false });
    if (path === "/v1/memory" && ++reads === 2) return old.promise;
    return json(data(path, "A"));
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    const refresh = await begin(() => host.runtime.fetchMemoryData());
    await act(async () => { await host.runtime.updateMemorySetting("use", false); });
    await act(async () => { old.resolve(json(data("/v1/memory", "A"))); await refresh.pending; });
    assert.equal(host.runtime.memorySettings?.revision, 2);
    assert.equal(host.runtime.memorySettings?.use_memory, false);
    assert.equal(host.runtime.memoryOverview?.long_term_enabled, false);
  } finally { await host.close(); }
});

for (const action of ["setting", "external", "vector"] as const) {
  test(`memory ${action} completion from a previous owner cannot refresh or overwrite the new owner`, async () => {
    const pending = deferred<Response>();
    let owner = "A";
    let calls = 0;
    const host = await mount(async (path, init) => {
      calls++;
      return init?.method === "POST" ? pending.promise : json(data(path, owner));
    });
    try {
      await act(async () => { await host.runtime.fetchMemoryData(); });
      const task = await begin(() => action === "setting" ? host.runtime.updateMemorySetting("generate", false)
        : action === "external" ? host.runtime.updateMemoryExternalPolicy("allow") : host.runtime.controlMemoryVector("pause"));
      owner = "B"; await host.scope("B");
      await act(async () => { await host.runtime.fetchMemoryData(); });
      const before = calls;
      await act(async () => {
        pending.resolve(json(action === "vector" ? data("/v1/memory/vector/pause", "A") : settings("A", 2)));
        await task.pending;
      });
      assertOwner(host.runtime, "B");
      assert.equal(calls, before);
      assert.equal(host.runtime.memoryMessage, null);
      assert.equal(host.runtime.memoryError, null);
    } finally { await host.close(); }
  });
}

test("memory previous-owner correction cannot unlock or refresh a newer pending mutation", async () => {
  const first = deferred<Response>();
  const second = deferred<Response>();
  let writes = 0;
  let reads = 0;
  const host = await mount(async (path, init) => {
    if (init?.method === "POST") return ++writes === 1 ? first.promise : second.promise;
    reads++; return json(data(path, "B"));
  });
  try {
    const a = await begin(() => host.runtime.correctMemoryItem("old", 1, "old"));
    await host.scope("B");
    const b = await begin(() => host.runtime.correctMemoryItem("fresh", 1, "fresh"));
    await act(async () => { first.resolve(json(data("mutation", "A"))); await a.pending; });
    assert.equal(host.runtime.memoryActionLoading, "correct:fresh");
    assert.equal(host.runtime.memoryUndoRevisionId, null);
    assert.equal(reads, 0);
    await act(async () => { second.resolve(json(data("mutation", "B"))); await b.pending; });
    assert.equal(host.runtime.memoryActionLoading, null);
    assert.equal(reads, 1);
  } finally { await host.close(); }
});

for (const kind of ["delete", "clear", "import"] as const) {
  test(`memory ${kind} confirmation cannot authorize a write after its owner changes`, async () => {
    const approval = deferred<boolean>();
    const writes: string[] = [];
    const host = await mount(async (path, init) => {
      if (init?.method === "POST") writes.push(path);
      return json(data(path, "A"));
    }, "A", () => approval.promise);
    try {
      const task = await begin(() => kind === "delete" ? host.runtime.deleteMemoryItemWithRevision("fixture", 1)
        : kind === "clear" ? host.runtime.clearMemoryScope()
          : host.runtime.importMemory({ text: async () => "{}" } as File));
      const before = [...writes];
      await host.scope("B");
      await act(async () => { approval.resolve(true); await task.pending; });
      assert.deepEqual(writes, before);
      assert.equal(host.runtime.memoryMessage, null);
      assert.equal(host.runtime.memoryUndoRevisionId, null);
    } finally { await host.close(); }
  });
}

test("memory import file parsing cannot send an old file through a changed owner", async () => {
  const content = deferred<string>();
  let calls = 0;
  const host = await mount(async (path) => { calls++; return json(data(path, "A")); });
  try {
    const task = await begin(() => host.runtime.importMemory({ text: () => content.promise } as File));
    await host.scope("B");
    await act(async () => { content.resolve("{}"); await task.pending; });
    assert.equal(calls, 0);
  } finally { await host.close(); }
});

test("memory export approval cannot download a previous owner's data after account change", async () => {
  const approval = deferred<boolean>();
  let requests = 0;
  let downloads = 0;
  const previous = Object.getOwnPropertyDescriptor(globalThis, "document");
  Object.defineProperty(globalThis, "document", { configurable: true, value: {
    documentElement: { lang: "zh" },
    createElement: () => ({ href: "", download: "", click: () => { downloads++; } }),
  } });
  const host = await mount(async () => {
    requests++;
    return json({ schema_version: 1, exported_at_ts: 1, scope_kind: "principal", items: [], checksum: "fixture" });
  }, "A", () => approval.promise);
  try {
    const task = await begin(() => host.runtime.exportMemory());
    await host.scope("B");
    await act(async () => { approval.resolve(true); await task.pending; });
    assert.equal(requests, 0);
    assert.equal(downloads, 0);
  } finally {
    await host.close();
    if (previous) Object.defineProperty(globalThis, "document", previous);
    else Reflect.deleteProperty(globalThis, "document");
  }
});

test("memory app supplies the same authenticated request scope as other private controls", async () => {
  const source = ts.createSourceFile("App.tsx", await readFile(new URL("../App.tsx", import.meta.url), "utf8"),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const scopes: string[] = [];
  function visit(node: ts.Node) {
    if (ts.isCallExpression(node) && node.expression.getText(source) === "useMemoryRuntime") {
      const parameters = node.arguments[0];
      assert.ok(parameters && ts.isObjectLiteralExpression(parameters));
      for (const property of parameters.properties) {
        if (ts.isPropertyAssignment(property) && property.name.getText(source) === "requestScope") {
          scopes.push(property.initializer.getText(source));
        }
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.deepEqual(scopes, ["activeUiRequestScope"]);
});
