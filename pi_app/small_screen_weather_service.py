import json
import urllib.parse
import urllib.request
from datetime import datetime

from small_screen_market_service import _load_small_screen_market_config

_LAST_SUCCESSFUL_WEATHER = None


def _pick_weather_text(values):
    if not isinstance(values, list):
        return ""
    for item in values:
        if not isinstance(item, dict):
            continue
        text = str(item.get("value") or "").strip()
        if text:
            return text
    return ""


def _weather_icon_for_code(code):
    try:
        value = int(str(code or "").strip() or "-1")
    except Exception:
        value = -1
    if value == 113:
        return "☀"
    if value == 116:
        return "☁"
    if value in (119, 122):
        return "☁"
    if value in (143, 248, 260):
        return "≋"
    if value in (176, 263, 266, 293, 296, 299, 353, 356):
        return "☂"
    if value in (182, 185, 281, 284, 302, 305, 308, 311, 314, 317, 320, 359, 362, 365):
        return "☂"
    if value in (179, 227, 230, 323, 326, 329, 332, 335, 338, 368, 371):
        return "❄"
    if value in (200, 386, 389, 392, 395):
        return "⚡"
    return "◌"


def _weather_desc_for_code(code, lang="CN", fallback=""):
    try:
        value = int(str(code or "").strip() or "-1")
    except Exception:
        value = -1
    lang = "EN" if str(lang).upper() == "EN" else "CN"
    mapping = {
        113: {"CN": "晴", "EN": "Clear"},
        116: {"CN": "局部多云", "EN": "Partly cloudy"},
        119: {"CN": "多云", "EN": "Cloudy"},
        122: {"CN": "阴", "EN": "Overcast"},
        143: {"CN": "薄雾", "EN": "Mist"},
        176: {"CN": "局地阵雨", "EN": "Patchy rain"},
        179: {"CN": "局地小雪", "EN": "Patchy snow"},
        182: {"CN": "局地雨夹雪", "EN": "Patchy sleet"},
        185: {"CN": "局地冻毛雨", "EN": "Patchy freezing drizzle"},
        200: {"CN": "局地雷暴", "EN": "Thundery nearby"},
        227: {"CN": "吹雪", "EN": "Blowing snow"},
        230: {"CN": "暴风雪", "EN": "Blizzard"},
        248: {"CN": "雾", "EN": "Fog"},
        260: {"CN": "冻雾", "EN": "Freezing fog"},
        263: {"CN": "零星毛毛雨", "EN": "Patchy drizzle"},
        266: {"CN": "毛毛雨", "EN": "Drizzle"},
        281: {"CN": "冻毛雨", "EN": "Freezing drizzle"},
        284: {"CN": "强冻毛雨", "EN": "Heavy freezing drizzle"},
        293: {"CN": "零星小雨", "EN": "Patchy light rain"},
        296: {"CN": "小雨", "EN": "Light rain"},
        299: {"CN": "间歇中雨", "EN": "Moderate rain at times"},
        302: {"CN": "中雨", "EN": "Moderate rain"},
        305: {"CN": "间歇大雨", "EN": "Heavy rain at times"},
        308: {"CN": "大雨", "EN": "Heavy rain"},
        311: {"CN": "轻度冻雨", "EN": "Light freezing rain"},
        314: {"CN": "冻雨", "EN": "Freezing rain"},
        317: {"CN": "轻度雨夹雪", "EN": "Light sleet"},
        320: {"CN": "雨夹雪", "EN": "Sleet"},
        323: {"CN": "零星小雪", "EN": "Patchy light snow"},
        326: {"CN": "小雪", "EN": "Light snow"},
        329: {"CN": "间歇中雪", "EN": "Patchy moderate snow"},
        332: {"CN": "中雪", "EN": "Moderate snow"},
        335: {"CN": "间歇大雪", "EN": "Patchy heavy snow"},
        338: {"CN": "大雪", "EN": "Heavy snow"},
        353: {"CN": "小阵雨", "EN": "Light shower"},
        356: {"CN": "阵雨", "EN": "Rain shower"},
        359: {"CN": "暴雨", "EN": "Torrential rain"},
        362: {"CN": "轻度雨夹雪阵雨", "EN": "Light sleet shower"},
        365: {"CN": "雨夹雪阵雨", "EN": "Sleet shower"},
        368: {"CN": "小阵雪", "EN": "Light snow shower"},
        371: {"CN": "阵雪", "EN": "Snow shower"},
        386: {"CN": "局地雷阵雨", "EN": "Patchy thunder rain"},
        389: {"CN": "强雷雨", "EN": "Heavy thunder rain"},
        392: {"CN": "局地雷阵雪", "EN": "Patchy thunder snow"},
        395: {"CN": "强雷阵雪", "EN": "Heavy thunder snow"},
    }
    if value in mapping:
        return mapping[value][lang]
    fallback = str(fallback or "").strip()
    if fallback:
        return fallback
    return {"CN": "多云", "EN": "Cloudy"}[lang]


