import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider } from "../components/UiDialogProvider";
import type { SkillsConfigResponse } from "../types/api";
import { useSkillsRuntime } from "./useSkillsRuntime";

const ALPHA = "switch_owner_alpha";
const BETA = "switch_owner_beta";
const t = (_zh: string, en: string) => en;
type FixtureConfig = SkillsConfigResponse & { revision: string };

function configuration(scope: string): FixtureConfig {
  return {
    config_path: `fixture-${scope}.toml`, skills_list: [ALPHA, BETA],
    skill_switches: { [ALPHA]: true, [BETA]: true }, managed_skills: [ALPHA, BETA],
    locked_skill_names: [], effective_enabled_skills_preview: [ALPHA, BETA],
    runtime_enabled_skills: [ALPHA, BETA], restart_required: false,
    revision: (scope === "B" ? "b" : "a").repeat(64),
  };
}

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(done => { resolve = done; });
  return { promise, resolve };
}

async function harness() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let current!: ReturnType<typeof useSkillsRuntime>;
  let renderer!: ReactTestRenderer;
  let sequence = 0;
  const servers = new Map([...["A", "B", ""].map(scope => [scope, configuration(scope)] as const)]);
  const reads = new Map<string, ReturnType<typeof deferred>>();
  const writes = new Map<string, ReturnType<typeof deferred>>();
  const posts: Array<{ scope: string; data: Record<string, unknown> }> = [];
  const calls: Array<{ scope: string; path: string; method: string }> = [];
  const apis = new Map<string, (path: string, init?: RequestInit) => Promise<Response>>();
  const response = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));
  function patchServer(scope: string, changes: Record<string, boolean>) {
    const previous = servers.get(scope)!;
    servers.set(scope, { ...previous, skill_switches: { ...previous.skill_switches, ...changes }, revision: (++sequence).toString(16).padStart(64, "0") });
  }
  function apiFor(scope: string) {
    if (!apis.has(scope)) apis.set(scope, async (path, init) => {
      const method = init?.method ?? "GET";
      calls.push({ scope, path, method });
      if (path === "/v1/skills") return response({ skills: [], skill_items: [] });
      assert.equal(path, "/v1/skills/config");
      let held: ReturnType<typeof deferred> | undefined;
      if (method === "POST") {
        const data = JSON.parse(String(init?.body)) as Record<string, unknown>;
        posts.push({ scope, data });
        patchServer(scope, data.skill_switches as Record<string, boolean>);
        held = writes.get(scope); writes.delete(scope);
      } else {
        held = reads.get(scope); reads.delete(scope);
      }
      const snapshot = JSON.parse(JSON.stringify(servers.get(scope))) as FixtureConfig;
      if (held) await held.promise;
      return response(snapshot);
    });
    return apis.get(scope)!;
  }
  function Probe({ scope }: { scope: string }) {
    const params = { apiFetch: apiFor(scope), t, requestScope: scope };
    current = useSkillsRuntime(params);
    return null;
  }
  const tree = (scope: string) => React.createElement(UiDialogProvider, null, React.createElement(Probe, { scope }));
  await act(async () => { renderer = create(tree("A")); });
  await act(async () => { await current.fetchSkillsConfig(); });
  return {
    get current() { return current; }, posts, calls, patchServer,
    holdRead(scope = "A") { const gate = deferred(); reads.set(scope, gate); return gate; },
    holdWrite(scope = "A") { const gate = deferred(); writes.set(scope, gate); return gate; },
    async startRead() { let pending!: Promise<void>; await act(async () => { pending = current.fetchSkillsConfig(); }); return { pending }; },
    async startSave() { let pending!: Promise<void>; await act(async () => { pending = current.saveSkillSwitches(); }); return { pending }; },
    async release(gate: ReturnType<typeof deferred>, pending: Promise<void>) { await act(async () => { gate.resolve(); await pending; }); },
    async refresh() { await act(async () => { await current.fetchSkillsConfig(); }); },
    async save() { await act(async () => { await current.saveSkillSwitches(); }); },
    async toggle(name: string, enabled: boolean) { await act(async () => { current.toggleSkillEnabled(name, enabled); }); },
    async scope(scope: string) { await act(async () => { renderer.update(tree(scope)); }); },
    async dispose() { await act(async () => { renderer.unmount(); }); },
  };
}

test("skill switch refresh rebases unrelated server changes without losing pending intent", async () => {
  const h = await harness();
  try {
    await h.toggle(ALPHA, false);
    h.patchServer("A", { [BETA]: false });
    await h.refresh();
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), false);
    assert.equal(h.current.configuredEnabledSkills.has(BETA), false);
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, true);
    await h.save();
    assert.deepEqual(h.posts[0].data.skill_switches, { [ALPHA]: false });
  } finally { await h.dispose(); }
});

