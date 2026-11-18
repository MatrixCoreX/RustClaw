import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { MemoryCapacityPanel } from "../components/MemoryCapacityPanel";
import { deferred, json, mount, report, settings, t } from "./memory-capacity.fixture";

test("capacity waits for policy loading and never performs a mutation request", async () => {
  const requests: Array<{ path: string; init?: RequestInit }> = [];
  const view = await mount(async (path, init) => { requests.push({ path, init }); return json(report()); }, { settings: null });
  assert.equal(requests.length, 0);
  await view.update({ settings: settings() });
  assert.equal(requests.length, 1);
  assert.equal(requests[0].path, "/v1/memory/availability?scope=principal");
  assert.equal(requests[0].init?.method, undefined);
  assert.equal(requests[0].init?.body, undefined);
  assert.equal(view.runtime.data?.settings.target_principal_id, "A");
  await view.close();
});

test("late account A read cannot render under account B", async () => {
  const old = deferred<Response>();
  let calls = 0;
  const view = await mount(async () => ++calls === 1 ? old.promise : json(report(settings("B"))));
  await view.update({ requestScope: "session-B", settings: settings("B") });
  await act(async () => { old.resolve(json(report())); });
  assert.equal(view.runtime.data?.settings.target_principal_id, "B");
  assert(view.renders.filter(row => row.requestScope === "session-B").every(row => !row.data || row.data.settings.target_principal_id === "B"));
  await view.close();
});

test("A B A owner epochs do not revive an old callback or response", async () => {
  const first = deferred<Response>();
  const third = deferred<Response>();
  let calls = 0;
  const view = await mount(async () => ++calls === 1 ? first.promise : calls === 2 ? json(report(settings("B"))) : third.promise);
  const oldRefresh = view.runtime.refresh;
  await view.update({ requestScope: "B", settings: settings("B") });
  await view.update({ requestScope: "session-A", settings: settings() });
  await act(async () => { first.resolve(json(report())); await oldRefresh(); });
  assert.equal(calls, 3);
  assert.equal(view.runtime.data, null);
  await act(async () => { third.resolve(json(report())); });
  assert.equal(view.runtime.data?.settings.target_principal_id, "A");
  await view.close();
});

test("policy revision changes hide old capacity during render and reject late reads", async () => {
  const pending = deferred<Response>();
  let calls = 0;
  const view = await mount(async () => ++calls === 1 ? pending.promise : json(report(settings("A", 2))));
  await view.update({ settings: settings("A", 2) });
  await act(async () => { pending.resolve(json(report())); });
  assert.equal(view.runtime.data?.settings.revision, 2);
  await view.close();
});

test("conversation switch waits for matching settings instead of requesting the previous conversation", async () => {
  const paths: string[] = [];
  const view = await mount(async path => { paths.push(path); return json(report(settings("A", 1, "new"))); });
  await view.update({ scope: "conversation", conversationId: "new", settings: settings("A", 1, "old") });
  assert.equal(paths.length, 1);
  assert.equal(view.runtime.data, null);
  await view.update({ settings: settings("A", 1, "new") });
  assert.equal(paths[1], "/v1/memory/availability?scope=conversation&conversation_id=new");
  assert.equal(view.runtime.data?.settings.conversation_id, "new");
  await view.close();
});

test("server-side policy changes fail visibly without auto retrying or showing stale availability", async () => {
  let calls = 0;
  const view = await mount(async () => { calls++; return json(report(settings("A", 2))); });
  assert.equal(calls, 1);
  assert.equal(view.runtime.failed, true);
  assert.equal(view.runtime.data, null);
  await view.close();
});

test("a failed refresh removes old capacity rather than presenting old green data", async () => {
  let calls = 0;
  const view = await mount(async () => ++calls === 1 ? json(report()) : new Response("internal sensitive detail", { status: 500 }));
  assert(view.runtime.data);
  await act(async () => { await view.runtime.refresh(); });
  assert.equal(view.runtime.data, null);
  assert.equal(view.runtime.failed, true);
  await view.close();
});

test("newest capacity refresh wins even when a transport ignores abort", async () => {
  const first = deferred<Response>();
  let calls = 0;
  const view = await mount(async () => ++calls === 1 ? first.promise : json({ ...report(), sampled_at_ts: 200 }));
  await act(async () => { await view.runtime.refresh(); });
  await act(async () => { first.resolve(json(report())); });
  assert.equal(view.runtime.data?.sampled_at_ts, 200);
  await view.close();
});

test("unmount invalidates captured capacity refresh and late responses", async () => {
  const pending = deferred<Response>();
  let calls = 0;
  const view = await mount(async () => { calls++; return pending.promise; });
  const refresh = view.runtime.refresh;
  await view.close();
  await act(async () => { pending.resolve(json(report())); await refresh(); });
  assert.equal(calls, 1);
});

test("capacity panel keeps diagnostics collapsed and does not call an undated observation ready", async () => {
  let renderer!: ReactTestRenderer;
  const value = report();
  value.pressure.observed_at_ts = null;
  const apiFetch = async () => json(value);
  await act(async () => { renderer = create(<MemoryCapacityPanel lang="zh" t={t} requestScope="A" settings={settings()}
    scope="principal" conversationId={null} apiFetch={apiFetch} />); });
  const text = JSON.stringify(renderer.toJSON());
  assert(text.includes("存储状态尚未取得有效采样"));
  assert(!text.includes("容量检查未阻止自动生成"));
  assert.equal(renderer.root.findByType("details").props.open, undefined);
  assert(!text.includes("数据库软阈值"), "ordinary users do not get global diagnostics");
  await act(async () => { renderer.unmount(); });
});

test("capacity panel renders missing quota as unavailable rather than fabricated zero", async () => {
  let renderer!: ReactTestRenderer;
  const value = report();
  value.capacity_decision = value.quota.decision = "principal_quota_missing";
  value.quota.counters = null;
  const apiFetch = async () => json(value);
  await act(async () => { renderer = create(<MemoryCapacityPanel lang="zh" t={t} requestScope="A" settings={settings()}
    scope="principal" conversationId={null} apiFetch={apiFetch} />); });
  const text = JSON.stringify(renderer.toJSON());
  assert(text.includes("当前账号缺少配额设置"));
  assert(text.includes("暂不可用"));
  assert(!text.includes("0 / 0"));
  await act(async () => { renderer.unmount(); });
});
