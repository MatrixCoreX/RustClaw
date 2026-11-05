import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useModelConfigRuntime } from "./useModelConfigRuntime";
import { MULTIMODAL_SKILL_BY_KEY, type MultimodalKey } from "../lib/model-config";
import type { LlmConfigResponse, SkillsConfigResponse } from "../types/api";

const t = (_zh: string, en: string) => en;
const moduleKey = Object.keys(MULTIMODAL_SKILL_BY_KEY)[0] as MultimodalKey;
const skillName = MULTIMODAL_SKILL_BY_KEY[moduleKey];
function deferred() { let resolve!: () => void; const promise = new Promise<void>(done => { resolve = done; }); return { promise, resolve }; }
function config(scope: string): LlmConfigResponse {
  return { config_path: `fixture-${scope}.toml`, selected_vendor: "custom", selected_model: `model-${scope}`,
    restart_required: false, vendors: [{ name: "custom", default_model: `model-${scope}`, models: [`model-${scope}`],
      base_url: "https://provider.example/v1", api_key_configured: true }] };
}

async function harness() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let current!: ReturnType<typeof useModelConfigRuntime>;
  let renderer!: ReactTestRenderer;
  let refuseSwitch = false;
  const gates = new Map<string, ReturnType<typeof deferred>>();
  const posts: Array<{ scope: string; path: string; data: Record<string, unknown> }> = [];
  const skills: SkillsConfigResponse = {
    revision: "a".repeat(64), config_path: "fixture.toml", skills_list: [],
    skill_switches: { [skillName]: false, unrelated_switch_fixture: true }, managed_skills: [skillName],
    effective_enabled_skills_preview: [], runtime_enabled_skills: [], restart_required: false,
  };
  function apiFor(scope: string) {
    return async (path: string, init?: RequestInit) => {
      const method = init?.method ?? "GET";
      const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : null;
      if (body) posts.push({ scope, path, data: body });
      let data: unknown = path === "/v1/llm/config" ? config(scope)
        : path === "/v1/models/catalog" ? { entries: [] } : skills;
      if (path === "/v1/skills/config" && body) data = {
        ...skills, revision: "b".repeat(64),
        skill_switches: { ...skills.skill_switches, [skillName]: refuseSwitch ? false : (body.skill_switches as Record<string, boolean>)[skillName] },
      };
      const snapshot = JSON.parse(JSON.stringify(data));
      const gate = gates.get(`${scope}:${path}:${method}`);
      gates.delete(`${scope}:${path}:${method}`);
      if (gate) await gate.promise;
      return Response.json({ ok: true, data: snapshot });
    };
  }
  function Probe({ scope }: { scope: string }) { current = useModelConfigRuntime({ apiFetch: apiFor(scope), t, requestScope: scope }); return null; }
  await act(async () => { renderer = create(React.createElement(Probe, { scope: "A" })); });
  await act(async () => { await current.fetchLlmConfig(); });
  return {
    get current() { return current; }, posts,
    hold(path: string, method = "GET", scope = "A") { const gate = deferred(); gates.set(`${scope}:${path}:${method}`, gate); return gate; },
    refuseSwitch() { refuseSwitch = true; },
    async scope(scope: string) { await act(async () => { renderer.update(React.createElement(Probe, { scope })); }); },
    async read() { await act(async () => { await current.fetchLlmConfig(); }); },
    async startRead() { let pending!: Promise<void>; await act(async () => { pending = current.fetchLlmConfig(); }); return { pending }; },
    async startSave() { let pending!: Promise<void>; await act(async () => { pending = current.saveLlmConfig(); }); return { pending }; },
    async startSwitch() { let pending!: Promise<void>; await act(async () => { pending = current.setMultimodalSkillEnabledNow(moduleKey, true); }); return { pending }; },
    async release(gate: ReturnType<typeof deferred>, pending: Promise<void>) { await act(async () => { gate.resolve(); await pending; }); },
    async key(value: string) { await act(async () => { current.setLlmDraftApiKey(value); }); },
    async dispose() { await act(async () => { renderer.unmount(); }); },
  };
}

test("model owner changes clear unsaved credentials and reject late previous-device reads", async () => {
  const h = await harness(); await h.key("fixture-old-key");
  const gate = h.hold("/v1/llm/config"); const read = await h.startRead();
  try {
    await h.scope("B"); assert.equal(h.current.llmDraftApiKey, ""); await h.read();
    await h.release(gate, read.pending);
    assert.equal(h.current.llmConfigData?.selected_model, "model-B");
    assert.equal(h.current.llmDraftApiKey, "");
  } finally { gate.resolve(); await read.pending; await h.dispose(); }
});

test("model save acknowledgements never erase a newer credential draft", async () => {
  const h = await harness(); await h.key("fixture-submitted-key");
  const gate = h.hold("/v1/llm/config", "POST"); const save = await h.startSave();
  try {
    await h.key("fixture-newer-key"); await h.release(gate, save.pending);
    assert.equal(h.current.llmDraftApiKey, "fixture-newer-key");
    assert.equal(h.posts[0].data.vendor_api_key, "fixture-submitted-key");
    assert.equal(h.current.hasUnsavedLlmChanges, true);
  } finally { gate.resolve(); await save.pending; await h.dispose(); }
});

test("multimodal switches submit only their intent with the observed revision", async () => {
  const h = await harness();
  try {
    const save = await h.startSwitch(); await act(async () => { await save.pending; });
    const post = h.posts.find(item => item.path === "/v1/skills/config")!;
    assert.deepEqual(post.data, { skill_switches: { [skillName]: true }, expected_revision: "a".repeat(64) });
    assert.equal(h.current.multimodalSkillEnabled[moduleKey], true);
  } finally { await h.dispose(); }
});

test("multimodal switches do not report success when the server did not apply the change", async () => {
  const h = await harness(); h.refuseSwitch();
  try {
    const save = await h.startSwitch(); await act(async () => { await save.pending; });
    assert.notEqual(h.current.multimodalSkillEnabled[moduleKey], true);
    assert.equal(h.current.multimodalSkillSwitchMessage, null);
    assert.match(h.current.multimodalConfigError ?? "", /not applied/);
  } finally { await h.dispose(); }
});

test("an old multimodal switch callback cannot write after a device change", async () => {
  const h = await harness(); const gate = h.hold("/v1/skills/config"); const save = await h.startSwitch();
  try {
    await h.scope("B"); await h.release(gate, save.pending);
    assert.equal(h.posts.filter(item => item.path === "/v1/skills/config").length, 0);
    assert.equal(h.current.multimodalSkillSwitchMessage, null);
  } finally { gate.resolve(); await save.pending; await h.dispose(); }
});

test("unmounted model callbacks cannot send stored draft credentials", async () => {
  const h = await harness(); await h.key("fixture-unsubmitted-key");
  const save = h.current.saveLlmConfig; await h.dispose();
  await act(async () => { await save(); }); assert.equal(h.posts.length, 0);
});

test("a previous model owner cannot change the next owner's draft or expand its controls", async () => {
  const h = await harness(); const old = h.current;
  try {
    await h.scope("B");
    await act(async () => { old.setLlmDraftApiKey("previous-device-key"); old.setModelsAdvancedOpen(true); });
    assert.equal(h.current.llmDraftApiKey, "");
    assert.equal(h.current.modelsAdvancedOpen, false);
  } finally { await h.dispose(); }
});
