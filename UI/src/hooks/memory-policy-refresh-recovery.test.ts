import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react";
import { assertOwner, begin, data, deferred, json, mount, settings } from "./memory-runtime.fixture";

test("memory a previous-owner post-save index read cannot replace a new owner's state", async () => {
  const pending = deferred<Response>();
  let owner = "A";
  let revision = 1;
  const host = await mount(async (path, init) => {
    if (path === "/v1/memory/settings" && init?.method === "POST") {
      revision++;
      return json({ ...settings(owner, revision), use_memory: false });
    }
    if (path === "/v1/memory/vector/status" && owner === "A" && revision === 2) return pending.promise;
    return json(data(path, owner));
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    const saving = await begin(() => host.runtime.updateMemorySetting("use", false));
    assert.equal(host.runtime.memorySettings?.revision, 2);
    owner = "B";
    await host.scope("B");
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => {
      pending.resolve(json({ ...data("/v1/memory/vector/status", "A"), policy_digest: settings("A", 2).policy_digest }));
      await saving.pending;
    });
    assertOwner(host.runtime, "B");
    assert.equal(host.runtime.memoryVectorStatus?.policy_digest, settings("B").policy_digest);
    assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});

test("memory an older post-save index read cannot overwrite a newer account policy", async () => {
  const pending = deferred<Response>();
  let revision = 1;
  const host = await mount(async (path, init) => {
    if (path === "/v1/memory/settings") {
      if (init?.method === "POST") revision++;
      return json({ ...settings("A", revision), use_memory: revision === 1 });
    }
    if (path === "/v1/memory/vector/status") {
      if (revision === 2) return pending.promise;
      return json({ ...data(path, "A"), policy_digest: settings("A", revision).policy_digest });
    }
    return json(data(path, "A"));
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    const first = await begin(() => host.runtime.updateMemorySetting("use", false));
    assert.equal(host.runtime.memorySettings?.revision, 2);
    await act(async () => { await host.runtime.updateMemorySetting("generate", false); });
    assert.equal(host.runtime.memoryVectorStatus?.policy_digest, settings("A", 3).policy_digest);
    await act(async () => {
      pending.resolve(json({ ...data("/v1/memory/vector/status", "A"), policy_digest: settings("A", 2).policy_digest }));
      await first.pending;
    });
    assert.equal(host.runtime.memoryVectorStatus?.policy_digest, settings("A", 3).policy_digest);
    assert.equal(host.runtime.memorySettings?.revision, 3);
    assert.equal(host.runtime.memorySettingsSaving, false);
    assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});
