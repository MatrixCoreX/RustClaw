import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider } from "../components/UiDialogProvider";
import { useSkillsRuntime } from "./useSkillsRuntime";
import type { SkillsConfigResponse } from "../types/api";

const ALPHA = "switch_ui_alpha";
const BETA = "switch_ui_beta";
const LOCKED = "switch_ui_locked";
const t = (_zh: string, en: string) => en;

function configuration(switches: Record<string, boolean> = {}): SkillsConfigResponse {
  return {
    config_path: "configs/config.toml",
    skills_list: [ALPHA, BETA, LOCKED],
    skill_switches: { [ALPHA]: true, [BETA]: true, [LOCKED]: true, ...switches },
    managed_skills: [ALPHA, BETA, LOCKED],
    locked_skill_names: [LOCKED],
    effective_enabled_skills_preview: [ALPHA, BETA, LOCKED],
    runtime_enabled_skills: [ALPHA, BETA, LOCKED],
    restart_required: false,
  };
}

async function harness(initial = configuration()) {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let current!: ReturnType<typeof useSkillsRuntime>;
  let server = initial;
  let failSave = false;
  const posts: Array<Record<string, unknown>> = [];
  const apiFetch = async (path: string, init?: RequestInit) => {
    if (path === "/v1/skills/config") {
      if (init?.method === "POST") {
        const request = JSON.parse(String(init.body)) as Record<string, unknown>;
        posts.push(request);
        if (failSave) {
          return new Response(JSON.stringify({ ok: false, data: null, error: "skill_admission_generation_changed" }), { status: 409 });
        }
        server = { ...server, skill_switches: { ...server.skill_switches, ...(request.skill_switches as Record<string, boolean>) } };
        return new Response(JSON.stringify({ ok: true, data: server }));
      }
      return new Response(JSON.stringify({ ok: true, data: server }));
    }
    if (path === "/v1/skills") return new Response(JSON.stringify({ ok: true, data: { skills: [], skill_items: [] } }));
    throw new Error(`unexpected fixture request: ${path}`);
  };
  function Probe() { current = useSkillsRuntime({ apiFetch, t }); return null; }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(UiDialogProvider, null, React.createElement(Probe))); });
  await act(async () => { await current.fetchSkillsConfig(); });
  return {
    get current() { return current; }, posts,
    failSave() { failSave = true; },
    async toggle(name: string, enabled: boolean) { await act(async () => { current.toggleSkillEnabled(name, enabled); }); },
    async save() { await act(async () => { await current.saveSkillSwitches(); }); },
    async dispose() { await act(async () => { renderer.unmount(); }); },
  };
}

test("locked display normalization is not an unsaved user change", async () => {
  const h = await harness(configuration({ [LOCKED]: false }));
  try {
    assert.equal(h.current.configuredEnabledSkills.has(LOCKED), true);
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, false);
  } finally { await h.dispose(); }
});

test("saving without an intentional edit sends no mutation request", async () => {
  const h = await harness();
  try {
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, false);
    await h.save();
    assert.equal(h.posts.length, 0);
    assert.equal(h.current.skillSwitchSaving, false);
  } finally { await h.dispose(); }
});

test("one changed switch does not resend unrelated or locked values", async () => {
  const h = await harness();
  try {
    await h.toggle(ALPHA, false);
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, true);
    await h.save();
    assert.equal(h.posts.length, 1);
    assert.deepEqual(h.posts[0].skill_switches, { [ALPHA]: false });
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), false);
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, false);
  } finally { await h.dispose(); }
});

test("reenabling a persisted override sends explicit true even when the draft removes its key", async () => {
  const h = await harness(configuration({ [ALPHA]: false }));
  try {
    await h.toggle(ALPHA, true);
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), true);
    await h.save();
    assert.deepEqual(h.posts[0].skill_switches, { [ALPHA]: true });
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), true);
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, false);
  } finally { await h.dispose(); }
});

test("successful runtime updates do not claim startup TOML was rewritten", async () => {
  const h = await harness();
  try {
    await h.toggle(ALPHA, false);
    await h.save();
    assert.ok(h.current.skillSwitchSaveMessage);
    assert.equal(h.current.skillSwitchSaveMessage.includes("config.toml"), false);
  } finally { await h.dispose(); }
});

test("a failed update keeps the unsaved intent and never reports success or retries", async () => {
  const h = await harness();
  try {
    await h.toggle(ALPHA, false);
    h.failSave();
    await h.save();
    assert.equal(h.posts.length, 1);
    assert.equal(h.current.configuredEnabledSkills.has(ALPHA), false);
    assert.equal(h.current.hasUnsavedSkillSwitchChanges, true);
    assert.equal(h.current.skillSwitchSaveMessage, null);
    assert.ok(h.current.skillsConfigError);
    assert.equal(h.current.skillSwitchSaving, false);
  } finally { await h.dispose(); }
});
