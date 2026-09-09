import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { imageSourceIdentity, recordIdentity } from "../src/media_identity.mjs";
import { canonicalCandidateUrls } from "../src/platforms.mjs";
import { commitPageRecords, readRecords } from "../src/storage.mjs";

const objectA = "1040g3k0324olqe0u6u005oj552v41uohmijva6o";
const objectB = "1040g3k0324olqe0u6u005oj552v41uohmijva6b";
const source = (object, variant = "first") => `https://${variant}.xhscdn.com/20260909/${variant}/notes_pre_post/${object}!${variant}?signature=${variant}`;
const record = (object, position = 1, post = "post-a", variant) => ({
  kind: "image", platform: "xiaohongshu", item_id: `xiaohongshu:${post}`,
  dedup_key: `xiaohongshu:${post}:image:${position}`, image_sequence: position,
  image_url: source(object, variant), title: "Same title", platform_text: "Same caption",
  source_page_url: `https://www.xiaohongshu.com/explore/${post}`,
  discovered_at: "2026-09-09T00:00:00Z",
});
async function temporaryRoot(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-identity-test-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return root;
}

test("reviewed image object identity ignores CDN delivery variants, not distinct images", () => {
  assert.equal(imageSourceIdentity("xiaohongshu", source(objectA)), imageSourceIdentity("xiaohongshu", source(objectA, "second")));
  assert.notEqual(imageSourceIdentity("xiaohongshu", source(objectA)), imageSourceIdentity("xiaohongshu", source(objectB)));
  for (const host of ["example.test", "xhscdn.com.example.test"]) {
    const a = `https://${host}/${objectA}?image=1`;
    assert.notEqual(imageSourceIdentity("xiaohongshu", a), imageSourceIdentity("xiaohongshu", a.replace("image=1", "image=2")));
  }
  assert.notEqual(imageSourceIdentity("douyin", source(objectA)), imageSourceIdentity("douyin", source(objectA, "second")));
  assert.equal(imageSourceIdentity("xiaohongshu", ""), null);
});

test("candidate dedup keeps the first usable URL and order across detail routes and signatures", () => {
  const a = "https://www.xiaohongshu.com/search_result/post-a?token=first";
  const b = "https://www.xiaohongshu.com/explore/post-b";
  assert.deepEqual(canonicalCandidateUrls("xiaohongshu", [a, b,
    "https://www.xiaohongshu.com/explore/post-a?token=second"]), [a, b]);
});

test("legacy image records dedup by asset across restart and reordered captures without losing distinct images", async t => {
  const root = await temporaryRoot(t);
  const first = await commitPageRecords(root, [record(objectA)]);
  const oldFile = path.join(root, "records", "000000000001.json");
  const original = await fs.readFile(oldFile, "utf8");
  const next = await commitPageRecords(root, [record(objectB, 1), record(objectA, 2, "post-a", "second")]);
  assert.equal(next.duplicateCount, 1);
  assert.equal(next.committed.length, 1);
  assert.equal(next.committed[0].post_sequence, first.committed[0].post_sequence);
  assert.equal(await fs.readFile(oldFile, "utf8"), original);
  assert.equal((await readRecords(root)).length, 2);
  const repeat = await commitPageRecords(root, [record(objectA, 1), record(objectB, 2)]);
  assert.equal(repeat.duplicateCount, 2);
  assert.equal(repeat.committed.length, 0);
});

test("same image in different posts stays separate and concurrent commits remain idempotent", async t => {
  const root = await temporaryRoot(t);
  const outcomes = await Promise.all(Array.from({ length: 4 }, () => commitPageRecords(root, [record(objectA), record(objectB)])));
  assert.equal(outcomes.reduce((sum, item) => sum + item.committed.length, 0), 2);
  const other = await commitPageRecords(root, [record(objectA, 1, "post-b")]);
  assert.equal(other.committed.length, 1);
  assert.equal(other.committed[0].post_sequence, 2);
});

test("video identity uses the platform post and records without sources keep explicit identity", () => {
  const a = { ...record(objectA), kind: "video" };
  assert.equal(recordIdentity(a), recordIdentity({ ...a, dedup_key: "another", source_page_url: "https://www.xiaohongshu.com/search_result/post-a?signature=second" }));
  assert.notEqual(recordIdentity({ ...record(objectA), image_url: "" }), recordIdentity({ ...record(objectA, 2), image_url: "" }));
});
