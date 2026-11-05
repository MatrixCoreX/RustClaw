import assert from "node:assert/strict";
import test from "node:test";
import type { SkillsConfigResponse } from "../types/api";
import { SkillSwitchDraft } from "./skill-switch-draft";

const ALPHA = "draft_alpha";
const BETA = "draft_beta";
const revision = "a".repeat(64);
function baseline(switches: Record<string, boolean> = {}): SkillsConfigResponse {
  return { revision, config_path: "fixture.toml", skills_list: [ALPHA, BETA],
    skill_switches: { [ALPHA]: true, [BETA]: true, ...switches }, managed_skills: [ALPHA, BETA],
    effective_enabled_skills_preview: [], runtime_enabled_skills: [], restart_required: false };
}
function model() { const draft = new SkillSwitchDraft(); draft.rebase(baseline()); return draft; }

test("switch draft rebases pending intent while adopting unrelated server changes", () => {
  const draft = model(); draft.set(ALPHA, false); draft.rebase(baseline({ [BETA]: false }));
  assert.deepEqual(draft.changes(), { [ALPHA]: false });
  assert.equal(draft.values()[BETA], false);
});

test("switch draft preserves a newer reversal of an in-flight change", () => {
  const draft = model(); draft.set(ALPHA, false); const changes = draft.changes(); const sequence = draft.beginWrite();
  draft.set(ALPHA, true);
  draft.acknowledge({ revision, restart_required: false, skill_switches: { [ALPHA]: false, [BETA]: true } }, sequence, changes);
  assert.deepEqual(draft.changes(), { [ALPHA]: true });
});

test("switch draft does not discard unsent intent after an ambiguous earlier write", () => {
  const draft = model(); draft.set(ALPHA, false); draft.beginWrite(); draft.set(ALPHA, true); draft.finishWrite();
  draft.set(BETA, false); const changes = draft.changes(); const sequence = draft.beginWrite();
  assert.deepEqual(changes, { [BETA]: false });
  draft.acknowledge({ revision, restart_required: false, skill_switches: { [ALPHA]: false, [BETA]: false } }, sequence, changes);
  assert.deepEqual(draft.changes(), { [ALPHA]: true });
});

test("switch draft drops satisfied intent only after the actual state is observed", () => {
  const draft = model(); draft.set(ALPHA, false); draft.rebase(baseline({ [ALPHA]: false }));
  assert.deepEqual(draft.changes(), {});
  draft.set(ALPHA, true); draft.set(ALPHA, false);
  assert.deepEqual(draft.changes(), {});
});

test("switch draft follows new locks without preserving prohibited edits", () => {
  const draft = model(); draft.set(ALPHA, false);
  draft.rebase({ ...baseline({ [ALPHA]: false }), locked_skill_names: [ALPHA] });
  assert.deepEqual(draft.changes(), {}); assert.equal(draft.values()[ALPHA], true);
  draft.set(ALPHA, false); assert.equal(draft.values()[ALPHA], true);
});

test("switch draft never edits an unloaded view and preserves prototype-shaped keys safely", () => {
  const draft = new SkillSwitchDraft(); draft.set(ALPHA, false); assert.deepEqual(draft.values(), {});
  draft.rebase(baseline(Object.fromEntries([["__proto__", false]])));
  draft.set("__proto__", true);
  assert.deepEqual(Object.entries(draft.changes()), [["__proto__", true]]);
  assert.equal(Object.getPrototypeOf(draft.values()), Object.prototype);
});
