import hashlib
import json
import os
import pathlib
import stat
import subprocess
import sys
import tempfile
import unittest

try:
    from pi_app import signature_simulator
except ModuleNotFoundError:
    import signature_simulator


class SignatureProtocolTests(unittest.TestCase):
    def run_helper(self, state_path, action, action_arg=None):
        script = pathlib.Path(__file__).with_name("signature.py")
        command = [sys.executable, str(script), action]
        if action_arg is not None:
            command.append(str(action_arg))
        env = os.environ.copy()
        env["APP_SIGNATURE_SIMULATOR_STATE"] = str(state_path)
        result = subprocess.run(
            command,
            capture_output=True,
            check=False,
            text=True,
            timeout=15,
            env=env,
        )
        output_lines = [line for line in result.stdout.splitlines() if line.strip()]
        self.assertEqual(len(output_lines), 1, result.stdout)
        self.assertNotIn("Traceback", result.stderr)
        return result, json.loads(output_lines[0])

    def test_pubkey_probe_returns_single_json_line_without_traceback(self):
        script = pathlib.Path(__file__).with_name("signature.py")
        result = subprocess.run(
            [sys.executable, str(script), "pubkey"],
            capture_output=True,
            check=False,
            text=True,
            timeout=15,
        )

        output_lines = [line for line in result.stdout.splitlines() if line.strip()]
        self.assertEqual(len(output_lines), 1, result.stdout)
        payload = json.loads(output_lines[0])
        self.assertIsInstance(payload.get("ok"), bool)
        self.assertNotIn("Traceback", result.stderr)

    def test_simulator_matches_pubkey_signing_and_certificate_protocol(self):
        with tempfile.TemporaryDirectory() as temporary:
            state_path = pathlib.Path(temporary) / "nni" / "signature-simulator.json"
            result, enabled = self.run_helper(state_path, "simulation_enable")
            self.assertEqual(result.returncode, 0)
            self.assertTrue(enabled["ok"])
            self.assertTrue(enabled["simulated"])
            self.assertTrue(enabled["signature_chip_present"])
            self.assertEqual(len(enabled["pubkey"]), 128)
            if os.name == "posix":
                self.assertEqual(stat.S_IMODE(state_path.stat().st_mode), 0o600)

            _, enabled_again = self.run_helper(state_path, "simulation_enable")
            self.assertEqual(enabled_again["pubkey"], enabled["pubkey"])

            _, pubkey = self.run_helper(state_path, "pubkey")
            self.assertEqual(pubkey["pubkey"], enabled["pubkey"])
            self.assertEqual(pubkey["i2c_address"], "virtual")

            timestamp = 1_800_000_000
            _, signed = self.run_helper(state_path, "sign_timestamp", timestamp)
            self.assertEqual(len(signed["signature"]), 128)
            digest = hashlib.sha256(str(timestamp).encode("utf-8")).digest()
            self.assertTrue(
                signature_simulator.verify_raw_signature(
                    enabled["pubkey"], digest, signed["signature"]
                )
            )

            for action, field, size_field in (
                ("tng_device_cert", "device_cert_hex", "device_cert_hex_size"),
                ("tng_signer_cert", "signer_cert_hex", "signer_cert_hex_size"),
                ("tng_root_cert", "root_cert_hex", "root_cert_hex_size"),
            ):
                _, certificate = self.run_helper(state_path, action)
                encoded = bytes.fromhex(certificate[field])
                self.assertEqual(encoded[0], 0x30)
                self.assertEqual(certificate[size_field], len(encoded))

            result, disabled = self.run_helper(state_path, "simulation_disable")
            self.assertEqual(result.returncode, 0)
            self.assertFalse(disabled["signature_chip_present"])
            self.assertTrue(state_path.exists())
            disabled_state = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertFalse(disabled_state["enabled"])
            private_key_fields = (
                "device_private_key",
                "signer_private_key",
                "root_private_key",
            )
            saved_private_keys = tuple(disabled_state[field] for field in private_key_fields)

            result, unavailable = self.run_helper(state_path, "pubkey")
            if result.returncode == 0:
                # A physical chip may still be available after the simulator is
                # disabled.  In that case the helper must use the real I2C
                # device instead of silently continuing with the simulator.
                self.assertTrue(unavailable["ok"])
                self.assertNotEqual(unavailable.get("i2c_address"), "virtual")
                self.assertNotEqual(unavailable.get("pubkey"), enabled["pubkey"])
            else:
                self.assertFalse(unavailable["ok"])

            result, reenabled = self.run_helper(state_path, "simulation_enable")
            self.assertEqual(result.returncode, 0)
            self.assertEqual(reenabled["pubkey"], enabled["pubkey"])
            reenabled_state = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertTrue(reenabled_state["enabled"])
            self.assertEqual(
                tuple(reenabled_state[field] for field in private_key_fields),
                saved_private_keys,
            )

            state_path.write_text("not-json\n", encoding="utf-8")
            result, invalid = self.run_helper(state_path, "simulation_enable")
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(invalid["ok"])
            self.assertEqual(invalid["error_code"], "signature_simulator_state_invalid")
            self.assertEqual(state_path.read_text(encoding="utf-8"), "not-json\n")

    def test_legacy_state_without_enabled_flag_remains_enabled(self):
        with tempfile.TemporaryDirectory() as temporary:
            state_path = pathlib.Path(temporary) / "signature-simulator.json"
            _, enabled = self.run_helper(state_path, "simulation_enable")
            state = json.loads(state_path.read_text(encoding="utf-8"))
            state.pop("enabled")
            state_path.write_text(json.dumps(state), encoding="utf-8")

            result, pubkey = self.run_helper(state_path, "pubkey")
            self.assertEqual(result.returncode, 0)
            self.assertEqual(pubkey["pubkey"], enabled["pubkey"])


if __name__ == "__main__":
    unittest.main()
