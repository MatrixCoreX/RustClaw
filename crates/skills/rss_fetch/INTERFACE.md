# rss_fetch Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the rss_fetch implementation.

## Capability Summary
- `rss_fetch` reads RSS/Atom feeds and returns normalized news items.
- Successful responses include user-visible `text` plus machine-readable `extra` evidence. Runtime quality checks consume `extra.field_value` / `extra.items`; do not require downstream code to parse localized `text`.
- **Category guardrails (planner / agent)**: For `latest` and `news`, `category` must be a runtime category token returned by `list_categories`. Do not invent a token or maintain language-specific alias tables. When no configured category fits, use `propose_category` with evidence-backed candidates; use `preview_category` for one-time results, and only use `promote_category` after explicit confirmation.
- **Missing-category workflow**: Call `list_categories` first. If no category fits, use an available browsing/search capability (or user-provided evidence) to collect RSS/Atom candidate URLs and a public `discovered_from` page for each candidate, then call `propose_category`. If evidence cannot be obtained, do not fabricate URLs or silently substitute an unrelated category.
- **`fetch`** is **direct-feed only**: one or more explicit `http(s)` URLs. It does **not** fall back to category/config sources.
- **`latest`** and **`news`** use **category mode**: all **active** sources for the category (from config) are fetched by default. Same merge/dedupe/sort behavior; `news` is an alias of `latest` (default category for `news` when omitted follows config / `general` as documented below).
- **Category semantics**: A category uses a single list of sources; all listed sources are fetched by default (no primary/fallback tiers). Single-source failure is skipped; only when all sources fail (or return no items) does the skill return an error.
- **Topic semantics**: `extra.items[].topic` is a stable machine token from `args.topic` / `args.topic_token` or `[rss.categories.<name>].topic`. The skill must not classify titles with language keyword lists; if no machine topic is configured, use `other`.
- **Deprecated sources**: Default fetch uses only active sources. Sources that consecutively fail (e.g. `deprecate_after_failures = 3` in config) are removed from the operator-owned active list and recorded in `rss_fetch` private state; success on a source resets its failure count.
- **Discovery ownership**: The model may propose evidence-backed `url` + `discovered_from` pairs, but deterministic public-network/feed validation stores them only as candidates; the skill never calls an LLM.
- **Candidate lifecycle**: `candidate` becomes `eligible` after repeated `refresh_candidates` success, `quarantined` after repeated failure, and active only through confirmed `promote_sources`. Trigger model discovery only when scheduled/user-requested `source_health` returns `needs_discovery=true`.

## Config Entry Points
- Main RSS config: `configs/rss.toml`.
- Category source lists: `configs/rss.toml` -> `[rss.categories.<name>]`.
- Defaults: `rss.default_category`, `rss.default_limit`, and `rss.timeout_seconds`.
- Optional category topic token: `[rss.categories.<name>].topic`, a lowercase machine token such as `macro_market`, `tech_ecosystem`, or `other`.
- Discovery policy: `[rss.discovery]` controls enablement, minimum active sources, required validation successes, candidate-pool size, and quarantine threshold.
- Private machine state: the registry declares `storage = { kind = "sqlite", schema_version = 1, migration_owner = "rss_fetch" }`; `skill-runner` supplies `context.skill_storage`, which resolves to this skill's own `data/skills/rss_fetch/state.db`.
- `configs/rss.toml` contains only operator-owned policy and active sources. Candidate records, validation evidence, source failure/health counters, and deprecation history must not be stored there.

