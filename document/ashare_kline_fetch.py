#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""抓取A股K线（OHLCV）脚本

数据源：Eastmoney（东方财富）公开接口（无需Token）

功能：
- 支持股票/指数代码（如 600519 / 000001 / 399001 等）
- 自动识别交易所（SH/SZ/BJ）
- 支持K线周期：1/5/15/30/60 分钟，日/周/月
- 输出为 CSV（默认写到当前目录）

示例：
  python ashare_kline_fetch.py --symbol 600519 --period daily --adjust qfq --start 2023-01-01 --out 600519_daily.csv

注意：
- 该接口可能存在访问频率限制；建议自行做重试/限速。
- 仅供学习研究使用。
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import sys
import time
from dataclasses import dataclass
from typing import Dict, Iterable, List, Optional, Tuple

import requests


@dataclass
class KlineBar:
    date: str
    open: float
    close: float
    high: float
    low: float
    volume: float
    amount: float
    amplitude: float
    pct_chg: float
    chg: float
    turnover: float


UA = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/537.36 (KHTML, like Gecko) "
    "Chrome/122.0.0.0 Safari/537.36"
)


def detect_market(symbol: str) -> int:
    """东方财富 market 参数：1=SH, 0=SZ, 116=BJ（常见）"""
    s = symbol.strip().upper()
    # 允许用户直接传 600519.SH / 000001.SZ
    if s.endswith(".SH"):
        return 1
    if s.endswith(".SZ"):
        return 0
    if s.endswith(".BJ"):
        return 116

    # 纯数字代码
    s = "".join(ch for ch in s if ch.isdigit())
    if len(s) < 6:
        raise ValueError("symbol 需要为6位股票代码，或带 .SH/.SZ/.BJ 后缀")

    # 上海：60/68/90；深圳：00/30；北交：83/87/88（常见）
    if s.startswith(("60", "68", "90")):
        return 1
    if s.startswith(("00", "30")):
        return 0
    if s.startswith(("83", "87", "88")):
        return 116

    # 兜底：默认深圳
    return 0


def normalize_symbol_digits(symbol: str) -> str:
    s = symbol.strip().upper()
    if "." in s:
        s = s.split(".")[0]
    s = "".join(ch for ch in s if ch.isdigit())
    if len(s) != 6:
        raise ValueError("symbol 必须为6位代码（例如 600519）")
    return s


def period_to_klt(period: str) -> int:
    """klt: 1/5/15/30/60 分钟, 101 日, 102 周, 103 月"""
    p = period.strip().lower()
    mapping = {
        "1m": 1,
        "5m": 5,
        "15m": 15,
        "30m": 30,
        "60m": 60,
        "daily": 101,
        "day": 101,
        "d": 101,
        "weekly": 102,
        "week": 102,
        "w": 102,
        "monthly": 103,
        "month": 103,
        "m": 103,
    }
    if p not in mapping:
        raise ValueError("period 不支持：请选择 1m/5m/15m/30m/60m/daily/weekly/monthly")
    return mapping[p]


def adjust_to_fqt(adjust: str) -> int:
    """fqt: 0 不复权, 1 前复权, 2 后复权"""
    a = adjust.strip().lower()
    mapping = {
        "none": 0,
        "nfq": 0,
        "": 0,
        "qfq": 1,
        "front": 1,
        "hfq": 2,
        "back": 2,
    }
    if a not in mapping:
        raise ValueError("adjust 不支持：请选择 none/qfq/hfq")
    return mapping[a]


