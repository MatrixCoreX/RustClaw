import subprocess
import unittest
from unittest import mock

import small_screen_wifi_service as wifi


def _completed(args=None, returncode=0, stdout="", stderr=""):
    return subprocess.CompletedProcess(args or ["nmcli"], returncode, stdout=stdout, stderr=stderr)


class SmallScreenWifiServiceTests(unittest.TestCase):
    def test_split_nmcli_escaped_preserves_colons_and_trailing_backslash(self):
        self.assertEqual(
            wifi._split_nmcli_escaped(r"*:Cafe\:Guest:WPA2:72", expected_parts=4),
            ["*", "Cafe:Guest", "WPA2", "72"],
        )
        self.assertEqual(wifi._split_nmcli_escaped("name\\", expected_parts=2), ["name\\", ""])

    def test_scan_normalizes_open_network_and_binds_saved_profile(self):
        responses = [
            (_completed(stdout=r"*:Cafe\:Guest:--:72" + "\n:Secure:WPA2:61\n"), None),
            (_completed(stdout="Cafe Profile:uuid-1:802-11-wireless\n"), None),
            (_completed(stdout="Cafe:Guest\n"), None),
        ]
        with mock.patch.object(wifi, "_run_nmcli", side_effect=responses):
            items, error = wifi.scan_wifi_networks()

        self.assertIsNone(error)
        self.assertEqual(items[0]["ssid"], "Cafe:Guest")
        self.assertEqual(items[0]["security"], "")
        self.assertTrue(items[0]["saved"])
        self.assertEqual(items[0]["profile_name"], "Cafe Profile")
        self.assertEqual(items[1]["security"], "WPA2")

    def test_permission_check_returns_stable_machine_error(self):
        result = _completed(
            stdout=(
                "org.freedesktop.NetworkManager.network-control:auth\n"
                "org.freedesktop.NetworkManager.wifi.scan:yes\n"
            )
        )
        with mock.patch.object(wifi, "_run_nmcli", return_value=(result, None)):
            error = wifi._network_control_error()

        self.assertEqual(error["error_code"], "permission_required")
        self.assertEqual(error["detail"], "auth")

    def test_saved_network_rejoins_without_password(self):
        result = _completed(stdout="Connection activated\n")
        with mock.patch.object(wifi, "_network_control_error", return_value=None), mock.patch.object(
            wifi, "_run_nmcli", return_value=(result, None)
        ) as run_nmcli:
            ok, payload = wifi.connect_wifi_network(
                "Home WiFi", password="", profile_name="Home Profile"
            )

        self.assertTrue(ok)
        self.assertIn("activated", payload["detail"])
        run_nmcli.assert_called_once_with(
            ["connection", "up", "id", "Home Profile"], timeout=40
        )

    def test_permission_failure_does_not_run_mutating_command(self):
        permission_error = {"error_code": "permission_required", "detail": "auth"}
        with mock.patch.object(
            wifi, "_network_control_error", return_value=permission_error
        ), mock.patch.object(wifi, "_run_nmcli") as run_nmcli:
            ok, payload = wifi.connect_wifi_network("Home WiFi", password="secret")

        self.assertFalse(ok)
        self.assertEqual(payload, permission_error)
        run_nmcli.assert_not_called()

    def test_disconnect_uses_active_device_for_custom_profile(self):
        responses = [
            (_completed(stdout="wlan0:wifi:connected:Home Profile\n"), None),
            (_completed(stdout="Device disconnected\n"), None),
        ]
        with mock.patch.object(wifi, "_network_control_error", return_value=None), mock.patch.object(
            wifi, "_run_nmcli", side_effect=responses
        ) as run_nmcli:
            ok, payload = wifi.disconnect_wifi_network(
                "Home WiFi", profile_name="Home Profile"
            )

        self.assertTrue(ok)
        self.assertIn("disconnected", payload["detail"])
        self.assertEqual(
            run_nmcli.call_args_list[-1],
            mock.call(["device", "disconnect", "wlan0"], timeout=30),
        )


if __name__ == "__main__":
    unittest.main()
