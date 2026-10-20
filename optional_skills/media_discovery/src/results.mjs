import { readRecords, readState, storageRoot } from "./storage.mjs";

export function resultOwner(request) {
  const value = request?.user_id;
  if (Number.isSafeInteger(value)) return String(value);
  if (typeof value !== "string" || !/^-?(?:0|[1-9]\d*)$/u.test(value)) return null;
  const id = BigInt(value);
  return id >= -(2n ** 63n) && id < 2n ** 63n ? String(id) : null;
}

function pageInteger(value, fallback, minimum) {
  if (value === undefined) return fallback;
  if (!Number.isSafeInteger(value) || value < minimum) throw new Error("invalid_args");
  return value;
}

export function resultPage(records, args = {}) {
  const limit = pageInteger(args.limit, 20, 1);
  const offset = pageInteger(args.offset, 0, 0);
  const posts = new Map();
  for (const record of records) {
    const key = `${record.platform}:${record.item_id || record.global_sequence}`;
    if (posts.has(key)) { posts.get(key).record_count += 1; continue; }
    posts.set(key, { global_sequence: record.global_sequence, platform: record.platform,
      item_id: record.item_id, kind: record.kind, record_count: 1,
      ...(record.title ? { title: record.title } : {}),
      engagement: record.engagement || { schema_version: 1, platform: record.platform, metrics: {} } });
  }
  const results = [...posts.values()].slice(offset, offset + limit);
  const nextOffset = offset + results.length;
  return { results, result_page: { offset, limit, returned: results.length, total: posts.size,
    has_more: nextOffset < posts.size, next_offset: nextOffset < posts.size ? nextOffset : null } };
}

export async function capturedRunResults(root, run, request) {
  const records = (await readRecords(root)).filter(record => record.run_id === run.run_id);
  const page = resultPage(records);
  return { ...page, filter_report: run.filter_report || [], result_query: resultOwner(request) == null ? null : {
    capability: "media_discovery.get_run_results", args: { run_id: run.run_id, limit: page.result_page.limit,
      offset: page.result_page.next_offset ?? 0 },
  } };
}

export async function getRunResults(request, args) {
  const owner = resultOwner(request);
  if (owner == null) throw new Error("result_actor_required");
  if (typeof args.run_id !== "string" || !/^run_[a-f0-9-]{36}$/u.test(args.run_id)) throw new Error("invalid_args");
  const root = storageRoot(request);
  const state = await readState(root);
  const run = [...(state.runs || []), ...Object.values(state.active_runs || {})]
    .find(item => item.run_id === args.run_id && item.owner_user_id === owner);
  if (!run) throw new Error("result_run_not_found");
  const records = (await readRecords(root)).filter(record => record.run_id === args.run_id && record.owner_user_id === owner);
  return { run_id: run.run_id, state: run.status || run.lifecycle_state,
    counts: run.counts, filter_report: run.filter_report || [],
    collection_outcome: run.collection_outcome, capture_summary: run.capture_summary,
    ...resultPage(records, args), side_effect_applied: false };
}