def _wind_level_from_kmh(speed_kmh):
    try:
        speed = float(str(speed_kmh or "").strip())
    except Exception:
        return None
    thresholds = [1, 5, 11, 19, 28, 38, 49, 61, 74, 88, 102, 117]
    for level, upper in enumerate(thresholds):
        if speed <= upper:
            return level
    return 12


def _format_weather_wind(speed_kmh, direction="", lang="CN"):
    speed_text = str(speed_kmh or "--").strip() or "--"
    direction_text = str(direction or "").strip()
    base = " ".join(part for part in (f"{speed_text} km/h", direction_text) if part).strip() or "--"
    level = _wind_level_from_kmh(speed_kmh)
    if level is None:
        return base
    if str(lang).upper() == "EN":
        return f"{base} (L{level})"
    return f"{base} ({level}级)"


def _load_small_screen_weather_config():
    cfg = _load_small_screen_market_config()
    section = (cfg.get("weather") or {}) if isinstance(cfg, dict) else {}
    city = str(section.get("city") or "").strip()
    return {"city": city}


def _weather_day_label(date_text, offset=0, lang="CN"):
    lang = "EN" if str(lang).upper() == "EN" else "CN"
    if offset == 0:
        return "Today" if lang == "EN" else "今天"
    if offset == 1:
        return "Tomorrow" if lang == "EN" else "明天"
    try:
        dt = datetime.strptime(str(date_text or "").strip(), "%Y-%m-%d")
        idx = dt.weekday()
    except Exception:
        return f"D+{offset}" if lang == "EN" else f"{offset}天后"
    cn_days = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
    en_days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    return (en_days if lang == "EN" else cn_days)[idx]


def _fetch_json(url, headers=None, timeout=12):
    req = urllib.request.Request(url, headers=headers or {})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = resp.read().decode("utf-8", "replace")
    return json.loads(body)


def _deg_to_compass(value):
    try:
        deg = float(str(value or "").strip())
    except Exception:
        return ""
    labels = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"]
    idx = int((deg % 360.0) / 22.5 + 0.5) % 16
    return labels[idx]


def _open_meteo_to_legacy_code(code):
    mapping = {
        0: 113,
        1: 116,
        2: 119,
        3: 122,
        45: 248,
        48: 260,
        51: 263,
        53: 266,
        55: 266,
        56: 281,
        57: 284,
        61: 296,
        63: 302,
        65: 308,
        66: 311,
        67: 314,
        71: 326,
        73: 332,
        75: 338,
        77: 323,
        80: 353,
        81: 356,
        82: 359,
        85: 368,
        86: 371,
        95: 386,
        96: 389,
        99: 389,
    }
    try:
        key = int(str(code or "").strip())
    except Exception:
        return -1
    return mapping.get(key, -1)


