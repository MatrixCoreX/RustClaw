import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react";
import { begin, data, deferred, json, mount, settings } from "./memory-runtime.fixture";

test("memory index controls use their displayed status snapshot rather than a settings read", async () => {
  const presented: string[] = [];
  const host = await mount(async (path, init) => {
    if (path === "/v1/memory/vector/status") return json({ ...data(path, "A"), policy_digest: "index-status-snapshot" });
    if (init?.method === "POST") presented.push(JSON.parse(String(init.body)).expected_policy_digest);
    return json(data(path, "A"));
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { await host.runtime.controlMemoryVector("pause"); });
    assert.deepEqual(presented, ["index-status-snapshot"]);
  } finally { await host.close(); }
});

test("memory an expired index policy is reported without automatic mutation replay", async () => {
  let writes = 0;
  const host = await mount(async (path, init) => {
    if (init?.method === "POST") {
      writes++;
      return new Response(JSON.stringify({ ok: false, error: "memory_settings_revision_conflict" }), { status: 409 });
    }
    return json(data(path, "A"));
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { await host.runtime.controlMemoryVector("pause"); });
    assert.equal(writes, 1);
    assert.ok(host.runtime.memoryError);
    assert.equal(host.runtime.memoryMessage, null);
  } finally { await host.close(); }
});

function olderStatus(path: string) {
  const result = { ...data(path, "A") } as Record<string, unknown>;
  delete result.policy_digest;
  return result;
}

test("memory older index status reads account policy without changing the selected settings view", async () => {
  const presented: string[] = [];
  const host = await mount(async (path, init) => {
    if (path === "/v1/memory/vector/status") return json(olderStatus(path));
    if (new URL(path, "https://fixture.invalid").searchParams.has("conversation_id")) {
      return json({ ...data(path, "A"), policy_digest: "conversation-policy" });
    }
    if (init?.method === "POST") presented.push(JSON.parse(String(init.body)).expected_policy_digest);
    return json(data(path, "A"));
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { await host.runtime.selectMemorySettingScope("conversation", "selected-conversation"); });
    await act(async () => { await host.runtime.controlMemoryVector("pause"); });
    assert.deepEqual(presented, [settings("A").policy_digest]);
    assert.equal(host.runtime.memorySettings?.conversation_id, "selected-conversation");
  } finally { await host.close(); }
});

test("memory an old device account-policy read cannot authorize a new owner's index mutation", async () => {
  const pending = deferred<Response>();
  let controlStarted = false;
  let writes = 0;
  const host = await mount(async (path, init) => {
    if (path === "/v1/memory/vector/status") return json(olderStatus(path));
    if (path === "/v1/memory/settings" && controlStarted) return pending.promise;
    if (init?.method === "POST") writes++;
    return json(data(path, "A"));
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    controlStarted = true;
    const operation = await begin(() => host.runtime.controlMemoryVector("pause"));
    await host.scope("B");
    await act(async () => { pending.resolve(json(settings("A"))); await operation.pending; });
    assert.equal(writes, 0);
    assert.equal(host.runtime.memoryError, null);
    assert.equal(host.runtime.memoryVectorStatus, null);
  } finally { await host.close(); }
});
