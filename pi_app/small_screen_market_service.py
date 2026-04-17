import json
import logging
import os
import re
import threading
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime
from time import perf_counter

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None

from small_screen_config import _pi_app_dir
from small_screen_formatters import _fmt_signed_pct, _strip_trailing_zeros

logger = logging.getLogger(__name__)

BINANCE_TICKER_URL = "https://api.binance.com/api/v3/ticker/price"
SINA_HQ_URL = "http://hq.sinajs.cn/list="
SINA_REFERER = "https://finance.sina.com.cn"
DEFAULT_A_SHARE_REFRESH_SEC = 15
DEFAULT_CRYPTO_REFRESH_SEC = 15
DEFAULT_US_STOCK_REFRESH_SEC = 15

DEFAULT_A_SHARE_ITEMS = [
    {"name": "中国移动", "code": "600941"},
    {"name": "贵州茅台", "code": "600519"},
    {"name": "宁德时代", "code": "300750"},
    {"name": "比亚迪", "code": "002594"},
]
DEFAULT_CRYPTO_ITEMS = [
    {"name": "BTC", "symbol": "BTCUSDT"},
    {"name": "ETH", "symbol": "ETHUSDT"},
    {"name": "BCH", "symbol": "BCHUSDT"},
    {"name": "LTC", "symbol": "LTCUSDT"},
    {"name": "SOL", "symbol": "SOLUSDT"},
    {"name": "BNB", "symbol": "BNBUSDT"},
    {"name": "XRP", "symbol": "XRPUSDT"},
    {"name": "DOGE", "symbol": "DOGEUSDT"},
    {"name": "PEPE", "symbol": "PEPEUSDT"},
    {"name": "SHIB", "symbol": "SHIBUSDT"},
]
DEFAULT_US_STOCK_ITEMS = [
    {"name": "Apple", "symbol": "AAPL"},
    {"name": "NVIDIA", "symbol": "NVDA"},
    {"name": "Microsoft", "symbol": "MSFT"},
    {"name": "Tesla", "symbol": "TSLA"},
]

_MARKET_CONFIG_CACHE_LOCK = threading.Lock()
_MARKET_CONFIG_CACHE = {"mtime": None, "data": {}}
_US_STOCK_RESULT_CACHE_LOCK = threading.Lock()
_US_STOCK_RESULT_CACHE = {"key": None, "at": 0.0, "value": None}
US_STOCK_RESULT_CACHE_TTL_SEC = 10
US_STOCK_MAX_WORKERS = 4


def _small_screen_market_config_path():
    return os.path.join(_pi_app_dir(), "small_screen_markets.toml")


def _load_small_screen_market_config():
    if tomllib is None:
        return {}
    path = _small_screen_market_config_path()
    try:
        mtime = os.path.getmtime(path)
    except OSError:
        mtime = None
    with _MARKET_CONFIG_CACHE_LOCK:
        cached_mtime = _MARKET_CONFIG_CACHE.get("mtime")
        cached_data = _MARKET_CONFIG_CACHE.get("data")
        if cached_mtime == mtime and isinstance(cached_data, dict):
            return cached_data
    try:
        with open(path, "rb") as f:
            cfg = tomllib.load(f)
        result = cfg if isinstance(cfg, dict) else {}
    except Exception:
        result = {}
    with _MARKET_CONFIG_CACHE_LOCK:
        _MARKET_CONFIG_CACHE["mtime"] = mtime
        _MARKET_CONFIG_CACHE["data"] = result
    return result


def _parse_refresh_seconds(value, default_value):
    if isinstance(value, (int, float)):
        return max(5, min(int(value), 3600))
    return default_value


def _load_small_screen_crypto_config():
    cfg = _load_small_screen_market_config()
    section = (cfg.get("crypto") or {}) if isinstance(cfg, dict) else {}
    refresh_seconds = _parse_refresh_seconds(section.get("refresh_seconds"), DEFAULT_CRYPTO_REFRESH_SEC)
    items = []
    for item in section.get("items") or []:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name") or "").strip()
        symbol = str(item.get("symbol") or "").strip().upper()
        if name and symbol:
            items.append({"name": name, "symbol": symbol})
    if not items:
        items = [dict(item) for item in DEFAULT_CRYPTO_ITEMS]
    return items, refresh_seconds


def fetch_crypto_prices(crypto_items=None):
    items = crypto_items or _load_small_screen_crypto_config()[0]
    try:
        req = urllib.request.Request(BINANCE_TICKER_URL)
        with urllib.request.urlopen(req, timeout=8) as r:
            data = json.loads(r.read().decode())
        if not isinstance(data, list):
            return None
        by_symbol = {
            item.get("symbol"): item.get("price")
            for item in data
            if isinstance(item, dict) and item.get("symbol") and item.get("price")
        }
        out = {}
        for item in items:
            name = item.get("name")
            symbol = item.get("symbol")
            if not name or not symbol:
                continue
            price = by_symbol.get(symbol)
            out[name] = _strip_trailing_zeros(price) if price is not None else "--"
        return out
    except Exception:
        return None


