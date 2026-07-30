use std::collections::HashSet;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{
    all_sources_with_state_for_category, feed_item_extra, feed_item_titles, now_iso_secs,
    parse_feed_items, RootConfig, SkillFailure, SkillOutput, SourceStateEntry,
};

const DEFAULT_MIN_ACTIVE_SOURCES: usize = 3;
const DEFAULT_PROMOTION_SUCCESSES: u32 = 3;
const DEFAULT_MAX_CANDIDATES_PER_CATEGORY: usize = 20;
const DEFAULT_QUARANTINE_FAILURES: u32 = 3;
const MAX_DISCOVERY_BATCH: usize = 10;
const MAX_FEED_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct RssDiscoveryConfig {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    min_active_sources: Option<usize>,
    #[serde(default)]
    promotion_successes: Option<u32>,
    #[serde(default)]
    max_candidates_per_category: Option<usize>,
    #[serde(default)]
    quarantine_after_failures: Option<u32>,
}

impl RssDiscoveryConfig {
    fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    fn min_active_sources(&self) -> usize {
        self.min_active_sources
            .unwrap_or(DEFAULT_MIN_ACTIVE_SOURCES)
            .clamp(1, 100)
    }

    fn promotion_successes(&self) -> u32 {
        self.promotion_successes
            .unwrap_or(DEFAULT_PROMOTION_SUCCESSES)
            .clamp(2, 20)
    }

    fn max_candidates_per_category(&self) -> usize {
        self.max_candidates_per_category
            .unwrap_or(DEFAULT_MAX_CANDIDATES_PER_CATEGORY)
            .clamp(1, 200)
    }

