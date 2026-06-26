use chrono::{Datelike, Duration as ChronoDuration, NaiveDateTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use rusqlite::params;
use serde_json::{json, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{llm_gateway, memory, AppState, ClaimedTask, ScheduleIntentOutput};
use claw_core::skill_registry::{SkillKind, SkillsRegistry};

// ---------- Schedule skill catalog & validation (dynamic from registry) ----------
// `create` persists jobs generically; no per-skill subprocess preflight or merge-on-create paths live here.
const SCHEDULE_INTENT_MIN_CONFIDENCE: f64 = 0.5;

#[derive(Debug, Clone)]
struct SkillContractHint {
    summary: String,
}

/// Generic one-line hint for catalog (no business semantics; skill owns its own contract).
fn schedule_skill_catalog_hint(
    entry: &claw_core::skill_registry::SkillRegistryEntry,
) -> &'static str {
    match entry.kind {
        SkillKind::Builtin => "builtin",
        SkillKind::Runner => "runner",
        _ => "external",
    }
}

/// Build a short skill catalog for schedule intent prompt from the loaded registry (same source as runtime).
/// Injects into `__SKILL_CATALOG__` and `__SKILLS_CATALOG__` (both identical for compatibility).
pub(crate) fn build_schedule_skill_catalog_from_registry(registry: &SkillsRegistry) -> String {
    let mut lines: Vec<String> = Vec::new();
    for name in registry.enabled_names() {
        let Some(entry) = registry.get(&name) else {
            continue;
        };
        let aliases_str = if entry.aliases.is_empty() {
            String::new()
        } else {
            format!(" (aliases: {})", entry.aliases.join(", "))
        };
        let hint = schedule_skill_catalog_hint(entry);
        lines.push(format!("- {name}{aliases_str} [enabled] — {hint}"));
    }
    let mut disabled: Vec<String> = registry
        .all_names()
        .into_iter()
        .filter(|n| registry.get(n).map(|e| !e.enabled).unwrap_or(false))
        .collect();
    disabled.sort();
    if !disabled.is_empty() {
        lines.push(format!(
            "- Note: registered but disabled — do NOT schedule run_skill for: {}",
            disabled.join(", ")
        ));
    }
    lines.join("\n")
}

/// Build schedule skill catalog from app state (`state.get_skills_registry()`).
pub(crate) fn build_schedule_skill_catalog(state: &AppState) -> String {
    state
        .get_skills_registry()
        .as_ref()
        .map(|arc| build_schedule_skill_catalog_from_registry(arc.as_ref()))
        .unwrap_or_default()
}

/// Same string as [`build_schedule_skill_catalog`]; name matches prompt assembly wording.
pub(crate) fn render_schedule_skill_catalog(state: &AppState) -> String {
    build_schedule_skill_catalog(state)
}

fn split_markdown_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|v| v.trim().to_string())
        .collect()
}

fn parse_skill_contract_hint(markdown: &str) -> Option<SkillContractHint> {
    let lines: Vec<&str> = markdown.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.trim().starts_with("## Parameter Contract"))?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| l.trim().starts_with("## "))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());
    let section = &lines[start + 1..end];

    let mut table_rows: Vec<Vec<String>> = Vec::new();
    for line in section {
        let t = line.trim();
        if t.starts_with('|') && t.matches('|').count() >= 2 {
            table_rows.push(split_markdown_table_row(t));
        }
    }
    if table_rows.len() < 2 {
        return None;
    }

    let header = &table_rows[0];
    let mut param_idx = None;
    let mut required_idx = None;
    let mut desc_idx = None;
    for (i, col) in header.iter().enumerate() {
        let c = col.trim().to_ascii_lowercase();
        if c == "param" {
            param_idx = Some(i);
        } else if c == "required" {
            required_idx = Some(i);
        } else if c == "description" {
            desc_idx = Some(i);
        }
    }
    let (Some(param_idx), Some(required_idx)) = (param_idx, required_idx) else {
        return None;
    };

    let mut rows: Vec<String> = Vec::new();
    for row in table_rows.iter().skip(2).take(6) {
        if row.len() <= param_idx || row.len() <= required_idx {
            continue;
        }
        let param = row[param_idx].trim();
        if param.is_empty() {
            continue;
        }
        let required = row[required_idx].trim();
        let desc = desc_idx
            .and_then(|idx| row.get(idx))
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .unwrap_or("");
        if desc.is_empty() {
            rows.push(format!("{param} (required: {required})"));
        } else {
            rows.push(format!("{param} (required: {required}) - {desc}"));
        }
    }
    if rows.is_empty() {
        return None;
    }
    Some(SkillContractHint {
        summary: rows.join("; "),
    })
}

