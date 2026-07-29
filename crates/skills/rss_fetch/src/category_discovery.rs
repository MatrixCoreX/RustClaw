use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::source_discovery::{
    minimum_active_sources, parse_discovery_candidates, request_timeout, validate_feed_url,
    validate_public_url_syntax,
};
use super::{
    all_sources_with_state_for_category, fetch_layered_news, normalize_topic_token, now_iso_secs,
    CandidateSourceEntry, RootConfig, RssCategoryConfig, SkillFailure, SkillOutput, SKILL_NAME,
};

const MAX_CATEGORY_CANDIDATES_PER_REQUEST: usize = 10;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct PendingCategoryEntry {
    #[serde(default)]
    pub(super) topic: String,
    #[serde(default)]
    pub(super) output_language: Option<String>,
    #[serde(default)]
    pub(super) bilingual_summary: Option<bool>,
    #[serde(default)]
    pub(super) candidates: Vec<CandidateSourceEntry>,
    #[serde(default)]
    pub(super) proposed_at: String,
    #[serde(default)]
    pub(super) updated_at: String,
}

pub(super) fn list_categories(cfg: &RootConfig) -> Result<SkillOutput, SkillFailure> {
    let minimum_sources = minimum_active_sources(cfg);
    let mut names = cfg.rss.categories.keys().cloned().collect::<Vec<_>>();
    names.sort();
    let categories = names
        .into_iter()
        .map(|category| {
            let config = &cfg.rss.categories[&category];
            json!({
                "category": category,
                "topic": config.topic,
                "active_source_count": all_sources_with_state_for_category(cfg, &category).len(),
            })
        })
        .collect::<Vec<_>>();

    let mut pending_names = cfg
        .rss
        .pending_categories
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    pending_names.sort();
    let pending_categories = pending_names
        .into_iter()
        .map(|category| {
            let pending = &cfg.rss.pending_categories[&category];
            let validated_source_count = validated_candidates(pending).count();
            json!({
                "category": category,
                "topic": pending.topic,
                "validated_source_count": validated_source_count,
                "minimum_source_count": minimum_sources,
                "ready_for_promotion": validated_source_count >= minimum_sources,
            })
        })
        .collect::<Vec<_>>();

    Ok(SkillOutput {
        text: format!(
            "categories={} pending_categories={}",
            categories.len(),
            pending_categories.len()
        ),
        extra: Some(json!({
            "schema_version": 1,
            "action": "list_categories",
            "default_category": cfg.rss.default_category,
            "categories": categories,
            "pending_categories": pending_categories,
        })),
    })
}