def _format_hhmm(value):
    text = str(value or "").strip()
    if not text:
        return "--"
    if "T" in text:
        text = text.split("T", 1)[1]
    if "+" in text:
        text = text.split("+", 1)[0]
    return text[:5] if len(text) >= 5 else text


def _format_location_text(city="", region="", country=""):
    parts = [str(part or "").strip() for part in (city, region, country)]
    parts = [part for part in parts if part]
    if not parts:
        return "--"
    if len(parts) >= 2 and parts[0] == parts[-1]:
        parts = parts[:-1]
    return ", ".join(parts[:2]) if len(parts) > 1 else parts[0]


def _best_hourly_sample(times, values_by_key, date_text):
    matches = []
    for idx, time_text in enumerate(times or []):
        if str(time_text).startswith(str(date_text or "")):
            matches.append(idx)
    if not matches:
        return None
    chosen = matches[0]
    for idx in matches:
        if "T12:" in str(times[idx]):
            chosen = idx
            break
    sample = {}
    for key, values in (values_by_key or {}).items():
        if isinstance(values, list) and chosen < len(values):
            sample[key] = values[chosen]
    sample["time"] = times[chosen]
    return sample


def _detect_location_ipwho():
    data = _fetch_json("https://ipwho.is/")
    if not isinstance(data, dict) or data.get("success") is False:
        raise ValueError("ipwho geolocation failed")
    latitude = data.get("latitude")
    longitude = data.get("longitude")
    if latitude is None or longitude is None:
        raise ValueError("ipwho missing coordinates")
    return {
        "latitude": float(latitude),
        "longitude": float(longitude),
        "city": str(data.get("city") or "").strip(),
        "region": str(data.get("region") or "").strip(),
        "country": str(data.get("country") or "").strip(),
    }


def _detect_location_ipapi():
    data = _fetch_json("https://ipapi.co/json/")
    if not isinstance(data, dict):
        raise ValueError("ipapi geolocation failed")
    latitude = data.get("latitude")
    longitude = data.get("longitude")
    if latitude is None or longitude is None:
        raise ValueError("ipapi missing coordinates")
    return {
        "latitude": float(latitude),
        "longitude": float(longitude),
        "city": str(data.get("city") or "").strip(),
        "region": str(data.get("region") or "").strip(),
        "country": str(data.get("country_name") or "").strip(),
    }


def _geocode_city_open_meteo(city):
    query = urllib.parse.urlencode({"name": city, "count": 1, "language": "en", "format": "json"})
    data = _fetch_json("https://geocoding-api.open-meteo.com/v1/search?" + query)
    results = (data or {}).get("results") or []
    first = results[0] if results else {}
    if not isinstance(first, dict) or first.get("latitude") is None or first.get("longitude") is None:
        raise ValueError("open-meteo geocoding failed")
    return {
        "latitude": float(first.get("latitude")),
        "longitude": float(first.get("longitude")),
        "city": str(first.get("name") or city).strip(),
        "region": str(first.get("admin1") or "").strip(),
        "country": str(first.get("country") or "").strip(),
    }


def _resolve_location(city=""):
    city = str(city or "").strip()
    attempts = []
    if city:
        attempts.append(("open-meteo-geocode", lambda: _geocode_city_open_meteo(city)))
    else:
        attempts.append(("ipwho", _detect_location_ipwho))
        attempts.append(("ipapi", _detect_location_ipapi))
    last_error = None
    for _name, func in attempts:
        try:
            return func()
        except Exception as exc:
            last_error = exc
    raise last_error or ValueError("location resolve failed")


