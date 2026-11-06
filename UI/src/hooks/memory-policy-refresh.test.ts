import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react";
import { begin, data, deferred, json, mount, settings, type ApiFetch } from "./memory-runtime.fixture";

function policyService(failStatusAfterSave = false) {
  let accountRevision = 1;
  let conversationRevision = 1;
  const accepted: string[] = [];
  const accountDigest = () => settings("A", accountRevision).policy_digest;
  const fetch: ApiFetch = async (path, init) => {
    const url = new URL(path, "https://fixture.invalid");
    if (url.pathname === "/v1/memory/settings") {
      const body = init?.method === "POST" ? JSON.parse(String(init.body)) : null;
      const conversation = body ? body.conversation_id : url.searchParams.get("conversation_id");
      if (body) {
        const revision = conversation ? conversationRevision : accountRevision;
        assert.equal(body.expected_revision, revision);
        if (conversation) conversationRevision++; else accountRevision++;
      }
      const revision = conversation ? conversationRevision : accountRevision;
      return json({ ...settings("A", revision, conversation || null), use_memory: revision === 1,
        policy_digest: conversation ? `conversation-${accountRevision}-${revision}` : accountDigest() });
    }
    if (url.pathname === "/v1/memory/vector/status") {
      return failStatusAfterSave && accountRevision > 1
        ? new Response(JSON.stringify({ ok: false, error: "memory_vector_status_failed" }), { status: 500 })
        : json({ ...data(path, "A"), policy_digest: accountDigest() });
    }
    if (url.pathname.startsWith("/v1/memory/vector/") && init?.method === "POST") {
      const body = JSON.parse(String(init.body));
      if (body.expected_policy_digest !== accountDigest()) {
        return new Response(JSON.stringify({ ok: false, error: "memory_settings_revision_conflict" }), { status: 409 });
      }
      accepted.push(url.pathname);
    }
    return json(data(path, "A"));
  };
  return { fetch, accepted, accountDigest };
}

test("memory an account settings save refreshes index policy before the next index action", async () => {
  const service = policyService();
  const host = await mount(service.fetch);
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { await host.runtime.updateMemorySetting("use", false); });
    assert.equal(host.runtime.memoryVectorStatus?.policy_digest, service.accountDigest());
    await act(async () => { await host.runtime.controlMemoryVector("pause"); });
    assert.equal(host.runtime.memoryError, null);
    assert.deepEqual(service.accepted, ["/v1/memory/vector/pause"]);
  } finally { await host.close(); }
});

test("memory an account save refreshes index policy even after the settings view changes", async () => {
  const release = deferred<void>();
  const service = policyService();
  const host = await mount(async (path, init) => {
    if (path === "/v1/memory/settings" && init?.method === "POST") await release.promise;
    return service.fetch(path, init);
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    const saving = await begin(() => host.runtime.updateMemorySetting("use", false));
    await act(async () => { await host.runtime.selectMemorySettingScope("conversation", "selected-conversation"); });
    await act(async () => { release.resolve(undefined); await saving.pending; });
    assert.equal(host.runtime.memoryVectorStatus?.policy_digest, service.accountDigest());
    assert.equal(host.runtime.memorySettings?.conversation_id, "selected-conversation");
    await act(async () => { await host.runtime.controlMemoryVector("pause"); });
    assert.equal(host.runtime.memoryError, null);
  } finally { await host.close(); }
});

test("memory a failed index status refresh does not misreport a successful settings save", async () => {
  const service = policyService(true);
  const host = await mount(service.fetch);
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { await host.runtime.updateMemorySetting("use", false); });
    assert.equal(host.runtime.memorySettings?.revision, 2);
    assert.equal(host.runtime.memorySettings?.use_memory, false);
    assert.equal(host.runtime.memoryError, null);
    assert.equal(host.runtime.memoryVectorStatus, null);
    assert.equal(host.runtime.memorySettingsSaving, false);
  } finally { await host.close(); }
});

test("memory an older index read cannot restore policy from before an account settings save", async () => {
  const late = deferred<Response>();
  const service = policyService();
  let hold = false;
  const host = await mount(async (path, init) => {
    if (hold && path === "/v1/memory/vector/status") { hold = false; return late.promise; }
    return service.fetch(path, init);
  });
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    hold = true;
    const reading = await begin(() => host.runtime.fetchMemoryData());
    await act(async () => { await host.runtime.updateMemorySetting("use", false); });
    await act(async () => {
      late.resolve(json({ ...data("/v1/memory/vector/status", "A"), policy_digest: settings("A").policy_digest }));
      await reading.pending;
    });
    assert.equal(host.runtime.memoryVectorStatus?.policy_digest, service.accountDigest());
    assert.equal(host.runtime.memorySettings?.revision, 2);
  } finally { await host.close(); }
});

test("memory a conversation-only settings save keeps the account index policy independent", async () => {
  const service = policyService();
  const host = await mount(service.fetch);
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    await act(async () => { await host.runtime.selectMemorySettingScope("conversation", "selected-conversation"); });
    await act(async () => { await host.runtime.updateMemorySetting("use", false); });
    assert.notEqual(host.runtime.memorySettings?.policy_digest, service.accountDigest());
    await act(async () => { await host.runtime.controlMemoryVector("pause"); });
    assert.equal(host.runtime.memoryError, null);
    assert.deepEqual(service.accepted, ["/v1/memory/vector/pause"]);
    assert.equal(host.runtime.memorySettings?.conversation_id, "selected-conversation");
  } finally { await host.close(); }
});