test("skill switch edits made while saving survive its acknowledgement and refresh", async () => {
  const h = await harness();
  const gate = h.holdWrite();
  let save: { pending: Promise<void> } | undefined;
  try {
    await h.toggle(ALPHA, false);
    save = await h.startSave();
    await h.toggle(ALPHA, true);
    await h.release(gate, save.pending);
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), true);
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, true);
    await h.save();
    assert.deepEqual(h.posts[1].data.skill_switches, { [ALPHA]: true });
  } finally { gate.resolve(); if (save) await save.pending; await h.dispose(); }
});

test("skill switch reads begun before a save cannot undo its applied result", async () => {
  const h = await harness(); const gate = h.holdRead(); const read = await h.startRead();
  try {
    await h.toggle(ALPHA, false); await h.save(); await h.release(gate, read.pending);
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), false);
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, false);
  } finally { gate.resolve(); await read.pending; await h.dispose(); }
});

test("skill switch newest refresh wins when older reads finish last", async () => {
  const h = await harness(); const gate = h.holdRead(); const read = await h.startRead();
  try {
    h.patchServer("A", { [ALPHA]: false }); await h.refresh(); await h.release(gate, read.pending);
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), false);
    assert.equal(h.current.skillsConfigLoading, false);
  } finally { gate.resolve(); await read.pending; await h.dispose(); }
});

test("skill switch scope A to B to A cannot revive the first A response", async () => {
  const h = await harness(); const gate = h.holdRead(); const read = await h.startRead();
  try {
    await h.scope("B"); await h.refresh();
    h.patchServer("A", { [ALPHA]: false }); await h.scope("A"); await h.refresh();
    await h.release(gate, read.pending);
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), false);
    assert.equal(h.current.skillsConfigData?.config_path, "fixture-A.toml");
  } finally { gate.resolve(); await read.pending; await h.dispose(); }
});

test("a previous device's save cannot refresh or replace the current device's switches", async () => {
  const h = await harness(); const gate = h.holdWrite();
  await h.toggle(ALPHA, false); const save = await h.startSave();
  try {
    await h.scope("B"); await h.refresh();
    const previousDeviceReads = h.calls.filter(call => call.scope === "A" && call.method === "GET").length;
    await h.release(gate, save.pending);
    assert.equal(h.current.skillsConfigData?.config_path, "fixture-B.toml");
    assert.equal(h.calls.filter(call => call.scope === "A" && call.method === "GET").length, previousDeviceReads);
  } finally { gate.resolve(); await save.pending; await h.dispose(); }
});

test("a previous device's pending save does not occupy the current device's save slot", async () => {
  const h = await harness(); const gateA = h.holdWrite("A"); const gateB = h.holdWrite("B");
  await h.toggle(ALPHA, false); const saveA = await h.startSave();
  let saveB: { pending: Promise<void> } | undefined;
  try {
    await h.scope("B"); await h.refresh(); await h.toggle(BETA, false); saveB = await h.startSave();
    assert.equal(h.posts.filter(post => post.scope === "B").length, 1);
    await h.release(gateA, saveA.pending);
    assert.equal(h.current.skillSwitchSaving, true);
    await h.release(gateB, saveB.pending);
    assert.equal(h.current.skillsConfigData?.config_path, "fixture-B.toml");
  } finally { gateA.resolve(); gateB.resolve(); await saveA.pending; if (saveB) await saveB.pending; await h.dispose(); }
});

test("unmounted skill switch callbacks issue no new requests", async () => {
  const h = await harness(); await h.toggle(ALPHA, false);
  const save = h.current.saveSkillSwitches; const read = h.current.fetchSkillsConfig;
  await h.dispose(); const count = h.calls.length;
  await act(async () => { await read(); await save(); });
  assert.equal(h.calls.length, count);
});

test("an empty authentication scope cannot read or save previous skill settings", async () => {
  const h = await harness();
  try {
    await h.scope(""); const count = h.calls.length;
    await h.refresh(); await h.toggle(ALPHA, false); await h.save();
    assert.equal(h.calls.length, count);
    assert.equal(h.current.skillsConfigData, null);
  } finally { await h.dispose(); }
});

test("skill switch saves carry the revision actually observed by the page", async () => {
  const h = await harness();
  try {
    await h.toggle(ALPHA, false); await h.save();
    assert.equal(h.posts[0].data.expected_revision, "a".repeat(64));
  } finally { await h.dispose(); }
});