    fn quarantine_after_failures(&self) -> u32 {
        self.quarantine_after_failures
            .unwrap_or(DEFAULT_QUARANTINE_FAILURES)
            .clamp(1, 20)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct CandidateSourceEntry {
    pub(super) url: String,
    #[serde(default)]
    pub(super) discovered_from: String,
    #[serde(default)]
    pub(super) first_seen_at: String,
    #[serde(default)]
    pub(super) last_checked_at: String,
    #[serde(default)]
    pub(super) success_count: u32,
    #[serde(default)]
    pub(super) failure_count: u32,
    #[serde(default)]
    pub(super) last_error: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) sample_titles: Vec<String>,
    #[serde(default)]
    pub(super) promoted_at: String,
}

#[derive(Debug)]
pub(super) struct ValidatedFeed {
    pub(super) sample_titles: Vec<String>,
    pub(super) item_count: usize,
}

pub(super) fn source_health(
    cfg: &RootConfig,
    args: &Map<String, Value>,
) -> Result<SkillOutput, SkillFailure> {
    let settings = discovery_settings(cfg);
    let requested = optional_category(cfg, args)?;
    let mut names = if let Some(category) = requested {
        vec![category]
    } else {
        let mut names = cfg.rss.categories.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    };
    names.dedup();

    let mut category_results = Vec::with_capacity(names.len());
    let mut any_needs_discovery = false;
    for category in names {
        let active_count = all_sources_with_state_for_category(cfg, &category).len();
        let entries = cfg
            .rss
            .categories
            .get(&category)
            .and_then(|value| value.candidate_entries.as_ref())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let candidate_count = entries
            .iter()
            .filter(|entry| normalized_candidate_status(entry) == "candidate")
            .count();
        let eligible_count = entries
            .iter()
            .filter(|entry| normalized_candidate_status(entry) == "eligible")
            .count();
        let quarantined_count = entries
            .iter()
            .filter(|entry| normalized_candidate_status(entry) == "quarantined")
            .count();
        let needs_discovery = settings.enabled() && active_count < settings.min_active_sources();
        any_needs_discovery |= needs_discovery;
        category_results.push(json!({
            "category": category,
            "active_count": active_count,
            "candidate_count": candidate_count,
            "eligible_count": eligible_count,
            "quarantined_count": quarantined_count,
            "minimum_active_sources": settings.min_active_sources(),
            "needs_discovery": needs_discovery,
            "recommended_action": if needs_discovery {
                "discover_sources"
            } else if candidate_count > 0 {
                "refresh_candidates"
            } else if eligible_count > 0 {
                "promote_sources"
            } else {
                "none"
            },
        }));
    }

    let text = format!(
        "categories={} needs_discovery={}",
        category_results.len(),
        any_needs_discovery
    );
    Ok(SkillOutput {
        text,
        extra: Some(json!({
            "schema_version": 1,
            "action": "source_health",
            "discovery_enabled": settings.enabled(),
            "needs_discovery": any_needs_discovery,
            "categories": category_results,
        })),
    })
}

pub(super) fn discover_sources(
    cfg: &mut RootConfig,
    args: &Map<String, Value>,
) -> Result<SkillOutput, SkillFailure> {
    let settings = discovery_settings(cfg);
    if !settings.enabled() {
        return Err(discovery_failure(
            "source_discovery_disabled",
            false,
            json!({"action": "discover_sources"}),
        ));
    }
    let category = required_category(cfg, args)?;
    let proposed = parse_discovery_candidates(args)?;
    if proposed.len() > MAX_DISCOVERY_BATCH {
        return Err(discovery_failure(
            "candidate_batch_too_large",
            false,
            json!({
                "action": "discover_sources",
                "category": category,
                "maximum_candidates": MAX_DISCOVERY_BATCH,
                "received_candidates": proposed.len(),
            }),
        ));
    }
    let timeout_seconds = request_timeout(args);
    let active_urls = all_sources_with_state_for_category(cfg, &category)
        .into_iter()
        .map(|(url, _)| url)
        .collect::<HashSet<_>>();
    let category_config = cfg
        .rss
        .categories
        .get_mut(&category)
        .expect("rss_candidate_category_invariant");
    let mut entries = category_config.candidate_entries.take().unwrap_or_default();
    let mut results = Vec::with_capacity(proposed.len());
    let mut accepted_count = 0usize;
    let mut seen = HashSet::new();

    for (url, discovered_from) in proposed {
        if !seen.insert(url.clone()) {
            results.push(candidate_result_error(&url, "duplicate_candidate"));
            continue;
        }
        if active_urls.contains(&url) {
            results.push(json!({
                "url": url,
                "status": "already_active",
                "error_code": Value::Null,
            }));
            continue;
        }
        if let Err(error_code) = validate_public_url_syntax(&discovered_from) {
            results.push(candidate_result_error(
                &url,
                &format!("invalid_discovery_evidence:{error_code}"),
            ));
            continue;
        }
        let existing_index = entries.iter().position(|entry| entry.url == url);
        let active_candidate_count = entries
            .iter()
            .filter(|entry| normalized_candidate_status(entry) != "promoted")
            .count();
        if existing_index.is_none()
            && active_candidate_count >= settings.max_candidates_per_category()
        {
            results.push(candidate_result_error(&url, "candidate_pool_full"));
            continue;
        }
        match validate_feed_url(&url, timeout_seconds) {
            Ok(validated) => {
                let now = now_iso_secs();
                let entry = if let Some(index) = existing_index {
                    &mut entries[index]
                } else {
                    entries.push(CandidateSourceEntry {
                        url: url.clone(),
                        discovered_from: discovered_from.clone(),
                        first_seen_at: now.clone(),
                        ..CandidateSourceEntry::default()
                    });
                    entries.last_mut().expect("candidate was appended")
                };
                entry.discovered_from = discovered_from;
                entry.last_checked_at = now;
                entry.success_count = entry.success_count.saturating_add(1);
                entry.failure_count = 0;
                entry.last_error.clear();
                entry.sample_titles = validated.sample_titles.clone();
                entry.status = status_after_success(entry, settings.promotion_successes());
                accepted_count += 1;
                results.push(json!({
                    "url": url,
                    "status": entry.status,
                    "item_count": validated.item_count,
                    "success_count": entry.success_count,
                    "required_successes": settings.promotion_successes(),
                    "sample_titles": entry.sample_titles,
                    "error_code": Value::Null,
                }));
            }
            Err(error_code) => {
                results.push(candidate_result_error(&url, &error_code));
            }
        }
    }
    entries.sort_by(|left, right| left.url.cmp(&right.url));
    category_config.candidate_entries = if entries.is_empty() {
        None
    } else {
        Some(entries)
    };

    if accepted_count == 0 {
        return Err(discovery_failure(
            "no_valid_source_candidates",
            true,
            json!({
                "action": "discover_sources",
                "category": category,
                "results": results,
            }),
        ));
    }
    Ok(SkillOutput {
        text: format!(
            "candidates_valid={} candidates_rejected={}",
            accepted_count,
            results.len().saturating_sub(accepted_count)
        ),
        extra: Some(json!({
            "schema_version": 1,
            "action": "discover_sources",
            "category": category,
            "accepted_count": accepted_count,
            "rejected_count": results.len().saturating_sub(accepted_count),
            "results": results,
            "promotion_requires_confirmation": true,
        })),
    })
}

pub(super) fn refresh_candidates(
    cfg: &mut RootConfig,
    args: &Map<String, Value>,
) -> Result<SkillOutput, SkillFailure> {
    let settings = discovery_settings(cfg);
    let category = required_category(cfg, args)?;
    let timeout_seconds = request_timeout(args);
    let selected = optional_url_set(args, "urls")?;
    let category_config = cfg
        .rss
        .categories
        .get_mut(&category)
        .expect("rss_candidate_category_invariant");
    let entries = category_config.candidate_entries.as_mut().ok_or_else(|| {
        discovery_failure(
            "no_source_candidates",
            true,
            json!({"action": "refresh_candidates", "category": category}),
        )
    })?;
    let mut results = Vec::new();
    let mut refreshed_count = 0usize;

    for entry in entries.iter_mut() {
        if normalized_candidate_status(entry) == "promoted" {
            continue;
        }
        if let Some(ref selected) = selected {
            if !selected.contains(&entry.url) {
                continue;
            }
        }
        refreshed_count += 1;
        entry.last_checked_at = now_iso_secs();
        match validate_feed_url(&entry.url, timeout_seconds) {
            Ok(validated) => {
                entry.success_count = entry.success_count.saturating_add(1);
                entry.failure_count = 0;
                entry.last_error.clear();
                entry.sample_titles = validated.sample_titles;
                entry.status = status_after_success(entry, settings.promotion_successes());
                results.push(json!({
                    "url": entry.url,
                    "status": entry.status,
                    "success_count": entry.success_count,
                    "failure_count": entry.failure_count,
                    "item_count": validated.item_count,
                    "error_code": Value::Null,
                }));
            }
            Err(error_code) => {
                entry.failure_count = entry.failure_count.saturating_add(1);
                entry.last_error = error_code.clone();
                if entry.failure_count >= settings.quarantine_after_failures() {
                    entry.status = "quarantined".to_string();
                } else if entry.status.trim().is_empty() {
                    entry.status = "candidate".to_string();
                }
                results.push(json!({
                    "url": entry.url,
                    "status": entry.status,
                    "success_count": entry.success_count,
                    "failure_count": entry.failure_count,
                    "error_code": error_code,
                }));
            }
        }
    }

    if refreshed_count == 0 {
        return Err(discovery_failure(
            "no_matching_source_candidates",
            true,
            json!({
                "action": "refresh_candidates",
                "category": category,
                "requested_urls": selected.map(|values| values.into_iter().collect::<Vec<_>>()),
            }),
        ));
    }
    let eligible_count = entries
        .iter()
        .filter(|entry| normalized_candidate_status(entry) == "eligible")
        .count();
    Ok(SkillOutput {
        text: format!(
            "candidates_refreshed={} candidates_eligible={}",
            refreshed_count, eligible_count
        ),
        extra: Some(json!({
            "schema_version": 1,
            "action": "refresh_candidates",
            "category": category,
            "refreshed_count": refreshed_count,
            "eligible_count": eligible_count,
            "results": results,
            "promotion_requires_confirmation": true,
        })),
    })
}

pub(super) fn promote_sources(
    cfg: &mut RootConfig,
    args: &Map<String, Value>,
) -> Result<SkillOutput, SkillFailure> {
    if args.get("confirm").and_then(Value::as_bool) != Some(true) {
        return Err(discovery_failure(
            "source_promotion_confirmation_required",
            true,
            json!({
                "action": "promote_sources",
                "confirmation_field": "confirm",
                "confirmation_value": true,
            }),
        ));
    }
    let settings = discovery_settings(cfg);
    let category = required_category(cfg, args)?;
    let requested_urls = required_url_set(args, "urls")?;
    let timeout_seconds = request_timeout(args);
    let mut active = all_sources_with_state_for_category(cfg, &category)
        .into_iter()
        .map(|(url, _)| url)
        .collect::<Vec<_>>();
    let mut active_set = active.iter().cloned().collect::<HashSet<_>>();
    let category_config = cfg
        .rss
        .categories
        .get_mut(&category)
        .expect("rss_candidate_category_invariant");
    let entries = category_config.candidate_entries.as_mut().ok_or_else(|| {
        discovery_failure(
            "no_source_candidates",
            false,
            json!({"action": "promote_sources", "category": category}),
        )
    })?;
    let mut promoted = Vec::new();
    let mut rejected = Vec::new();

    for url in requested_urls {
        let Some(entry) = entries.iter_mut().find(|entry| entry.url == url) else {
            rejected.push(candidate_result_error(&url, "candidate_not_found"));
            continue;
        };
        if normalized_candidate_status(entry) != "eligible"
            || entry.success_count < settings.promotion_successes()
        {
            rejected.push(candidate_result_error(&url, "candidate_not_eligible"));
            continue;
        }
        match validate_feed_url(&url, timeout_seconds) {
            Ok(validated) => {
                if active_set.insert(url.clone()) {
                    active.push(url.clone());
                }
                entry.success_count = entry.success_count.saturating_add(1);
                entry.failure_count = 0;
                entry.last_error.clear();
                entry.last_checked_at = now_iso_secs();
                entry.promoted_at = entry.last_checked_at.clone();
                entry.sample_titles = validated.sample_titles;
                entry.status = "promoted".to_string();
                promoted.push(json!({
                    "url": url,
                    "status": "promoted",
                    "item_count": validated.item_count,
                    "promoted_at": entry.promoted_at,
                }));
            }
            Err(error_code) => {
                entry.failure_count = entry.failure_count.saturating_add(1);
                entry.last_error = error_code.clone();
                entry.last_checked_at = now_iso_secs();
                rejected.push(candidate_result_error(&url, &error_code));
            }
        }
    }
    category_config.sources = Some(active);
    ensure_active_source_state(category_config, &promoted);

    if promoted.is_empty() {
        return Err(discovery_failure(
            "no_sources_promoted",
            true,
            json!({
                "action": "promote_sources",
                "category": category,
                "rejected": rejected,
            }),
        ));
    }
    Ok(SkillOutput {
        text: format!(
            "sources_promoted={} sources_rejected={}",
            promoted.len(),
            rejected.len()
        ),
        extra: Some(json!({
            "schema_version": 1,
            "action": "promote_sources",
            "category": category,
            "promoted_count": promoted.len(),
            "rejected_count": rejected.len(),
            "promoted": promoted,
            "rejected": rejected,
        })),
    })
}

pub(super) fn fetch_public_feed_xml(raw_url: &str, timeout_seconds: u64) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds.clamp(3, 60)))
        .redirect(Policy::none())
        .build()
        .map_err(|error| format!("http_client_build_failed:{error}"))?;
    let mut current = parse_public_url(raw_url)?;

    for redirect_count in 0..=MAX_REDIRECTS {
        ensure_public_resolution(&current)?;
        let mut response = client
            .get(current.clone())
            .header("User-Agent", "Agent-System-RSS-Fetch/1.1")
            .send()
            .map_err(|error| format!("http_request_failed:{error}"))?;
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err("redirect_limit_exceeded".to_string());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| "redirect_location_missing".to_string())?
                .to_str()
                .map_err(|_| "redirect_location_invalid".to_string())?;
            let next = current
                .join(location)
                .map_err(|_| "redirect_url_invalid".to_string())?;
            current = parse_public_url(next.as_str())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("http_status_{}", response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_FEED_BYTES)
        {
            return Err("feed_body_too_large".to_string());
        }
        let mut body = Vec::new();
        response
            .by_ref()
            .take(MAX_FEED_BYTES + 1)
            .read_to_end(&mut body)
            .map_err(|error| format!("response_body_read_failed:{error}"))?;
        if body.len() as u64 > MAX_FEED_BYTES {
            return Err("feed_body_too_large".to_string());
        }
        return Ok(String::from_utf8_lossy(&body).into_owned());
    }
    Err("redirect_limit_exceeded".to_string())
}