def _build_weather_payload_from_parts(lang, location, current_code, current_temp, current_desc_fallback, current_feels_like, current_humidity, current_wind_kmh, current_wind_dir, today_max, today_min, today_rain, sunrise, sunset, forecast_rows, details_rows, updated_at):
    legacy_code = _open_meteo_to_legacy_code(current_code)
    return {
        "location": location,
        "code": str(legacy_code if legacy_code >= 0 else current_code).strip(),
        "icon": _weather_icon_for_code(legacy_code),
        "temperature": f"{str(current_temp or '--').strip()}°C",
        "description": _weather_desc_for_code(legacy_code, lang=lang, fallback=current_desc_fallback or "--"),
        "feels_like": f"{str(current_feels_like or '--').strip()}°C",
        "high_low": f"{str(today_max or '--').strip()}°C / {str(today_min or '--').strip()}°C",
        "humidity": f"{str(current_humidity or '--').strip()}%",
        "wind": _format_weather_wind(current_wind_kmh, current_wind_dir, lang=lang),
        "rain": today_rain,
        "sunrise": sunrise,
        "sunset": sunset,
        "details": details_rows,
        "forecast": forecast_rows,
        "updated_at": updated_at,
    }


def _weather_source_label(name, lang="CN"):
    lang = "EN" if str(lang).upper() == "EN" else "CN"
    source_name = str(name or "").strip().lower()
    if source_name.startswith("wttr"):
        return "wttr.in"
    if source_name.startswith("open-meteo"):
        return "Open-Meteo"
    return "Weather"


def _weather_source_short(name):
    source_name = str(name or "").strip().lower()
    if source_name.startswith("wttr"):
        return "WT"
    if source_name.startswith("open-meteo"):
        return "OM"
    return "--"


def _attach_weather_source(weather, name, lang="CN"):
    if not isinstance(weather, dict):
        return weather
    result = dict(weather)
    result["source"] = _weather_source_label(name, lang=lang)
    result["source_short"] = _weather_source_short(name)
    return result


