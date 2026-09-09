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

export const COLLECTION_PAGE_SIZE = 20;

// Page by posts, not image rows. Finish the boundary gallery, but leave the next
// post unconsumed so both time orders use the same stable server row cursor.
export async function completeCollectionPage(
  initial: AippMediaPageResponse,
  loadNext: (cursor: number) => Promise<AippMediaPageResponse>,
  isCurrent: () => boolean = () => true,
  cardLimit = COLLECTION_PAGE_SIZE,
): Promise<AippMediaPageResponse> {
  if (!isCurrent()) return initial;
  const page = { ...initial, items: [] as AippMediaItem[] };
  const groups = new Set<string>();
  const seen = new Set<number>();
  const cursors = new Set<number>();
  const limit = Math.max(1, Math.min(COLLECTION_PAGE_SIZE, cardLimit));
  let batch = initial;
  // Bound remote/malformed feeds and the work of each automatic refresh.
  batches: for (let request = 0; request <= 20 && isCurrent(); request++) {
    for (const item of batch.items) {
      const key = collectionGroupKey(item);
      if ((!groups.has(key) && groups.size >= limit) || seen.has(item.global_sequence)) break batches;
      groups.add(key);
      seen.add(item.global_sequence);
      page.items.push(item);
      page.next_cursor_sequence = item.global_sequence;
    }
    page.next_cursor_sequence = batch.next_cursor_sequence;
    const tail = page.items.at(-1);
    const cursor = batch.next_cursor_sequence;
    if (cursor == null || !batch.items.length || cursors.has(cursor) || request === 20) break;
    if (groups.size >= limit && (tail?.kind !== "image" || !tail.post_sequence)) break;
    cursors.add(cursor);
    batch = await loadNext(cursor);
  }
  page.next_before_sequence = page.sort_order === "newest" ? page.next_cursor_sequence : null;
  return page;
}
