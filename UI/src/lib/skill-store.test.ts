import test from "node:test";
import assert from "node:assert/strict";

import {
  filterSkillStoreItems,
  removableSkillNames,
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
  source_kind: "bundled_optional",
  skill: { name },
});

test("filters store items by machine name, localized description, and registry group", () => {
  const items = [
    { ...item("weather", true, "information"), description_zh: "查询天气预报" },
    item("photo_organize", false, "media"),
  ];

  assert.deepEqual(filterSkillStoreItems(items, "PHOTO").map((entry) => entry.name), ["photo_organize"]);
  assert.deepEqual(filterSkillStoreItems(items, "information").map((entry) => entry.name), ["weather"]);
  assert.deepEqual(filterSkillStoreItems(items, "天气").map((entry) => entry.name), ["weather"]);
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
  const missingPackage = {
    ...item("invest_copy", false, "finance"),
    configured_installed: true,
    package_available: false,
    installation_issue: "package_missing" as const,
  };

  assert.equal(skillStoreInstallState(installedButDisabled), "installed");
  assert.equal(skillStoreInstallState(removed), "not_installed");
  assert.equal(skillStoreInstallState(missingPackage), "repair_required");
});

test("renders structured store errors in the selected UI language", () => {
  const zh = (zhText: string) => zhText;
  const en = (_zhText: string, enText: string) => enText;

  assert.match(skillStoreErrorMessage("skill_store_install_failed", zh), /安装未完成/);
  assert.match(skillStoreErrorMessage("skill_store_install_failed", en), /installation did not finish/i);
  assert.match(skillStoreErrorMessage("skill_store_operation_busy", en), /another skill/i);
  assert.match(skillStoreErrorMessage("skill_store_network_approval_required", en), /network access/i);
  assert.match(skillStoreErrorMessage("skill_store_host_dependency_admin_required", zh), /管理员账号/);
  assert.match(skillStoreErrorMessage("skill_store_host_dependency_unknown", en), /does not recognize/i);
  assert.match(skillStoreErrorMessage("skill_store_host_dependency_install_failed", zh), /系统依赖/);
  assert.match(skillStoreErrorMessage("skill_store_resource_insufficient", en), /free disk space/i);
  assert.match(skillStoreErrorMessage("future_error_code", en), /try again/i);
});

test("restores the active skill action from server catalog state after refresh", () => {
  const store: SkillStoreResponse = {
    items: [],
    uninstalled_skill_names: [],
    active_operation: {
      schema_version: 1,
      operation_id: "8bb19ae2-e0ab-4dab-b0a4-cbdd31112ccc",
      skill_name: "weather",
      action: "install",
      status: "running",
      stage: "build",
      created_at_unix: 1_790_000_000,
      updated_at_unix: 1_790_000_001,
      heartbeat_at_unix: 1_790_000_001,
      cancel_requested: false,
      stages: [{ stage: "queued", recorded_at_unix: 1_790_000_000 }],
    },
  };

  assert.equal(resolveSkillStoreActionName(null, store), "weather");
  assert.equal(resolveSkillStoreActionName("stock", store), "stock");
  assert.equal(resolveSkillStoreActionName(null, { ...store, active_operation: null }), null);
});

test("lets imported skills be removed regardless of their display group", () => {
  const removable = removableSkillNames(
    ["weather"],
    new Set(["image_partner", "audio_partner"]),
    new Set(["audio_partner"]),
  );

  assert.deepEqual(Array.from(removable).sort(), ["image_partner", "weather"]);
});