def _fetch_today_weather_wttr(lang="CN", city=""):
    city = str(city or "").strip()
    query = urllib.parse.urlencode({"format": "j1", "lang": "zh" if str(lang).upper() == "CN" else "en"})
    base_url = "https://wttr.in/"
    if city:
        base_url += urllib.parse.quote(city)
    payload = _fetch_json(
        base_url + "?" + query,
        headers={"User-Agent": "RustClawSmallScreen/1.0", "Accept": "application/json"},
    )
    current = ((payload or {}).get("current_condition") or [{}])[0]
    today = ((payload or {}).get("weather") or [{}])[0]
    daily_items = (payload or {}).get("weather") or []
    nearest = ((payload or {}).get("nearest_area") or [{}])[0]
    if not isinstance(payload, dict) or not current or not today:
        raise ValueError("invalid weather payload")

    astronomy = (today.get("astronomy") or [{}])[0]
    hourly = today.get("hourly") or []
    area_name = _pick_weather_text(nearest.get("areaName"))
    region_name = _pick_weather_text(nearest.get("region"))
    country_name = _pick_weather_text(nearest.get("country"))
    location = _format_location_text(area_name, region_name, country_name)

    rain_values = []
    for item in hourly:
        try:
            rain_values.append(int(str(item.get("chanceofrain") or "0").strip() or "0"))
        except Exception:
            continue
    rain_chance = f"{max(rain_values)}%" if rain_values else "--"

    details = [{
        "day": _weather_day_label(today.get("date"), offset=0, lang=lang),
        "location": location,
        "code": str(current.get("weatherCode") or "").strip(),
        "icon": _weather_icon_for_code(current.get("weatherCode")),
        "temperature": f"{str(current.get('temp_C') or '--').strip()}°C",
        "description": _weather_desc_for_code(current.get("weatherCode"), lang=lang, fallback=_pick_weather_text(current.get("weatherDesc")) or "--"),
        "feels_like": f"{str(current.get('FeelsLikeC') or '--').strip()}°C",
        "high_low": f"{str(today.get('maxtempC') or '--').strip()}°C / {str(today.get('mintempC') or '--').strip()}°C",
        "humidity": f"{str(current.get('humidity') or '--').strip()}%",
        "wind": _format_weather_wind(current.get("windspeedKmph"), current.get("winddir16Point"), lang=lang),
        "rain": rain_chance,
        "sunrise": str(astronomy.get("sunrise") or "--").strip() or "--",
        "sunset": str(astronomy.get("sunset") or "--").strip() or "--",
        "updated_at": datetime.now().strftime("%H:%M"),
    }]
    forecast = []
    for offset, item in enumerate(daily_items[1:4], start=1):
        if not isinstance(item, dict):
            continue
        hourly_items = item.get("hourly") or []
        sample = hourly_items[min(4, len(hourly_items) - 1)] if hourly_items else {}
        code = str(sample.get("weatherCode") or "").strip()
        astronomy_item = (item.get("astronomy") or [{}])[0]
        day_rain_values = []
        for hourly_item in hourly_items:
            try:
                day_rain_values.append(int(str(hourly_item.get("chanceofrain") or "0").strip() or "0"))
            except Exception:
                continue
        day_rain = f"{max(day_rain_values)}%" if day_rain_values else "--"
        detail = {
            "day": _weather_day_label(item.get("date"), offset=offset, lang=lang),
            "location": location,
            "code": code,
            "icon": _weather_icon_for_code(code),
            "temperature": f"{str(sample.get('tempC') or item.get('avgtempC') or '--').strip()}°C",
            "description": _weather_desc_for_code(code, lang=lang, fallback=_pick_weather_text(sample.get("weatherDesc")) or "--"),
            "feels_like": f"{str(sample.get('FeelsLikeC') or sample.get('tempC') or item.get('avgtempC') or '--').strip()}°C",
            "high_low": f"{str(item.get('maxtempC') or '--').strip()}°C / {str(item.get('mintempC') or '--').strip()}°C",
            "humidity": f"{str(sample.get('humidity') or '--').strip()}%",
            "wind": _format_weather_wind(sample.get("windspeedKmph"), sample.get("winddir16Point"), lang=lang),
            "rain": day_rain,
            "sunrise": str(astronomy_item.get("sunrise") or "--").strip() or "--",
            "sunset": str(astronomy_item.get("sunset") or "--").strip() or "--",
            "updated_at": datetime.now().strftime("%H:%M"),
        }
        forecast.append({
            "offset": offset,
            "day": detail["day"],
            "icon": detail["icon"],
            "description": detail["description"],
            "high_low": f"{str(item.get('maxtempC') or '--').strip()}° / {str(item.get('mintempC') or '--').strip()}°",
        })
        details.append(detail)

    return {
        "location": location,
        "code": str(current.get("weatherCode") or "").strip(),
        "icon": _weather_icon_for_code(current.get("weatherCode")),
        "temperature": f"{str(current.get('temp_C') or '--').strip()}°C",
        "description": _weather_desc_for_code(current.get("weatherCode"), lang=lang, fallback=_pick_weather_text(current.get("weatherDesc")) or "--"),
        "feels_like": f"{str(current.get('FeelsLikeC') or '--').strip()}°C",
        "high_low": f"{str(today.get('maxtempC') or '--').strip()}°C / {str(today.get('mintempC') or '--').strip()}°C",
        "humidity": f"{str(current.get('humidity') or '--').strip()}%",
        "wind": _format_weather_wind(current.get("windspeedKmph"), current.get("winddir16Point"), lang=lang),
        "rain": rain_chance,
        "sunrise": str(astronomy.get("sunrise") or "--").strip() or "--",
        "sunset": str(astronomy.get("sunset") or "--").strip() or "--",
        "details": details,
        "forecast": forecast,
        "updated_at": datetime.now().strftime("%H:%M"),
    }


