//! 天气技能：单行 JSON stdin -> 单行 JSON stdout，调用 Open-Meteo；支持 i18n 与多日预报钳制说明。

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml::Value as TomlValue;

const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
/// Open-Meteo 免费接口预报天数上限；超出时在 `extra` 中标注并钳制为此值。
const MAX_FORECAST_DAYS: u32 = 16;
const HTTP_RETRY_ATTEMPTS: usize = 3;

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct WeatherRootConfig {
    #[serde(default)]
    weather: WeatherSection,
}

#[derive(Debug, Deserialize, Default)]
struct WeatherSection {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    i18n_path: Option<String>,
}

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct ForecastDaysSpec {
    requested: u32,
    applied: u32,
    capped: bool,
}

#[derive(Debug, Deserialize)]
struct GeocodeResults {
    results: Option<Vec<GeocodeResult>>,
}

#[derive(Debug, Deserialize)]
struct GeocodeResult {
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    country: String,
    #[serde(default)]
    admin1: String,
}

#[derive(Debug, Deserialize)]
struct ForecastResponse {
    current_weather: Option<CurrentWeather>,
}

#[derive(Debug, Deserialize)]
struct CurrentWeather {
    temperature: f64,
    windspeed: f64,
    winddirection: f64,
    weathercode: u32,
    is_day: u8,
}

#[derive(Debug, Clone)]
struct CurrentWeatherResult {
    text: String,
    temperature: f64,
    weather_code: u32,
    weather_desc: String,
}

#[derive(Debug, Deserialize)]
struct DailyForecastResponse {
    daily: Option<DailySeries>,
}

#[derive(Debug, Deserialize)]
struct DailySeries {
    time: Vec<String>,
    weathercode: Vec<u32>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                let cfg = load_weather_config(&workspace_root);
                let lang = resolve_lang(&req, &cfg);
                let cat = load_catalog(&workspace_root, &cfg, &lang);
                match execute(&req.args, &cat, &lang) {
                    Ok((text, extra)) => Resp {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text,
                        extra: Some(extra),
                        error_text: None,
                    },
                    Err(err) => Resp {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        extra: None,
                        error_text: Some(err),
                    },
                }
            }
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: None,
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn load_weather_config(workspace_root: &Path) -> WeatherRootConfig {
    let path = workspace_root.join("configs/weather.toml");
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return WeatherRootConfig::default(),
    };
    toml::from_str::<WeatherRootConfig>(&raw).unwrap_or_default()
}

fn resolve_lang(req: &Req, cfg: &WeatherRootConfig) -> String {
    if let Some(obj) = req.args.as_object() {
        if let Some(s) = obj
            .get("locale")
            .or_else(|| obj.get("lang"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return normalize_lang_tag(s);
        }
    }
    if let Some(ctx) = &req.context {
        if let Some(obj) = ctx.as_object() {
            for key in ["locale", "language", "lang"] {
                if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                    let t = s.trim();
                    if !t.is_empty() {
                        return normalize_lang_tag(t);
                    }
                }
            }
        }
    }
    cfg.weather
        .language
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_lang_tag)
        .unwrap_or_else(|| "zh-CN".to_string())
}

fn normalize_lang_tag(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase().replace('_', "-");
    match lower.as_str() {
        "zh" | "zh-cn" => "zh-CN".to_string(),
        "en" | "en-us" => "en-US".to_string(),
        _ => raw.trim().to_string(),
    }
}

fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: TomlValue = toml::from_str(&raw).ok()?;
    let dict = value.get("dict")?.as_table()?;
    let mut out = HashMap::new();
    for (k, v) in dict {
        collect_i18n_entries(k, v, &mut out);
    }
    Some(out)
}

fn collect_i18n_entries(prefix: &str, value: &TomlValue, out: &mut HashMap<String, String>) {
    if let Some(text) = value.as_str() {
        out.insert(prefix.to_string(), text.to_string());
        return;
    }
    if let Some(table) = value.as_table() {
        for (k, v) in table {
            let next = format!("{prefix}.{k}");
            collect_i18n_entries(&next, v, out);
        }
    }
}

fn default_embedded_strings() -> HashMap<String, String> {
    // 仅作缺失文件时的兜底（英文）
    let mut m = HashMap::new();
    m.insert(
        "weather.err.need_location".to_string(),
        "Provide city or latitude + longitude".to_string(),
    );
    m.insert(
        "weather.err.args_object".to_string(),
        "args must be object".to_string(),
    );
    m
}

fn load_catalog(workspace_root: &Path, cfg: &WeatherRootConfig, lang: &str) -> TextCatalog {
    let mut current = default_embedded_strings();
    let path = cfg
        .weather
        .i18n_path
        .as_deref()
        .map(|p| workspace_root.join(p))
        .unwrap_or_else(|| workspace_root.join(format!("configs/i18n/weather.{lang}.toml")));
    if let Some(overrides) = load_external_i18n(&path) {
        for (k, v) in overrides {
            current.insert(k, v);
        }
    } else if lang != "en-US" {
        let fb = workspace_root.join("configs/i18n/weather.en-US.toml");
        if let Some(overrides) = load_external_i18n(&fb) {
            for (k, v) in overrides {
                current.entry(k).or_insert(v);
            }
        }
    }
    TextCatalog { current }
}