## Actions
- `fetch` — direct RSS/Atom URL(s) only; requires `url` or `feed_url` or `feed_urls`.
- `list_categories` — returns configured category tokens, the configured default, and pending category proposals without network access.
- `latest` — category-based; uses configured sources for `category` (or default category).
- `news` — same pipeline as `latest` (alias); default `category` when omitted is typically `general` per config.
- `propose_category` — validates model-proposed `url` + `discovered_from` candidates for a category that does not exist and saves accepted candidates only in skill-private state.
- `preview_category` — temporarily fetches a pending category's validated feeds without activating the category.
- `promote_category` — revalidates a pending category and writes it to active config only with `confirm=true` and enough valid sources.
- `source_health` / `discover_sources` — inspect lifecycle counts, or validate and store evidence-backed proposals without activating them.
- `refresh_candidates` / `promote_sources` — revalidate candidate state, or activate eligible sources with `confirm=true` plus high-risk runtime approval.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | no* | string | `latest` | One of `fetch`, `latest`, `news`, `list_categories`, `source_health`, `propose_category`, `preview_category`, `promote_category`, `discover_sources`, `refresh_candidates`, or `promote_sources`. If omitted, behavior is **`latest`**. |
| `list_categories` | - | - | - | - | Returns the current machine category catalog; call this when the category token is uncertain. |
| `fetch` | `url` or `feed_url` or `feed_urls` | yes | string/array | - | **At least one** http(s) feed URL. `feed_urls`: JSON array of strings; empty or all-invalid → error. |
| `fetch` | `limit` | no | number | impl default | Per-feed item cap (single URL). |
| `fetch` | `timeout_seconds` | no | number | impl default | Request timeout override. |
| `fetch` | `topic` / `topic_token` | no | string | `other` | Stable lowercase machine topic token for `extra.items[].topic`; do not pass user-language phrases. |
| `latest` | `category` | no | string | impl default | Must be a token returned by `list_categories`; all **active** sources for that category are fetched. If no category fits, follow the missing-category proposal workflow instead of substituting an unrelated category. |
| `latest` | `limit` | no | number | impl default | Maximum returned items (applied after merge/dedupe/sort). |
| `latest` | `timeout_seconds` | no | number | impl default | Request timeout override. |
| `latest` | `topic` / `topic_token` | no | string | category config / `other` | Stable lowercase machine topic override for `extra.items[].topic`; invalid sentence-like values are ignored. |
| `latest` | `url` / `feed_url` / `feed_urls` | no | string/array | - | Optional: if provided, fetches **only** these URLs (explicit list) instead of category config; still uses `latest` merge/deprecation rules for **non-explicit** category fetches only — when using explicit URLs, deprecation state is not updated. |
| `news` | `category` | no | string | config default | Same category catalog and missing-category workflow as `latest`. |
| `news` | `limit` | no | number | impl default | Same as `latest`. |
| `news` | `timeout_seconds` | no | number | impl default | Same as `latest`. |
| `news` | `topic` / `topic_token` | no | string | category config / `other` | Same topic-token rule as `latest`. |
| `propose_category` | `category` | yes | string | - | New lowercase machine token matching `[a-z0-9][a-z0-9_]{0,63}`; never pass user-language prose. |
| `propose_category` | `candidates` | yes | array<object> | - | Up to 10 `{url, discovered_from}` records. The model proposes; deterministic public-network and feed validation accepts or rejects. |
| `propose_category` | `topic` / `topic_token` | no | string | category token | Stable machine topic token. |
| `propose_category` | `output_language` / `bilingual_summary` | no | string/bool | unset | Optional output policy saved with the pending proposal. |
| `preview_category` | `category` | yes | string | - | Existing pending category token; returns temporary results and does not change active config. |
| `preview_category` | `limit` / `timeout_seconds` | no | number | impl default | Temporary fetch bounds. |
| `promote_category` | `category` | yes | string | - | Existing pending category token. |
| `promote_category` | `confirm` | yes | boolean | `false` | Must be exactly `true`; high-risk runtime approval still applies. |
| `promote_category` | `urls` | no | array<string> | all validated candidates | Optional validated candidate subset; the final subset must still meet the configured minimum source count. |
| `source_health` | `category` | no | string | all categories | Configured category token; omission returns all category health records. |
| discovery mutations | `category` | yes* | string | config default | Existing category token for `discover_sources`, `refresh_candidates`, or `promote_sources`. |
| `discover_sources` | `candidates` | yes | array<object> | - | Maximum 10 objects per call. Each requires a proposed feed `url` and a public `discovered_from` evidence URL. |
| `discover_sources` | `timeout_seconds` | no | number | 15 | Per-candidate deterministic validation timeout, clamped to 3–60 seconds. |
| `refresh_candidates` | `urls` | no | array<string> | all non-promoted candidates | Optional candidate subset. |
| `promote_sources` | `urls` | yes | array<string> | - | Eligible candidate URLs to revalidate and activate. |
| `promote_sources` | `confirm` | yes | boolean | `false` | Must be exactly `true`; runtime high-risk confirmation still applies. |

## Config (configs/rss.toml)
- `[rss.categories.<name>]`: each **`<name>`** is a valid `category` value for `latest` / `news`. Use `list_categories` as the planner-visible runtime catalog rather than copying these keys into a second static list.
- `[rss.categories.<name>].topic`: optional stable topic token used for grouping and `extra.items[].topic`; this replaces title-keyword classification and keeps topic behavior language-neutral.
- `rss.default_category` / `rss.default_limit` / `rss.timeout_seconds`: defaults when args omit them.
- `rss.deprecate_after_failures`: number of consecutive failures before a source is moved to deprecated (default 3).
- `[rss.discovery]`: `enabled`, `min_active_sources`, `promotion_successes`, `max_candidates_per_category`, and `quarantine_after_failures`.
- Private SQLite state contains candidate metadata, evidence URL, first/last check, success/failure counters, lifecycle status, sample titles, promotion time, source health, and deprecation history.
- Legacy `source_entries`, `candidate_entries`, and `rss.deprecated` data is migrated once into private storage, verified by row count and digest, and then removed from operator config.
- Operator config updates use a short file lock, stale-snapshot rejection, and atomic replacement. Private-state updates use an immediate SQLite transaction, stale-state digest rejection, and an integrity check.

