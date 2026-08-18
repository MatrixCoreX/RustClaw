from decimal import Decimal, InvalidOperation


def _decimal(value):
    try:
        parsed = Decimal(str(value).strip())
    except (InvalidOperation, TypeError, ValueError):
        return None
    return parsed if parsed.is_finite() else None


def _compact_decimal(value, minimum_fraction=0):
    parsed = _decimal(value)
    if parsed is None:
        return "--"
    text = format(parsed, "f")
    if "." in text:
        integer, fraction = text.split(".", 1)
        fraction = fraction.rstrip("0")
        if len(fraction) < minimum_fraction:
            fraction += "0" * (minimum_fraction - len(fraction))
        text = integer + ("." + fraction if fraction else "")
    elif minimum_fraction:
        text += "." + "0" * minimum_fraction
    return text


def _reserve_amount(value):
    parsed = _decimal(value)
    if parsed is None:
        return "--"
    return f"{parsed:,.2f}"


def _change_percent(value):
    parsed = _decimal(value)
    if parsed is None:
        return "--", "flat"
    prefix = "+" if parsed > 0 else ""
    direction = "up" if parsed > 0 else "down" if parsed < 0 else "flat"
    return f"{prefix}{_compact_decimal(parsed, 2)}%", direction


def build_bancor_market_view(market, lang="CN", error=""):
    selected_lang = "EN" if lang == "EN" else "CN"
    copy = {
        "CN": {
            "unavailable": "市场信息暂时无法读取",
            "status": "市场",
            "open": "开放",
            "paused": "暂停",
            "disabled": "未启用",
            "fee": "手续费",
            "trades": "今日成交",
            "high": "当日最高",
            "low": "当日最低",
            "change": "日涨跌",
            "reserves": "池子储备",
        },
        "EN": {
            "unavailable": "Market information is temporarily unavailable",
            "status": "Market",
            "open": "Open",
            "paused": "Paused",
            "disabled": "Disabled",
            "fee": "Fee",
            "trades": "Daily trades",
            "high": "Daily high",
            "low": "Daily low",
            "change": "Daily change",
            "reserves": "Pool reserves",
        },
    }[selected_lang]
    if not isinstance(market, dict) or not market:
        return {
            "price": "--",
            "daily": copy["unavailable"],
            "reserves": "POINT --  ·  USD --",
            "meta": str(error or copy["unavailable"]),
            "change_direction": "flat",
        }

    daily = market.get("daily_marginal_price")
    daily = daily if isinstance(daily, dict) else {}
    change, direction = _change_percent(daily.get("change_percent"))
    status = str(market.get("status") or "disabled").strip().lower()
    status_label = copy.get(status, status)
    fee = _decimal(market.get("fee_bps"))
    fee_text = f"{fee / Decimal(100):.2f}%" if fee is not None else "--"
    trade_count = daily.get("trade_count")
    if not isinstance(trade_count, int) or isinstance(trade_count, bool) or trade_count < 0:
        trade_count = "--"

    return {
        "price": f"{_compact_decimal(market.get('marginal_price_usd_per_point'))} USD",
        "daily": (
            f"{copy['high']} {_compact_decimal(daily.get('high_usd_per_point'))}  ·  "
            f"{copy['low']} {_compact_decimal(daily.get('low_usd_per_point'))}  ·  "
            f"{copy['change']} {change}"
        ),
        "reserves": (
            f"{copy['reserves']}: {_reserve_amount(market.get('point_reserve'))} POINT  ·  "
            f"{_reserve_amount(market.get('usd_reserve'))} USD"
        ),
        "meta": (
            f"{copy['status']}: {status_label}  ·  {copy['fee']}: {fee_text}  ·  "
            f"{copy['trades']}: {trade_count}"
        ),
        "change_direction": direction,
    }
