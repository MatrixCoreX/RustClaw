use chrono::{Datelike, Duration as ChronoDuration, NaiveDateTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use rusqlite::params;
use serde_json::{json, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use claw_core::skill_registry::{SkillKind, SkillsRegistry};
use crate::{execution_adapters, llm_gateway, memory, AppState, ClaimedTask, ScheduleIntentOutput};

// ---------- Schedule skill catalog & validation (dynamic from registry) ----------
// `create` persists jobs generically; no per-skill subprocess preflight or merge-on-create paths live here.

/// Generic one-line hint for catalog (no business semantics; skill owns its own contract).
fn schedule_skill_catalog_hint(entry: &claw_core::skill_registry::SkillRegistryEntry) -> &'static str {
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
        lines.push(format!(
            "- {name}{aliases_str} [enabled] — {hint}"
        ));
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

/// Minimal validation for `run_skill` before persisting: skill exists, enabled, canonical name;
/// `args` if present must be an object. No per-skill business logic and **no skill subprocess** (no symbol preflight).
pub(crate) fn validate_schedule_run_skill(
    state: &AppState,
    payload: &Value,
) -> Result<Value, String> {
    let registry_arc = state.get_skills_registry().ok_or_else(|| {
        "skills registry not available".to_string()
    })?;
    validate_schedule_run_skill_with_registry(registry_arc.as_ref(), payload)
}

/// Core validation/normalization using a registry reference. Used by validate_schedule_run_skill and by tests.
// TODO: Per-action / schema checks belong in skill runtime or registry metadata, not here.
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

    let args = obj.get("args").cloned().unwrap_or(Value::Object(serde_json::Map::new()));
    if !args.is_object() {
        return Err("run_skill payload args must be an object".to_string());
    }
    let mut out = serde_json::Map::new();
    out.insert("skill_name".to_string(), Value::String(canonical.to_string()));
    out.insert("args".to_string(), args);
    Ok(Value::Object(out))
}

pub(crate) async fn parse_schedule_intent(
    state: &AppState,
    task: &ClaimedTask,
    request: &str,
) -> Option<ScheduleIntentOutput> {
    let tz = parse_timezone(&state.schedule.timezone);
    let now_local = Utc::now().with_timezone(&tz);
    let structured = memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        request,
        state.memory.recall_limit.max(1),
        state.memory.schedule_memory_include_long_term,
        state.memory.schedule_memory_include_preferences,
    );
    let memory_context = memory::service::structured_memory_context_block(
        &structured,
        memory::retrieval::MemoryContextMode::Schedule,
        state.memory.schedule_memory_max_chars.max(384),
    );
    let skill_catalog = render_schedule_skill_catalog(state);
    let prompt = crate::render_prompt_template(
        &state.schedule.intent_prompt_template,
        &[
            (
                "__NOW__",
                &now_local.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
            ),
            ("__TIMEZONE__", &state.schedule.timezone),
            ("__RULES__", &state.schedule.intent_rules_template),
            ("__SKILL_CATALOG__", &skill_catalog),
            ("__SKILLS_CATALOG__", &skill_catalog),
            ("__MEMORY_CONTEXT__", &memory_context),
            ("__REQUEST__", request),
        ],
    );
    crate::log_prompt_render(
        &task.task_id,
        "schedule_intent_prompt",
        &state.schedule.intent_prompt_file,
        None,
    );

    let llm_out = match llm_gateway::run_with_fallback_with_prompt_file(
        state,
        task,
        &prompt,
        &state.schedule.intent_prompt_file,
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

    let parsed: ScheduleIntentOutput = crate::parse_llm_json_extract_or_any(&llm_out)?;
    let kind = parsed.kind.trim().to_ascii_lowercase();
    if kind.is_empty() || kind == "none" {
        return None;
    }
    if parsed.confidence > 0.0 && parsed.confidence < 0.5 {
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
    let dt = NaiveDateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M").ok())?;
    tz.from_local_datetime(&dt)
        .earliest()
        .map(|v| v.with_timezone(&Utc).timestamp())
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

/// Returns (now_iso, timezone, schedule_rules) for intent normalizer prompt.
pub(crate) fn schedule_context_for_normalizer(state: &AppState) -> (String, String, String) {
    let tz = parse_timezone(&state.schedule.timezone);
    let now_local = Utc::now().with_timezone(&tz);
    let now_iso = now_local.format("%Y-%m-%d %H:%M:%S %:z").to_string();
    let timezone = state.schedule.timezone.clone();
    let rules = state.schedule.intent_rules_template.clone();
    (now_iso, timezone, rules)
}

pub(crate) fn schedule_timezone_from_intent(state: &AppState, intent_tz: &str) -> String {
    let chosen = if intent_tz.trim().is_empty() {
        state.schedule.timezone.clone()
    } else {
        intent_tz.trim().to_string()
    };
    if chosen.parse::<Tz>().is_ok() {
        chosen
    } else {
        state.schedule.timezone.clone()
    }
}

fn schedule_t(state: &AppState, key: &str) -> String {
    state
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

fn summarize_task_content(task_kind: &str, payload: &Value, fallback_prompt: &str) -> String {
    if task_kind == "ask" {
        if let Some(text) = payload.get("text").and_then(|v| v.as_str()) {
            let t = text.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
        let p = fallback_prompt.trim();
        if !p.is_empty() {
            return p.to_string();
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

/// Key-value pairs to inject into task payload when schedule triggers execution.
/// Skills can read `invocation_source`, `schedule_job_id`, `scheduled` to know they were invoked by schedule.
pub(crate) fn schedule_invocation_metadata(job_id: &str) -> Vec<(String, Value)> {
    vec![
        ("schedule_triggered".to_string(), Value::Bool(true)),
        ("schedule_job_id".to_string(), Value::String(job_id.to_string())),
        ("invocation_source".to_string(), Value::String("schedule".to_string())),
        ("scheduled".to_string(), Value::Bool(true)),
    ]
}

pub(crate) async fn try_handle_schedule_request(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
) -> Result<Option<String>, String> {
    let compile_args = json!({
        "action": "compile",
        "text": prompt
    });
    let compiled_text =
        match execution_adapters::run_skill(state, task, "schedule", compile_args).await {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
    let intent = match serde_json::from_str::<ScheduleIntentOutput>(&compiled_text) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let kind_preview = clean_schedule_kind(&intent.kind);
    if kind_preview.is_empty() || kind_preview == "none" {
        return Ok(None);
    }

    let kind = clean_schedule_kind(&intent.kind);
    debug!(
        "schedule intent parsed: task_id={} kind={} confidence={}",
        task.task_id, kind, intent.confidence
    );
    match kind.as_str() {
        "list" => {
            let db = state
                .db
                .lock()
                .map_err(|_| "db lock poisoned".to_string())?;
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
                    .map(|v| v.to_string())
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
            let db = state
                .db
                .lock()
                .map_err(|_| "db lock poisoned".to_string())?;
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
            let db = state
                .db
                .lock()
                .map_err(|_| "db lock poisoned".to_string())?;
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

            let payload = if task_kind == "run_skill" {
                match validate_schedule_run_skill(state, &payload) {
                    Ok(normalized) => normalized,
                    Err(err) => return Ok(Some(err)),
                }
            } else {
                payload
            };

            let job_id = format!("job_{}", &Uuid::new_v4().simple().to_string()[..10]);
            let created_at = crate::now_ts();
            let db = state
                .db
                .lock()
                .map_err(|_| "db lock poisoned".to_string())?;
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

            let next_run_human = humanize_next_run_at(next_run_at, &timezone);
            let task_content = summarize_task_content(&task_kind, &payload, prompt);
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
mod schedule_skill_tests {
    use super::*;

    fn test_registry() -> SkillsRegistry {
        let toml = r#"
[[skills]]
name = "rss_fetch"
enabled = true
kind = "runner"
aliases = ["rss", "rss_reader", "rss_fetcher", "news", "news_fetcher"]
timeout_seconds = 30
prompt_file = "prompts/skills/rss_fetch.md"
output_kind = "text"

[[skills]]
name = "crypto"
enabled = true
kind = "runner"
aliases = []
timeout_seconds = 30
prompt_file = "prompts/skills/crypto.md"
output_kind = "text"

[[skills]]
name = "demo_runner"
enabled = true
kind = "runner"
aliases = ["demo"]
timeout_seconds = 30
prompt_file = "prompts/skills/rss_fetch.md"
output_kind = "text"

[[skills]]
name = "health_check"
enabled = false
kind = "runner"
aliases = []
timeout_seconds = 30
prompt_file = "prompts/skills/health_check.md"
output_kind = "text"
"#;
        let path = std::env::temp_dir().join("schedule_skill_test_registry.toml");
        std::fs::write(&path, toml).unwrap();
        let reg = SkillsRegistry::load_from_path(&path).unwrap();
        let _ = std::fs::remove_file(path);
        reg
    }

    #[test]
    fn build_schedule_skill_catalog_from_registry_includes_enabled_skills() {
        let reg = test_registry();
        let catalog = build_schedule_skill_catalog_from_registry(&reg);
        assert!(catalog.contains("rss_fetch"));
        assert!(catalog.contains("demo_runner"));
        assert!(catalog.contains("rss") || catalog.contains("aliases"));
        assert!(catalog.contains("[enabled]"));
        assert!(catalog.contains("health_check"));
        assert!(catalog.contains("do NOT schedule") || catalog.contains("disabled"));
    }

    #[test]
    fn schedule_prompt_template_substitutes_skill_catalog_placeholders() {
        let cat = "- rss_fetch (aliases: rss) [enabled] — runner";
        let tpl = "SKILL=__SKILL_CATALOG__\nLEGACY=__SKILLS_CATALOG__";
        let out = crate::render_prompt_template(tpl, &[
            ("__SKILL_CATALOG__", cat),
            ("__SKILLS_CATALOG__", cat),
        ]);
        assert!(!out.contains("__SKILL_CATALOG__"));
        assert!(!out.contains("__SKILLS_CATALOG__"));
        assert!(out.contains("rss_fetch"));
    }

    #[test]
    fn validate_schedule_run_skill_news_fetcher_alias_resolves_to_rss_fetch() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "news_fetcher",
            "args": { "action": "latest", "category": "world" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
        assert_eq!(out.get("skill_name").and_then(|v| v.as_str()), Some("rss_fetch"));
    }

    #[test]
    fn validate_schedule_run_skill_unknown_skill_fails() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "totally_fake_skill_xyz",
            "args": { "action": "latest", "category": "tech" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload);
        assert!(out.is_err());
        assert!(out.unwrap_err().contains("unknown skill"));
    }

    #[test]
    fn validate_schedule_run_skill_alias_resolved_to_canonical() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "rss",
            "args": { "action": "latest", "category": "tech" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
        assert_eq!(out.get("skill_name").and_then(|v| v.as_str()), Some("rss_fetch"));
    }

    /// Schedule does not rewrite `rss_fetch` args; `fetch_feed` is handled inside the skill.
    #[test]
    fn validate_schedule_run_skill_rss_fetch_fetch_feed_passes_through() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "rss_fetch",
            "args": { "action": "fetch_feed", "category": "tech" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
        let args = out.get("args").and_then(|v| v.as_object()).unwrap();
        assert_eq!(args.get("action").and_then(|v| v.as_str()), Some("fetch_feed"));
        assert_eq!(args.get("category").and_then(|v| v.as_str()), Some("tech"));
    }

    #[test]
    fn validate_schedule_run_skill_args_must_be_object() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "rss_fetch",
            "args": "not_an_object"
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload);
        assert!(out.is_err());
        assert!(out.unwrap_err().contains("args"));
    }

    #[test]
    fn validate_schedule_run_skill_disabled_skill_fails() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "health_check",
            "args": {}
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload);
        assert!(out.is_err());
        assert!(out.unwrap_err().contains("disabled"));
    }

    #[test]
    fn validate_schedule_run_skill_valid_rss_fetch_kept() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "rss_fetch",
            "args": { "action": "latest", "category": "science" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
        assert_eq!(out.get("skill_name").and_then(|v| v.as_str()), Some("rss_fetch"));
        let args = out.get("args").and_then(|v| v.as_object()).unwrap();
        assert_eq!(args.get("action").and_then(|v| v.as_str()), Some("latest"));
        assert_eq!(args.get("category").and_then(|v| v.as_str()), Some("science"));
    }

    #[test]
    fn validate_schedule_run_skill_rss_fetch_legacy_action_not_rewritten() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "rss_fetch",
            "args": { "action": "fetch_crypto_news" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
        let args = out.get("args").and_then(|v| v.as_object()).unwrap();
        assert_eq!(args.get("action").and_then(|v| v.as_str()), Some("fetch_crypto_news"));
        assert!(!args.contains_key("category"));
    }

    #[test]
    fn validate_schedule_run_skill_rss_fetch_unknown_action_passes_through() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "rss_fetch",
            "args": { "action": "totally_fake_action" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
        assert_eq!(
            out.get("args")
                .and_then(|v| v.as_object())
                .and_then(|a| a.get("action"))
                .and_then(|v| v.as_str()),
            Some("totally_fake_action")
        );
    }

    /// Schedule does not validate `crypto` actions; bogus actions pass through for the skill to reject at runtime.
    #[test]
    fn validate_schedule_run_skill_crypto_action_passes_through_unvalidated() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "crypto",
            "args": { "action": "totally_bogus_crypto_action_xyz", "symbol": "BTCUSDT" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
        assert_eq!(out.get("skill_name").and_then(|v| v.as_str()), Some("crypto"));
        let args = out.get("args").and_then(|v| v.as_object()).unwrap();
        assert_eq!(
            args.get("action").and_then(|v| v.as_str()),
            Some("totally_bogus_crypto_action_xyz")
        );
    }

    /// Production must not embed rss_fetch legacy action normalization (skill owns aliases).
    #[test]
    fn schedule_production_has_no_rss_legacy_action_strings_in_validation() {
        const SRC: &str = include_str!("schedule_service.rs");
        let prod = SRC.split("#[cfg(test)]").next().unwrap_or(SRC);
        for needle in ["fetch_crypto_news", "fetch_tech_news", "fetch_news"] {
            assert!(
                !prod.contains(needle),
                "schedule layer must not embed rss legacy alias `{needle}` (handled in rss_fetch skill)"
            );
        }
    }

    /// Generic `run_skill` validation must not rewrite args (no per-skill merge paths or symbol subprocesses).
    #[test]
    fn validate_schedule_run_skill_args_pass_through_for_any_enabled_skill() {
        let reg = test_registry();
        let payload = json!({
            "skill_name": "demo_runner",
            "args": { "action": "noop", "symbol": "TEST" }
        });
        let out = validate_schedule_run_skill_with_registry(&reg, &payload).unwrap();
        assert_eq!(out.get("skill_name").and_then(|v| v.as_str()), Some("demo_runner"));
        let args = out.get("args").and_then(|v| v.as_object()).unwrap();
        assert_eq!(args.get("action").and_then(|v| v.as_str()), Some("noop"));
        assert_eq!(args.get("symbol").and_then(|v| v.as_str()), Some("TEST"));
        assert!(!args.contains_key("window_minutes"));
    }

    /// Schedule layer must not invoke arbitrary skills (only `schedule` compile via adapter).
    #[test]
    fn schedule_service_only_uses_adapter_for_schedule_compile_skill() {
        const SRC: &str = include_str!("schedule_service.rs");
        let n = SRC.matches("execution_adapters::run_skill").count();
        assert_eq!(
            n, 1,
            "schedule_service must only call execution_adapters::run_skill for schedule intent compile"
        );
        assert!(
            SRC.contains("run_skill(state, task, \"schedule\", compile_args)"),
            "adapter target must remain the schedule skill only"
        );
    }

    /// Production must stay free of legacy coin-monitoring business logic (profiles, merge, extract).
    #[test]
    fn schedule_service_production_source_has_no_legacy_coin_markers() {
        const SRC: &str = include_str!("schedule_service.rs");
        let prod = SRC.split("#[cfg(test)]").next().unwrap_or(SRC);
        let markers = [
            concat!("Cr", "ypto", "Price", "Alert", "Profile"),
            concat!("Existing", "Cr", "ypto", "Price", "Alert", "Job"),
            concat!("extract_", "cryp", "to", "_price", "_alert", "_profile"),
            concat!("load_", "existing", "_cryp", "to", "_price", "_alert", "_jobs"),
            concat!("schedule_", "content", "_matches"),
            concat!("normalize_", "direction"),
            concat!("normalize_", "threshold", "_pct"),
            concat!("create_", "exists", "_same"),
            concat!("update_", "existing", "_ok"),
        ];
        for m in markers {
            assert!(
                !prod.contains(m),
                "production source must not contain legacy coin-monitoring marker `{m}`"
            );
        }
    }

    fn try_handle_schedule_create_arm_source() -> &'static str {
        const SRC: &str = include_str!("schedule_service.rs");
        let start = SRC
            .find("\"create\" => {")
            .expect("schedule match arm create");
        let tail = &SRC[start..];
        let end_rel = tail
            .find("\n        _ => Ok(None),")
            .expect("end of schedule match (before catch-all arm)");
        &tail[..end_rel]
    }

    /// `create` arm: generic job insert only — no removed coin-monitoring helpers, preflight, or VIP update paths.
    #[test]
    fn schedule_create_arm_inserts_job_without_coin_business_branches() {
        let create_arm = try_handle_schedule_create_arm_source();
        assert!(
            create_arm.contains("INSERT INTO scheduled_jobs"),
            "create must persist via scheduled_jobs insert"
        );
        assert_eq!(
            create_arm.matches("INSERT INTO scheduled_jobs").count(),
            1,
            "create must perform a single insert (no alternate merge/update path)"
        );
        assert!(
            create_arm.contains("schedule.msg.create_ok"),
            "create success path must still use generic create_ok message key"
        );

        let forbidden = [
            concat!("cryp", "to"),
            concat!("price_", "alert", "_check"),
            concat!("price_", "monitor"),
            concat!("monitor_", "price"),
            concat!("volatility", "_alert"),
            concat!("binance", "_symbol", "_check"),
            concat!("de", "dupe"),
            concat!("Pro", "file"),
            concat!("Cr", "ypto", "Price", "Alert", "Profile"),
            concat!("Existing", "Cr", "ypto", "Price", "Alert", "Job"),
            concat!("extract_", "cryp", "to", "_price", "_alert", "_profile"),
            concat!("schedule_", "content", "_matches"),
            concat!("load_", "existing", "_cryp", "to", "_price", "_alert", "_jobs"),
            concat!("normalize_", "direction"),
            concat!("normalize_", "threshold", "_pct"),
        ];
        for needle in forbidden {
            assert!(
                !create_arm.contains(needle),
                "create arm must not contain coin-specific marker `{needle}`"
            );
        }
    }

    /// Root schedule intent few-shots must not embed built-in monitoring defaults (belong in target skills).
    #[test]
    fn schedule_intent_prompt_root_avoids_builtin_monitoring_defaults_in_examples() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("prompts/schedule_intent_prompt.md");
        let s = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            !s.contains("\"window_minutes\":15"),
            "schedule_intent_prompt must not spell default window 15 in examples"
        );
        assert!(
            !s.contains("\"threshold_pct\":5"),
            "schedule_intent_prompt must not spell default threshold 5 in examples"
        );
        assert!(
            !s.contains("\"direction\":\"both\""),
            "schedule_intent_prompt must not spell default direction both in examples"
        );
    }

    #[test]
    fn schedule_invocation_metadata_contains_required_keys() {
        let meta = schedule_invocation_metadata("job_abc123");
        let keys: std::collections::HashSet<_> = meta.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains("schedule_triggered"));
        assert!(keys.contains("schedule_job_id"));
        assert!(keys.contains("invocation_source"));
        assert!(keys.contains("scheduled"));
        let job_id = meta.iter().find(|(k, _)| *k == "schedule_job_id").map(|(_, v)| v);
        assert_eq!(job_id.and_then(|v| v.as_str()), Some("job_abc123"));
        let src = meta.iter().find(|(k, _)| *k == "invocation_source").map(|(_, v)| v);
        assert_eq!(src.and_then(|v| v.as_str()), Some("schedule"));
    }
}
