export const contents = Array.from({length: 48}, (_, index) =>
  (`数据样本${index}:` + '离线内容校验'.repeat(18000)).slice(0, 18000));
const settings = {schema_version: 1, scope: 'principal', target_principal_id: 'fixture-owner',
  conversation_id: null, requested: {use_mode: 'disabled', generate_mode: 'disabled', external_context_policy: 'exclude'},
  use_memory: false, generate_memory: false, external_context_policy: 'exclude',
  use_source: 'principal', generate_source: 'principal', external_context_source: 'principal',
  managed_deny_reason: null, revision: 1, policy_digest: 'fixture-policy', restart_required: false};
const facts = contents.map((content, index) => ({id: `fixture-fact-${index}`, namespace: 'fixture',
  fact_key: `fixture-${index}`, fact_value: content, fact_text: content, confidence: 1,
  source_kind: 'memory_import', source_ref: 'fixture', reason: 'fixture', updated_at_ts: 1, status: 'active'}));
const items = contents.slice(0, 20).map((content, index) => ({id: `fixture-fact-${index}`, revision: 1,
  kind: 'fact', scope_kind: 'principal', origin: 'imported_legacy', status: 'active', content,
  source: 'memory_import', evidence_available: false, trust_tier: 'imported_legacy',
  updated_at_ts: 1, freshness: 'fresh'}));

export function memoryDisplayFixture(path) {
  switch (path) {
    case '/v1/auth/me': return {user_id: 1, chat_id: 2, role: 'user', user_key: 'fixture-ordinary-key'};
    case '/webd/login': return {csrf_token: 'ab'.repeat(16)};
    case '/v1/memory': return {long_term_enabled: false, hybrid_recall_enabled: false,
      counts: {recent: 0, preferences: 0, facts_active: 48, facts_total: 48, long_term_summaries: 0}};
    case '/v1/memory/settings': return settings;
    case '/v1/memory/preferences': case '/v1/memory/recent': return [];
    case '/v1/memory/facts': return facts;
    case '/v1/memory/items': return {schema_version: 1, page: 1, page_size: 20, total: 48, has_more: true, items};
    case '/v1/memory/remote-disclosure': return {schema_version: 1, consent_state: 'exclude',
      extraction_provider: 'fixture', extraction_model: 'fixture', consolidation_provider: 'fixture',
      consolidation_model: 'fixture', extraction_sends: [], embedding_sends: [], withdrawal_effect: 'stop'};
    case '/v1/memory/vector/status': return {schema_version: 1, provider_location: 'local', state: 'disabled',
      policy_digest: 'fixture-policy', active_generation: 1, queued_jobs: 0, running_jobs: 0,
      failed_jobs: 0, indexed_rows: 0, remote_consent: 'exclude'};
    default: return undefined;
  }
}
