//! 天气技能：单行 JSON stdin -> 单行 JSON stdout，调用 Open-Meteo 查询当前天气。

use std::io::{self, BufRead, Write};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
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

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
                Ok(text) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn execute(args: Value) -> Result<String, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;

    let city = obj
        .get("city")
        .or_else(|| obj.get("location"))
        .or_else(|| obj.get("place"))
        .or_else(|| obj.get("q"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let lat = obj.get("latitude").and_then(|v| v.as_f64());
    let lon = obj.get("longitude").and_then(|v| v.as_f64());

    let (lat, lon, place_name) = match (city, lat, lon) {
        (Some(c), _, _) => {
            let (lat, lon, name) = geocode(c)?;
            (lat, lon, name)
        }
        (None, Some(lat), Some(lon)) => (lat, lon, format!("{lat:.2}°N, {lon:.2}°E")),
        _ => return Err("请提供城市名（city/location/place/q）或经纬度（latitude + longitude）".to_string()),
    };

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!(
        "{}?latitude={}&longitude={}&current_weather=true",
        FORECAST_URL, lat, lon
    );
    let res = client
        .get(&url)
        .send()
        .map_err(|e| format!("请求天气接口失败: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("天气接口返回错误: {}", res.status()));
    }
    let body: ForecastResponse = res.json().map_err(|e| format!("解析天气数据失败: {}", e))?;

    let cur = body
        .current_weather
        .ok_or_else(|| "无当前天气数据".to_string())?;

    let desc = wmo_weather_desc(cur.weathercode);
    let day_night = if cur.is_day == 1 { "白天" } else { "夜间" };
    let text = format!(
        "{} {}：{}，气温 {}°C，风速 {} km/h，风向 {}°。",
        place_name,
        day_night,
        desc,
        cur.temperature,
        cur.windspeed,
        cur.winddirection as u32
    );
    Ok(text)
}

fn geocode(query: &str) -> Result<(f64, f64, String), String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}?name={}&count=1", GEOCODE_URL, urlencoding::encode(query));
    let res = client
        .get(&url)
        .send()
        .map_err(|e| format!("地理编码请求失败: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("地理编码接口返回: {}", res.status()));
    }
    let body: GeocodeResults = res.json().map_err(|e| format!("解析地理编码失败: {}", e))?;
    let results = body
        .results
        .ok_or_else(|| "未找到该地点，请换一个城市或地名".to_string())?;
    let first = results
        .into_iter()
        .next()
        .ok_or_else(|| "未找到该地点".to_string())?;
    let name = if first.admin1.is_empty() {
        format!("{}, {}", first.name, first.country)
    } else {
        format!("{}, {}, {}", first.name, first.admin1, first.country)
    };
    Ok((first.latitude, first.longitude, name))
}

/// WMO 天气现象代码 -> 简短描述（中文）
fn wmo_weather_desc(code: u32) -> &'static str {
    match code {
        0 => "晴",
        1 => "大部晴朗",
        2 => "局部多云",
        3 => "多云",
        45 => "雾",
        48 => "雾凇",
        51 | 53 | 55 => "毛毛雨",
        56 | 57 => "冻毛毛雨",
        61 | 63 | 65 => "雨",
        66 | 67 => "冻雨",
        71 | 73 | 75 => "雪",
        77 => "雪粒",
        80 | 81 | 82 => "阵雨",
        85 | 86 => "阵雪",
        95 => "雷暴",
        96 | 99 => "雷暴伴冰雹",
        _ => "未知",
    }
}
