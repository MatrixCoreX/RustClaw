//! invest_copy: single-line JSON stdin -> single-line JSON stdout.
//! LLM mode uses the same OpenAI-compatible gateway as clawd when available.
//! Heuristic mode returns structured evidence for downstream rendering.

mod llm_client;

use anyhow::Context;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAX_DATA_CHARS: usize = 12_000;
const MIN_DATA_CHARS: usize = 10;
const SHORT_SUMMARY_MAX: usize = 4;
const ARTICLE_SUMMARY_MAX: usize = 7;
const SKILL_NAME: &str = "invest_copy";
const CONFIG_REL: &str = "configs/invest_copy.toml";

static PERSONAS_TOML: &str = include_str!("../personas.toml");

#[derive(Debug, Deserialize, Clone)]
struct PersonaToml {
    slug: String,
    aliases: Vec<String>,
    #[serde(default)]
    display_name_zh: String,
    #[serde(default)]
    display_name_en: String,
    #[serde(default)]
    one_liner_zh: String,
    #[serde(default)]
    facets_zh: Vec<String>,
    #[serde(default)]
    prefer_zh: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PersonaRegistry {
    personas: Vec<PersonaToml>,
}

#[derive(Debug, Deserialize, Default)]
struct InvestCopyRootConfig {
    #[serde(default)]
    invest_copy: InvestCopyConfig,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct InvestCopyConfig {
    #[serde(default)]
    compliance_sensitive_terms: Vec<String>,
}

#[derive(Debug, Clone)]
struct PolicyMatch {
    term_index: usize,
    reason_code: &'static str,
}

#[derive(Debug, Deserialize)]
struct SkillReqLine {
    request_id: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize)]
struct SkillResp {
    request_id: String,
    status: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_text: Option<String>,
}

fn sentence_split_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[。\.\!?;；\n\r\t]+").expect("regex"))
}

fn runtime_config() -> &'static InvestCopyConfig {
    static CFG: OnceLock<InvestCopyConfig> = OnceLock::new();
    CFG.get_or_init(load_runtime_config)
}

fn load_runtime_config() -> InvestCopyConfig {
    let Some(root) = find_workspace_root() else {
        return InvestCopyConfig::default();
    };
    let path = root.join(CONFIG_REL);
    let Ok(raw) = std::fs::read_to_string(path) else {
        return InvestCopyConfig::default();
    };
    toml::from_str::<InvestCopyRootConfig>(&raw)
        .map(|root| root.invest_copy)
        .unwrap_or_default()
}

fn find_workspace_root() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("WORKSPACE_ROOT") {
        let path = PathBuf::from(raw.trim());
        if config_exists(path.as_path()) {
            return Some(path);
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if config_exists(dir.as_path()) {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
    }
}

fn config_exists(root: &Path) -> bool {
    root.join(CONFIG_REL).is_file()
}

fn main() -> anyhow::Result<()> {
    let registry: PersonaRegistry =
        toml::from_str(PERSONAS_TOML).context("parse embedded personas.toml")?;
    let lookup = build_persona_lookup(&registry.personas);
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<SkillReqLine>(trimmed) {
            Ok(r) => handle_request(r, &lookup, &registry.personas),
            Err(e) => error_resp(
                "unknown",
                "invalid_input",
                format!("invalid request JSON: {e}"),
                Some(json!({ "source": "request_json" })),
            ),
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn build_persona_lookup(personas: &[PersonaToml]) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for (idx, p) in personas.iter().enumerate() {
        m.entry(norm_key(&p.slug)).or_insert(idx);
        for a in &p.aliases {
            m.entry(norm_key(a)).or_insert(idx);
        }
    }
    m
}

fn norm_key(s: &str) -> String {
    s.trim().chars().flat_map(|c| c.to_lowercase()).collect()
}

fn handle_request(
    req: SkillReqLine,
    lookup: &HashMap<String, usize>,
    personas: &[PersonaToml],
) -> SkillResp {
    let rid = req.request_id.clone();
    let args = &req.args;
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("draft")
        .trim()
        .to_ascii_lowercase();
    match action.as_str() {
        "list_investors" => list_investors(rid, personas, args),
        "draft" => draft(rid, args, lookup, personas),
        _ => error_resp(
            rid,
            "unsupported_action",
            format!("code=unsupported_action action={action} allowed=draft,list_investors"),
            Some(json!({ "action": action })),
        ),
    }
}

fn error_resp(
    request_id: impl Into<String>,
    error_kind: &str,
    error_text: impl Into<String>,
    details: Option<Value>,
) -> SkillResp {
    SkillResp {
        request_id: request_id.into(),
        status: "error",
        text: String::new(),
        extra: Some(error_extra_with_details(error_kind, details)),
        error_text: Some(error_text.into()),
    }
}

fn error_extra_with_details(error_kind: &str, details: Option<Value>) -> Value {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    });
    if let Some(details) = details {
        if let (Some(base), Some(details_obj)) = (extra.as_object_mut(), details.as_object()) {
            for (key, value) in details_obj {
                base.entry(key.clone()).or_insert_with(|| value.clone());
            }
        } else if let Some(base) = extra.as_object_mut() {
            base.insert("details".to_string(), details);
        }
    }
    extra
}