def _fetch_today_weather_open_meteo(lang="CN", city=""):
    loc = _resolve_location(city=city)
    params = urllib.parse.urlencode({
        "latitude": loc["latitude"],
        "longitude": loc["longitude"],
        "current": "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,wind_direction_10m",
        "hourly": "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,precipitation_probability,wind_speed_10m,wind_direction_10m",
        "daily": "weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset,precipitation_probability_max",
        "forecast_days": 4,
        "timezone": "auto",
    })
    payload = _fetch_json("https://api.open-meteo.com/v1/forecast?" + params)
    current = (payload or {}).get("current") or {}
    daily = (payload or {}).get("daily") or {}
    hourly = (payload or {}).get("hourly") or {}
    daily_times = daily.get("time") or []
    if not current or not daily_times:
        raise ValueError("invalid open-meteo payload")

    location = _format_location_text(loc.get("city"), loc.get("region"), loc.get("country"))
    details = []
    forecast = []
    hourly_times = hourly.get("time") or []

    today_detail = {
        "day": _weather_day_label(daily_times[0], offset=0, lang=lang),
        "location": location,
        "code": str(_open_meteo_to_legacy_code(current.get("weather_code"))),
        "icon": _weather_icon_for_code(_open_meteo_to_legacy_code(current.get("weather_code"))),
        "temperature": f"{str(current.get('temperature_2m') if current.get('temperature_2m') is not None else '--').strip()}°C",
        "description": _weather_desc_for_code(_open_meteo_to_legacy_code(current.get("weather_code")), lang=lang),
        "feels_like": f"{str(current.get('apparent_temperature') if current.get('apparent_temperature') is not None else '--').strip()}°C",
        "high_low": f"{str((daily.get('temperature_2m_max') or ['--'])[0]).strip()}°C / {str((daily.get('temperature_2m_min') or ['--'])[0]).strip()}°C",
        "humidity": f"{str(current.get('relative_humidity_2m') if current.get('relative_humidity_2m') is not None else '--').strip()}%",
        "wind": _format_weather_wind(current.get("wind_speed_10m"), _deg_to_compass(current.get("wind_direction_10m")), lang=lang),
        "rain": f"{str((daily.get('precipitation_probability_max') or ['--'])[0]).strip()}%",
        "sunrise": _format_hhmm((daily.get("sunrise") or ["--"])[0]),
        "sunset": _format_hhmm((daily.get("sunset") or ["--"])[0]),
        "updated_at": _format_hhmm(current.get("time")) if current.get("time") else datetime.now().strftime("%H:%M"),
    }
    details.append(today_detail)

    value_keys = {
        "temperature_2m": hourly.get("temperature_2m") or [],
        "apparent_temperature": hourly.get("apparent_temperature") or [],
        "relative_humidity_2m": hourly.get("relative_humidity_2m") or [],
        "weather_code": hourly.get("weather_code") or [],
        "precipitation_probability": hourly.get("precipitation_probability") or [],
        "wind_speed_10m": hourly.get("wind_speed_10m") or [],
        "wind_direction_10m": hourly.get("wind_direction_10m") or [],
    }
    for offset, date_text in enumerate(daily_times[1:4], start=1):
        sample = _best_hourly_sample(hourly_times, value_keys, date_text) or {}
        code = _open_meteo_to_legacy_code(sample.get("weather_code"))
        detail = {
            "day": _weather_day_label(date_text, offset=offset, lang=lang),
            "location": location,
            "code": str(code),
            "icon": _weather_icon_for_code(code),
            "temperature": f"{str(sample.get('temperature_2m') if sample.get('temperature_2m') is not None else (daily.get('temperature_2m_max') or ['--'])[offset]).strip()}°C",
            "description": _weather_desc_for_code(code, lang=lang),
            "feels_like": f"{str(sample.get('apparent_temperature') if sample.get('apparent_temperature') is not None else sample.get('temperature_2m') if sample.get('temperature_2m') is not None else '--').strip()}°C",
            "high_low": f"{str((daily.get('temperature_2m_max') or ['--'])[offset]).strip()}°C / {str((daily.get('temperature_2m_min') or ['--'])[offset]).strip()}°C",
            "humidity": f"{str(sample.get('relative_humidity_2m') if sample.get('relative_humidity_2m') is not None else '--').strip()}%",
            "wind": _format_weather_wind(sample.get("wind_speed_10m"), _deg_to_compass(sample.get("wind_direction_10m")), lang=lang),
            "rain": f"{str((daily.get('precipitation_probability_max') or ['--'])[offset]).strip()}%",
            "sunrise": _format_hhmm((daily.get("sunrise") or ["--"])[offset]),
            "sunset": _format_hhmm((daily.get("sunset") or ["--"])[offset]),
            "updated_at": _format_hhmm(sample.get("time")) if sample.get("time") else datetime.now().strftime("%H:%M"),
        }
        forecast.append({
            "offset": offset,
            "day": detail["day"],
            "icon": detail["icon"],
            "description": detail["description"],
            "high_low": f"{str((daily.get('temperature_2m_max') or ['--'])[offset]).strip()}° / {str((daily.get('temperature_2m_min') or ['--'])[offset]).strip()}°",
        })
        details.append(detail)

    return _build_weather_payload_from_parts(
        lang=lang,
        location=location,
        current_code=current.get("weather_code"),
        current_temp=current.get("temperature_2m"),
        current_desc_fallback="",
        current_feels_like=current.get("apparent_temperature"),
        current_humidity=current.get("relative_humidity_2m"),
        current_wind_kmh=current.get("wind_speed_10m"),
        current_wind_dir=_deg_to_compass(current.get("wind_direction_10m")),
        today_max=(daily.get("temperature_2m_max") or ["--"])[0],
        today_min=(daily.get("temperature_2m_min") or ["--"])[0],
        today_rain=f"{str((daily.get('precipitation_probability_max') or ['--'])[0]).strip()}%",
        sunrise=_format_hhmm((daily.get("sunrise") or ["--"])[0]),
        sunset=_format_hhmm((daily.get("sunset") or ["--"])[0]),
        forecast_rows=forecast,
        details_rows=details,
        updated_at=_format_hhmm(current.get("time")) if current.get("time") else datetime.now().strftime("%H:%M"),
    )


