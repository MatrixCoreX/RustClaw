import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { act } from "react";
import { data, json, mount, settings } from "./memory-runtime.fixture";

const actions = ["reindex", "pause", "resume", "cancel"] as const;

function accountPolicyContract() {
  const account = settings("A");
  const accepted: string[] = [];
  const presented: string[] = [];
  return {
    accepted, presented,
    async fetch(path: string, init?: RequestInit) {
      const url = new URL(path, "https://fixture.invalid");
      if (url.pathname === "/v1/memory/settings" && init?.method !== "POST") {
        const conversation = url.searchParams.get("conversation_id");
        return json(conversation ? { ...settings("A", 1, conversation),
          policy_digest: `${account.policy_digest}:conversation:${conversation}` } : account);
      }
      if (url.pathname.startsWith("/v1/memory/vector/") && init?.method === "POST") {
        const body = JSON.parse(String(init.body)) as { expected_policy_digest: string };
        presented.push(body.expected_policy_digest);
        // The real handlers compare only the account policy, never a selected
        // conversation's digest. An incorrect digest has no write side effect.
        if (body.expected_policy_digest !== account.policy_digest) {
          return new Response(JSON.stringify({ ok: false, error: "memory_settings_revision_conflict" }), { status: 409 });
        }
        accepted.push(url.pathname);
      }
      return json(data(path, "A"));
    },
  };
}

test("memory index policy fixture follows the unchanged server account contract", async () => {
  const root = new URL("../../../", import.meta.url);
  const routes = await readFile(new URL("crates/clawd/src/http/memory_routes.rs", root), "utf8");
  const source = await readFile(new URL("crates/clawd/src/memory/settings.rs", root), "utf8");
  assert.equal((routes.match(/if settings\.policy_digest != request\.expected_policy_digest/g) || []).length, 2);
  assert.match(routes, /resolve_principal_memory_settings\([\s\S]*?if settings\.policy_digest != request\.expected_policy_digest/);
  assert.match(source, /let digest_input = serde_json::json!\(\{[\s\S]*?"conversation_id": conversation_id/);
});

test("memory all index actions retain the existing account-settings happy path", async () => {
  const service = accountPolicyContract();
  const host = await mount(service.fetch);
  try {
    await act(async () => { await host.runtime.fetchMemoryData(); });
    for (const action of actions) {
      await act(async () => { await host.runtime.controlMemoryVector(action); });
      assert.equal(host.runtime.memoryError, null);
    }
    assert.equal(service.accepted.length, actions.length);
  } finally { await host.close(); }
});

for (const action of actions) {
  test(`memory index ${action} uses account policy while conversation settings stay selected`, async () => {
    const service = accountPolicyContract();
    const host = await mount(service.fetch);
    try {
      await act(async () => { await host.runtime.fetchMemoryData(); });
      await act(async () => { await host.runtime.selectMemorySettingScope("conversation", "selected-conversation"); });
      assert.notEqual(host.runtime.memorySettings?.policy_digest, settings("A").policy_digest);
      await act(async () => { await host.runtime.controlMemoryVector(action); });
      assert.equal(host.runtime.memoryError, null);
      assert.deepEqual(service.accepted, [`/v1/memory/vector/${action}`]);
      assert.deepEqual(service.presented, [settings("A").policy_digest]);
      assert.equal(host.runtime.memorySettingScope, "conversation");
      assert.equal(host.runtime.memorySettings?.conversation_id, "selected-conversation");
    } finally { await host.close(); }
  });
}