fn load_skill_contract_hint(
    state: &AppState,
    registry: &SkillsRegistry,
    canonical_name: &str,
) -> Option<SkillContractHint> {
    let registry_prompt_rel_path = registry.prompt_file(canonical_name)?;
    let vendor = crate::bootstrap::prompts::active_prompt_vendor_name(state);
    let (markdown, _) = crate::bootstrap::prompts::load_prompt_template_for_vendor(
        &state.skill_rt.workspace_root,
        Some(&vendor),
        registry_prompt_rel_path,
        "",
    );
    parse_skill_contract_hint(&markdown)
}

fn render_schedule_skill_contracts(state: &AppState) -> String {
    let Some(registry_arc) = state.get_skills_registry() else {
        return String::new();
    };
    let registry = registry_arc.as_ref();
    let mut lines: Vec<String> = Vec::new();
    for name in registry.enabled_names() {
        if let Some(hint) = load_skill_contract_hint(state, registry, &name) {
            lines.push(format!("- {name}: {}", hint.summary));
        }
    }
    lines.join("\n")
}

/// Minimal validation for `run_skill` before persisting: skill exists, enabled, canonical name;
/// `args` if present must be an object. No per-skill business logic and **no skill subprocess** (no symbol preflight).
pub(crate) fn validate_schedule_run_skill(
    state: &AppState,
    payload: &Value,
) -> Result<Value, String> {
    let registry_arc = state
        .get_skills_registry()
        .ok_or_else(|| "skills registry not available".to_string())?;
    validate_schedule_run_skill_with_registry(registry_arc.as_ref(), payload)
}

/// Core validation/normalization using a registry reference. Used by validate_schedule_run_skill and by tests.
/// Per-action/schema checks remain in skill runtime (or future registry metadata), not in schedule layer.
pub(crate) fn validate_schedule_run_skill_with_registry(
    registry: &SkillsRegistry,
    payload: &Value,
) -> Result<Value, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "run_skill payload must be an object".to_string())?;
    let raw_skill = obj
        .get("skill_name")
        .or_else(|| obj.get("skill"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if raw_skill.is_empty() {
        return Err("run_skill payload must contain skill_name".to_string());
    }

    let canonical = registry
        .resolve_canonical(raw_skill)
        .ok_or_else(|| format!("unknown skill: {raw_skill}"))?;

    let entry = registry
        .get(canonical)
        .ok_or_else(|| format!("unknown skill: {raw_skill}"))?;
    if !entry.enabled {
        return Err(format!("skill is disabled: {canonical}"));
    }

    let args = obj
        .get("args")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    if !args.is_object() {
        return Err("run_skill payload args must be an object".to_string());
    }
    let mut out = serde_json::Map::new();
    out.insert(
        "skill_name".to_string(),
        Value::String(canonical.to_string()),
    );
    out.insert("args".to_string(), args);
    Ok(Value::Object(out))
}

pub(crate) async fn parse_schedule_intent(
    state: &AppState,
    task: &ClaimedTask,
    request: &str,
) -> Option<ScheduleIntentOutput> {
    let tz = parse_timezone(&state.policy.schedule.timezone);
    let now_local = Utc::now().with_timezone(&tz);
    let structured = memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        request,
        state.policy.memory.recall_limit.max(1),
        state.policy.memory.schedule_memory_include_long_term,
        state.policy.memory.schedule_memory_include_preferences,
    );
    let memory_context = memory::service::structured_memory_context_block(
        &structured,
        memory::retrieval::MemoryContextMode::Schedule,
        state.policy.memory.schedule_memory_max_chars.max(384),
    );
    let skill_catalog = render_schedule_skill_catalog(state);
    let skill_contracts = render_schedule_skill_contracts(state);
    // §3.5d: 模板字段封装为 `Arc<RwLock<String>>`，每次取一份 owned snapshot 用于
    // render（写锁短命，避免 reload 时阻塞当前 LLM 调用）。
    let intent_prompt_template = state.policy.schedule.intent_prompt_template_string();
    let intent_rules_template = state.policy.schedule.intent_rules_template_string();
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, request);
    let prompt = crate::render_prompt_template(
        &intent_prompt_template,
        &[
            (
                "__NOW__",
                &now_local.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
            ),
            ("__TIMEZONE__", &state.policy.schedule.timezone),
            ("__RULES__", &intent_rules_template),
            ("__SKILL_CATALOG__", &skill_catalog),
            ("__SKILLS_CATALOG__", &skill_catalog),
            ("__SKILL_CONTRACTS__", &skill_contracts),
            ("__MEMORY_CONTEXT__", &memory_context),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.schedule.locale,
            ),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            ("__REQUEST__", request),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "schedule_intent_prompt",
        &state.policy.schedule.intent_prompt_source,
        None,
    );

    let llm_out = match llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &state.policy.schedule.intent_prompt_source,
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "parse_schedule_intent llm failed: task_id={} err={err}",
                task.task_id
            );
            return None;
        }
    };

    let parsed = match crate::prompt_utils::validate_against_schema::<ScheduleIntentOutput>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::ScheduleIntent,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok {
                warn!(
                    "parse_schedule_intent schema_parse_recovery: task_id={} schema_normalized={}",
                    task.task_id, validated.schema_normalized
                );
            }
            validated.value
        }
        Err(err) => {
            warn!(
                "parse_schedule_intent schema_validation_failed: task_id={} err={}",
                task.task_id, err
            );
            return None;
        }
    };
    if parsed.needs_clarify && !parsed.clarify_question.trim().is_empty() {
        return Some(parsed);
    }
    let kind = parsed.kind.trim().to_ascii_lowercase();
    if kind.is_empty() || kind == "none" {
        return None;
    }
    if parsed.confidence > 0.0 && parsed.confidence < SCHEDULE_INTENT_MIN_CONFIDENCE {
        return None;
    }
    Some(parsed)
}

