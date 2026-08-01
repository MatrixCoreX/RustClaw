//! 离线老黄历查询：单行 JSON stdin -> 单行 JSON stdout。

use std::io::{self, BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};

use chrono::{Datelike, Local, NaiveDate};
use lunar_rust::{
    lunar::LunarRefHelper,
    solar::{self, SolarRefHelper},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const SKILL_NAME: &str = "chinese_almanac";
const DISCLAIMER: &str =
    "老黄历内容属于传统民俗信息，仅供文化参考，不应替代医疗、法律、财务、安全或其他专业决策。";

#[derive(Debug, Deserialize)]
struct Request {
    request_id: String,
    args: Value,
    #[serde(default, rename = "context")]
    _context: Option<Value>,
    #[serde(default, rename = "user_id")]
    _user_id: Option<i64>,
    #[serde(default, rename = "chat_id")]
    _chat_id: Option<i64>,
}

#[derive(Debug, Serialize)]
struct Response {
    request_id: String,
    status: String,
    text: String,
    extra: Value,
    error_text: Option<String>,
}

#[derive(Debug)]
struct SkillError {
    code: &'static str,
    message: String,
}

impl SkillError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailLevel {
    Summary,
    Full,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let response = handle_line(&line?);
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle_line(line: &str) -> Response {
    match serde_json::from_str::<Request>(line) {
        Ok(request) => match execute(&request.args) {
            Ok((text, extra)) => Response {
                request_id: request.request_id,
                status: "ok".to_string(),
                text,
                extra,
                error_text: None,
            },
            Err(error) => error_response(request.request_id, error),
        },
        Err(error) => error_response(
            "unknown".to_string(),
            SkillError::new("invalid_input", format!("输入不是有效的请求 JSON：{error}")),
        ),
    }
}

fn error_response(request_id: String, error: SkillError) -> Response {
    Response {
        request_id,
        status: "error".to_string(),
        text: String::new(),
        extra: json!({
            "schema_version": 1,
            "source_skill": SKILL_NAME,
            "status": "error",
            "error_code": error.code,
            "message_key": format!("skill.{SKILL_NAME}.{}", error.code),
            "retryable": false,
        }),
        error_text: Some(error.message),
    }
}

fn execute(args: &Value) -> Result<(String, Value), SkillError> {
    let object = args
        .as_object()
        .ok_or_else(|| SkillError::new("invalid_arguments", "args 必须是 JSON object"))?;
    let action = optional_string(object, "action")?.unwrap_or("query");
    if !matches!(action, "query" | "lookup") {
        return Err(SkillError::new(
            "unsupported_action",
            "不支持该 action；请使用 query",
        ));
    }

    let detail = match optional_string(object, "detail")?.unwrap_or("full") {
        "summary" => DetailLevel::Summary,
        "full" => DetailLevel::Full,
        _ => {
            return Err(SkillError::new(
                "invalid_detail",
                "detail 只能是 summary 或 full",
            ))
        }
    };
    let sect = optional_i64(object, "yi_ji_sect")?.unwrap_or(2);
    if !matches!(sect, 1 | 2) {
        return Err(SkillError::new(
            "invalid_yi_ji_sect",
            "yi_ji_sect 只能是 1 或 2",
        ));
    }
    let date = resolve_date(object)?;

    catch_unwind(AssertUnwindSafe(|| build_result(date, detail, sect))).map_err(|_| {
        SkillError::new(
            "unsupported_date",
            "底层历法库无法可靠计算该日期，请换一个日期后重试",
        )
    })?
}

fn resolve_date(args: &Map<String, Value>) -> Result<NaiveDate, SkillError> {
    let date_text = optional_string(args, "date")?;
    let components_present = ["year", "month", "day"]
        .iter()
        .filter(|key| args.contains_key(**key))
        .count();
    if date_text.is_some() && components_present > 0 {
        return Err(SkillError::new(
            "ambiguous_date",
            "date 与 year/month/day 不能同时提供",
        ));
    }

    let base = if let Some(raw) = date_text {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|_| SkillError::new("invalid_date", "date 必须是有效的 YYYY-MM-DD 日期"))?
    } else if components_present > 0 {
        if components_present != 3 {
            return Err(SkillError::new(
                "incomplete_date",
                "使用日期分量时必须同时提供 year、month、day",
            ));
        }
        let year = required_i64(args, "year")?;
        let month = required_i64(args, "month")?;
        let day = required_i64(args, "day")?;
        let year = i32::try_from(year)
            .map_err(|_| SkillError::new("invalid_date", "year 超出可表示范围"))?;
        let month = u32::try_from(month)
            .map_err(|_| SkillError::new("invalid_date", "month 必须是有效月份"))?;
        let day = u32::try_from(day)
            .map_err(|_| SkillError::new("invalid_date", "day 必须是有效日期"))?;
        NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| SkillError::new("invalid_date", "year/month/day 不是有效日期"))?
    } else {
        Local::now().date_naive()
    };

    let offset = optional_i64(args, "offset_days")?.unwrap_or(0);
    let delta = chrono::Duration::try_days(offset)
        .ok_or_else(|| SkillError::new("invalid_date", "offset_days 超出可表示范围"))?;
    base.checked_add_signed(delta)
        .ok_or_else(|| SkillError::new("invalid_date", "offset_days 使日期超出可表示范围"))
}