## Error Contract
- Unknown or unconfigured `category` (no entry under `[rss.categories]` or no active sources) returns readable `error_text` plus machine fields in `extra`: `error_kind=category_not_configured`, `failure_phase=pre_dispatch`, `side_effect_applied=false`, `recovery_action=replan_arguments`, `invalid_argument=category`, `rejected_value`, `default_category`, and sorted `available_categories`. Runtime recovery must consume these fields, not parse `error_text`.
- Every `status=error` response includes `extra.error_code` and `extra.message_key`; the strict skill protocol rejects and replaces errors that omit either field.
- Category proposals require at least one valid feed to persist. `ready_for_promotion` becomes true only at the configured minimum active-source count. Promotion always revalidates and fails without changing active config when too few sources remain valid.
- If one proposal does not reach the minimum, accepted candidates remain in private state and a later task may add different evidence-backed candidates. Each task gets one bounded proposal batch of at most 10 candidates.
- Pending category proposals are stored in the `rss_fetch` private SQLite state. They are excluded from `configs/rss.toml` until confirmed promotion.
- `action` outside the canonical action set returns `rss_fetch.unsupported_action`.
- **`fetch`** without `url`/`feed_url`/non-empty valid `feed_urls`, or with non-http(s) URLs → clear `error_text` (e.g. `fetch requires url, feed_url, or feed_urls`).
- Empty/invalid URL values for `fetch`.
- URLs with credentials, localhost/private/reserved IPs, private DNS results, unsafe redirects, excessive redirects, or bodies over 4 MiB are rejected by stable machine error codes.
- HTTP success without at least one parseable RSS/Atom item is a source failure (`no_parseable_feed_items`), not a successful health check.
- Discovery errors use machine `error_kind` and per-candidate `error_code`; runtime/agent recovery must not parse `error_text`.
- Operator-config conflicts/failures return `error_kind=config_persist_failed`; private-state failures return `error_kind=skill_storage_failed`. Both include `failure_phase`, truthful `side_effect_applied`, and a stable `cause_code`.
- For `latest`/`news`: only when **all** configured sources for the category fail or return no items does the skill return an error. Partial success returns the successfully fetched items plus a summary (e.g. sources_ok / sources_failed / items).

## Success Response Extra
- `extra.schema_version`: number, currently `1`.
- `extra.action`: canonical action (`fetch`, `latest`, or `news` alias normalized to `latest` internally).
- `extra.mode`: `direct`, `category`, or `explicit_urls`.
- `extra.field_value`: object containing stable execution counters such as `sources_ok`, `sources_failed`, and `items` / `item_count`, plus a compact `titles` array for grounding brief headline answers before evidence truncation.
- `extra.items`: array of normalized feed item objects with `title`, `link`, `date`, `source`, `source_host`, `layer`, and `topic`.
- Discovery responses expose `results[]` with URL, lifecycle status, validation counters, sample titles, and machine error code. `source_health` exposes per-category `active_count`, `candidate_count`, `eligible_count`, `quarantined_count`, and `recommended_action`.
- Generic evidence extractors treat `extra.items` as candidate/list evidence; do not duplicate the same item array under another key.
- `text` remains the localized, user-visible news listing. Consumers must use `extra` for machine evidence instead of parsing `text`.

## Request/Response Examples
### Example 1 (category latest)
Request:
```json
{"request_id":"demo-1","args":{"action":"latest","category":"crypto","limit":5}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"sources_ok=3 sources_failed=0 items=5\n1) ...\n2) ...","extra":{"schema_version":1,"action":"latest","category":"crypto","mode":"category","sources_ok":3,"sources_failed":0,"item_count":5,"field_value":{"sources_ok":3,"sources_failed":0,"items":5,"titles":["..."]},"items":[{"title":"...","link":"https://example.com/news","source_host":"example.com","layer":"feed","topic":"macro_market"}]},"error_text":null}
```

### Example 2 (direct fetch)
Request:
```json
{"request_id":"demo-2","args":{"action":"fetch","url":"https://example.com/feed.xml","limit":10}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"...","extra":{"schema_version":1,"action":"fetch","mode":"direct","source_count":1,"item_count":10,"field_value":{"source_count":1,"item_count":10,"titles":["..."]},"items":[{"title":"...","link":"https://example.com/item","source_host":"example.com","layer":"feed","topic":"other"}]},"error_text":null}
```

### Example 3 (evidence-backed discovery)
Request:
```json
{"request_id":"demo-3","args":{"action":"discover_sources","category":"general","candidates":[{"url":"https://publisher.example/feed.xml","discovered_from":"https://publisher.example/news"}]}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"candidates_valid=1 candidates_rejected=0","extra":{"schema_version":1,"action":"discover_sources","category":"general","accepted_count":1,"results":[{"url":"https://publisher.example/feed.xml","status":"candidate","success_count":1,"required_successes":3,"error_code":null}],"promotion_requires_confirmation":true},"error_text":null}
```
