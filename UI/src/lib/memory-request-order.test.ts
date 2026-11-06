import assert from "node:assert/strict";
import test from "node:test";
import { memoryRequests } from "../hooks/memory-runtime-state";

test("memory request lanes preserve independent reads and supersede only their own lane", () => {
  const requests = memoryRequests(() => true);
  const old = requests.token("page", ["items"]);
  const settings = requests.token("settings", ["settings:principal"]);
  const fresh = requests.token("page", ["items"]);
  assert.equal(old(), false);
  assert.equal(settings(), true);
  assert.equal(fresh(), true);
});

test("memory writes invalidate reads at both write boundaries and finish only once", () => {
  const requests = memoryRequests(() => true);
  const before = requests.token("page", ["items"]);
  const finish = requests.write("items");
  assert.equal(before(), false);
  const during = requests.token("page", ["items"]);
  assert.equal(during(), true);
  finish();
  assert.equal(during(), false);
  const after = requests.token("page", ["items"]);
  finish();
  assert.equal(after(), true);
});

test("memory item writes do not invalidate another settings scope", () => {
  const requests = memoryRequests(() => true);
  const settings = requests.token("settings", ["settings:conversation"]);
  requests.write("items")();
  assert.equal(settings(), true);
});

test("memory departed settings domains never revive an old read when selected again", () => {
  const requests = memoryRequests(() => true);
  const old = requests.token("old-settings", ["settings:A"]);
  const items = requests.token("page", ["items"]);
  requests.selectSettingsDomain("settings:B");
  assert.equal(old(), false);
  requests.selectSettingsDomain("settings:A");
  requests.token("fresh-settings", ["settings:A"]);
  assert.equal(old(), false);
  assert.equal(items(), true);
});

test("memory request tickets remain governed by the existing owner guard", () => {
  let active = true;
  const requests = memoryRequests(() => active);
  const pending = requests.token("page", ["items"]);
  active = false;
  assert.equal(pending(), false);
  assert.equal(requests.token("page")(), false);
});
