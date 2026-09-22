import assert from "node:assert/strict";
import test from "node:test";
import { AippSkillRemovalError, removeAippAndSkill } from "./aipp-skill-removal";

const operation = (status: string, extra = {}) => ({
  operation_id: "remove-1", skill_name: "example", action: "remove", status, ...extra,
});
const response = (value: unknown, status = 200) => new Response(JSON.stringify({ ok: true, data: { operation: value } }), { status });

test("uninstalls the owning skill with retained config/data and waits for server success", async () => {
  const calls: Array<{ path: string; init?: RequestInit }> = [];
  const replies = [operation("queued"), operation("running"), operation("success", { result: { installed: false } })];
  await removeAippAndSkill(async (path, init) => {
    calls.push({ path, init });
    return response(replies.shift());
  }, "example", { pollDelayMs: 0 });
  assert.equal(calls[0].path, "/v1/skills/store/remove");
  assert.equal(calls[0].init?.method, "POST");
  assert.deepEqual(JSON.parse(String(calls[0].init?.body)), { skill_name: "example", preserve_config: true, preserve_data: true });
  assert.deepEqual(calls.slice(1).map((call) => call.path), Array(2).fill("/v1/skills/store/operations/remove-1"));
  assert.ok(calls.every((call) => !call.path.startsWith("/v1/aipps/")));
});

test("does not report queued, failed, cancelled or mismatched operations as success", async () => {
  for (const value of [
    operation("failure", { failure: { error_code: "skill_store_package_remove_failed" } }),
    operation("cancelled"),
    operation("success", { result: { installed: true } }),
    operation("success", { skill_name: "unrelated", result: { installed: false } }),
    operation("success", { action: "install", result: { installed: false } }),
    operation("invalid"),
  ]) {
    await assert.rejects(removeAippAndSkill(async () => response(value), "example"), AippSkillRemovalError);
  }
  await assert.rejects(removeAippAndSkill(async () => response(operation("queued")), "example", { pollAttempts: 0 }),
    (error: unknown) => error instanceof AippSkillRemovalError && error.pending);
});

test("propagates rejection and rejects a different job returned while polling", async () => {
  await assert.rejects(removeAippAndSkill(async () => new Response(JSON.stringify({ ok: false, error: "skill_store_locked_skill" }), { status: 403 }), "example"),
    (error: unknown) => error instanceof AippSkillRemovalError && error.code === "skill_store_locked_skill");
  let count = 0;
  await assert.rejects(removeAippAndSkill(async () => response(count++ ? operation("success", { operation_id: "other", result: { installed: false } }) : operation("queued")), "example", { pollDelayMs: 0 }), AippSkillRemovalError);
});

test("leaving the page stops polling without cancelling the server removal job", async () => {
  const controller = new AbortController();
  const paths: string[] = [];
  await assert.rejects(removeAippAndSkill(async (path) => {
    paths.push(path);
    controller.abort();
    return response(operation("queued"));
  }, "example", { signal: controller.signal, pollDelayMs: 0 }), { name: "AbortError" });
  assert.deepEqual(paths, ["/v1/skills/store/remove"]);
});