fn optional_string<'a>(
    args: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, SkillError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.trim())),
        Some(Value::String(_)) => Err(SkillError::new(
            "invalid_arguments",
            format!("{key} 不能为空"),
        )),
        Some(_) => Err(SkillError::new(
            "invalid_arguments",
            format!("{key} 必须是字符串"),
        )),
    }
}

fn optional_i64(args: &Map<String, Value>, key: &str) -> Result<Option<i64>, SkillError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| SkillError::new("invalid_arguments", format!("{key} 必须是整数"))),
    }
}

fn required_i64(args: &Map<String, Value>, key: &str) -> Result<i64, SkillError> {
    optional_i64(args, key)?
        .ok_or_else(|| SkillError::new("incomplete_date", format!("缺少 {key}")))
}

fn build_result(
    date: NaiveDate,
    detail: DetailLevel,
    sect: i64,
) -> Result<(String, Value), SkillError> {
    let solar = solar::from_ymd(
        i64::from(date.year()),
        i64::from(date.month()),
        i64::from(date.day()),
    );
    let lunar = solar.get_lunar();

    let date_text = date.format("%Y-%m-%d").to_string();
    let weekday = format!("星期{}", solar.clone().get_week_in_chinese());
    let lunar_text = lunar.to_string();
    let lunar_year = lunar.get_year();
    let lunar_month_raw = lunar.get_month();
    let lunar_day = lunar.get_day();
    let lunar_month = lunar_month_raw.abs();
    let is_leap_month = lunar_month_raw < 0;
    let zodiac = lunar.get_year_sheng_xiao();
    let year_ganzhi = lunar.get_year_in_gan_zhi();
    let month_ganzhi = lunar.get_month_in_gan_zhi_exact();
    let day_ganzhi = lunar.get_day_in_gan_zhi();
    let solar_term = empty_to_none(lunar.get_jie_qi());
    let solar_festivals = solar.clone().get_festivals();
    let solar_other_festivals = solar.get_other_festivals();
    let lunar_festivals = lunar.get_festivals();
    let lunar_other_festivals = lunar.get_other_festivals();
    let yi = lunar.get_day_yi(Some(sect));
    let ji = lunar.get_day_ji(Some(sect));
    let auspicious_spirits = lunar.get_day_ji_shen();
    let inauspicious_spirits = lunar.get_day_xiong_sha();
    let day_officer = lunar.get_zhi_xing();
    let day_god = lunar.get_day_tian_shen();
    let day_god_type = lunar.get_day_tian_shen_type();
    let day_god_luck = lunar.get_day_tian_shen_luck();
    let lunar_mansion = lunar.get_xiu();
    let lunar_mansion_luck = lunar.get_xiu_luck();
    let clash = lunar.get_day_chong_desc();
    let sha = lunar.get_day_sha();
    let pengzu = vec![lunar.get_peng_zu_gan(), lunar.get_peng_zu_zhi()];
    let fetal_god = lunar.get_day_position_tai();
    let directions = json!({
        "joy": {"trigram": lunar.get_day_position_xi(), "direction": lunar.get_day_position_xi_desc()},
        "fortune": {"trigram": lunar.get_day_position_fu(Some(2)), "direction": lunar.get_day_position_fu_desc(Some(2))},
        "wealth": {"trigram": lunar.get_day_position_cai(), "direction": lunar.get_day_position_cai_desc()},
        "yang_noble": {"trigram": lunar.get_day_position_yang_gui(), "direction": lunar.get_day_position_yang_gui_desc()},
        "yin_noble": {"trigram": lunar.get_day_position_yin_gui(), "direction": lunar.get_day_position_yin_gui_desc()},
    });

    let mut lines = vec![
        format!("{date_text} {weekday}"),
        format!("农历：{lunar_text}（{year_ganzhi}年，生肖{zodiac}）"),
        format!("干支：{year_ganzhi}年 {month_ganzhi}月 {day_ganzhi}日"),
    ];
    if let Some(term) = &solar_term {
        lines.push(format!("节气：{term}"));
    }
    let all_festivals = combine_festivals(&[
        &solar_festivals,
        &solar_other_festivals,
        &lunar_festivals,
        &lunar_other_festivals,
    ]);
    if !all_festivals.is_empty() {
        lines.push(format!("节日：{}", all_festivals.join("、")));
    }
    lines.extend([
        format!("宜：{}", join_or_none(&yi)),
        format!("忌：{}", join_or_none(&ji)),
        format!("值日：{day_officer}日，{day_god}（{day_god_type}，{day_god_luck}）"),
        format!("冲煞：冲{clash}，煞{sha}"),
    ]);
    if detail == DetailLevel::Full {
        lines.extend([
            format!("吉神宜趋：{}", join_or_none(&auspicious_spirits)),
            format!("凶煞宜忌：{}", join_or_none(&inauspicious_spirits)),
            format!("彭祖百忌：{}", pengzu.join("；")),
            format!("二十八宿：{lunar_mansion}（{lunar_mansion_luck}）"),
            format!("胎神：{fetal_god}"),
        ]);
    }
    lines.push(format!("说明：{DISCLAIMER}"));

    let extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "ok",
        "action": "query",
        "date": date_text,
        "weekday": weekday,
        "detail": match detail { DetailLevel::Summary => "summary", DetailLevel::Full => "full" },
        "lunar": {
            "year": lunar_year,
            "month": lunar_month,
            "day": lunar_day,
            "is_leap_month": is_leap_month,
            "text": lunar_text,
        },
        "ganzhi": {
            "year": year_ganzhi,
            "month": month_ganzhi,
            "day": day_ganzhi,
        },
        "zodiac": zodiac,
        "solar_term": solar_term,
        "festivals": {
            "solar": solar_festivals,
            "solar_other": solar_other_festivals,
            "lunar": lunar_festivals,
            "lunar_other": lunar_other_festivals,
        },
        "almanac": {
            "yi": yi,
            "ji": ji,
            "yi_ji_sect": sect,
            "auspicious_spirits": auspicious_spirits,
            "inauspicious_spirits": inauspicious_spirits,
            "day_officer": day_officer,
            "day_god": day_god,
            "day_god_type": day_god_type,
            "day_god_luck": day_god_luck,
            "lunar_mansion": lunar_mansion,
            "lunar_mansion_luck": lunar_mansion_luck,
            "clash": clash,
            "sha": sha,
            "pengzu": pengzu,
            "fetal_god": fetal_god,
            "directions": directions,
        },
        "basis": {
            "calendar_convention": "traditional_chinese_lunisolar",
            "library": "lunar_rust",
            "library_version": "1.0.1",
            "offline": true,
        },
        "disclaimer": DISCLAIMER,
    });
    Ok((lines.join("\n"), extra))
}