pub(crate) fn parse_timezone(raw: &str) -> Tz {
    raw.trim()
        .parse::<Tz>()
        .unwrap_or(chrono_tz::Asia::Shanghai)
}

pub(crate) fn parse_local_datetime(raw: &str, tz: Tz) -> Option<i64> {
    let trimmed = raw.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Utc).timestamp());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S %:z") {
        return Some(dt.with_timezone(&Utc).timestamp());
    }
    let normalized = trimmed.replace('T', " ");
    let dt = NaiveDateTime::parse_from_str(normalized.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(normalized.trim(), "%Y-%m-%d %H:%M").ok())?;
    tz.from_local_datetime(&dt)
        .earliest()
        .map(|v| v.with_timezone(&Utc).timestamp())
}

pub(crate) fn normalize_schedule_intent_alias_fields(intent: &mut ScheduleIntentOutput) {
    if intent.mode.trim().is_empty()
        && (intent.dry_run || intent.preview_only || intent.create_real == Some(false))
    {
        intent.mode = "compile_only".to_string();
    }
    if intent.timezone.trim().is_empty() && !intent.schedule.timezone.trim().is_empty() {
        intent.timezone = intent.schedule.timezone.trim().to_string();
    }
    let content = sanitize_schedule_task_text(&intent.schedule.content);
    if !content.is_empty() {
        if intent.task.kind.trim().is_empty() {
            intent.task.kind = "ask".to_string();
        }
        if !intent.task.payload.is_object() {
            intent.task.payload = json!({});
        }
        if let Value::Object(map) = &mut intent.task.payload {
            let has_text = map
                .get("text")
                .and_then(|value| value.as_str())
                .is_some_and(|value| !sanitize_schedule_task_text(value).is_empty());
            if !has_text {
                map.insert("text".to_string(), Value::String(content));
            }
        }
    }
}

