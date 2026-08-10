import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  beginRun,
  cleanupExpiredDiagnostics,
  commitPageRecords,
  copyExportsTo,
  finishRun,
  readRecords,
  readState,
  rebuildExports,
} from "../src/storage.mjs";

async function temporaryRoot(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-test-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return root;
}

test("immutable records keep global, type, post, and image order", async (t) => {
  const root = await temporaryRoot(t);
  const first = await commitPageRecords(root, [
    {
      kind: "image",
      dedup_key: "post:1:image:1",
      image_sequence: 1,
      platform: "xiaohongshu",
      title: "one",
      platform_text: "",
      recognized_text: "a",
      image_url: "",
      source_page_url: "https://www.xiaohongshu.com/explore/1",
      discovered_at: "2026-08-10T00:00:00Z",
    },
    {
      kind: "image",
      dedup_key: "post:1:image:2",
      image_sequence: 2,
      platform: "xiaohongshu",
      title: "one",
      platform_text: "",
      recognized_text: "b",
      image_url: "",
      source_page_url: "https://www.xiaohongshu.com/explore/1",
      discovered_at: "2026-08-10T00:00:00Z",
    },
  ]);
  assert.deepEqual(first.committed.map((record) => record.sequence), [1, 2]);
  assert.deepEqual(first.committed.map((record) => record.post_sequence), [1, 1]);

  const second = await commitPageRecords(root, [{
    kind: "video",
    dedup_key: "post:2:video",
    platform: "douyin",
    title: "two",
    platform_text: "",
    recognized_text: "",
    video_page_url: "https://www.douyin.com/video/2",
    discovered_at: "2026-08-10T00:01:00Z",
  }]);
  assert.equal(second.committed[0].global_sequence, 3);
  assert.equal(second.committed[0].sequence, 1);
  assert.equal((await readRecords(root)).length, 3);
});

test("duplicates do not append rows and exports are rebuilt deterministically", async (t) => {
  const root = await temporaryRoot(t);
  const record = {
    kind: "video",
    dedup_key: "same",
    platform: "douyin",
    title: "same",
    platform_text: "",
    recognized_text: "",
    video_page_url: "https://www.douyin.com/video/1",
    discovered_at: "2026-08-10T00:00:00Z",
  };
  await commitPageRecords(root, [record]);
  const duplicate = await commitPageRecords(root, [record]);
  assert.equal(duplicate.committed.length, 0);
  assert.equal(duplicate.duplicateCount, 1);
  const exported = await rebuildExports(root);
  assert.equal(exported.videoCount, 1);
  assert.equal(exported.imageCount, 0);
});

test("export copies persisted rendered video covers beside the CSV files", async (t) => {
  const root = await temporaryRoot(t);
  const coverDirectory = path.join(root, "exports", "video_covers");
  await fs.mkdir(coverDirectory, { recursive: true });
  await fs.writeFile(path.join(coverDirectory, "douyin_1.png"), "rendered-cover");
  const output = path.join(root, "delivery");
  const exported = await copyExportsTo(root, output);
  assert.deepEqual(exported.coverPaths, [path.join(output, "video_covers", "douyin_1.png")]);
  assert.equal(await fs.readFile(exported.coverPaths[0], "utf8"), "rendered-cover");
});

test("one-shot runs use an ephemeral config without enabling continuous collection", async (t) => {
  const root = await temporaryRoot(t);
  const config = {
    source_mode: "topics",
    topics: ["fixture"],
    browser_mode: "silent",
    max_run_minutes: 5,
  };
  const { run } = await beginRun(root, ["douyin"], {
    mode: "one_shot",
    config,
    config_explicit: true,
  });
  assert.equal(run.run_mode, "one_shot");
  assert.deepEqual(run.platform_configs.douyin, config);
  assert.equal((await readState(root)).platforms.douyin, undefined);

  const completed = await finishRun(root, run, "completed_batch");
  assert.equal(completed.lifecycle_state, "completed_batch");
  assert.equal((await readState(root)).active_run, null);
});

test("diagnostic retention removes expired files and directories", async (t) => {
  const root = await temporaryRoot(t);
  const diagnostics = path.join(root, "diagnostics");
  const oldDirectory = path.join(diagnostics, "old-run");
  const oldFile = path.join(diagnostics, "old-file.png");
  await fs.mkdir(oldDirectory, { recursive: true });
  await fs.writeFile(path.join(oldDirectory, "capture.png"), "old");
  await fs.writeFile(oldFile, "old");
  const oldDate = new Date(Date.now() - 3 * 60 * 60 * 1000);
  await fs.utimes(oldDirectory, oldDate, oldDate);
  await fs.utimes(oldFile, oldDate, oldDate);

  await cleanupExpiredDiagnostics(root, 1);

  await assert.rejects(fs.stat(oldDirectory), { code: "ENOENT" });
  await assert.rejects(fs.stat(oldFile), { code: "ENOENT" });
});