pub(super) fn propose_category(
    cfg: &mut RootConfig,
    args: &Map<String, Value>,
) -> Result<SkillOutput, SkillFailure> {
    let category = required_category_token(args)?;
    if cfg.rss.categories.contains_key(&category) {
        return Err(category_failure(
            "category_already_configured",
            false,
            json!({
                "action": "propose_category",
                "category": category,
                "recommended_action": "latest",
            }),
        ));
    }
    let proposed = parse_discovery_candidates(args)?;
    if proposed.len() > MAX_CATEGORY_CANDIDATES_PER_REQUEST {
        return Err(category_failure(
            "candidate_batch_too_large",
            false,
            json!({
                "action": "propose_category",
                "category": category,
                "maximum_candidates": MAX_CATEGORY_CANDIDATES_PER_REQUEST,
                "received_candidates": proposed.len(),
            }),
        ));
    }

    let timeout_seconds = request_timeout(args);
    let now = now_iso_secs();
    let mut pending = cfg
        .rss
        .pending_categories
        .get(&category)
        .cloned()
        .unwrap_or_else(|| PendingCategoryEntry {
            topic: category.clone(),
            proposed_at: now.clone(),
            ..PendingCategoryEntry::default()
        });
    if let Some(topic) = optional_topic_token(args)? {
        pending.topic = topic;
    }
    if let Some(output_language) = optional_output_language(args)? {
        pending.output_language = Some(output_language);
    }
    if let Some(bilingual_summary) = args.get("bilingual_summary").and_then(Value::as_bool) {
        pending.bilingual_summary = Some(bilingual_summary);
    }
    pending.updated_at = now;

    let mut results = Vec::with_capacity(proposed.len());
    let mut accepted_count = 0usize;
    let mut seen = HashSet::new();
    for (url, discovered_from) in proposed {
        if !seen.insert(url.clone()) {
            results.push(candidate_error(&url, "duplicate_candidate"));
            continue;
        }
        if let Err(error_code) = validate_public_url_syntax(&discovered_from) {
            results.push(candidate_error(
                &url,
                &format!("invalid_discovery_evidence:{error_code}"),
            ));
            continue;
        }
        match validate_feed_url(&url, timeout_seconds) {
            Ok(validated) => {
                let checked_at = now_iso_secs();
                let entry = if let Some(entry) =
                    pending.candidates.iter_mut().find(|entry| entry.url == url)
                {
                    entry
                } else {
                    pending.candidates.push(CandidateSourceEntry {
                        url: url.clone(),
                        discovered_from: discovered_from.clone(),
                        first_seen_at: checked_at.clone(),
                        ..CandidateSourceEntry::default()
                    });
                    pending
                        .candidates
                        .last_mut()
                        .expect("pending category candidate was appended")
                };
                entry.discovered_from = discovered_from;
                entry.last_checked_at = checked_at;
                entry.success_count = entry.success_count.saturating_add(1);
                entry.failure_count = 0;
                entry.last_error.clear();
                entry.sample_titles = validated.sample_titles.clone();
                entry.status = "validated".to_string();
                accepted_count += 1;
                results.push(json!({
                    "url": url,
                    "status": "validated",
                    "item_count": validated.item_count,
                    "sample_titles": validated.sample_titles,
                    "error_code": Value::Null,
                }));
            }
            Err(error_code) => results.push(candidate_error(&url, &error_code)),
        }
    }
    pending
        .candidates
        .sort_by(|left, right| left.url.cmp(&right.url));
    let validated_source_count = validated_candidates(&pending).count();
    if validated_source_count == 0 {
        return Err(category_failure(
            "no_valid_category_sources",
            true,
            json!({
                "action": "propose_category",
                "category": category,
                "results": results,
                "recovery_action": "replan_arguments",
            }),
        ));
    }
    let minimum_source_count = minimum_active_sources(cfg);
    let ready_for_promotion = validated_source_count >= minimum_source_count;
    cfg.rss.pending_categories.insert(category.clone(), pending);

    Ok(SkillOutput {
        text: format!(
            "category_candidates_valid={} category_candidates_rejected={} ready_for_promotion={}",
            accepted_count,
            results.len().saturating_sub(accepted_count),
            ready_for_promotion
        ),
        extra: Some(json!({
            "schema_version": 1,
            "action": "propose_category",
            "category": category,
            "accepted_count": accepted_count,
            "rejected_count": results.len().saturating_sub(accepted_count),
            "validated_source_count": validated_source_count,
            "minimum_source_count": minimum_source_count,
            "ready_for_promotion": ready_for_promotion,
            "results": results,
            "temporary_preview_action": "preview_category",
            "promotion_requires_confirmation": true,
        })),
    })
}