fn empty_to_none(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn combine_festivals(groups: &[&Vec<String>]) -> Vec<String> {
    let mut result = Vec::new();
    for item in groups.iter().flat_map(|group| group.iter()) {
        if !result.contains(item) {
            result.push(item.clone());
        }
    }
    result
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "无".to_string()
    } else {
        values.join("、")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(args: Value) -> String {
        json!({
            "request_id": "test-1",
            "args": args,
            "context": null,
            "user_id": 1,
            "chat_id": 1
        })
        .to_string()
    }

    #[test]
    fn spring_festival_2024_has_expected_lunar_date() {
        let response = handle_line(&request(json!({"date": "2024-02-10"})));
        assert_eq!(response.status, "ok");
        assert_eq!(response.extra["lunar"]["year"], 2024);
        assert_eq!(response.extra["lunar"]["month"], 1);
        assert_eq!(response.extra["lunar"]["day"], 1);
        assert_eq!(response.extra["lunar"]["is_leap_month"], false);
        assert_eq!(response.extra["ganzhi"]["year"], "甲辰");
        assert_eq!(response.extra["zodiac"], "龙");
        assert!(response.extra["festivals"]["lunar"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "春节"));
    }

    #[test]
    fn offset_days_resolves_relative_date() {
        let response = handle_line(&request(json!({
            "date": "2024-02-09",
            "offset_days": 1,
            "detail": "summary"
        })));
        assert_eq!(response.status, "ok");
        assert_eq!(response.extra["date"], "2024-02-10");
        assert_eq!(response.extra["detail"], "summary");
    }

    #[test]
    fn full_result_contains_structured_almanac_and_disclaimer() {
        let response = handle_line(&request(json!({"date": "2026-08-01"})));
        assert_eq!(response.status, "ok");
        assert!(response.extra["almanac"]["yi"].is_array());
        assert!(response.extra["almanac"]["ji"].is_array());
        assert_eq!(response.extra["basis"]["offline"], true);
        assert!(response.text.contains("传统民俗信息"));
    }

    #[test]
    fn invalid_date_returns_canonical_error_contract() {
        let response = handle_line(&request(json!({"date": "2024-02-30"})));
        assert_eq!(response.status, "error");
        assert_eq!(response.extra["status"], "error");
        assert_eq!(response.extra["error_code"], "invalid_date");
        assert_eq!(
            response.extra["message_key"],
            "skill.chinese_almanac.invalid_date"
        );
        assert_eq!(response.extra["retryable"], false);
    }

    #[test]
    fn component_date_requires_all_three_fields() {
        let response = handle_line(&request(json!({"year": 2024, "month": 2})));
        assert_eq!(response.status, "error");
        assert_eq!(response.extra["error_code"], "incomplete_date");
    }
}
