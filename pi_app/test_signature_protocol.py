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
            self.assertFalse(state_path.exists())

            state_path.write_text("not-json\n", encoding="utf-8")
            result, repaired = self.run_helper(state_path, "simulation_enable")
            self.assertEqual(result.returncode, 0)
            self.assertTrue(repaired["ok"])
            self.assertEqual(len(repaired["pubkey"]), 128)


if __name__ == "__main__":
    unittest.main()