pub(super) fn preview_category(
    cfg: &mut RootConfig,
    args: &Map<String, Value>,
) -> Result<SkillOutput, SkillFailure> {
    let category = required_category_token(args)?;
    let pending = cfg
        .rss
        .pending_categories
        .get(&category)
        .cloned()
        .ok_or_else(|| {
            category_replan_failure(
                "category_proposal_not_found",
                json!({
                    "action": "preview_category",
                    "category": category,
                    "recommended_action": "propose_category",
                }),
            )
        })?;
    let urls = validated_candidates(&pending)
        .map(|candidate| Value::String(candidate.url.clone()))
        .collect::<Vec<_>>();
    if urls.is_empty() {
        return Err(category_replan_failure(
            "category_proposal_has_no_valid_sources",
            json!({
                "action": "preview_category",
                "category": category,
                "recommended_action": "propose_category",
            }),
        ));
    }

    let mut preview_args = args.clone();
    preview_args.insert("action".to_string(), json!("latest"));
    preview_args.insert("category".to_string(), json!(category));
    preview_args.insert("feed_urls".to_string(), Value::Array(urls));
    if !preview_args.contains_key("topic") && !preview_args.contains_key("topic_token") {
        preview_args.insert("topic_token".to_string(), json!(pending.topic));
    }
    if !preview_args.contains_key("output_language") {
        if let Some(output_language) = pending.output_language {
            preview_args.insert("output_language".to_string(), json!(output_language));
        }
    }
    if !preview_args.contains_key("bilingual_summary") {
        if let Some(bilingual_summary) = pending.bilingual_summary {
            preview_args.insert("bilingual_summary".to_string(), json!(bilingual_summary));
        }
    }
    let mut output = fetch_layered_news(cfg, &preview_args)?;
    if let Some(extra) = output.extra.as_mut().and_then(Value::as_object_mut) {
        extra.insert("action".to_string(), json!("preview_category"));
        extra.insert("temporary".to_string(), json!(true));
        extra.insert("promotion_requires_confirmation".to_string(), json!(true));
    }
    Ok(output)
}

pub(super) fn promote_category(
    cfg: &mut RootConfig,
    args: &Map<String, Value>,
) -> Result<SkillOutput, SkillFailure> {
    if args.get("confirm").and_then(Value::as_bool) != Some(true) {
        return Err(category_failure(
            "category_promotion_confirmation_required",
            false,
            json!({
                "action": "promote_category",
                "confirmation_field": "confirm",
                "confirmation_value": true,
            }),
        ));
    }
    let category = required_category_token(args)?;
    if cfg.rss.categories.contains_key(&category) {
        return Err(category_failure(
            "category_already_configured",
            false,
            json!({
                "action": "promote_category",
                "category": category,
                "recommended_action": "latest",
            }),
        ));
    }
    let mut pending = cfg
        .rss
        .pending_categories
        .get(&category)
        .cloned()
        .ok_or_else(|| {
            category_replan_failure(
                "category_proposal_not_found",
                json!({
                    "action": "promote_category",
                    "category": category,
                    "recommended_action": "propose_category",
                }),
            )
        })?;
    let selected = selected_urls(args, &pending)?;
    let timeout_seconds = request_timeout(args);
    let mut promoted_sources = Vec::new();
    let mut rejected = Vec::new();
    for candidate in &mut pending.candidates {
        if !selected.contains(&candidate.url) {
            continue;
        }
        match validate_feed_url(&candidate.url, timeout_seconds) {
            Ok(validated) => {
                candidate.last_checked_at = now_iso_secs();
                candidate.success_count = candidate.success_count.saturating_add(1);
                candidate.failure_count = 0;
                candidate.last_error.clear();
                candidate.sample_titles = validated.sample_titles;
                candidate.status = "promoted".to_string();
                candidate.promoted_at = candidate.last_checked_at.clone();
                promoted_sources.push(candidate.url.clone());
            }
            Err(error_code) => rejected.push(candidate_error(&candidate.url, &error_code)),
        }
    }
    promoted_sources.sort();
    promoted_sources.dedup();
    let minimum_source_count = minimum_active_sources(cfg);
    if promoted_sources.len() < minimum_source_count {
        return Err(category_replan_failure(
            "category_promotion_insufficient_sources",
            json!({
                "action": "promote_category",
                "category": category,
                "validated_source_count": promoted_sources.len(),
                "minimum_source_count": minimum_source_count,
                "rejected": rejected,
                "recommended_action": "propose_category",
            }),
        ));
    }

    let output_language = optional_output_language(args)?.or(pending.output_language.clone());
    let bilingual_summary = args
        .get("bilingual_summary")
        .and_then(Value::as_bool)
        .or(pending.bilingual_summary);
    let topic = optional_topic_token(args)?
        .unwrap_or_else(|| pending.topic.clone())
        .trim()
        .to_string();
    cfg.rss.categories.insert(
        category.clone(),
        RssCategoryConfig {
            sources: Some(promoted_sources.clone()),
            candidate_entries: Some(pending.candidates),
            output_language,
            bilingual_summary,
            topic: Some(topic),
            ..RssCategoryConfig::default()
        },
    );
    cfg.rss.pending_categories.remove(&category);

    Ok(SkillOutput {
        text: format!(
            "category_promoted={} sources_promoted={}",
            category,
            promoted_sources.len()
        ),
        extra: Some(json!({
            "schema_version": 1,
            "action": "promote_category",
            "category": category,
            "status": "promoted",
            "source_count": promoted_sources.len(),
            "sources": promoted_sources,
            "rejected": rejected,
        })),
    })
}