def _normalize_stock_code(input_text):
    s = str(input_text or "").strip().lower()
    digits = "".join(ch for ch in s if ch.isdigit())
    if s.startswith(("sh", "sz")) and len(digits) == 6:
        return s[:2] + digits
    if len(digits) == 6:
        return ("sh" if digits.startswith("6") else "sz") + digits
    return ""


def _load_small_screen_stock_config():
    cfg = _load_small_screen_market_config()
    section = (cfg.get("stocks") or {}) if isinstance(cfg, dict) else {}
    refresh_seconds = _parse_refresh_seconds(section.get("refresh_seconds"), DEFAULT_A_SHARE_REFRESH_SEC)
    items = []
    for item in section.get("items") or []:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name") or "").strip()
        code = _normalize_stock_code(item.get("code"))
        if code:
            items.append({"name": name or code.upper(), "code": code})
    if not items:
        items = [{"name": item["name"], "code": _normalize_stock_code(item["code"])} for item in DEFAULT_A_SHARE_ITEMS]
    return items, refresh_seconds


def _normalize_us_stock_symbol(input_text):
    s = str(input_text or "").strip().upper()
    return re.sub(r"[^A-Z0-9\.\-]", "", s)


def _load_small_screen_us_stock_config():
    cfg = _load_small_screen_market_config()
    section = (cfg.get("us_stocks") or {}) if isinstance(cfg, dict) else {}
    refresh_seconds = _parse_refresh_seconds(section.get("refresh_seconds"), DEFAULT_US_STOCK_REFRESH_SEC)
    items = []
    for item in section.get("items") or []:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name") or "").strip()
        symbol = _normalize_us_stock_symbol(item.get("symbol"))
        if symbol:
            items.append({"name": name or symbol, "symbol": symbol})
    if not items:
        items = [dict(item) for item in DEFAULT_US_STOCK_ITEMS]
    return items, refresh_seconds


def _us_stock_cache_key(items):
    normalized = []
    for item in items or []:
        if not isinstance(item, dict):
            continue
        normalized.append(
            (
                str(item.get("name") or "").strip(),
                str(item.get("symbol") or "").strip().upper(),
            )
        )
    return tuple(normalized)


def _fetch_us_stock_quote_meta(symbol):
    url = "https://query1.finance.yahoo.com/v8/finance/chart/" + urllib.parse.quote(symbol) + "?interval=1d&range=5d"
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(req, timeout=8) as r:
        data = json.loads(r.read().decode("utf-8", "replace"))
    result = (((data or {}).get("chart") or {}).get("result") or [None])[0] or {}
    meta = (result.get("meta") or {}) if isinstance(result, dict) else {}
    return meta if meta else None


def _decode_sina_body(raw):
    try:
        text = raw.decode("utf-8")
        if "var hq_str_" in text:
            return text
    except UnicodeDecodeError:
        pass
    return raw.decode("gbk", errors="ignore")


def _parse_sina_quotes(body):
    out = {}
    for code, payload in re.findall(r'var hq_str_([a-z]{2}\d{6})="([^"]*)";', body, flags=re.I):
        parts = [part.strip() for part in payload.split(",")]
        if len(parts) < 32:
            continue
        name = parts[0]
        if not name:
            continue
        norm_code = code.lower()
        out[norm_code] = {
            "name": name,
            "code": norm_code[2:],
            "open": parts[1] or "--",
            "prev_close": parts[2] or "--",
            "current": parts[3] or "--",
            "high": parts[4] or "--",
            "low": parts[5] or "--",
            "time": parts[31] or "--",
        }
        out[norm_code]["pct"] = _fmt_signed_pct(out[norm_code]["current"], out[norm_code]["prev_close"])
    return out


def fetch_a_share_quotes(stock_items=None):
    items = stock_items or _load_small_screen_stock_config()[0]
    stock_codes = [item["code"] for item in items if item.get("code")]
    quotes = {}
    error = None
    if stock_codes:
        try:
            req = urllib.request.Request(SINA_HQ_URL + ",".join(stock_codes))
            req.add_header("Referer", SINA_REFERER)
            req.add_header("User-Agent", "RustClaw-Small-Screen/1.0")
            with urllib.request.urlopen(req, timeout=8) as r:
                quotes = _parse_sina_quotes(_decode_sina_body(r.read()))
        except Exception as exc:
            error = str(exc)

    out = []
    for item in items:
        code = item.get("code") or ""
        quote = quotes.get(code.lower()) if code else None
        if quote:
            display_name = item.get("name") or quote.get("name") or code.upper()
            out.append({
                "title": f"{display_name} · {quote.get('code') or '--'}",
                "price": quote.get("current") or "--",
                "pct": quote.get("pct") or "--",
                "meta1": f"今开 {quote.get('open') or '--'}  昨收 {quote.get('prev_close') or '--'}",
                "meta2": f"高/低 {quote.get('high') or '--'}/{quote.get('low') or '--'}  {quote.get('time') or '--'}",
            })
            continue
        reason = "行情获取失败" if error else "暂无今日行情"
        out.append({
            "title": item.get("name") or code.upper() or "--",
            "price": "--",
            "pct": "--",
            "meta1": reason[:28],
            "meta2": code.upper()[:28],
        })
    return {"items": out, "error": error}