def fetch_kline_eastmoney(
    symbol: str,
    period: str = "daily",
    adjust: str = "qfq",
    start: Optional[str] = None,
    end: Optional[str] = None,
    limit: int = 10000,
    timeout: int = 10,
    max_retries: int = 3,
    sleep_sec: float = 0.5,
) -> List[KlineBar]:
    market = detect_market(symbol)
    code = normalize_symbol_digits(symbol)
    klt = period_to_klt(period)
    fqt = adjust_to_fqt(adjust)

    # 日期格式：YYYYMMDD；默认取很早到很晚
    def to_ymd(s: Optional[str], default: str) -> str:
        if not s:
            return default
        s = s.strip()
        if "-" in s:
            return s.replace("-", "")
        return s

    beg = to_ymd(start, "19900101")
    endd = to_ymd(end, "20991231")

    url = "https://push2his.eastmoney.com/api/qt/stock/kline/get"
    params = {
        "secid": f"{market}.{code}",
        "fields1": "f1,f2,f3,f4,f5,f6",
        "fields2": "f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61",
        "klt": str(klt),
        "fqt": str(fqt),
        "beg": beg,
        "end": endd,
        "lmt": str(limit),
        "rtntype": "6",
        "ut": "fa5fd1943c7b386f172d6893dbfba10b",
        "_": str(int(time.time() * 1000)),
    }

    headers = {"User-Agent": UA, "Referer": "https://quote.eastmoney.com/"}

    last_err: Optional[Exception] = None
    for i in range(max_retries):
        try:
            r = requests.get(url, params=params, headers=headers, timeout=timeout)
            r.raise_for_status()
            data = r.json()
            if not isinstance(data, dict) or data.get("data") is None:
                raise RuntimeError(f"unexpected response: {data}")

            klines = data["data"].get("klines") or []
            out: List[KlineBar] = []
            for line in klines:
                # line 示例：
                # '2024-01-02,167.50,168.20,170.00,166.80,1234567,123456789.00,1.92,0.42,0.70,1.23'
                parts = line.split(",")
                if len(parts) < 11:
                    continue
                out.append(
                    KlineBar(
                        date=parts[0],
                        open=float(parts[1]),
                        close=float(parts[2]),
                        high=float(parts[3]),
                        low=float(parts[4]),
                        volume=float(parts[5]),
                        amount=float(parts[6]),
                        amplitude=float(parts[7]),
                        pct_chg=float(parts[8]),
                        chg=float(parts[9]),
                        turnover=float(parts[10]),
                    )
                )
            return out
        except Exception as e:
            last_err = e
            if i < max_retries - 1:
                time.sleep(sleep_sec)
                continue
            raise

    # 理论上不会到这里
    raise RuntimeError(last_err)  # type: ignore


def write_csv(bars: List[KlineBar], out_path: str) -> None:
    with open(out_path, "w", newline="", encoding="utf-8-sig") as f:
        w = csv.writer(f)
        w.writerow(
            [
                "date",
                "open",
                "close",
                "high",
                "low",
                "volume",
                "amount",
                "amplitude",
                "pct_chg",
                "chg",
                "turnover",
            ]
        )
        for b in bars:
            w.writerow(
                [
                    b.date,
                    b.open,
                    b.close,
                    b.high,
                    b.low,
                    b.volume,
                    b.amount,
                    b.amplitude,
                    b.pct_chg,
                    b.chg,
                    b.turnover,
                ]
            )


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description="抓取A股K线并保存为CSV（东方财富接口）")
    ap.add_argument("--symbol", required=True, help="6位代码，如 600519；也可 600519.SH/000001.SZ")
    ap.add_argument("--period", default="daily", help="1m/5m/15m/30m/60m/daily/weekly/monthly")
    ap.add_argument("--adjust", default="qfq", help="none/qfq/hfq")
    ap.add_argument("--start", default=None, help="开始日期 YYYY-MM-DD 或 YYYYMMDD")
    ap.add_argument("--end", default=None, help="结束日期 YYYY-MM-DD 或 YYYYMMDD")
    ap.add_argument("--out", default=None, help="输出CSV路径")
    ap.add_argument("--limit", type=int, default=10000, help="最多返回条数")
    args = ap.parse_args(argv)

    out_path = args.out
    if not out_path:
        out_path = f"{normalize_symbol_digits(args.symbol)}_{args.period}_{args.adjust}.csv"

    bars = fetch_kline_eastmoney(
        symbol=args.symbol,
        period=args.period,
        adjust=args.adjust,
        start=args.start,
        end=args.end,
        limit=args.limit,
    )

    if not bars:
        print("No data.", file=sys.stderr)
        return 2

    write_csv(bars, out_path)
    print(out_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
