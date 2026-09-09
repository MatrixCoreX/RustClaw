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
    });
    assert.equal(result.items.length, 100);
    assert.equal(groupCollectionItems(result.items).length, 1);
    assert.equal(result.next_cursor_sequence, rows[99].global_sequence);
    assert.equal(first.items.length, 20);
    assert.equal(result.next_before_sequence, order === "newest" ? rows[99].global_sequence : null);
  });
}

test("lookahead consumes only the tail post, leaves other posts for the next page, and detects EOF", async () => {
  const initial = page([image(10, 2), image(9, 1)], 9);
  const result = await completeCollectionPage(initial, async () => page([image(8, 1), image(7, 3)], null));
  assert.deepEqual(result.items.map(i => i.global_sequence), [10, 9, 8]);
  assert.equal(result.next_cursor_sequence, 8);
  const end = await completeCollectionPage(initial, async () => page([image(8, 1)], null));
  assert.equal(end.next_cursor_sequence, null);
});

test("refresh replacement, stale requests and nonadvancing cursors cannot append duplicated items", async () => {
  const initial = page([image(3), image(2)], 2);
  const repeated = await completeCollectionPage(initial, async () => page([image(2)], 2));
  assert.equal(repeated.items.length, 2);
  const stale = await completeCollectionPage(initial, async () => { throw new Error("unexpected request"); }, () => false);
  assert.deepEqual(stale, initial);
  const withoutPost = page([{ ...image(3), post_sequence: null }], 3);
  assert.deepEqual(await completeCollectionPage(withoutPost, async () => { throw new Error("unexpected request"); }), withoutPost);
});