pub(super) fn validate_public_url_syntax(raw_url: &str) -> Result<(), String> {
    parse_public_url(raw_url).map(|_| ())
}

pub(super) fn validate_feed_url(url: &str, timeout_seconds: u64) -> Result<ValidatedFeed, String> {
    let body = fetch_public_feed_xml(url, timeout_seconds)?;
    validate_feed_document(&body)
}

fn validate_feed_document(body: &str) -> Result<ValidatedFeed, String> {
    let items = parse_feed_items(body, 10);
    if items.is_empty() {
        return Err("no_parseable_feed_items".to_string());
    }
    let topic = "other";
    let extra_items = items
        .iter()
        .map(|item| feed_item_extra(item, topic))
        .collect::<Vec<_>>();
    Ok(ValidatedFeed {
        sample_titles: feed_item_titles(&extra_items).into_iter().take(3).collect(),
        item_count: items.len(),
    })
}

fn discovery_settings(cfg: &RootConfig) -> RssDiscoveryConfig {
    cfg.rss.discovery.clone().unwrap_or_default()
}

fn optional_category(
    cfg: &RootConfig,
    args: &Map<String, Value>,
) -> Result<Option<String>, SkillFailure> {
    let category = args
        .get("category")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(ref category) = category {
        if !cfg.rss.categories.contains_key(category) {
            return Err(SkillFailure::category_not_configured(cfg, category));
        }
    }
    Ok(category)
}

