#!/usr/bin/env python3
import argparse
import hashlib
import hmac
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import tomllib


def mask_secret(raw: str) -> str:
    s = (raw or "").strip()
    if not s:
        return "<empty>"
    if len(s) <= 8:
        return "***"
    return f"{s[:4]}***{s[-4:]}"


def is_placeholder(raw: str) -> bool:
    s = (raw or "").strip()
    return (not s) or s.startswith("REPLACE_ME_") or s == "__REDACTED__"


def load_binance_config(config_path: Path) -> dict:
    cfg = tomllib.loads(config_path.read_text(encoding="utf-8"))
    binance = (cfg.get("binance") or {}) if isinstance(cfg, dict) else {}
    return {
        "enabled": bool(binance.get("enabled", False)),
        "api_key": str(binance.get("api_key", "")).strip(),
        "api_secret": str(binance.get("api_secret", "")).strip(),
        "base_url": str(binance.get("base_url", "https://api.binance.com")).strip().rstrip("/"),
        "recv_window": int(binance.get("recv_window", 5000)),
    }


def signed_query(api_secret: str, params: dict) -> str:
    qs = urllib.parse.urlencode(params, doseq=True)
    sign = hmac.new(api_secret.encode("utf-8"), qs.encode("utf-8"), hashlib.sha256).hexdigest()
    return f"{qs}&signature={sign}"


def call_signed(
    *,
    base_url: str,
    api_key: str,
    api_secret: str,
    method: str,
    path: str,
    params: dict,
    timeout: float,
) -> dict:
    qs_signed = signed_query(api_secret, params)
    url = f"{base_url}{path}?{qs_signed}"
    req = urllib.request.Request(url=url, method=method.upper())
    req.add_header("X-MBX-APIKEY", api_key)
    req.add_header("Content-Type", "application/x-www-form-urlencoded")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read().decode("utf-8", errors="replace")
            data = json.loads(body) if body else {}
            return {"ok": True, "status": resp.status, "data": data}
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        try:
            data = json.loads(body) if body else {}
        except json.JSONDecodeError:
            data = {"raw": body}
        return {"ok": False, "status": e.code, "data": data}
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "status": -1, "data": {"error": str(e)}}


def now_ms() -> int:
    return int(time.time() * 1000)


def check_account(cfg: dict, timeout: float) -> dict:
    params = {
        "timestamp": now_ms(),
        "recvWindow": max(1, int(cfg["recv_window"])),
    }
    return call_signed(
        base_url=cfg["base_url"],
        api_key=cfg["api_key"],
        api_secret=cfg["api_secret"],
        method="GET",
        path="/api/v3/account",
        params=params,
        timeout=timeout,
    )


def place_market_buy_once(cfg: dict, symbol: str, quote_usdt: float, timeout: float) -> dict:
    params = {
        "symbol": symbol.upper(),
        "side": "BUY",
        "type": "MARKET",
        "quoteOrderQty": f"{quote_usdt:.8f}".rstrip("0").rstrip("."),
        "newOrderRespType": "RESULT",
        "timestamp": now_ms(),
        "recvWindow": max(1, int(cfg["recv_window"])),
    }
    return call_signed(
        base_url=cfg["base_url"],
        api_key=cfg["api_key"],
        api_secret=cfg["api_secret"],
        method="POST",
        path="/api/v3/order",
        params=params,
        timeout=timeout,
    )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Binance key test: auth check + optional one market buy order."
    )
    parser.add_argument(
        "--config",
        default="configs/crypto.toml",
        help="Path to crypto.toml (default: configs/crypto.toml)",
    )
    parser.add_argument("--symbol", default="ETHUSDT", help="Trading symbol (default: ETHUSDT)")
    parser.add_argument(
        "--quote-usdt",
        type=float,
        default=1.0,
        help="Market buy quote amount in USDT (default: 1.0)",
    )
    parser.add_argument(
        "--buy-once",
        action="store_true",
        help="Actually place one MARKET BUY order after auth check",
    )
    parser.add_argument("--timeout", type=float, default=12.0, help="HTTP timeout seconds")
    args = parser.parse_args()

    config_path = Path(args.config)
    if not config_path.exists():
        print(f"[ERR] config not found: {config_path}")
        return 2

    cfg = load_binance_config(config_path)
    print(f"[INFO] config={config_path}")
    print(f"[INFO] base_url={cfg['base_url']}")
    print(f"[INFO] enabled={cfg['enabled']}")
    print(f"[INFO] api_key={mask_secret(cfg['api_key'])}")

    if not cfg["enabled"]:
        print("[ERR] [binance].enabled=false in config")
        return 2
    if is_placeholder(cfg["api_key"]) or is_placeholder(cfg["api_secret"]):
        print("[ERR] api_key/api_secret is empty or placeholder")
        return 2
    if args.quote_usdt <= 0:
        print("[ERR] --quote-usdt must be > 0")
        return 2

    print("[STEP] 1/2 checking signed account endpoint ...")
    auth = check_account(cfg, timeout=args.timeout)
    if not auth["ok"]:
        print(f"[FAIL] auth check failed status={auth['status']} body={json.dumps(auth['data'], ensure_ascii=False)}")
        return 1
    print("[PASS] auth check succeeded")

    if not args.buy_once:
        print("[DONE] dry-run mode: auth is valid. Add --buy-once to place one order.")
        return 0

    print(f"[STEP] 2/2 placing market BUY once: symbol={args.symbol.upper()} quoteOrderQty={args.quote_usdt}")
    order = place_market_buy_once(cfg, args.symbol, args.quote_usdt, timeout=args.timeout)
    if not order["ok"]:
        print(f"[FAIL] order failed status={order['status']} body={json.dumps(order['data'], ensure_ascii=False)}")
        return 1

    data = order["data"]
    order_id = data.get("orderId", "unknown")
    status = data.get("status", "unknown")
    executed_qty = data.get("executedQty", "0")
    cumm_quote_qty = data.get("cummulativeQuoteQty", "0")
    print(
        "[PASS] order submitted "
        f"orderId={order_id} status={status} executedQty={executed_qty} cummulativeQuoteQty={cumm_quote_qty}"
    )
    print(f"[RAW] {json.dumps(data, ensure_ascii=False)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

