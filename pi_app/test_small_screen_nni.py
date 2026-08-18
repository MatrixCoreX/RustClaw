import json
import tkinter as tk
import unittest
from types import SimpleNamespace
from unittest import mock

import agent_small_screen as screen
from small_screen_nni import (
    format_nni_runtime_summary,
    nni_previous_window_metrics,
    nni_runtime_is_active,
)


class SmallScreenNniSummaryTests(unittest.TestCase):
    def test_active_hardware_runtime_matches_web_ui_state(self):
        config = {
            "joined": True,
            "worker_running": True,
            "heartbeat_state": "active",
            "network_authorization": "authorized",
            "last_success_node_host": "api.example.test",
            "heartbeat_request_count": 42,
            "last_heartbeat_at_ts": 1_787_000_000,
        }
        device = {
            "signature_chip_present": True,
            "hardware_chip_present": True,
            "simulated": False,
        }

        summary = format_nni_runtime_summary(
            config,
            device,
            "CN",
            rewards={
                "network_devices": {"active_device_count": 8},
                "network_rewards": {"latest_period_end_unix": 1_800_000_600},
                "records": [
                    {
                        "period_end_unix": 1_800_000_600,
                        "reward_points": "625.00000000",
                    }
                ],
            },
        )

        self.assertTrue(nni_runtime_is_active(config))
        self.assertIn("状态: 运行中", summary)
        self.assertIn("心跳: 正常", summary)
        self.assertIn("芯片: 硬件", summary)
        self.assertIn("节点: api.example.test", summary)
        self.assertIn("授权: 已授权", summary)
        self.assertIn("请求: 42", summary)
        self.assertIn("全网活跃设备: 8", summary)
        self.assertIn("上一窗口奖励: 625", summary)
        self.assertNotIn("上一窗口奖励: 625 POINT", summary)

    def test_stopped_runtime_and_missing_device_are_explicit(self):
        summary = format_nni_runtime_summary(
            {
                "joined": False,
                "worker_running": True,
                "heartbeat_state": "disabled",
                "selected_node_url": "https://node.example.test/v1",
                "heartbeat_request_count": 0,
            },
            {},
            "EN",
        )
        self.assertIn("Status: Stopped", summary)
        self.assertIn("Chip: Unavailable", summary)
        self.assertIn("Node: node.example.test", summary)

    def test_refresh_error_keeps_last_good_runtime_visible(self):
        summary = format_nni_runtime_summary(
            {"joined": True, "worker_running": True, "heartbeat_state": "active"},
            {"hardware_chip_present": True},
            "CN",
            "temporary read failure",
        )
        self.assertIn("状态: 运行中", summary)
        self.assertIn("状态同步失败，保留上次结果", summary)

    def test_missing_grant_in_latest_network_window_reports_zero_reward(self):
        active, reward = nni_previous_window_metrics(
            {
                "network_devices": {"active_device_count": 3},
                "network_rewards": {"latest_period_end_unix": 1_800_001_200},
                "records": [
                    {
                        "period_end_unix": 1_800_000_600,
                        "reward_points": "10.50000000",
                    }
                ],
            }
        )

        self.assertEqual(active, 3)
        self.assertEqual(reward, "0")

    def test_runtime_sync_auto_starts_and_stops_the_page_visual(self):
        class FakeButton:
            def __init__(self):
                self.values = {}

            def config(self, **values):
                self.values.update(values)

        join_button = FakeButton()
        test_button = FakeButton()
        app = SimpleNamespace(
            _nni_runtime_config={"joined": True, "worker_running": True},
            _llm_join_btn=join_button,
            _llm_chip_test_btn=test_button,
            _llm_join_in_progress=False,
            _llm_lobster_job=None,
            _t=lambda key: {"llm_join": "加入", "llm_stop": "停止"}[key],
            _start_nni_runtime_visual=mock.Mock(),
            _stop_llm_animation=mock.Mock(),
            _refresh_llm_join_button_state=mock.Mock(),
        )

        screen.SmallScreenApp._sync_nni_runtime_view(app)

        self.assertEqual(join_button.values["text"], "停止")
        self.assertEqual(test_button.values["state"], tk.DISABLED)
        app._start_nni_runtime_visual.assert_called_once_with()

        app._nni_runtime_config = {"joined": False, "worker_running": True}
        app._llm_lobster_job = object()
        screen.SmallScreenApp._sync_nni_runtime_view(app)
        self.assertEqual(join_button.values["text"], "加入")
        self.assertEqual(test_button.values["state"], tk.NORMAL)
        app._stop_llm_animation.assert_called_once_with()

    def test_fetch_overview_uses_the_same_two_endpoints_as_web_ui(self):
        responses = [
            {"ok": True, "data": {"joined": True, "remote_nodes": ["https://node.test"]}},
            {"ok": True, "data": {"signature_chip_present": True}},
            {
                "ok": True,
                "data": {
                    "network_devices": {"active_device_count": 8},
                    "records": [{"reward_points": "625.00000000"}],
                },
            },
        ]
        with mock.patch.object(
            screen,
            "localhost_api_request",
            side_effect=[json.dumps(item).encode("utf-8") for item in responses],
        ) as request:
            config, device, rewards, error = screen.fetch_nni_runtime_overview("key")

        self.assertEqual(error, "")
        self.assertTrue(config["joined"])
        self.assertTrue(device["signature_chip_present"])
        self.assertEqual(rewards["network_devices"]["active_device_count"], 8)
        self.assertEqual(
            [call.args[1] for call in request.call_args_list],
            [
                "/v1/nni/config",
                "/v1/nni/device/status",
                "/v1/nni/rewards?page=1&per_page=1",
            ],
        )

    def test_silent_status_refresh_can_skip_the_reward_signature(self):
        responses = [
            {"ok": True, "data": {"joined": True}},
            {"ok": True, "data": {"signature_chip_present": True}},
        ]
        with mock.patch.object(
            screen,
            "localhost_api_request",
            side_effect=[json.dumps(item).encode("utf-8") for item in responses],
        ) as request:
            config, device, rewards, error = screen.fetch_nni_runtime_overview(
                "key",
                include_rewards=False,
            )

        self.assertEqual(error, "")
        self.assertIsNone(rewards)
        self.assertEqual(len(request.call_args_list), 2)

    def test_stop_updates_persisted_join_state(self):
        response = {"ok": True, "data": {"joined": False, "remote_nodes": []}}
        with mock.patch.object(
            screen,
            "localhost_api_request",
            return_value=json.dumps(response).encode("utf-8"),
        ) as request:
            config, error = screen.update_nni_joined_state("key", False)

        self.assertEqual(error, "")
        self.assertFalse(config["joined"])
        body = json.loads(request.call_args.kwargs["body"].decode("utf-8"))
        self.assertEqual(body, {"joined": False})

    def test_successful_join_can_persist_the_running_state(self):
        response = {
            "ok": True,
            "data": {
                "joined": True,
                "worker_running": True,
                "heartbeat_state": "enabling",
            },
        }
        with mock.patch.object(
            screen,
            "localhost_api_request",
            return_value=json.dumps(response).encode("utf-8"),
        ) as request:
            config, error = screen.update_nni_joined_state("key", True)

        self.assertEqual(error, "")
        self.assertTrue(config["joined"])
        body = json.loads(request.call_args.kwargs["body"].decode("utf-8"))
        self.assertEqual(body, {"joined": True})

    def test_join_request_sends_the_single_node_url_runtime_contract(self):
        response = {
            "ok": True,
            "data": {
                "task_id": "nni-join-test",
                "challenge": "ab" * 32,
                "node_url": "https://api.example.test",
            },
        }
        with mock.patch.object(
            screen,
            "localhost_api_request",
            return_value=json.dumps(response).encode("utf-8"),
        ) as request:
            task, error = screen.request_nni_join_task(
                "key",
                "https://api.example.test",
            )

        self.assertIsNone(error)
        self.assertEqual(task["task_id"], "nni-join-test")
        body = json.loads(request.call_args.kwargs["body"].decode("utf-8"))
        self.assertEqual(body, {"node_url": "https://api.example.test"})


if __name__ == "__main__":
    unittest.main()
