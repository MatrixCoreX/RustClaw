use chrono::{Datelike, Duration as ChronoDuration, NaiveDateTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{execution_adapters, llm_gateway, memory, AppState, ClaimedTask, ScheduleIntentOutput};

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
    let prompt = crate::render_prompt_template(
        &state.schedule.intent_prompt_template,
        &[
            (
                "__NOW__",
                &now_local.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
            ),
            ("__TIMEZONE__", &state.schedule.timezone),
            ("__RULES__", &state.schedule.intent_rules_template),
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

#[derive(Debug, Clone, PartialEq)]
struct CryptoPriceAlertProfile {
    symbol: String,
    direction: String,
    window_minutes: u64,
    threshold_pct: f64,
}

#[derive(Debug, Clone)]
struct ExistingCryptoPriceAlertJob {
    job_id: String,
    channel: String,
    external_user_id: Option<String>,
    external_chat_id: Option<String>,
    schedule_type: String,
    run_at: Option<i64>,
    time_of_day: Option<String>,
    weekday: Option<i64>,
    every_minutes: Option<i64>,
    timezone: String,
    next_run_at: Option<i64>,
    profile: CryptoPriceAlertProfile,
}

fn normalize_direction(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "up" | "long" | "rise" => "up".to_string(),
        "down" | "short" | "fall" => "down".to_string(),
        _ => "both".to_string(),
    }
}

fn normalize_threshold_pct(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

fn extract_crypto_price_alert_profile(payload: &Value) -> Option<CryptoPriceAlertProfile> {
    let obj = payload.as_object()?;
    let skill_name = obj
        .get("skill_name")
        .or_else(|| obj.get("skill"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if skill_name != "crypto" {
        return None;
    }
    let args = obj.get("args")?.as_object()?;
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        action.as_str(),
        "price_alert_check"
            | "price_monitor"
            | "monitor_price"
            | "price_alert"
            | "volatility_alert"
    ) {
        return None;
    }
    let symbol = args
        .get("symbol")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_ascii_uppercase();
    let direction = normalize_direction(
        args.get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("both"),
    );
    let window_minutes = args
        .get("window_minutes")
        .or_else(|| args.get("minutes"))
        .and_then(|v| v.as_u64())
        .unwrap_or(15)
        .max(1);
    let threshold_pct = normalize_threshold_pct(
        args.get("threshold_pct")
            .or_else(|| args.get("pct"))
            .and_then(|v| v.as_f64())
            .unwrap_or(5.0)
            .abs(),
    );
    Some(CryptoPriceAlertProfile {
        symbol,
        direction,
        window_minutes,
        threshold_pct,
    })
}

fn schedule_content_matches(
    existing: &ExistingCryptoPriceAlertJob,
    schedule_type: &str,
    run_at: Option<i64>,
    time_of_day: Option<&str>,
    weekday: Option<i64>,
    every_minutes: Option<i64>,
    timezone: &str,
) -> bool {
    let existing_time = existing
        .time_of_day
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let input_time = time_of_day.map(str::trim).filter(|v| !v.is_empty());
    existing.schedule_type == schedule_type
        && existing.run_at == run_at
        && existing_time == input_time
        && existing.weekday == weekday
        && existing.every_minutes == every_minutes
        && existing.timezone.trim() == timezone.trim()
}

fn schedule_channel_binding_matches(existing: &ExistingCryptoPriceAlertJob, task: &ClaimedTask) -> bool {
    existing.channel.trim().eq_ignore_ascii_case(task.channel.trim())
        && existing.external_user_id == task.external_user_id
        && existing.external_chat_id == task.external_chat_id
}

fn load_existing_crypto_price_alert_jobs(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
) -> Result<Vec<ExistingCryptoPriceAlertJob>, String> {
    let mut stmt = db
        .prepare(
            "SELECT job_id, channel, external_user_id, external_chat_id, schedule_type, run_at, time_of_day, weekday, every_minutes, timezone, next_run_at, task_payload_json
             FROM scheduled_jobs
             WHERE user_id = ?1 AND chat_id = ?2 AND task_kind = 'run_skill'
             ORDER BY enabled DESC, id DESC
             LIMIT 100",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![user_id, chat_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, String>(11)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows {
        let (
            job_id,
            channel,
            external_user_id,
            external_chat_id,
            schedule_type,
            run_at,
            time_of_day,
            weekday,
            every_minutes,
            timezone,
            next_run_at,
            task_payload_json,
        ) = row.map_err(|e| e.to_string())?;
        let payload =
            serde_json::from_str::<Value>(&task_payload_json).unwrap_or_else(|_| json!({}));
        let Some(profile) = extract_crypto_price_alert_profile(&payload) else {
            continue;
        };
        out.push(ExistingCryptoPriceAlertJob {
            job_id,
            channel,
            external_user_id,
            external_chat_id,
            schedule_type,
            run_at,
            time_of_day,
            weekday,
            every_minutes,
            timezone,
            next_run_at,
            profile,
        });
    }
    Ok(out)
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

            if task_kind == "run_skill" {
                if let Some(profile) = extract_crypto_price_alert_profile(&payload) {
                    let check_args = json!({
                        "action": "binance_symbol_check",
                        "symbol": profile.symbol
                    });
                    if let Err(err) =
                        execution_adapters::run_skill(state, task, "crypto", check_args).await
                    {
                        return Ok(Some(err));
                    }

                    let db = state
                        .db
                        .lock()
                        .map_err(|_| "db lock poisoned".to_string())?;
                    let existing_jobs =
                        load_existing_crypto_price_alert_jobs(&db, task.user_id, task.chat_id)?;

                    if let Some(existing) = existing_jobs.iter().find(|v| {
                        v.profile == profile
                            && schedule_content_matches(
                                v,
                                &schedule_type,
                                run_at,
                                time_of_day.as_deref(),
                                weekday,
                                every_minutes,
                                &timezone,
                            )
                            && schedule_channel_binding_matches(v, task)
                    }) {
                        let effective_next_run = existing.next_run_at.unwrap_or(next_run_at);
                        let next_run_human = humanize_next_run_at(effective_next_run, &timezone);
                        let task_content = summarize_task_content(&task_kind, &payload, prompt);
                        return Ok(Some(schedule_t_with(
                            state,
                            "schedule.msg.create_exists_same",
                            &[
                                ("job_id", &existing.job_id),
                                ("type", &intent.schedule.r#type),
                                ("timezone", &timezone),
                                ("next_run_human", &next_run_human),
                                ("task_content", &task_content),
                            ],
                        )));
                    }

                    if let Some(existing) = existing_jobs
                        .iter()
                        .find(|v| v.profile.symbol == profile.symbol)
                    {
                        let updated_at = crate::now_ts();
                        db.execute(
                            "UPDATE scheduled_jobs
                             SET schedule_type = ?4,
                                 run_at = ?5,
                                 time_of_day = ?6,
                                 weekday = ?7,
                                 every_minutes = ?8,
                                 timezone = ?9,
                                 task_payload_json = ?10,
                                 channel = ?11,
                                 external_user_id = ?12,
                                 external_chat_id = ?13,
                                 enabled = 1,
                                 next_run_at = ?14,
                                 updated_at = ?15
                             WHERE job_id = ?1 AND user_id = ?2 AND chat_id = ?3",
                            params![
                                existing.job_id,
                                task.user_id,
                                task.chat_id,
                                schedule_type,
                                run_at,
                                time_of_day,
                                weekday,
                                every_minutes,
                                timezone,
                                payload.to_string(),
                                task.channel,
                                task.external_user_id,
                                task.external_chat_id,
                                next_run_at,
                                updated_at,
                            ],
                        )
                        .map_err(|e| e.to_string())?;
                        let next_run_human = humanize_next_run_at(next_run_at, &timezone);
                        let task_content = summarize_task_content(&task_kind, &payload, prompt);
                        return Ok(Some(schedule_t_with(
                            state,
                            "schedule.msg.update_existing_ok",
                            &[
                                ("job_id", &existing.job_id),
                                ("type", &intent.schedule.r#type),
                                ("timezone", &timezone),
                                ("next_run_human", &next_run_human),
                                ("task_content", &task_content),
                            ],
                        )));
                    }
                }
            }

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
