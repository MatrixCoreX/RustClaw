import test from "node:test";
import assert from "node:assert/strict";

import {
  memoryFactStatusLabel,
  memorySafetyLabel,
  shouldHideMemoryRecentContent,
} from "./memory-display.ts";

test("formats memory fact statuses", () => {
  assert.equal(memoryFactStatusLabel("active", "en"), "Active");
  assert.equal(memoryFactStatusLabel("expired", "zh"), "已过期");
  assert.equal(memoryFactStatusLabel("custom", "en"), "custom");
  assert.equal(memoryFactStatusLabel("", "en"), "--");
});

test("formats memory safety labels and hiding behavior", () => {
  assert.equal(memorySafetyLabel("safe", "en"), "Normal");
  assert.equal(memorySafetyLabel("normal", "zh"), "普通");
  assert.equal(memorySafetyLabel("sensitive", "en"), "Flagged");
  assert.equal(shouldHideMemoryRecentContent("safe"), false);
  assert.equal(shouldHideMemoryRecentContent("sensitive"), true);
});
