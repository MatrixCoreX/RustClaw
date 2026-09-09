import assert from "node:assert/strict";
import test from "node:test";
import { completeCollectionPage, groupCollectionItems } from "./aipp-collection";
import type { AippMediaItem, AippMediaPageResponse } from "../types/api";

const image = (id: number, post = 1, position = id): AippMediaItem => ({
  schema_version: 1, global_sequence: id, sequence: id, post_sequence: post,
  image_sequence: position, kind: "image", platform: "example", source_mode: "home_feed",
  search_keyword: "", title: "Same title", platform_text: "Same caption", source_url: null,
  image_url: null, preview_available: true, discovered_at: null, engagement: null,
});
const page = (items: AippMediaItem[], cursor: number | null, order: "newest" | "oldest" = "newest"): AippMediaPageResponse => ({
  schema_version: 1, items, next_cursor_sequence: cursor, next_before_sequence: cursor,
  sort_order: order, matching_total: 100, platform_states: {}, active_run: null, updated_at: null,
});

test("groups only machine-identified post images and keeps gallery order without dropping images", () => {
  const items = [image(7, 1, 3), image(6, 1, 2), image(5, 1, 1), image(4, 2),
    { ...image(3), platform: "other" }, { ...image(2), kind: "video" as const },
    { ...image(1), post_sequence: null }];
  const groups = groupCollectionItems([...items, items[0]]);
  assert.deepEqual(groups.map(group => group.map(i => i.global_sequence)), [[5, 6, 7], [4], [3], [2], [1]]);
  assert.equal(items[0].global_sequence, 7);
});

for (const order of ["newest", "oldest"] as const) {
  test(`a 100-image gallery crosses API pages without splitting or consuming the next post (${order})`, async () => {
    const images = Array.from({ length: 100 }, (_, i) => image(i + 1));
    const rows = order === "oldest" ? images : images.reverse();
    rows.push(image(101, 2));
    const first = page(rows.slice(0, 20), rows[19].global_sequence, order);
    const result = await completeCollectionPage(first, async cursor => {
      const from = rows.findIndex(i => i.global_sequence === cursor) + 1;
      const next = rows.slice(from, from + 20);
      return page(next, from + 20 < rows.length ? next.at(-1)!.global_sequence : null, order);
    }, () => true, 1);
    assert.equal(result.items.length, 100);
    assert.equal(groupCollectionItems(result.items).length, 1);
    assert.equal(result.next_cursor_sequence, rows[99].global_sequence);
    assert.equal(first.items.length, 20);
    assert.equal(result.next_before_sequence, order === "newest" ? rows[99].global_sequence : null);
  });
}

test("lookahead consumes only the tail post, leaves other posts for the next page, and detects EOF", async () => {
  const initial = page([image(10, 2), image(9, 1)], 9);
  const result = await completeCollectionPage(initial, async () => page([image(8, 1), image(7, 3)], null), () => true, 2);
  assert.deepEqual(result.items.map(i => i.global_sequence), [10, 9, 8]);
  assert.equal(result.next_cursor_sequence, 8);
  const end = await completeCollectionPage(initial, async () => page([image(8, 1)], null), () => true, 2);
  assert.equal(end.next_cursor_sequence, null);
});

test("refresh replacement, stale requests and nonadvancing cursors cannot append duplicated items", async () => {
  const initial = page([image(3), image(2)], 2);
  const repeated = await completeCollectionPage(initial, async () => page([image(2)], 2));
  assert.equal(repeated.items.length, 2);
  const stale = await completeCollectionPage(initial, async () => { throw new Error("unexpected request"); }, () => false);
  assert.deepEqual(stale, initial);
  const withoutPost = page([{ ...image(3), post_sequence: null }], 3);
  assert.deepEqual(await completeCollectionPage(withoutPost, async () => { throw new Error("unexpected request"); }, () => true, 1), withoutPost);
});

for (const order of ["newest", "oldest"] as const) {
  test(`fills all-platform pages with posts after a gallery-only row page (${order})`, async () => {
    const rows = [
      ...Array.from({ length: 40 }, (_, i) => ({ ...image(i + 1, i < 20 ? 1 : 2), platform: "xiaohongshu" })),
      ...Array.from({ length: 24 }, (_, i) => ({ ...image(i + 41, i + 3), platform: i % 2 ? "douyin" : "kuaishou" })),
    ].map((item, i) => ({ ...item, global_sequence: order === "oldest" ? i + 1 : 100 - i }));
    const load = async (cursor: number | null) => {
      const start = cursor == null ? 0 : rows.findIndex(i => i.global_sequence === cursor) + 1;
      const items = rows.slice(start, start + 20);
      return page(items, start + 20 < rows.length ? items.at(-1)!.global_sequence : null, order);
    };
    const first = await completeCollectionPage(await load(null), load);
    assert.equal(groupCollectionItems(first.items).length, 20);
    assert.deepEqual(new Set(first.items.map(i => i.platform)), new Set(["xiaohongshu", "douyin", "kuaishou"]));
    assert.equal(first.items.length, 58);
    const second = await completeCollectionPage(await load(first.next_cursor_sequence), load);
    assert.equal(groupCollectionItems(second.items).length, 6);
    assert.equal(second.next_cursor_sequence, null);
    assert.deepEqual([...first.items, ...second.items], rows);
  });
}

test("all-platform completion also fills pages after a video followed by galleries", async () => {
  const initial = page([{ ...image(30), kind: "video" }], 30);
  let calls = 0;
  const result = await completeCollectionPage(initial, async () => {
    calls++;
    return page([image(29), image(28)], null);
  });
  assert.equal(calls, 1);
  assert.equal(groupCollectionItems(result.items).length, 2);
  assert.equal(result.next_cursor_sequence, null);
});

test("completion bounds a malformed endless gallery without dropping the continuation cursor", async () => {
  const initial = page([image(100)], 100);
  let calls = 0;
  const result = await completeCollectionPage(initial, async cursor => {
    calls++;
    return page([image(cursor - 1)], cursor - 1);
  });
  assert.equal(calls, 20);
  assert.equal(result.items.length, 21);
  assert.equal(result.next_cursor_sequence, 80);
});