fn required_category(cfg: &RootConfig, args: &Map<String, Value>) -> Result<String, SkillFailure> {
    let category = args
        .get("category")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            cfg.rss
                .default_category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| {
            discovery_failure(
                "category_required",
                true,
                json!({"invalid_argument": "category"}),
            )
        })?;
    if !cfg.rss.categories.contains_key(&category) {
        return Err(SkillFailure::category_not_configured(cfg, &category));
    }
    Ok(category)
}

pub(super) fn parse_discovery_candidates(
    args: &Map<String, Value>,
) -> Result<Vec<(String, String)>, SkillFailure> {
    let candidates = args
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            discovery_failure(
                "candidates_required",
                true,
                json!({"invalid_argument": "candidates"}),
            )
        })?;
    if candidates.is_empty() {
        return Err(discovery_failure(
            "candidates_required",
            true,
            json!({"invalid_argument": "candidates"}),
        ));
    }
    let mut parsed = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let object = candidate.as_object().ok_or_else(|| {
            discovery_failure(
                "candidate_object_required",
                true,
                json!({"invalid_argument": "candidates"}),
            )
        })?;
        let url = required_string(object, "url")?;
        let discovered_from = required_string(object, "discovered_from")?;
        parsed.push((url, discovered_from));
    }
    Ok(parsed)
}