fn is_en(locale: Option<&str>) -> bool {
    locale
        .map(|s| s.trim().to_ascii_lowercase())
        .map(|s| s.starts_with("en"))
        .unwrap_or(false)
}

fn list_investors(request_id: String, personas: &[PersonaToml], args: &Value) -> SkillResp {
    let locale = args
        .get("locale")
        .or_else(|| args.get("language"))
        .or_else(|| args.get("lang"))
        .and_then(Value::as_str);
    let en = is_en(locale);
    let mut lines = vec![format!("personas count={}", personas.len())];
    for p in personas {
        let name = if en && !p.display_name_en.is_empty() {
            p.display_name_en.as_str()
        } else {
            p.display_name_zh.as_str()
        };
        lines.push(format!(
            "slug={} display_name={} one_liner={}",
            p.slug, name, p.one_liner_zh
        ));
    }
    let text = lines.join("\n");
    SkillResp {
        request_id,
        status: "ok",
        text,
        extra: Some(json!({ "action": "list_investors", "count": personas.len() })),
        error_text: None,
    }
}

fn extract_text_field<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for k in keys {
        if let Some(Value::String(s)) = args.get(*k) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn draft(
    request_id: String,
    args: &Value,
    lookup: &HashMap<String, usize>,
    personas: &[PersonaToml],
) -> SkillResp {
    let locale = args
        .get("locale")
        .or_else(|| args.get("language"))
        .or_else(|| args.get("lang"))
        .and_then(Value::as_str);
    let en = is_en(locale);
    let data_raw = extract_text_field(args, &["data", "material", "user_data"]);
    let brief = extract_text_field(args, &["brief", "focus"]).unwrap_or("");
    let source_note = extract_text_field(args, &["source_note", "data_source"]);

    let person_raw = args
        .get("person")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    if person_raw.is_empty() {
        return error_resp(
            request_id,
            "missing_person",
            "code=missing_person field=args.person",
            Some(json!({ "field": "args.person" })),
        );
    }

    let pdata = match data_raw {
        Some(s) if s.chars().count() >= MIN_DATA_CHARS => s,
        Some(s) => {
            return error_resp(
                request_id,
                "data_too_short",
                format!(
                    "code=data_too_short current_chars={} min_chars={}",
                    s.chars().count(),
                    MIN_DATA_CHARS
                ),
                Some(json!({
                    "current_chars": s.chars().count(),
                    "min_chars": MIN_DATA_CHARS,
                })),
            );
        }
        None => {
            return error_resp(
                request_id,
                "missing_data",
                "code=missing_data fields=args.data,args.material,args.user_data",
                Some(json!({ "fields": ["args.data", "args.material", "args.user_data"] })),
            );
        }
    };

    if let Some(policy_match) = compliance_policy_match(pdata, brief, runtime_config()) {
        return error_resp(
            request_id,
            "compliance_sensitive_input",
            "code=compliance_sensitive_input",
            Some(json!({
                "policy": "compliance_sensitive_input",
                "reason_code": policy_match.reason_code,
                "term_index": policy_match.term_index,
            })),
        );
    }

    let nk = norm_key(&person_raw);
    let persona_idx = match lookup.get(&nk) {
        Some(i) => *i,
        None => {
            return error_resp(
                request_id,
                "unknown_person",
                format!("code=unknown_person person={person_raw} recovery_action=list_investors"),
                Some(json!({ "person": person_raw })),
            );
        }
    };

    let p = personas[persona_idx].clone();
    let channel = args
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or("article")
        .trim()
        .to_ascii_lowercase();
    let max_bullets = if channel == "short" {
        SHORT_SUMMARY_MAX
    } else {
        ARTICLE_SUMMARY_MAX
    };

    let compliance = args
        .get("compliance")
        .and_then(Value::as_str)
        .unwrap_or("standard")
        .trim()
        .to_ascii_lowercase();
    let compliance = if compliance == "light" {
        "light"
    } else {
        "standard"
    };

    let use_heuristic = args
        .get("use_heuristic")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (owned_body, truncated) = truncate_owned(pdata);
    let body_for_summary = owned_body.as_str();

    if use_heuristic {
        let bullets = summarize_bullets(body_for_summary, max_bullets);
        let text = draft_machine_text(&p.slug, "heuristic", bullets.len(), truncated, compliance);
        let word_count = bullets
            .iter()
            .map(|item| item.chars().count())
            .sum::<usize>();
        return SkillResp {
            request_id,
            status: "ok",
            text,
            extra: Some(json!({
                "schema_version": 1,
                "source_skill": SKILL_NAME,
                "status": "ok",
                "message_key": "skill.invest_copy.draft_ready",
                "action": "draft",
                "person_slug": p.slug,
                "summary_mode": "heuristic",
                "data_truncated": truncated,
                "summary_bullet_count": bullets.len(),
                "summary_bullets": bullets,
                "brief": brief,
                "source_note": source_note,
                "compliance": compliance,
                "disclaimer_required": true,
                "rendering": {
                    "requires_language_rendering": true,
                    "recommended_owner": "finalizer_i18n_or_llm"
                },
                "word_count": word_count
            })),
            error_text: None,
        };
    }

    let system = build_llm_system_prompt(locale, compliance);
    let user = build_llm_user_prompt(
        en,
        &p,
        body_for_summary,
        brief,
        source_note,
        truncated,
        &channel,
        max_bullets,
    );

    let generated = match llm_client::chat_completion_default(&system, &user) {
        Ok(t) => t,
        Err(e) => {
            return error_resp(
                request_id,
                "llm_failed",
                format!("code=llm_failed reason={e}"),
                Some(json!({ "reason": e })),
            );
        }
    };

    if let Some(policy_match) = compliance_policy_match(&generated.text, "", runtime_config()) {
        return error_resp(
            request_id,
            "compliance_sensitive_output",
            "code=compliance_sensitive_output offline_arg=use_heuristic",
            Some(json!({
                "offline_arg": "use_heuristic",
                "policy": "compliance_sensitive_output",
                "reason_code": policy_match.reason_code,
                "term_index": policy_match.term_index,
            })),
        );
    }

    let text = generated.text.trim().to_string();
    let word_count = text.chars().count();

    SkillResp {
        request_id,
        status: "ok",
        text,
        extra: Some(json!({
            "schema_version": 1,
            "source_skill": SKILL_NAME,
            "status": "ok",
            "message_key": "skill.invest_copy.draft_ready",
            "action": "draft",
            "person_slug": p.slug,
            "summary_mode": "llm",
            "llm": {
                "credential_source": generated.source,
                "model": generated.model,
            },
            "data_truncated": truncated,
            "compliance": compliance,
            "disclaimer_required": true,
            "word_count": word_count
        })),
        error_text: None,
    }
}

fn build_llm_system_prompt(locale: Option<&str>, compliance: &str) -> String {
    let output_language = locale
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("match_user_material");
    format!(
        "You produce investor-education copy, not personalized advice.\n\
Output_language={output_language}\n\
Rules:\n\
1) Faithfully summarize only facts present in USER_MATERIAL; do not invent facts, symbols, prices, or performance outcomes.\n\
2) The style section may borrow public thinking lenses associated with the persona, but must not impersonate the person or imply endorsement.\n\
3) Do not provide buy/sell/hold directions, ticker picks, position sizing, leverage advice, or return promises.\n\
4) Use Markdown with natural section headings in Output_language for data highlights, style-inspired commentary, limitations, and disclaimer.\n\
5) compliance={compliance}: if standard, briefly mention third-party excerpt, terms, copyright, and regional suitability limitations in the disclaimer."
    )
}

fn build_llm_user_prompt(
    en: bool,
    p: &PersonaToml,
    data: &str,
    brief: &str,
    source_note: Option<&str>,
    truncated: bool,
    channel: &str,
    _max_bullets_hint: usize,
) -> String {
    let facets = p.facets_zh.join(" / ");
    let disp = if en && !p.display_name_en.trim().is_empty() {
        p.display_name_en.as_str()
    } else {
        p.display_name_zh.as_str()
    };
    format!(
        "Persona display: {}\nSlug: {}\nOne-liner: {}\nFacets:\n{}\nPreferred wording hints: {}\nChannel preference: {}\nTruncated_input: {}\nFocus_optional: {}\nSource_note_optional: {}\n\nUSER_MATERIAL:\n{}",
        disp,
        p.slug,
        p.one_liner_zh,
        facets,
        p.prefer_zh.join(", "),
        channel,
        truncated,
        brief,
        source_note.unwrap_or("(none)"),
        data
    )
}

fn truncate_owned(s: &str) -> (String, bool) {
    let count = s.chars().count();
    if count <= MAX_DATA_CHARS {
        return (s.to_string(), false);
    }
    (s.chars().take(MAX_DATA_CHARS).collect::<String>(), true)
}

fn draft_machine_text(
    person_slug: &str,
    summary_mode: &str,
    bullet_count: usize,
    truncated: bool,
    compliance: &str,
) -> String {
    format!(
        "message_key=skill.invest_copy.draft_ready person_slug={person_slug} summary_mode={summary_mode} summary_bullet_count={bullet_count} data_truncated={truncated} compliance={compliance}"
    )
}

fn compliance_policy_match(data: &str, brief: &str, cfg: &InvestCopyConfig) -> Option<PolicyMatch> {
    let pack = format!("{}{}", data, brief);
    let pack_ascii_lower = pack.to_ascii_lowercase();
    cfg.compliance_sensitive_terms
        .iter()
        .enumerate()
        .find_map(|(idx, term)| {
            let normalized = term.trim();
            if normalized.is_empty() {
                return None;
            }
            let matched = if normalized.is_ascii() {
                pack_ascii_lower.contains(&normalized.to_ascii_lowercase())
            } else {
                pack.contains(normalized)
            };
            matched.then_some(PolicyMatch {
                term_index: idx,
                reason_code: "configured_compliance_term",
            })
        })
}

fn split_sentences(s: &str) -> Vec<String> {
    sentence_split_regex()
        .split(s)
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .filter(|x| x.chars().count() > 10)
        .map(ToString::to_string)
        .collect()
}

fn score_sentence(s: &str) -> i32 {
    let mut score = 0_i32;
    if s.contains('%')
        || s.contains('％')
        || s.contains("pct")
        || s.chars().any(|c| c.is_ascii_digit())
    {
        score += 3;
    }
    if s.contains('？') || s.contains('?') {
        score += 1;
    }
    if contains_currency_marker(s) {
        score += 2;
    }
    score
}

fn contains_currency_marker(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    s.chars().any(|c| matches!(c, '$' | '€' | '£' | '¥'))
        || ["cny", "rmb", "usd", "eur", "gbp", "jpy"]
            .iter()
            .any(|token| lower.contains(token))
}

fn summarize_bullets(text: &str, max_items: usize) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    for paragraph in text.split('\n').map(str::trim).filter(|p| !p.is_empty()) {
        if paragraph.len() <= 15 {
            continue;
        }
        if let Some(fs) = paragraph
            .split(|c| c == '。' || c == '.' || c == '；' || c == ';')
            .next()
            .map(str::trim)
            .filter(|x| x.chars().count() > 10)
        {
            candidates.push(fs.to_string());
        }
    }
    for s in split_sentences(text) {
        candidates.push(s);
    }

    let mut seen = std::collections::HashSet::<String>::new();
    let mut uniq: Vec<String> = Vec::new();
    for c in candidates {
        let k = norm_space(&c);
        if seen.insert(k.clone()) {
            uniq.push(c);
        }
    }
    let mut scored: Vec<(i32, usize, String)> = uniq
        .into_iter()
        .enumerate()
        .map(|(i, s)| (-score_sentence(&s), i, s))
        .collect();
    scored.sort();
    scored
        .into_iter()
        .take(max_items.max(1))
        .map(|(_, _, s)| s)
        .collect()
}

fn norm_space(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