fn tr(cat: &TextCatalog, key: &str) -> String {
    cat.current
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn tr_with(cat: &TextCatalog, key: &str, vars: &[(&str, &str)]) -> String {
    let mut out = tr(cat, key);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn execute(args: &Value, cat: &TextCatalog, lang: &str) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| tr(cat, "weather.err.args_object"))?;

    let city = obj
        .get("city")
        .or_else(|| obj.get("location"))
        .or_else(|| obj.get("place"))
        .or_else(|| obj.get("q"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let display_location = obj
        .get("display_location")
        .or_else(|| obj.get("requested_location"))
        .or_else(|| obj.get("original_location"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let lat = obj.get("latitude").and_then(|v| v.as_f64());
    let lon = obj.get("longitude").and_then(|v| v.as_f64());

    let (lat, lon, place_name) = match (city, lat, lon) {
        (Some(c), _, _) => {
            let (lat, lon, name) = geocode(c, cat)?;
            (lat, lon, name)
        }
        (None, Some(lat), Some(lon)) => (
            lat,
            lon,
            tr_with(
                cat,
                "weather.msg.coord_place",
                &[("lat", &format!("{lat:.2}")), ("lon", &format!("{lon:.2}"))],
            ),
        ),
        _ => return Err(tr(cat, "weather.err.need_location")),
    };

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let location = weather_location_display(display_location, city, &place_name);

    if let Some(spec) = parse_forecast_days_arg(obj, cat)? {
        let text = fetch_daily_forecast(&client, lat, lon, &place_name, &spec, cat)?;
        let extra = json!({
            "action": "query",
            "mode": "daily",
            "locale": lang,
            "location": location,
            "resolved_location": place_name,
            "latitude": lat,
            "longitude": lon,
            "forecast_days_requested": spec.requested,
            "forecast_days_applied": spec.applied,
            "forecast_days_capped": spec.capped,
        });
        Ok((text, extra))
    } else {
        let current = fetch_current_weather(&client, lat, lon, &place_name, cat)?;
        let extra = json!({
            "action": "query",
            "mode": "current",
            "locale": lang,
            "location": location,
            "resolved_location": place_name,
            "latitude": lat,
            "longitude": lon,
            "temperature": current.temperature,
            "weather_code": current.weather_desc,
            "weather_code_raw": current.weather_code,
        });
        Ok((current.text, extra))
    }
}

fn weather_location_display(
    display_location: Option<&str>,
    city: Option<&str>,
    resolved_place_name: &str,
) -> String {
    display_location
        .or(city)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(resolved_place_name)
        .to_string()
}

fn parse_forecast_days_arg(
    obj: &Map<String, Value>,
    cat: &TextCatalog,
) -> Result<Option<ForecastDaysSpec>, String> {
    let key = if obj.contains_key("days") {
        Some("days")
    } else if obj.contains_key("forecast_days") {
        Some("forecast_days")
    } else {
        None
    };
    let Some(k) = key else {
        return Ok(None);
    };
    let v = obj.get(k).ok_or_else(|| format!("missing {k}"))?;
    let n = json_to_u32(v).ok_or_else(|| tr(cat, "weather.err.days_not_number"))?;
    if n == 0 {
        return Err(tr(cat, "weather.err.days_zero"));
    }
    let capped = n > MAX_FORECAST_DAYS;
    let applied = n.min(MAX_FORECAST_DAYS);
    Ok(Some(ForecastDaysSpec {
        requested: n,
        applied,
        capped,
    }))
}

fn json_to_u32(v: &Value) -> Option<u32> {
    if let Some(u) = v.as_u64() {
        return u32::try_from(u).ok();
    }
    if let Some(i) = v.as_i64() {
        if i >= 0 {
            return u32::try_from(i).ok();
        }
        return None;
    }
    v.as_f64()
        .filter(|f| f.is_finite() && *f >= 0.0)
        .map(|f| f.round() as u64)
        .and_then(|u| u32::try_from(u).ok())
}

fn wmo_weather_desc(cat: &TextCatalog, code: u32) -> String {
    let key = format!("weather.wmo.{code}");
    cat.current
        .get(&key)
        .cloned()
        .or_else(|| cat.current.get("weather.wmo._").cloned())
        .unwrap_or_else(|| tr(cat, "weather.wmo._"))
}

fn send_get_with_retry(client: &Client, url: &str) -> Result<Response, reqwest::Error> {
    let mut last_err = None;
    for attempt in 0..HTTP_RETRY_ATTEMPTS {
        match client.get(url).send() {
            Ok(res) => return Ok(res),
            Err(err) => {
                last_err = Some(err);
                if attempt + 1 < HTTP_RETRY_ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(
                        250 * (attempt as u64 + 1),
                    ));
                }
            }
        }
    }
    Err(last_err.expect("retry attempts always records the last error"))
}

fn fetch_current_weather(
    client: &Client,
    lat: f64,
    lon: f64,
    place_name: &str,
    cat: &TextCatalog,
) -> Result<CurrentWeatherResult, String> {
    let url = format!(
        "{}?latitude={}&longitude={}&current_weather=true",
        FORECAST_URL, lat, lon
    );
    let res = send_get_with_retry(client, &url).map_err(|e| {
        tr_with(
            cat,
            "weather.err.request_failed",
            &[("error", &e.to_string())],
        )
    })?;
    if !res.status().is_success() {
        return Err(tr_with(
            cat,
            "weather.err.current_http",
            &[("status", &res.status().to_string())],
        ));
    }
    let body: ForecastResponse = res.json().map_err(|e| {
        tr_with(
            cat,
            "weather.err.current_parse",
            &[("error", &e.to_string())],
        )
    })?;

    let cur = body
        .current_weather
        .ok_or_else(|| tr(cat, "weather.err.current_no_data"))?;

    let desc = wmo_weather_desc(cat, cur.weathercode);
    let day_night_key = if cur.is_day == 1 {
        "weather.msg.day_night_day"
    } else {
        "weather.msg.day_night_night"
    };
    let day_night = tr(cat, day_night_key);
    let text = tr_with(
        cat,
        "weather.msg.current_line",
        &[
            ("place", place_name),
            ("day_night", &day_night),
            ("desc", &desc),
            ("temp", &format!("{}", cur.temperature)),
            ("wind", &format!("{}", cur.windspeed)),
            ("dir", &format!("{}", cur.winddirection as u32)),
        ],
    );
    Ok(CurrentWeatherResult {
        text,
        temperature: cur.temperature,
        weather_code: cur.weathercode,
        weather_desc: desc,
    })
}

fn fetch_daily_forecast(
    client: &Client,
    lat: f64,
    lon: f64,
    place_name: &str,
    spec: &ForecastDaysSpec,
    cat: &TextCatalog,
) -> Result<String, String> {
    let days = spec.applied;
    let url = format!(
        "{}?latitude={}&longitude={}&daily=weathercode,temperature_2m_max,temperature_2m_min&forecast_days={}&timezone=auto",
        FORECAST_URL, lat, lon, days
    );
    let res = send_get_with_retry(client, &url).map_err(|e| {
        tr_with(
            cat,
            "weather.err.forecast_request_failed",
            &[("error", &e.to_string())],
        )
    })?;
    if !res.status().is_success() {
        return Err(tr_with(
            cat,
            "weather.err.forecast_http",
            &[("status", &res.status().to_string())],
        ));
    }
    let body: DailyForecastResponse = res.json().map_err(|e| {
        tr_with(
            cat,
            "weather.err.forecast_parse",
            &[("error", &e.to_string())],
        )
    })?;
    let daily = body
        .daily
        .ok_or_else(|| tr(cat, "weather.err.forecast_no_daily"))?;
    let n = daily.time.len();
    if n == 0
        || daily.weathercode.len() != n
        || daily.temperature_2m_max.len() != n
        || daily.temperature_2m_min.len() != n
    {
        return Err(tr(cat, "weather.err.forecast_incomplete"));
    }

    let mut parts = Vec::with_capacity(n + 1);
    parts.push(tr_with(
        cat,
        "weather.msg.forecast_intro",
        &[("place", place_name), ("days", &format!("{days}"))],
    ));
    for i in 0..n {
        let date = &daily.time[i];
        let desc = wmo_weather_desc(cat, daily.weathercode[i]);
        let tmax = daily.temperature_2m_max[i];
        let tmin = daily.temperature_2m_min[i];
        parts.push(tr_with(
            cat,
            "weather.msg.forecast_day",
            &[
                ("date", date),
                ("desc", &desc),
                ("tmin", &round1(tmin)),
                ("tmax", &round1(tmax)),
            ],
        ));
    }
    Ok(parts.join(" "))
}

fn round1(x: f64) -> String {
    format!("{:.1}", (x * 10.0).round() / 10.0)
}

fn geocode(query: &str, cat: &TextCatalog) -> Result<(f64, f64, String), String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!(
        "{}?name={}&count=1",
        GEOCODE_URL,
        urlencoding::encode(query)
    );
    let res = send_get_with_retry(&client, &url).map_err(|e| {
        tr_with(
            cat,
            "weather.err.request_failed",
            &[("error", &e.to_string())],
        )
    })?;
    if !res.status().is_success() {
        return Err(tr_with(
            cat,
            "weather.err.geocode_http",
            &[("status", &res.status().to_string())],
        ));
    }
    let body: GeocodeResults = res.json().map_err(|e| {
        tr_with(
            cat,
            "weather.err.geocode_parse",
            &[("error", &e.to_string())],
        )
    })?;
    let results = body
        .results
        .ok_or_else(|| tr(cat, "weather.err.geocode_not_found"))?;
    let first = results
        .into_iter()
        .next()
        .ok_or_else(|| tr(cat, "weather.err.geocode_empty"))?;
    let name = if first.admin1.is_empty() {
        format!("{}, {}", first.name, first.country)
    } else {
        format!("{}, {}, {}", first.name, first.admin1, first.country)
    };
    Ok((first.latitude, first.longitude, name))
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
