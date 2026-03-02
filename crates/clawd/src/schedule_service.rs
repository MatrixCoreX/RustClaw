use chrono::{Datelike, Duration as ChronoDuration, NaiveDateTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use rusqlite::params;
use serde_json::{Value, json};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{llm_gateway, memory, AppState, ClaimedTask, ScheduleIntentOutput};

pub(crate) async fn parse_schedule_intent(
    state: &AppState,
    task: &ClaimedTask,
    request: &str,
) -> Option<ScheduleIntentOutput> {
    let tz = parse_timezone(&state.schedule.timezone);
    let now_local = Utc::now().with_timezone(&tz);
    let (long_term_summary, preferences, recalled) = memory::service::recall_memory_context_parts(
        state,
        task.user_id,
        task.chat_id,
        request,
        state.memory.recall_limit.max(1),
        state.memory.schedule_memory_include_long_term,
        state.memory.schedule_memory_include_preferences,
    );
    let memory_context = memory::service::memory_context_block(
        long_term_summary.as_deref(),
        &preferences,
        &recalled,
        state.memory.schedule_memory_max_chars.max(384),
    );
    let prompt = state
        .schedule
        .intent_prompt_template
        .replace("__NOW__", &now_local.format("%Y-%m-%d %H:%M:%S %:z").to_string())
        .replace("__TIMEZONE__", &state.schedule.timezone)
        .replace("__RULES__", &state.schedule.intent_rules_template)
        .replace("__MEMORY_CONTEXT__", &memory_context)
        .replace("__REQUEST__", request);

    let llm_out = match llm_gateway::run_with_fallback(state, task, &prompt).await {
        Ok(v) => v,
        Err(err) => {
            warn!("parse_schedule_intent llm failed: task_id={} err={err}", task.task_id);
            return None;
        }
    };

    let json_str =
        crate::extract_json_object(&llm_out).or_else(|| crate::extract_first_json_object_any(&llm_out))?;
    let parsed: ScheduleIntentOutput = serde_json::from_str(&json_str).ok()?;
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
    raw.trim().parse::<Tz>().unwrap_or(chrono_tz::Asia::Shanghai)
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

pub(crate) async fn try_handle_schedule_request(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
) -> Result<Option<String>, String> {
    let Some(intent) = parse_schedule_intent(state, task, prompt).await else {
        return Ok(None);
    };

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
                    "SELECT job_id, schedule_type, time_of_day, weekday, every_minutes, timezone, enabled, next_run_at
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
                    ))
                })
                .map_err(|e| e.to_string())?;

            let mut lines = Vec::new();
            for row in rows {
                let (job_id, schedule_type, time_of_day, weekday, every_minutes, timezone, enabled, next_run_at) =
                    row.map_err(|e| e.to_string())?;
                let desc = schedule_kind_desc(state, &schedule_type, time_of_day, weekday, every_minutes);
                let status = if enabled == 1 {
                    schedule_t(state, "schedule.status.enabled")
                } else {
                    schedule_t(state, "schedule.status.paused")
                };
                let next = next_run_at.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string());
                lines.push(format!("- {} | {} | tz={} | {} | next={}", job_id, desc, timezone, status, next));
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
                return Ok(Some(schedule_t(state, "schedule.msg.create_fail_task_kind")));
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
                    if intent.schedule.time.trim().is_empty() {
                        None
                    } else {
                        Some(intent.schedule.time.trim())
                    },
                    if intent.schedule.weekday <= 0 {
                        None
                    } else {
                        Some(intent.schedule.weekday)
                    },
                    if intent.schedule.every_minutes <= 0 {
                        None
                    } else {
                        Some(intent.schedule.every_minutes)
                    },
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
                    v
                } else {
                    json!({ "text": prompt })
                }
            } else {
                intent.task.payload.clone()
            };

            let job_id = format!("job_{}", &Uuid::new_v4().simple().to_string()[..10]);
            let created_at = crate::now_ts();
            let db = state
                .db
                .lock()
                .map_err(|_| "db lock poisoned".to_string())?;
            db.execute(
                "INSERT INTO scheduled_jobs (
                    job_id, user_id, chat_id, schedule_type, run_at, time_of_day, weekday, every_minutes, cron_expr,
                    timezone, task_kind, task_payload_json, enabled, notify_on_success, notify_on_failure,
                    last_run_at, next_run_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, ?11, 1, 1, 1, NULL, ?12, ?13, ?13)",
                params![
                    job_id,
                    task.user_id,
                    task.chat_id,
                    schedule_type,
                    run_at,
                    if intent.schedule.time.trim().is_empty() {
                        None::<String>
                    } else {
                        Some(intent.schedule.time.trim().to_string())
                    },
                    if intent.schedule.weekday <= 0 {
                        None::<i64>
                    } else {
                        Some(intent.schedule.weekday)
                    },
                    if intent.schedule.every_minutes <= 0 {
                        None::<i64>
                    } else {
                        Some(intent.schedule.every_minutes)
                    },
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
                    ("next_run_at", &next_run_at.to_string()),
                ],
            )))
        }
        _ => Ok(None),
    }
}
