import React from "react";
import assert from "node:assert/strict";
import test from "node:test";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { MemoryCapacityPanel } from "../components/MemoryCapacityPanel";
import { json, report, settings, t } from "./memory-capacity.fixture";

test("manual status is separate from background status and refresh remains read only", async () => {
  const value = report();
  value.capacity_decision = "storage_paused";
  value.pressure.state = "explicit_write_blocked";
  value.manual_growth = { schema_version: 1, scope: "memory_domain_assigned_btree_content_growth",
    decision: "available", configured_limit_bytes: { status: "admin_only" } };
  const requests: Array<{ path: string; init?: RequestInit }> = [];
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(<MemoryCapacityPanel t={t} lang="zh"
    requestScope="session-A" settings={settings()} scope="principal" conversationId={null}
    apiFetch={async (path, init) => { requests.push({ path, init }); return json(value); }} />); });
  const text = JSON.stringify(renderer.toJSON());
  assert.match(text, /自动生成暂停/);
  assert.match(text, /记忆存储预算可用/);
  assert.match(text, /不禁用删除、导出/);
  const buttons = renderer.root.findAllByType("button");
  assert.equal(buttons.length, 1);
  await act(async () => { buttons[0].props.onClick(); });
  assert.ok(requests.length >= 2);
  assert.ok(requests.every(({ path, init }) => path.startsWith("/v1/memory/availability?")
    && (!init?.method || init.method === "GET")));
  await act(async () => { renderer.unmount(); });
});

test("a missing manual observation is visible without inventing permission or a repair action", async () => {
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(<MemoryCapacityPanel t={t} lang="zh"
    requestScope="session-A" settings={settings()} scope="principal" conversationId={null}
    apiFetch={async () => json(report())} />); });
  const line = renderer.root.findByProps({ "data-memory-growth-status": true }).children.join("");
  assert.match(line, /尚不可用/);
  assert.doesNotMatch(line, /预算可用/);
  assert.equal(renderer.root.findAllByType("button").length, 1);
  await act(async () => { renderer.unmount(); });
});
