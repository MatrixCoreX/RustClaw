import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react";
import type { UiConfirmOptions } from "../components/UiDialogProvider";
import { data, json, mount } from "./memory-runtime.fixture";

for (const language of ["zh", "en"] as const) {
  for (const scope of ["recent", "all"] as const) {
    test(`memory ${scope} clear explains irreversible effects before consent in ${language}`, async () => {
      const prompts: UiConfirmOptions[] = [];
      const writes: string[] = [];
      const mode = scope === "recent" ? "transcript" : "transcript_and_derived";
      const preview = { schema_version: 1, mode, transcript_rows: 13,
        derived_rows: scope === "recent" ? 0 : 7, pending_jobs: 3 };
      const host = await mount(async (path, init) => {
        if (init?.method === "POST") writes.push(path);
        assert.equal(path, `/v1/memory/clear/preview?mode=${mode}`);
        return json(preview);
      }, "A", async options => { prompts.push(options); return false; },
      (zh, en) => language === "zh" ? zh : en);
      try {
        await act(async () => { host.runtime.setMemoryClearScope(scope); });
        await act(async () => { await host.runtime.clearMemoryScope(); });
        assert.equal(prompts.length, 1);
        assert.equal(prompts[0].tone, "danger");
        const text = prompts[0].message;
        for (const content of language === "zh"
          ? ["无法撤销", "待确认导入", "失效", "先导出", "13 条近期记录", `${preview.derived_rows} 条派生记忆`, "3 个后台作业"]
          : ["cannot be undone", "pending imports", "invalidated", "Export", "13 recent records", `${preview.derived_rows} derived memories`, "3 background jobs"]) {
          assert.ok(text.includes(content), content);
        }
        assert.deepEqual(writes, [], "cancelled consent must never send a clear request");
        assert.equal(host.runtime.memoryMessage, null);
      } finally { await host.close(); }
    });
  }
}

test("memory clear notice preserves the confirmed scope and compare-and-clear counts", async () => {
  const writes: unknown[] = [];
  let approved = false;
  const preview = { schema_version: 1, mode: "transcript_and_derived", transcript_rows: 13,
    derived_rows: 7, pending_jobs: 3 };
  const host = await mount(async (path, init) => {
    if (path.startsWith("/v1/memory/clear/preview")) return json(preview);
    if (init?.method === "POST") {
      assert.ok(approved, "confirmation must precede the destructive request");
      assert.equal(path, "/v1/memory/clear/scoped");
      writes.push(JSON.parse(String(init.body)));
      return json(preview);
    }
    return json(data(path, "A"));
  }, "A", async options => {
    assert.ok(options.message.includes("无法撤销"));
    approved = true;
    return true;
  });
  try {
    await act(async () => { host.runtime.setMemoryClearScope("all"); });
    await act(async () => { await host.runtime.clearMemoryScope(); });
    assert.deepEqual(writes, [{ mode: preview.mode, expected_transcript_rows: 13, expected_derived_rows: 7 }]);
    assert.equal(host.runtime.memoryError, null);
    assert.equal(host.runtime.memoryMessage, "清理完成。");
  } finally { await host.close(); }
});