def fetch_us_stock_quotes(stock_items=None):
    items = stock_items or _load_small_screen_us_stock_config()[0]
    started_at = perf_counter()
    cache_key = _us_stock_cache_key(items)
    now_ts = datetime.now().timestamp()
    with _US_STOCK_RESULT_CACHE_LOCK:
        if (
            _US_STOCK_RESULT_CACHE.get("key") == cache_key
            and _US_STOCK_RESULT_CACHE.get("value") is not None
            and (now_ts - float(_US_STOCK_RESULT_CACHE.get("at") or 0.0)) < US_STOCK_RESULT_CACHE_TTL_SEC
        ):
            logger.debug("US stock quotes served from cache (%s items)", len(items))
            return _US_STOCK_RESULT_CACHE["value"]
    quotes = {}
    errors = []
    symbols = [str(item.get("symbol") or "").strip().upper() for item in items if str(item.get("symbol") or "").strip()]
    max_workers = max(1, min(US_STOCK_MAX_WORKERS, len(symbols)))
    if symbols:
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            future_map = {executor.submit(_fetch_us_stock_quote_meta, symbol): symbol for symbol in symbols}
            for future in as_completed(future_map):
                symbol = future_map[future]
                try:
                    meta = future.result()
                    if meta:
                        quotes[symbol] = meta
                except Exception as exc:
                    errors.append(f"{symbol}: {exc}")

    out = []
    for item in items:
        symbol = item.get("symbol") or ""
        quote = quotes.get(symbol)
        if quote:
            display_name = item.get("name") or quote.get("shortName") or quote.get("longName") or symbol
            price = quote.get("regularMarketPrice")
            price_text = _strip_trailing_zeros(str(price)) if price is not None else "--"
            prev_close = quote.get("previousClose")
            if prev_close is None:
                prev_close = quote.get("chartPreviousClose")
            pct_text = _fmt_signed_pct(price, prev_close)
            exchange = str(quote.get("fullExchangeName") or quote.get("exchangeName") or "").strip()
            open_price = quote.get("regularMarketOpen")
            if open_price is None:
                open_price = quote.get("chartPreviousClose")
            high = quote.get("regularMarketDayHigh")
            low = quote.get("regularMarketDayLow")
            market_ts = quote.get("regularMarketTime")
            meta1 = "Open {open}  Prev {prev}".format(
                open=_strip_trailing_zeros(str(open_price)) if open_price is not None else "--",
                prev=_strip_trailing_zeros(str(prev_close)) if prev_close is not None else "--",
            )
            meta2_parts = [
                "H/L {high}/{low}".format(
                    high=_strip_trailing_zeros(str(high)) if high is not None else "--",
                    low=_strip_trailing_zeros(str(low)) if low is not None else "--",
                )
            ]
            if exchange:
                meta2_parts.append(exchange[:18])
            if isinstance(market_ts, (int, float)) and market_ts > 0:
                try:
                    meta2_parts.append(datetime.fromtimestamp(market_ts).strftime("%H:%M"))
                except Exception:
                    pass
            out.append({
                "title": f"{display_name} · {symbol}",
                "price": price_text,
                "pct": pct_text,
                "meta1": meta1,
                "meta2": "  ".join(meta2_parts),
            })
            continue
        reason = "行情获取失败" if errors else "暂无行情"
        out.append({
            "title": item.get("name") or symbol or "--",
            "price": "--",
            "pct": "--",
            "meta1": reason[:28],
            "meta2": symbol[:28],
        })
    result = {"items": out, "error": " | ".join(errors[:3]) if errors else None}
    with _US_STOCK_RESULT_CACHE_LOCK:
        _US_STOCK_RESULT_CACHE["key"] = cache_key
        _US_STOCK_RESULT_CACHE["at"] = now_ts
        _US_STOCK_RESULT_CACHE["value"] = result
    logger.debug(
        "US stock quotes fetched in %sms (%s symbols, %s errors)",
        int((perf_counter() - started_at) * 1000),
        len(symbols),
        len(errors),
    )
    return result
