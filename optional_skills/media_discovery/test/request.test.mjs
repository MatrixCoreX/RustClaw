import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { parseSkillRequest } from "../src/request.mjs";
import { resultOwner } from "../src/results.mjs";
import { handleRequest } from "../src/main.mjs";

test("i64 actor IDs survive JSON decoding without rounding neighboring owners together", () => {
  for (const actor of ["9223372036854775807", "9223372036854775806", "-9223372036854775808", "306417032017051587"]) {
    const request = parseSkillRequest(`{"user_id":${actor},"chat_id":${actor}}`);
    assert.equal(request.user_id, actor);
    assert.equal(resultOwner(request), actor);
  }
  assert.notEqual(resultOwner(parseSkillRequest('{"user_id":9223372036854775807}')),
    resultOwner(parseSkillRequest('{"user_id":9223372036854775806}')));
});

test("safe integers, fractions, and JSON strings retain their normal types", () => {
  const result = parseSkillRequest('{"safe":9007199254740991,"fraction":0.12345678912345678,"quoted":"9223372036854775807","normal":42}');
  assert.equal(result.safe, Number.MAX_SAFE_INTEGER);
  assert.equal(typeof result.fraction, "number");
  assert.equal(result.quoted, "9223372036854775807");
  assert.equal(result.normal, 42);
  assert.equal(resultOwner({ user_id: "9223372036854775808" }), null);
  assert.equal(resultOwner({ user_id: "01" }), null);
  assert.equal(resultOwner({ user_id: "__proto__" }), null);
});

test("ambiguous duplicate keys and prototype mutation are rejected", () => {
  for (const line of ['{"user_id":7,"user_id":8}', '{"__proto__":{"admin":true}}', '{"constructor":{"prototype":{"admin":true}}}']) {
    assert.throws(() => parseSkillRequest(line));
  }
  assert.equal({}.admin, undefined);
});

test("neighboring large actor IDs cannot query each other's stored metric results", async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-i64-owner-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const request = parseSkillRequest(`{"user_id":9223372036854775807,"chat_id":1,"args":{"action":"run_once","platform":"weibo"},"context":{"skill_storage":{"storage_kind":"directory","directory_path":${JSON.stringify(root)}}}}`);
  const result = await handleRequest(request, { collectPlatform: async ({ onPage }) => {
    await onPage({ records: [{ kind: "video", platform: "weibo", item_id: "weibo:1", dedup_key: "weibo:1:video", engagement: { platform: "weibo", metrics: { likes: { display: "2", value: 2 } } } }], temporaryPaths: [] });
    return { stop_reason: "target_reached" };
  } });
  assert.equal(result.status, "ok");
  assert.ok(result.extra.result_query);
  request.args = { action: "get_run_results", run_id: result.extra.run.run_id };
  assert.equal((await handleRequest(request)).extra.results[0].engagement.metrics.likes.value, 2);
  request.user_id = "9223372036854775806";
  const rejected = await handleRequest(request);
  assert.equal(rejected.extra.error_code, "result_run_not_found");
  assert.equal(rejected.extra.results, undefined);
});