fn validated_candidates(
    pending: &PendingCategoryEntry,
) -> impl Iterator<Item = &CandidateSourceEntry> {
    pending.candidates.iter().filter(|candidate| {
        matches!(candidate.status.as_str(), "validated" | "promoted")
            && candidate.last_error.is_empty()
    })
}

fn selected_urls(
    args: &Map<String, Value>,
    pending: &PendingCategoryEntry,
) -> Result<HashSet<String>, SkillFailure> {
    let available = validated_candidates(pending)
        .map(|candidate| candidate.url.clone())
        .collect::<HashSet<_>>();
    let Some(value) = args.get("urls") else {
        return Ok(available);
    };
    let urls = value.as_array().ok_or_else(|| {
        category_replan_failure(
            "source_urls_array_required",
            json!({"invalid_argument": "urls"}),
        )
    })?;
    let selected = urls
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if selected.is_empty() || !selected.is_subset(&available) {
        return Err(category_replan_failure(
            "category_candidate_urls_invalid",
            json!({
                "invalid_argument": "urls",
                "available_urls": available,
            }),
        ));
    }
    Ok(selected)
}

fn required_category_token(args: &Map<String, Value>) -> Result<String, SkillFailure> {
    let raw = args
        .get("category")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            category_replan_failure("category_required", json!({"invalid_argument": "category"}))
        })?;
    let category = raw.to_ascii_lowercase();
    let valid = category.len() <= 64
        && category
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && category.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if !valid {
        return Err(category_replan_failure(
            "category_token_invalid",
            json!({
                "invalid_argument": "category",
                "rejected_value": raw,
                "expected_pattern": "[a-z0-9][a-z0-9_]{0,63}",
            }),
        ));
    }
    Ok(category)
}

fn optional_topic_token(args: &Map<String, Value>) -> Result<Option<String>, SkillFailure> {
    let Some(raw) = args
        .get("topic")
        .or_else(|| args.get("topic_token"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    normalize_topic_token(raw).map(Some).ok_or_else(|| {
        category_replan_failure(
            "topic_token_invalid",
            json!({
                "invalid_argument": "topic_token",
                "rejected_value": raw,
            }),
        )
    })
}

fn optional_output_language(args: &Map<String, Value>) -> Result<Option<String>, SkillFailure> {
    let Some(raw) = args.get("output_language").and_then(Value::as_str) else {
        return Ok(None);
    };
    let value = raw.trim();
    let valid = !value.is_empty()
        && value.len() <= 32
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if !valid {
        return Err(category_replan_failure(
            "output_language_invalid",
            json!({
                "invalid_argument": "output_language",
                "rejected_value": raw,
            }),
        ));
    }
    Ok(Some(value.to_string()))
}

fn candidate_error(url: &str, error_code: &str) -> Value {
    json!({
        "url": url,
        "status": "rejected",
        "error_code": error_code,
    })
}

fn category_replan_failure(error_code: &str, fields: Value) -> SkillFailure {
    let mut fields = fields;
    if let Some(object) = fields.as_object_mut() {
        object.insert("recovery_action".to_string(), json!("replan_arguments"));
    }
    category_failure(error_code, true, fields)
}

fn category_failure(error_code: &str, retryable: bool, fields: Value) -> SkillFailure {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_code": error_code,
        "message_key": format!("skill.rss_fetch.{error_code}"),
        "retryable": retryable,
        "failure_phase": "pre_dispatch",
        "side_effect_applied": false,
    });
    if let (Some(target), Some(source)) = (extra.as_object_mut(), fields.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    SkillFailure {
        error_text: error_code.to_string(),
        extra,
    }
}

#[cfg(test)]
#[path = "category_discovery_tests.rs"]
mod tests;
