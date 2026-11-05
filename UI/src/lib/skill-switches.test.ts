import assert from "node:assert/strict";
import test from "node:test";
import { confirmSkillSwitchUpdate, hasSkillSwitchBaseline, skillSwitchChanges, skillSwitchErrorMessage, skillSwitchIsChanged } from "./skill-switches";

const locked = new Set(["fixture_locked"]);
const baseline = {
  skills_list: ["fixture_enabled", "fixture_locked"],
  skill_switches: { fixture_enabled: true, fixture_disabled: false, fixture_locked: false },
};

test("skill switch comparison ignores locked display normalization and unchanged explicit defaults", () => {
  const draft = { ...baseline.skill_switches, fixture_locked: true };
  assert.deepEqual(skillSwitchChanges(baseline, draft, locked), {});
  assert.equal(skillSwitchIsChanged(baseline, draft, "fixture_locked", locked), false);
  assert.equal(skillSwitchIsChanged(baseline, draft, "fixture_enabled", locked), false);
});

test("skill switch patches contain only intentional changes and row markers agree", () => {
  const draft = { ...baseline.skill_switches, fixture_enabled: false, fixture_locked: true };
  assert.deepEqual(skillSwitchChanges(baseline, draft, locked), { fixture_enabled: false });
  assert.equal(skillSwitchIsChanged(baseline, draft, "fixture_enabled", locked), true);
  assert.equal(skillSwitchIsChanged(baseline, draft, "fixture_disabled", locked), false);
});

test("removing a persisted false override explicitly restores an enabled startup default", () => {
  const config = { skills_list: ["fixture"], skill_switches: { fixture: false } };
  assert.deepEqual(skillSwitchChanges(config, {}, locked), { fixture: true });
});

test("removing a persisted true override explicitly restores a disabled startup default", () => {
  const config = { skills_list: [], skill_switches: { fixture: true } };
  assert.deepEqual(skillSwitchChanges(config, {}, locked), { fixture: false });
});

test("toggling back to the startup default is not a change without a persisted override", () => {
  const config = { skills_list: ["fixture"], skill_switches: {} };
  assert.deepEqual(skillSwitchChanges(config, {}, locked), {});
  assert.deepEqual(skillSwitchChanges(config, { fixture: true }, locked), {});
  assert.deepEqual(skillSwitchChanges(config, { fixture: false }, locked), { fixture: false });
  assert.deepEqual(skillSwitchChanges(null, { fixture: true }, locked), {});
});

test("hidden switches are never sent by the visible switch editor", () => {
  const config = { skills_list: [], skill_switches: { chat: false } };
  assert.deepEqual(skillSwitchChanges(config, { chat: true }, locked), {});
});

test("skill switch dictionaries use own properties and preserve prototype-shaped names", () => {
  const config = { skills_list: [], skill_switches: Object.fromEntries([["__proto__", false], ["constructor", false]]) };
  const draft = Object.fromEntries([["__proto__", true], ["constructor", true]]);
  const patch = skillSwitchChanges(config, draft, locked);
  assert.deepEqual(Object.entries(patch), [["__proto__", true], ["constructor", true]]);
  assert.equal(Object.getPrototypeOf(patch), Object.prototype);
  const inherited = Object.create({ fixture_inherited: true }) as Record<string, boolean>;
  assert.deepEqual(skillSwitchChanges({ skills_list: [], skill_switches: {} }, inherited, locked), {});
});

const readData = {
  ...baseline, managed_skills: [], effective_enabled_skills_preview: [], runtime_enabled_skills: [],
};

test("switch baseline validates lists and boolean maps before accepting a read", () => {
  assert.equal(hasSkillSwitchBaseline(readData), true);
  for (const value of [null, [], {}, { ...readData, skills_list: "fixture" }, { ...readData, managed_skills: {} },
    { ...readData, skills_list: [false] }, { ...readData, skill_switches: [] },
    { ...readData, skill_switches: { fixture: "false" } }, { ...readData, locked_skill_names: {} },
    { ...readData, skill_items: [null] }, { ...readData, skill_items: [{ name: false }] }]) {
    assert.equal(hasSkillSwitchBaseline(value), false, JSON.stringify(value));
  }
  assert.equal(hasSkillSwitchBaseline({ ...readData, locked_skill_names: [], skill_items: [{ name: "fixture" }] }), true);
});

test("switch acknowledgements require explicit restart state and the requested values", () => {
  assert.equal(confirmSkillSwitchUpdate({ restart_required: false, skill_switches: { fixture: true } }, { fixture: true }), false);
  assert.equal(confirmSkillSwitchUpdate({ restart_required: true, skill_switches: { fixture: true } }, { fixture: true }), true);
  for (const value of [null, {}, { skill_switches: {} }, { restart_required: false },
    { restart_required: "false", skill_switches: {} }, { restart_required: false, skill_switches: [] },
    { restart_required: false, skill_switches: { fixture: "true" } }]) {
    assert.throws(() => confirmSkillSwitchUpdate(value, { fixture: true }), /skill_config_update_invalid/);
  }
  for (const switches of [{}, { fixture: false }, Object.create({ fixture: true })]) {
    assert.throws(() => confirmSkillSwitchUpdate({ restart_required: false, skill_switches: switches }, { fixture: true }), /skill_config_update_not_applied/);
  }
});

test("acknowledgements confirm prototype-shaped keys only from own response fields", () => {
  const changes = Object.fromEntries([["__proto__", true]]);
  assert.equal(confirmSkillSwitchUpdate({ restart_required: false, skill_switches: changes }, changes), false);
  assert.throws(() => confirmSkillSwitchUpdate({ restart_required: false, skill_switches: {} }, changes), /not_applied/);
});

test("switch errors provide localized recovery guidance without showing machine tokens", () => {
  const en = (_zh: string, english: string) => english;
  const zh = (chinese: string) => chinese;
  assert.match(skillSwitchErrorMessage(new Error("skill_admission_generation_changed"), en), /another operation/i);
  assert.match(skillSwitchErrorMessage(new Error("skill_admission_generation_changed"), zh), /另一个操作/);
  assert.match(skillSwitchErrorMessage(new Error("skill_config_update_not_applied"), en), /not applied/);
  assert.match(skillSwitchErrorMessage(new Error("skill_config_update_invalid"), en), /could not be saved/);
});