fn required_string(object: &Map<String, Value>, field: &str) -> Result<String, SkillFailure> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            discovery_failure(
                "candidate_field_required",
                true,
                json!({"invalid_argument": field}),
            )
        })
}

fn required_url_set(
    args: &Map<String, Value>,
    field: &str,
) -> Result<HashSet<String>, SkillFailure> {
    optional_url_set(args, field)?.ok_or_else(|| {
        discovery_failure(
            "source_urls_required",
            true,
            json!({"invalid_argument": field}),
        )
    })
}

fn optional_url_set(
    args: &Map<String, Value>,
    field: &str,
) -> Result<Option<HashSet<String>>, SkillFailure> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    let array = value.as_array().ok_or_else(|| {
        discovery_failure(
            "source_urls_array_required",
            true,
            json!({"invalid_argument": field}),
        )
    })?;
    let values = array
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if values.is_empty() {
        return Err(discovery_failure(
            "source_urls_required",
            true,
            json!({"invalid_argument": field}),
        ));
    }
    Ok(Some(values))
}

pub(super) fn request_timeout(args: &Map<String, Value>) -> u64 {
    args.get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(15)
        .clamp(3, 60)
}

pub(super) fn minimum_active_sources(cfg: &RootConfig) -> usize {
    discovery_settings(cfg).min_active_sources()
}