fn parse_hhmm(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw.trim().split(':');
    let h = parts.next()?.trim().parse::<u32>().ok()?;
    let m = parts.next()?.trim().parse::<u32>().ok()?;
    if parts.next().is_some() || h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

fn weekday_from_monday_num(n: i64) -> Option<Weekday> {
    match n {
        1 => Some(Weekday::Mon),
        2 => Some(Weekday::Tue),
        3 => Some(Weekday::Wed),
        4 => Some(Weekday::Thu),
        5 => Some(Weekday::Fri),
        6 => Some(Weekday::Sat),
        7 => Some(Weekday::Sun),
        _ => None,
    }
}

pub(crate) fn compute_next_run_for_schedule(
    schedule_type: &str,
    time_of_day: Option<&str>,
    weekday: Option<i64>,
    every_minutes: Option<i64>,
    timezone: &str,
    now_ts: i64,
) -> Option<i64> {
    let tz = parse_timezone(timezone);
    let now_utc = chrono::DateTime::<Utc>::from_timestamp(now_ts, 0)?;
    let now_local = now_utc.with_timezone(&tz);
    let now_local_naive = now_local.naive_local();

    match schedule_type {
        "once" => None,
        "interval" => {
            let mins = every_minutes.unwrap_or(0).max(1);
            Some(now_ts + mins * 60)
        }
        "daily" => {
            let (h, m) = parse_hhmm(time_of_day?)?;
            let mut date = now_local.date_naive();
            let mut candidate = date.and_hms_opt(h, m, 0)?;
            if candidate <= now_local_naive {
                date += ChronoDuration::days(1);
                candidate = date.and_hms_opt(h, m, 0)?;
            }
            tz.from_local_datetime(&candidate)
                .earliest()
                .map(|v| v.with_timezone(&Utc).timestamp())
        }
        "weekly" => {
            let target = weekday_from_monday_num(weekday?)?;
            let (h, m) = parse_hhmm(time_of_day?)?;
            let current = now_local.weekday().number_from_monday() as i64;
            let target_num = target.number_from_monday() as i64;
            let mut days = (target_num - current + 7) % 7;
            let mut date = now_local.date_naive() + ChronoDuration::days(days);
            let mut candidate = date.and_hms_opt(h, m, 0)?;
            if days == 0 && candidate <= now_local_naive {
                days = 7;
                date = now_local.date_naive() + ChronoDuration::days(days);
                candidate = date.and_hms_opt(h, m, 0)?;
            }
            tz.from_local_datetime(&candidate)
                .earliest()
                .map(|v| v.with_timezone(&Utc).timestamp())
        }
        _ => None,
    }
}

pub(crate) fn clean_schedule_kind(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// §7.5: env 名 —— `cargo test` / nl-replay 通过它把 normalizer prompt 里的
/// `__NOW__` 字段冻结到固定值，让 fixture FNV hash 在 `Utc::now()` 漂移下仍稳定。
/// 生产路径 unset 时走 `Utc::now()`，行为与历史版本完全一致。
pub(crate) const TEST_FREEZE_NOW_ENV: &str = "RUSTCLAW_TEST_FREEZE_NOW";

/// §7.5: 解析 [`TEST_FREEZE_NOW_ENV`] 字符串。
///
/// 接受两种格式（`%:z` = `+08:00` 这类带冒号的 offset）：
///   * RFC-3339-ish：`2026-04-19T12:00:00+08:00`
///   * normalizer 形态：`2026-04-19 12:00:00 +08:00`
///
/// 解析成功 → 转换到 `tz`；失败 → **panic**（这是 test-only env，写错就该立刻
/// 炸出来，避免静默 fallback 到 `Utc::now()` 让 fixture 跑出"看似稳定其实漂移"
/// 的诡异行为）。
fn parse_freeze_now_or_panic(raw: &str, tz: &Tz) -> chrono::DateTime<Tz> {
    let trimmed = raw.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return dt.with_timezone(tz);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S %:z") {
        return dt.with_timezone(tz);
    }
    panic!(
        "{TEST_FREEZE_NOW_ENV}={trimmed:?} failed to parse; accepted formats: \
         RFC-3339 (`2026-04-19T12:00:00+08:00`) or `%Y-%m-%d %H:%M:%S %:z` \
         (`2026-04-19 12:00:00 +08:00`)"
    );
}

/// §7.5: 计算 normalizer prompt 用的"现在"，可被 [`TEST_FREEZE_NOW_ENV`] 冻结。
fn effective_now_for_normalizer(tz: &Tz) -> chrono::DateTime<Tz> {
    match std::env::var(TEST_FREEZE_NOW_ENV) {
        Ok(v) if !v.trim().is_empty() => parse_freeze_now_or_panic(&v, tz),
        _ => Utc::now().with_timezone(tz),
    }
}

/// Returns (now_iso, timezone, schedule_rules) for intent normalizer prompt.
pub(crate) fn schedule_context_for_normalizer(state: &AppState) -> (String, String, String) {
    let tz = parse_timezone(&state.policy.schedule.timezone);
    let now_local = effective_now_for_normalizer(&tz);
    let now_iso = now_local.format("%Y-%m-%d %H:%M:%S %:z").to_string();
    let timezone = state.policy.schedule.timezone.clone();
    let rules = state.policy.schedule.intent_rules_template_string();
    (now_iso, timezone, rules)
}

pub(crate) fn schedule_timezone_from_intent(state: &AppState, intent_tz: &str) -> String {
    let chosen = if intent_tz.trim().is_empty() {
        state.policy.schedule.timezone.clone()
    } else {
        intent_tz.trim().to_string()
    };
    if chosen.parse::<Tz>().is_ok() {
        chosen
    } else {
        state.policy.schedule.timezone.clone()
    }
}

fn schedule_t(state: &AppState, key: &str) -> String {
    state
        .policy
        .schedule
        .i18n_dict
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn schedule_t_with(state: &AppState, key: &str, vars: &[(&str, &str)]) -> String {
    let mut out = schedule_t(state, key);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn schedule_kind_desc(
    state: &AppState,
    schedule_type: &str,
    time_of_day: Option<String>,
    weekday: Option<i64>,
    every_minutes: Option<i64>,
) -> String {
    match schedule_type {
        "daily" => schedule_t_with(
            state,
            "schedule.desc.daily",
            &[("time", time_of_day.as_deref().unwrap_or("??:??"))],
        ),
        "weekly" => schedule_t_with(
            state,
            "schedule.desc.weekly",
            &[
                ("weekday", &weekday.unwrap_or(0).to_string()),
                ("time", time_of_day.as_deref().unwrap_or("??:??")),
            ],
        ),
        "interval" => schedule_t_with(
            state,
            "schedule.desc.interval",
            &[("minutes", &every_minutes.unwrap_or(0).to_string())],
        ),
        "once" => schedule_t(state, "schedule.desc.once"),
        "cron" => "cron".to_string(),
        _ => schedule_type.to_string(),
    }
}

fn humanize_next_run_at(next_run_at: i64, timezone: &str) -> String {
    let tz = parse_timezone(timezone);
    chrono::DateTime::<Utc>::from_timestamp(next_run_at, 0)
        .map(|dt| {
            dt.with_timezone(&tz)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        })
        .unwrap_or_else(|| next_run_at.to_string())
}

fn is_supported_schedule_type(schedule_type: &str) -> bool {
    matches!(
        schedule_type,
        "once" | "interval" | "daily" | "weekly" | "cron"
    )
}

fn summarize_task_content(task_kind: &str, payload: &Value, fallback_prompt: &str) -> String {
    if task_kind == "ask" {
        if let Some(text) = payload.get("text").and_then(|v| v.as_str()) {
            let t = sanitize_schedule_task_text(text);
            if !t.is_empty() {
                return t;
            }
        }
        let p = sanitize_schedule_task_text(fallback_prompt);
        if !p.is_empty() {
            return p;
        }
    } else if task_kind == "run_skill" {
        let skill = payload
            .get("skill_name")
            .or_else(|| payload.get("skill"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let args_str = payload
            .get("args")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".to_string());
        if !skill.is_empty() {
            return format!("run_skill:{skill} args={args_str}");
        }
    }
    let payload_str = payload.to_string();
    if payload_str.trim().is_empty() {
        "-".to_string()
    } else {
        payload_str
    }
}

fn sanitize_schedule_task_text(raw: &str) -> String {
    const INTERNAL_CONTEXT_MARKERS: &[&str] = &[
        "### RUNTIME_CONTEXT",
        "### ACTIVE_TASK_CONTEXT",
        "### ACTIVE_EXECUTION_ANCHOR",
        "### SESSION_ALIAS_BINDINGS",
        "### REQUEST_SURFACE_HINTS",
        "### RECENT_EXECUTION_CONTEXT",
    ];

    let mut end = raw.len();
    for marker in INTERNAL_CONTEXT_MARKERS {
        if let Some(idx) = raw.find(marker) {
            end = end.min(idx);
        }
    }
    raw[..end].trim().to_string()
}

fn sanitize_schedule_ask_payload_text(payload: &mut Value, fallback_prompt: &str) {
    let Value::Object(map) = payload else {
        return;
    };
    let cleaned = map
        .get("text")
        .and_then(|value| value.as_str())
        .map(sanitize_schedule_task_text)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let fallback = sanitize_schedule_task_text(fallback_prompt);
            (!fallback.is_empty()).then_some(fallback)
        });
    if let Some(cleaned) = cleaned {
        map.insert("text".to_string(), Value::String(cleaned));
    }
}

/// Key-value pairs to inject into task payload when schedule triggers execution.
/// Skills can read these machine tokens to know they were invoked by schedule.
pub(crate) fn schedule_invocation_metadata(job_id: &str, run_id: &str) -> Vec<(String, Value)> {
    let resume_trigger = crate::task_lifecycle::ResumeTrigger::ScheduledWakeup.status_code();
    let mut metadata = vec![
        ("schedule_triggered".to_string(), Value::Bool(true)),
        (
            "schedule_job_id".to_string(),
            Value::String(job_id.to_string()),
        ),
        (
            "automation_ref".to_string(),
            Value::String(job_id.to_string()),
        ),
        (
            "automation_kind".to_string(),
            Value::String("scheduled_job".to_string()),
        ),
        (
            "invocation_source".to_string(),
            Value::String("schedule".to_string()),
        ),
        (
            "resume_trigger".to_string(),
            Value::String(resume_trigger.to_string()),
        ),
        (
            "resume_directive".to_string(),
            Value::String("run_scheduled_task".to_string()),
        ),
        ("thread_resume".to_string(), Value::Bool(true)),
        (
            "thread_resume_source".to_string(),
            Value::String("scheduled_wakeup".to_string()),
        ),
        (
            "automation_checkpoint_required".to_string(),
            Value::Bool(true),
        ),
        ("scheduled".to_string(), Value::Bool(true)),
    ];
    metadata.extend(crate::scheduled_run_contract::scheduled_run_payload_metadata(job_id, run_id));
    metadata
}

fn inherit_schedule_delivery_context(task: &ClaimedTask, payload: Value) -> Value {
    if !task.channel.trim().eq_ignore_ascii_case("wechat") {
        return payload;
    }
    let Some(source_payload) = serde_json::from_str::<Value>(&task.payload_json).ok() else {
        return payload;
    };
    let Some(source_token) = source_payload
        .get("context_token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return payload;
    };
    let mut payload = payload;
    let Value::Object(map) = &mut payload else {
        return payload;
    };
    let has_context_token = map
        .get("context_token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some();
    if !has_context_token {
        map.insert(
            "context_token".to_string(),
            Value::String(source_token.to_string()),
        );
    }
    payload
}

fn schedule_needs_more_info_fallback_text(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, prompt);
    let prefer_english = language_hint.to_ascii_lowercase().starts_with("en");
    crate::bilingual_t_with_default_vars(
        state,
        "schedule.msg.create_needs_more_info",
        "请补充必要信息后，我再帮你创建这个定时任务。",
        "Please provide the necessary details first, then I can create this scheduled job for you.",
        prefer_english,
        &[],
    )
}

fn schedule_intent_mode(intent: &ScheduleIntentOutput) -> &str {
    let mode = intent.mode.trim();
    if mode.is_empty() {
        "execute"
    } else {
        mode
    }
}

fn schedule_intent_is_compile_only(intent: &ScheduleIntentOutput) -> bool {
    matches!(schedule_intent_mode(intent), "compile_only" | "dry_run")
}

fn schedule_compile_only_response(
    intent: &ScheduleIntentOutput,
    kind: &str,
    would_mutate: bool,
    extra: Value,
) -> String {
    json!({
        "schema_version": 1,
        "semantic_kind": "schedule_intent_preview",
        "status": "ok",
        "mode": schedule_intent_mode(intent),
        "kind": kind,
        "would_mutate": would_mutate,
        "intent": intent,
        "extra": extra,
    })
    .to_string()
}

pub(crate) async fn try_handle_schedule_request(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    precompiled_intent: Option<&ScheduleIntentOutput>,
) -> Result<Option<String>, String> {
    let mut intent = if let Some(intent) = precompiled_intent
        .filter(|intent| intent.needs_clarify || !clean_schedule_kind(&intent.kind).is_empty())
    {
        intent.clone()
    } else {
        let compile_args = json!({
            "action": "compile",
            "text": prompt
        });
        let compiled_text =
            match crate::execution_adapters::run_skill(state, task, "schedule", compile_args).await
            {
                Ok(v) => v,
                Err(_) => return Ok(None),
            };
        match serde_json::from_str::<ScheduleIntentOutput>(&compiled_text) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        }
    };
    normalize_schedule_intent_alias_fields(&mut intent);
    let kind = clean_schedule_kind(&intent.kind);
    if intent.needs_clarify {
        let q = intent.clarify_question.trim();
        if !q.is_empty() {
            return Ok(Some(q.to_string()));
        }
        let reason = intent.reason.trim();
        if !reason.is_empty() {
            return Ok(Some(reason.to_string()));
        }
        return Ok(Some(schedule_needs_more_info_fallback_text(
            state, task, prompt,
        )));
    }
    if kind.is_empty() || kind == "none" {
        return Ok(None);
    }
    debug!(
        "schedule intent parsed: task_id={} kind={} confidence={}",
        task.task_id, kind, intent.confidence
    );
    if schedule_intent_is_compile_only(&intent) && kind != "create" {
        return Ok(Some(schedule_compile_only_response(
            &intent,
            &kind,
            matches!(kind.as_str(), "delete" | "pause" | "resume"),
            json!({}),
        )));
    }
    match kind.as_str() {
        "list" => {
            let db = state.core.db.get().map_err(|e| format!("db pool: {e}"))?;
            let mut stmt = db
                .prepare(
                    "SELECT job_id, schedule_type, time_of_day, weekday, every_minutes, timezone, enabled, next_run_at, task_kind, task_payload_json
                     FROM scheduled_jobs
                     WHERE user_id = ?1 AND chat_id = ?2
                     ORDER BY id DESC
                     LIMIT 20",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![task.user_id, task.chat_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                })
                .map_err(|e| e.to_string())?;

            let mut lines = Vec::new();
            for row in rows {
                let (
                    job_id,
                    schedule_type,
                    time_of_day,
                    weekday,
                    every_minutes,
                    timezone,
                    enabled,
                    next_run_at,
                    task_kind,
                    task_payload_json,
                ) = row.map_err(|e| e.to_string())?;
                let desc =
                    schedule_kind_desc(state, &schedule_type, time_of_day, weekday, every_minutes);
                let status = if enabled == 1 {
                    schedule_t(state, "schedule.status.enabled")
                } else {
                    schedule_t(state, "schedule.status.paused")
                };
                let next = next_run_at
                    .map(|v| humanize_next_run_at(v, &timezone))
                    .unwrap_or_else(|| "-".to_string());
                let payload =
                    serde_json::from_str::<Value>(&task_payload_json).unwrap_or_else(|_| json!({}));
                let task_content = summarize_task_content(&task_kind, &payload, "-");
                lines.push(format!(
                    "- {} | {} | tz={} | {} | next={} | task={}",
                    job_id, desc, timezone, status, next, task_content
                ));
            }
            if lines.is_empty() {
                Ok(Some(schedule_t(state, "schedule.msg.list_empty")))
            } else {
                Ok(Some(schedule_t_with(
                    state,
                    "schedule.msg.list_header",
                    &[("lines", &lines.join("\n"))],
                )))
            }
        }
        "delete" => {
            let db = state.core.db.get().map_err(|e| format!("db pool: {e}"))?;
            let target = intent.target_job_id.trim();
            let (affected, bulk_mode) = if target.is_empty() {
                (
                    db.execute(
                        "DELETE FROM scheduled_jobs WHERE user_id = ?1 AND chat_id = ?2",
                        params![task.user_id, task.chat_id],
                    )
                    .map_err(|e| e.to_string())?,
                    true,
                )
            } else {
                (
                    db.execute(
                        "DELETE FROM scheduled_jobs WHERE job_id = ?1 AND user_id = ?2 AND chat_id = ?3",
                        params![target, task.user_id, task.chat_id],
                    )
                    .map_err(|e| e.to_string())?,
                    false,
                )
            };
            if affected == 0 {
                if bulk_mode {
                    Ok(Some(schedule_t(state, "schedule.msg.delete_none")))
                } else {
                    Ok(Some(schedule_t_with(
                        state,
                        "schedule.msg.job_id_not_found",
                        &[("job_id", target)],
                    )))
                }
            } else if bulk_mode {
                Ok(Some(schedule_t_with(
                    state,
                    "schedule.msg.delete_all_ok",
                    &[("count", &affected.to_string())],
                )))
            } else {
                Ok(Some(schedule_t_with(
                    state,
                    "schedule.msg.delete_one_ok",
                    &[("job_id", target)],
                )))
            }
        }
        "pause" | "resume" => {
            let enabled = if kind == "resume" { 1 } else { 0 };
            let db = state.core.db.get().map_err(|e| format!("db pool: {e}"))?;
            let target = intent.target_job_id.trim();
            let (affected, bulk_mode) = if target.is_empty() {
                (
                    db.execute(
                        "UPDATE scheduled_jobs SET enabled = ?3, updated_at = ?4
                         WHERE user_id = ?1 AND chat_id = ?2",
                        params![task.user_id, task.chat_id, enabled, crate::now_ts()],
                    )
                    .map_err(|e| e.to_string())?,
                    true,
                )
            } else {
                (
                    db.execute(
                        "UPDATE scheduled_jobs SET enabled = ?4, updated_at = ?5
                         WHERE job_id = ?1 AND user_id = ?2 AND chat_id = ?3",
                        params![target, task.user_id, task.chat_id, enabled, crate::now_ts()],
                    )
                    .map_err(|e| e.to_string())?,
                    false,
                )
            };
            if affected == 0 {
                if bulk_mode {
                    Ok(Some(schedule_t(state, "schedule.msg.update_none")))
                } else {
                    Ok(Some(schedule_t_with(
                        state,
                        "schedule.msg.job_id_not_found",
                        &[("job_id", target)],
                    )))
                }
            } else if bulk_mode && enabled == 1 {
                Ok(Some(schedule_t_with(
                    state,
                    "schedule.msg.resume_all_ok",
                    &[("count", &affected.to_string())],
                )))
            } else if bulk_mode {
                Ok(Some(schedule_t_with(
                    state,
                    "schedule.msg.pause_all_ok",
                    &[("count", &affected.to_string())],
                )))
            } else if enabled == 1 {
                Ok(Some(schedule_t_with(
                    state,
                    "schedule.msg.resume_one_ok",
                    &[("job_id", target)],
                )))
            } else {
                Ok(Some(schedule_t_with(
                    state,
                    "schedule.msg.pause_one_ok",
                    &[("job_id", target)],
                )))
            }
        }
        "create" => {
            let timezone = schedule_timezone_from_intent(state, &intent.timezone);
            let schedule_type = clean_schedule_kind(&intent.schedule.r#type);
            if !is_supported_schedule_type(&schedule_type) {
                return Ok(Some(schedule_t(
                    state,
                    "schedule.msg.create_fail_cannot_compute_next_run",
                )));
            }
            let task_kind = clean_schedule_kind(&intent.task.kind);
            if !matches!(task_kind.as_str(), "ask" | "run_skill") {
                return Ok(Some(schedule_t(
                    state,
                    "schedule.msg.create_fail_task_kind",
                )));
            }
            if schedule_type == "cron" {
                if intent.schedule.cron.trim().is_empty() {
                    return Ok(Some(schedule_t(state, "schedule.msg.cron_not_supported")));
                }
                return Ok(Some(schedule_t_with(
                    state,
                    "schedule.msg.cron_not_supported_with_expr",
                    &[("cron", intent.schedule.cron.trim())],
                )));
            }

            let now = crate::now_ts_u64() as i64;
            let time_of_day = if intent.schedule.time.trim().is_empty() {
                None
            } else {
                Some(intent.schedule.time.trim().to_string())
            };
            let weekday = if intent.schedule.weekday <= 0 {
                None
            } else {
                Some(intent.schedule.weekday)
            };
            let every_minutes = if intent.schedule.every_minutes <= 0 {
                None
            } else {
                Some(intent.schedule.every_minutes)
            };
            let run_at = if schedule_type == "once" {
                let ts = parse_local_datetime(&intent.schedule.run_at, parse_timezone(&timezone));
                let Some(ts) = ts else {
                    return Ok(Some(schedule_t(
                        state,
                        "schedule.msg.create_fail_invalid_run_at",
                    )));
                };
                if ts <= now {
                    return Ok(Some(schedule_t(
                        state,
                        "schedule.msg.create_fail_run_at_must_be_future",
                    )));
                }
                Some(ts)
            } else {
                None
            };

            let next_run_at = if schedule_type == "once" {
                run_at
            } else {
                compute_next_run_for_schedule(
                    &schedule_type,
                    time_of_day.as_deref(),
                    weekday,
                    every_minutes,
                    &timezone,
                    now,
                )
            };
            let Some(next_run_at) = next_run_at else {
                return Ok(Some(schedule_t(
                    state,
                    "schedule.msg.create_fail_cannot_compute_next_run",
                )));
            };

            let payload = if task_kind == "ask" {
                let mut v = intent.task.payload.clone();
                if let Value::Object(map) = &mut v {
                    let has_text = map
                        .get("text")
                        .and_then(|x| x.as_str())
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false);
                    if !has_text {
                        map.insert("text".to_string(), Value::String(prompt.to_string()));
                    }
                    if map
                        .get("schedule_task_mode")
                        .and_then(|x| x.as_str())
                        .map(|s| s.trim().is_empty())
                        .unwrap_or(true)
                    {
                        // 定时 ask 默认按“原文提醒”发送，避免触发时二次意图误判。
                        map.insert(
                            "schedule_task_mode".to_string(),
                            Value::String("direct_text".to_string()),
                        );
                    }
                    v
                } else {
                    json!({
                        "text": prompt,
                        "schedule_task_mode": "direct_text"
                    })
                }
            } else {
                intent.task.payload.clone()
            };

            let mut payload = inherit_schedule_delivery_context(task, payload);
            if task_kind == "ask" {
                sanitize_schedule_ask_payload_text(&mut payload, prompt);
            }

            let payload = if task_kind == "run_skill" {
                match validate_schedule_run_skill(state, &payload) {
                    Ok(normalized) => normalized,
                    Err(err) => return Ok(Some(err)),
                }
            } else {
                payload
            };

            let next_run_human = humanize_next_run_at(next_run_at, &timezone);
            let task_content = summarize_task_content(&task_kind, &payload, prompt);
            if schedule_intent_is_compile_only(&intent) {
                return Ok(Some(schedule_compile_only_response(
                    &intent,
                    &kind,
                    false,
                    json!({
                        "schedule_type": schedule_type,
                        "timezone": timezone,
                        "next_run_at": next_run_at,
                        "next_run_human": next_run_human,
                        "task_kind": task_kind,
                        "task_payload": payload,
                        "task_content": task_content,
                    }),
                )));
            }

            let job_id = format!("job_{}", &Uuid::new_v4().simple().to_string()[..10]);
            let created_at = crate::now_ts();
            let db = state.core.db.get().map_err(|e| format!("db pool: {e}"))?;
            db.execute(
                "INSERT INTO scheduled_jobs (
                    job_id, user_id, chat_id, channel, external_user_id, external_chat_id, user_key, schedule_type, run_at, time_of_day, weekday, every_minutes, cron_expr,
                    timezone, task_kind, task_payload_json, enabled, notify_on_success, notify_on_failure,
                    last_run_at, next_run_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?14, ?15, 1, 1, 1, NULL, ?16, ?17, ?17)",
                params![
                    job_id,
                    task.user_id,
                    task.chat_id,
                    task.channel,
                    task.external_user_id,
                    task.external_chat_id,
                    task.user_key,
                    schedule_type,
                    run_at,
                    time_of_day,
                    weekday,
                    every_minutes,
                    timezone,
                    task_kind,
                    payload.to_string(),
                    next_run_at,
                    created_at
                ],
            )
            .map_err(|e| e.to_string())?;

            Ok(Some(schedule_t_with(
                state,
                "schedule.msg.create_ok",
                &[
                    ("job_id", &job_id),
                    ("type", &intent.schedule.r#type),
                    ("timezone", &timezone),
                    ("next_run_human", &next_run_human),
                    ("task_content", &task_content),
                ],
            )))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
#[path = "schedule_service_schedule_skill_tests.rs"]
mod schedule_skill_tests;
