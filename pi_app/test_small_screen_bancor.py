import json
import unittest
from unittest import mock

import agent_small_screen as screen
from small_screen_bancor import build_bancor_market_view


class SmallScreenBancorTests(unittest.TestCase):
    def test_formats_the_same_market_fields_as_the_web_ui(self):
        view = build_bancor_market_view(
            {
                "status": "open",
                "point_reserve": "24109944.93576292",
                "usd_reserve": "41477.82591267",
                "marginal_price_usd_per_point": "0.00172036",
                "fee_bps": 50,
                "daily_marginal_price": {
                    "high_usd_per_point": "0.00172065",
                    "low_usd_per_point": "0.00124710",
                    "change_percent": "16.97",
                    "trade_count": 275,
                },
            },
            "CN",
        )

        self.assertEqual(view["price"], "0.00172036 USD")
        self.assertIn("当日最高 0.00172065", view["daily"])
        self.assertIn("当日最低 0.0012471", view["daily"])
        self.assertIn("日涨跌 +16.97%", view["daily"])
        self.assertIn("24,109,944.94 POINT", view["reserves"])
        self.assertIn("41,477.83 USD", view["reserves"])
        self.assertIn("市场: 开放", view["meta"])
        self.assertIn("手续费: 0.50%", view["meta"])
        self.assertIn("今日成交: 275", view["meta"])
        self.assertEqual(view["change_direction"], "up")

    def test_market_fetch_uses_the_web_ui_read_endpoint(self):
        response = {
            "ok": True,
            "data": {"status": "open", "marginal_price_usd_per_point": "0.001"},
        }
        with mock.patch.object(
            screen,
            "localhost_api_request",
            return_value=json.dumps(response).encode("utf-8"),
        ) as request:
            market, error = screen.fetch_nni_bancor_market("key")

        self.assertEqual(error, "")
        self.assertEqual(market["status"], "open")
        self.assertEqual(request.call_args.args[0:2], ("GET", "/v1/nni/bancor/market"))

    def test_missing_market_has_a_readable_fallback(self):
        view = build_bancor_market_view({}, "EN", "temporary failure")

        self.assertEqual(view["price"], "--")
        self.assertEqual(view["meta"], "temporary failure")
        self.assertEqual(view["change_direction"], "flat")


if __name__ == "__main__":
    unittest.main()
