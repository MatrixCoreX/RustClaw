import tomllib
import unittest
from disposable_case_environment import docker_create_args, test_host_config, redact_config_credentials


class DisposableHostTests(unittest.TestCase):
    def test_nested_sandbox_is_explicit_and_keeps_container_boundaries(self):
        args = docker_create_args("fixture-image", "fixture-container", nested_sandbox=True)
        for flag in ("--privileged", "--mount", "-v", "--volume", "--publish", "-p", "--network=host", "--pid=host"):
            self.assertNotIn(flag, args)
        for capability in ("SYS_ADMIN", "SYS_CHROOT", "NET_ADMIN"):
            self.assertIn(capability, args)
        self.assertIn("no-new-privileges:true", args)
        self.assertEqual(args[args.index("--cap-drop") + 1], "ALL")

    def test_no_host_escalation_or_mount(self):
        args = docker_create_args("fixture-image", "fixture-container")
        for flag in ("--privileged", "--mount", "-v", "--volume", "--publish", "-p", "--network=host", "--pid=host"):
            self.assertNotIn(flag, args)
        self.assertIn("no-new-privileges:true", args)
        self.assertEqual(args[args.index("--cap-drop") + 1], "ALL")
        self.assertNotIn("SYS_ADMIN", args)

    def test_only_disposable_sandbox_changes(self):
        source = '[tools]\nsandbox_mode = "workspace_write"\nallow_sudo = false\n[other]\nvalue = 1\n'
        expected = tomllib.loads(source)
        expected["tools"]["sandbox_mode"] = "danger_full"
        self.assertEqual(tomllib.loads(test_host_config(source)), expected)
        self.assertEqual(tomllib.loads(source)["tools"]["sandbox_mode"], "workspace_write")

    def test_unexpected_policy_rejected(self):
        with self.assertRaises(ValueError):
            test_host_config('[tools]\nsandbox_mode = "read_only"\n')

    def test_channel_credentials_not_forwarded(self):
        source = '[channel]\napp_secret = "fixture-secret"\napi_key = "${MODEL_API_KEY}"\npublic_key = "public"\n'
        parsed = tomllib.loads(redact_config_credentials(source))["channel"]
        self.assertEqual(parsed["app_secret"], "")
        self.assertEqual(parsed["api_key"], "${MODEL_API_KEY}")
        self.assertEqual(parsed["public_key"], "public")


if __name__ == "__main__":
    unittest.main()
