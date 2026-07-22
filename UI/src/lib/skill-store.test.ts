import test from "node:test";
import assert from "node:assert/strict";

import { filterSkillStoreItems, skillStoreInstallState } from "./skill-store.ts";
import type { SkillStoreItem } from "../types/api.ts";

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

  assert.equal(skillStoreInstallState(installedButDisabled), "installed");
  assert.equal(skillStoreInstallState(removed), "not_installed");
});