def _fetch_today_weather_once(lang="CN", city=""):
    city = str(city or "").strip()
    attempts = [lambda: _fetch_today_weather_wttr(lang=lang, city=city)]
    if city:
        attempts.append(lambda: _fetch_today_weather_open_meteo(lang=lang, city=city))
    else:
        attempts.append(lambda: _fetch_today_weather_open_meteo(lang=lang, city=""))
    last_error = None
    for func in attempts:
        try:
            weather = func()
            if isinstance(weather, dict) and weather.get("temperature"):
                return weather
        except Exception as exc:
            last_error = exc
    raise last_error or ValueError("weather fetch failed")


def fetch_today_weather(lang="CN"):
    global _LAST_SUCCESSFUL_WEATHER
    weather_cfg = _load_small_screen_weather_config()
    city = str(weather_cfg.get("city") or "").strip()
    attempts = []
    if city:
        attempts.append(("wttr-city", lambda: _fetch_today_weather_wttr(lang=lang, city=city)))
        attempts.append(("open-meteo-city", lambda: _fetch_today_weather_open_meteo(lang=lang, city=city)))
    attempts.append(("wttr-auto", lambda: _fetch_today_weather_wttr(lang=lang, city="")))
    attempts.append(("open-meteo-auto", lambda: _fetch_today_weather_open_meteo(lang=lang, city="")))

    errors = []
    for name, func in attempts:
        try:
            weather = func()
            if isinstance(weather, dict) and weather.get("temperature"):
                weather = _attach_weather_source(weather, name, lang=lang)
                _LAST_SUCCESSFUL_WEATHER = weather
                return weather, None
        except Exception as exc:
            errors.append(f"{name}: {exc}")

    if isinstance(_LAST_SUCCESSFUL_WEATHER, dict):
        return _LAST_SUCCESSFUL_WEATHER, None
    return None, " | ".join(errors) if errors else "weather fetch failed"
