import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { RuntimeCheckpointPanel } from "../components/RuntimeCheckpointPanel";
const t = (_zh: string, en: string) => en;
async function show(data: unknown) {
  const apiFetch = async () => new Response(JSON.stringify({ ok: true, data }), { headers: { "Content-Type": "application/json" } });
  let view!: ReactTestRenderer;
  await act(async () => { view = create(<RuntimeCheckpointPanel t={t} apiFetch={apiFetch} requestScope="owner" />); });
  const text = JSON.stringify(view.toJSON());
  await act(async () => { view.unmount(); });
  return text;
}
test("remote failure retains the verified local result and reports the independent failure", async () => {
  const text = await show({ status: "verified_local_remote_failed", bytes: 1000, created_at_ts: 1000 });
  assert(text.includes("passed verification"));
  assert(text.includes("remote copy did not complete"));
  assert(!text.includes("No verified checkpoint"));
});
test("quota failures explain the actual next step and cannot appear verified", async () => {
  const text = await show({ status: "failed", error_code: "checkpoint_snapshot_space_or_quota_unavailable" });
  assert(text.includes("Temporary space or quota is insufficient"));
  assert(!text.includes("passed verification"));
});
test("compression timeout identifies the budget instead of claiming a storage failure", async () => {
  const text = await show({ status: "failed", error_code: "checkpoint_read_fence_timeout" });
  assert(text.includes("Compression exceeded the checkpoint time budget"));
  assert(!text.includes("passed verification"));
  assert(!text.includes("Check destination space"));
});
test("deferred encrypted backups explain retries and disclose only included scopes", async () => {
  const text=await show({status:"verified",schedule:{phase:"remote",next_retry_at_ts:2000},vault:{status:"verified_encrypted",encrypted:true,retained_versions:3,payload_scopes:["packages","models"]}});
  assert(text.includes("automatic retry"));
  assert(text.includes("3 versions retained"));
  assert(text.includes("Installed skill packages included"));
  assert(text.includes("Local models included"));
  assert(!text.includes("Task attachments included"));
});
test("exhausted retry budgets remain actionable without presenting a failed copy as verified",async()=>{
  const text=await show({status:"failed",schedule:{notice_key:"backup.attention_required"}});
  assert(text.includes("Automatic retries did not complete"));
  assert(text.includes("recovery key"));
  assert(!text.includes("Incremental encrypted copy verified"));
});
