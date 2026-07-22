import test from "node:test";
import assert from "node:assert/strict";

import {
  filterSkillStoreItems,
  resolveSkillStoreActionName,
  skillStoreErrorMessage,
  skillStoreInstallState,
} from "./skill-store.ts";
import type { SkillStoreItem, SkillStoreResponse } from "../types/api.ts";

const item = (name: string, installed: boolean, group: string): SkillStoreItem => ({
  name,
  installed,
  enabled: installed,
  group,
  catalog_section: "other",
  kind: "builtin",
  source_kind: "bundled",
  skill: { name },
});

test("filters store items by machine name and registry group", () => {
  const items = [item("weather", true, "information"), item("photo_organize", false, "media")];

  assert.deepEqual(filterSkillStoreItems(items, "PHOTO").map((entry) => entry.name), ["photo_organize"]);
  assert.deepEqual(filterSkillStoreItems(items, "information").map((entry) => entry.name), ["weather"]);
  assert.equal(filterSkillStoreItems(items, "missing").length, 0);
});

test("keeps only items assigned to the tools and skills other group", () => {
  const items = [
    item("weather", true, "information"),
    { ...item("image_generate", true, "image"), catalog_section: "image" },
    { ...item("schedule", true, "workflow"), catalog_section: "base" },
  ];

  assert.deepEqual(filterSkillStoreItems(items, "").map((entry) => entry.name), ["weather"]);
});

test("keeps removed skills distinct from disabled installed skills", () => {
  const installedButDisabled = { ...item("weather", true, "information"), enabled: false };
  const removed = item("photo_organize", false, "media");
  const missingRunner = {
    ...item("invest_copy", false, "finance"),
    configured_installed: true,
    runner_available: false,
    installation_issue: "runner_missing" as const,
  };

  assert.equal(skillStoreInstallState(installedButDisabled), "installed");
  assert.equal(skillStoreInstallState(removed), "not_installed");
  assert.equal(skillStoreInstallState(missingRunner), "repair_required");
});

test("renders structured store errors in the selected UI language", () => {
  const zh = (zhText: string) => zhText;
  const en = (_zhText: string, enText: string) => enText;

  assert.match(skillStoreErrorMessage("skill_store_build_failed", zh), /编译失败/);
  assert.match(skillStoreErrorMessage("skill_store_build_failed", en), /build failed/i);
  assert.match(skillStoreErrorMessage("skill_store_operation_busy", en), /another skill/i);
  assert.match(skillStoreErrorMessage("future_error_code", en), /try again/i);
});

test("restores the active skill action from server catalog state after refresh", () => {
  const store: SkillStoreResponse = {
    items: [],
    uninstalled_skill_names: [],
    active_operation: {
      skill_name: "weather",
      action: "install",
      started_ts: 1_790_000_000,
    },
  };

  assert.equal(resolveSkillStoreActionName(null, store), "weather");
  assert.equal(resolveSkillStoreActionName("stock", store), "stock");
  assert.equal(resolveSkillStoreActionName(null, { ...store, active_operation: null }), null);
});
