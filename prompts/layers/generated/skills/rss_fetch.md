<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Interface Source
crates/skills/rss_fetch/INTERFACE.md

Read RSS/Atom. Select only registry-granted capabilities. Use `extra.items` for evidence, preserving each title, URL, source, date and excerpt. Summaries and translations must preserve attribution, time, modality, direction, uncertainty and degree. Use a comparative or superlative only when the source states that comparison or rank. Preserve quantities, currencies and units across number systems. A headline supports only its literal claims. If the excerpt lacks detail, report what the source says without adding explanations.

- `list_categories` returns active category tokens, the default and pending proposals offline. Choose a returned token for `latest` / `news` (alias). Pass the requested `limit` after merge, deduplication and sort. All active category feeds are fetched. Partial success supplies usable items. All-empty/all-failed is an error. Config provides the default category and limit.
- `fetch` requires explicit public HTTP(S) `url`, `feed_url` or `feed_urls`. `latest` also accepts explicit feed URLs to override configured sources without updating health. Optional `timeout_seconds` bounds fetching. `topic` / `topic_token` is a stable machine token from args or config, default `other`.
- Missing category: gather actual public feed URLs and discovery pages through available search or user evidence. `propose_category` takes a new lowercase machine `category`, up to ten `candidates` ({url, discovered_from}), optional topic, output_language and bilingual_summary. Without evidence, report the gap. `preview_category` reads a pending category (optional limit/timeout). `promote_category` activates it only with sufficient validated sources, explicit `confirm=true` and runtime approval; optional `urls` selects a subset.
- `source_health` accepts an optional category. Start scheduled/requested source discovery when it reports `needs_discovery=true`. `discover_sources` validates up to ten evidence-backed candidates in an existing category (timeout 3–60 seconds, default 15). `refresh_candidates` revalidates optional URLs. `promote_sources` needs eligible URLs, explicit confirmation and high-risk approval. Proposals remain private until promotion.
- Operator settings and active feeds live in `configs/rss.toml` under `rss`, `rss.categories` and `rss.discovery`. Health, candidate, quarantine and deprecation records use the runner-provided private SQLite store. Use structured errors and side-effect evidence for recovery. The skill rejects unsafe destinations and invalid feeds.

## Planner Selection Notes
- For general current-information or news research when the user does not name a source type, load both `rss_fetch` and `web_search_extract`. Call `list_categories`, fetch a matching configured category when one exists, run normal multi-source web search, then synthesize and deduplicate across both evidence sets.
- If the user explicitly requests RSS-only retrieval, stay within `rss_fetch`.
- If the user explicitly requests web-only search, a named search engine, or a specific website/domain, stay within web capabilities and do not add RSS.
- Never invent a category token merely to satisfy the dual-source policy. If `list_categories` has no matching category, preserve that observation and continue with web evidence only.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Interpret Chinese colloquial phrasing by capability semantics and requested task shape, not by a fixed phrase list.
- Judge Chinese delivery intent semantically: if the user asks to receive a file/result rather than inline body text, plan toward delivery without depending on fixed wording.
- Preserve Chinese brevity and format constraints as final output contracts when the skill can support them; do not convert those constraints into token-level matching rules.
- Treat Chinese style constraints as audience/tone constraints for the eventual explanation, not as skill-selection shortcuts.
- Resolve Chinese deictic references only from immediate, concrete, type-compatible context; do not guess unsupported targets or invent missing args just to force a skill call.
