import { recordPostIdentity } from "./media_identity.mjs";

// Zero/omitted means no user-requested ceiling, not a very large hidden quota.
export function optionalLimit(value) {
  return value > 0 ? value : Infinity;
}

// Stop a stalled/end-of-results page, not a healthy traversal after N scrolls.
export function pageProgress() {
  const observed = new Set();
  let unchanged = 0;
  return (identities) => {
    let added = false;
    for (const identity of identities) {
      if (!observed.has(identity)) { observed.add(identity); added = true; }
    }
    unchanged = added ? 0 : unchanged + 1;
    return unchanged < 3;
  };
}

export function completedDiscoveryPosts(records, platform, config) {
  const incomplete = new Set(records.filter(record => record.collection_truncated).map(recordPostIdentity));
  return new Set(records.filter(record => record.platform === platform
    && record.source_mode === config.source_mode
    && (config.source_mode !== "topics" || config.topics.includes(record.search_keyword))
    && !record.collection_truncated)
    // A truncated gallery is not a completed post, including its earlier images.
    .map(recordPostIdentity).filter(identity => identity && !incomplete.has(identity)));
}

export function rememberCompletedPost(completed, records) {
  if (!records.length || records.some(record => record.collection_truncated)) return;
  for (const record of records) {
    const identity = recordPostIdentity(record);
    if (identity) completed.add(identity);
  }
}

// Consume each currently rendered page before scrolling. This works with
// virtualized lists; never collect a giant URL list and then reopen stale cards.
export async function collectOrderedPages({ readCandidates, identity, collect,
  scroll, shouldStop, limit, maxScrolls = 0, completed = new Set(),
  onFailure, stopsCollection }) {
  const seen = new Set();
  const progressing = pageProgress();
  let handled = 0;
  let scrolls = 0;
  let skipped = 0;
  let failures = 0;
  let stopReason = "source_exhausted";
  while (handled < limit && !await shouldStop()) {
    const candidates = await readCandidates();
    if (!progressing(candidates.map(identity))) { stopReason = "no_new_results"; break; }
    for (const candidate of candidates) {
      if (handled >= limit || await shouldStop()) break;
      const key = identity(candidate);
      if (seen.has(key)) continue;
      seen.add(key);
      if (completed.has(key)) { skipped += 1; continue; }
      try {
        const result = await collect(candidate);
        if (result?.skipped) skipped += 1;
        else handled += 1;
      } catch (error) {
        failures += 1;
        await onFailure?.(error);
        if (stopsCollection(error)) throw error;
      }
    }
    if (handled >= limit || await shouldStop()) break;
    if (scrolls >= optionalLimit(maxScrolls)) { stopReason = "requested_scroll_limit"; break; }
    await scroll();
    scrolls += 1;
  }
  if (handled >= limit) stopReason = "target_reached";
  else if (await shouldStop()) stopReason = "collection_stopped";
  return { handled, skipped, failures, scrolls, stop_reason: stopReason };
}
