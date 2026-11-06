import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { act } from "react";
import ts from "typescript";
import { backend, begin, data, deferred, json, mount, settings } from "./memory-runtime.fixture";

test("memory private data is absent on the new owner's first render before cleanup effects", async () => {
  const host = await mount(backend(() => "A"));
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    const count = host.renders.length;
    await host.scope("B");
    const fresh = host.renders.slice(count).filter(row => row.scope === "B");
    assert.ok(fresh.length > 0);
    assert.ok(fresh.every(row => row.settings === null && row.page === null));
  } finally { await host.close(); }
});

test("memory a same-scope read crossing a save cannot replace its authoritative acknowledgement", async () => {
  const write = deferred<Response>();
  const read = deferred<Response>();
  const host = await mount(async (path, init) => init?.method === "POST" ? write.promise
    : path.includes("scope=principal") ? read.promise : json(data(path, "A")));
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    const saving = await begin(() => host.runtime.updateMemorySetting("use", false));
    const viewing = await begin(() => host.runtime.selectMemorySettingScope("principal"));
    await act(async () => { write.resolve(json({ ...settings("A", 2), use_memory: false })); await saving.pending; });
    assert.equal(host.runtime.memorySettings?.revision, 2);
    await act(async () => { read.resolve(json(settings("A"))); await viewing.pending; });
    assert.equal(host.runtime.memorySettings?.revision, 2);
    assert.equal(host.runtime.memorySettings?.use_memory, false);
  } finally { await host.close(); }
});

test("memory a vector refresh preserves the selected conversation settings view", async () => {
  const host = await mount(backend(() => "A"));
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { await host.runtime.selectMemorySettingScope("conversation", "selected-conversation"); });
    await act(async () => { await host.runtime.controlMemoryVector("pause"); });
    assert.equal(host.runtime.memorySettingScope, "conversation");
    assert.equal(host.runtime.memorySettings?.conversation_id, "selected-conversation");
  } finally { await host.close(); }
});

test("memory old clear-scope setters cannot change a new owner's destructive-action intent", async () => {
  const host = await mount(backend(() => "A"));
  try {
    const old = host.runtime.setMemoryClearScope;
    await host.scope("B");
    await act(async () => { old("all"); });
    assert.equal(host.runtime.memoryClearScope, "recent");
  } finally { await host.close(); }
});

test("memory App remounts local drafts and reloads data on the authenticated owner scope", async () => {
  const source = ts.createSourceFile("App.tsx", await readFile(new URL("../App.tsx", import.meta.url), "utf8"),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const keys: string[] = [];
  const dependencies: string[] = [];
  function visit(node: ts.Node) {
    if (ts.isJsxSelfClosingElement(node) && node.tagName.getText(source) === "MemoryPage") {
      for (const property of node.attributes.properties) {
        if (ts.isJsxAttribute(property) && property.name.getText(source) === "key") {
          assert.ok(property.initializer && ts.isJsxExpression(property.initializer));
          keys.push(property.initializer.expression!.getText(source));
        }
      }
    }
    if (ts.isCallExpression(node) && node.expression.getText(source) === "useEffect"
      && node.arguments[0]?.getText(source).includes("void fetchMemoryData()")) {
      const values = node.arguments[1];
      assert.ok(values && ts.isArrayLiteralExpression(values));
      dependencies.push(...values.elements.map(value => value.getText(source)));
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.deepEqual(keys, ["activeUiRequestScope"]);
  assert.ok(dependencies.includes("activeUiRequestScope"));
});
