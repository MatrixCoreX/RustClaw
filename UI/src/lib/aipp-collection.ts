import type { AippMediaItem, AippMediaPageResponse } from "../types/api";

export function collectionGroupKey(item: AippMediaItem): string {
  return item.kind === "image" && Number.isSafeInteger(item.post_sequence) && item.post_sequence! > 0
    ? `${item.platform}:post:${item.post_sequence}`
    : `${item.platform}:${item.kind}:${item.global_sequence}`;
}

export function groupCollectionItems(items: AippMediaItem[]): AippMediaItem[][] {
  const groups = new Map<string, AippMediaItem[]>();
  const seen = new Set<number>();
  for (const item of items) {
    if (seen.has(item.global_sequence)) continue;
    seen.add(item.global_sequence);
    const key = collectionGroupKey(item);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push(item);
  }
  return [...groups.values()].map(group => group.sort((a, b) =>
    (a.image_sequence ?? a.global_sequence) - (b.image_sequence ?? b.global_sequence)
    || a.global_sequence - b.global_sequence));
}

// Finish a contiguous gallery before advancing the row cursor. Lookahead rows belonging
// to the next post are not consumed; the next page still requests them from the API.
export async function completeCollectionPage(
  initial: AippMediaPageResponse,
  loadNext: (cursor: number) => Promise<AippMediaPageResponse>,
  isCurrent: () => boolean = () => true,
): Promise<AippMediaPageResponse> {
  const page = { ...initial, items: [...initial.items] };
  const tail = page.items.at(-1);
  if (!tail || tail.kind !== "image" || !tail.post_sequence) return page;
  const key = collectionGroupKey(tail);
  const seen = new Set(page.items.map(item => item.global_sequence));
  // Normal skill galleries contain at most 100 images; bound malformed/remote feeds too.
  for (let request = 0; request < 20 && page.next_cursor_sequence != null && isCurrent(); request++) {
    const cursor = page.next_cursor_sequence;
    const next = await loadNext(cursor);
    let consumed = 0;
    for (const item of next.items) {
      if (collectionGroupKey(item) !== key) break;
      if (seen.has(item.global_sequence)) break;
      seen.add(item.global_sequence);
      page.items.push(item);
      page.next_cursor_sequence = item.global_sequence;
      consumed++;
    }
    if (consumed === next.items.length && next.next_cursor_sequence == null) page.next_cursor_sequence = null;
    if (consumed !== next.items.length || consumed === 0) break;
    if (page.next_cursor_sequence === cursor) break;
  }
  page.next_before_sequence = page.sort_order === "newest" ? page.next_cursor_sequence : null;
  return page;
}