fn status_after_success(entry: &CandidateSourceEntry, required_successes: u32) -> String {
    if entry.success_count >= required_successes {
        "eligible".to_string()
    } else {
        "candidate".to_string()
    }
}

fn normalized_candidate_status(entry: &CandidateSourceEntry) -> &str {
    match entry.status.trim() {
        "" => "candidate",
        value => value,
    }
}

fn candidate_result_error(url: &str, error_code: &str) -> Value {
    json!({
        "url": url,
        "status": "rejected",
        "error_code": error_code,
    })
}

fn discovery_failure(error_kind: &str, retryable: bool, fields: Value) -> SkillFailure {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": super::SKILL_NAME,
        "status": "error",
        "error_code": error_kind,
        "message_key": format!("skill.{}.{}", super::SKILL_NAME, error_kind),
        "retryable": retryable,
        "failure_phase": "source_discovery",
        "side_effect_applied": false,
    });
    if let (Some(target), Some(source)) = (extra.as_object_mut(), fields.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    SkillFailure {
        error_text: error_kind.to_string(),
        extra,
    }
}

fn ensure_active_source_state(category: &mut super::RssCategoryConfig, promoted: &[Value]) {
    let mut states = category.source_entries.take().unwrap_or_default();
    let known = states
        .iter()
        .map(|entry| entry.url.clone())
        .collect::<HashSet<_>>();
    for url in promoted
        .iter()
        .filter_map(|entry| entry.get("url"))
        .filter_map(Value::as_str)
    {
        if !known.contains(url) {
            states.push(SourceStateEntry {
                url: url.to_string(),
                failure_count: 0,
                last_error: String::new(),
                last_failed_at: String::new(),
            });
        }
    }
    states.sort_by(|left, right| left.url.cmp(&right.url));
    category.source_entries = if states.is_empty() {
        None
    } else {
        Some(states)
    };
}

fn parse_public_url(raw_url: &str) -> Result<Url, String> {
    let url = Url::parse(raw_url.trim()).map_err(|_| "invalid_url".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("unsupported_url_scheme".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("url_credentials_forbidden".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "url_host_required".to_string())?;
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    let normalized_ip = normalized
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(&normalized);
    if normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || normalized.ends_with(".internal")
        || normalized.ends_with(".home.arpa")
    {
        return Err("private_host_forbidden".to_string());
    }
    if let Ok(ip) = normalized_ip.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err("private_ip_forbidden".to_string());
        }
    }
    Ok(url)
}

fn ensure_public_resolution(url: &Url) -> Result<(), String> {
    let host = url
        .host_str()
        .ok_or_else(|| "url_host_required".to_string())?;
    let ip_host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = ip_host.parse::<IpAddr>() {
        return if is_public_ip(ip) {
            Ok(())
        } else {
            Err("private_ip_forbidden".to_string())
        };
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "url_port_unknown".to_string())?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("dns_resolution_failed:{error}"))?
        .map(|address| address.ip())
        .collect::<HashSet<_>>();
    if addresses.is_empty() {
        return Err("dns_resolution_empty".to_string());
    }
    if addresses
        .iter()
        .any(|ip| !is_public_ip(*ip) && !is_synthetic_egress_ip(*ip))
    {
        return Err("dns_private_address_forbidden".to_string());
    }
    Ok(())
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_synthetic_egress_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, _, _] = ip.octets();
            first == 198 && (second == 18 || second == 19)
        }
        IpAddr::V6(_) => false,
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 240)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    if ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
    {
        return false;
    }
    ip.to_ipv4().map(is_public_ipv4).unwrap_or(true)
}

#[cfg(test)]
#[path = "source_discovery_tests.rs"]
mod tests;
