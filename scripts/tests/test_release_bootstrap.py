"""Offline checks of the public Release-only bootstrap entry point."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[2]
COMMIT = "a" * 40


class ReleaseBootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.fixture = self.root / "fixture"
        for relative in ("scripts/product_identity.sh", "scripts/verify_release_binary.py",
                         "scripts/security/release_manifest.py", "configs/product_identity.toml",
                         "configs/release_allowed_signers"):
            target = self.fixture / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        self.repository = tomllib.loads((ROOT / "configs/product_identity.toml").read_text())["release_repository"]
        self.install = self.root / "runtime with spaces"
        self.log = self.root / "calls.jsonl"
        self.env = {**os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                    "FIXTURE": str(self.fixture), "CALL_LOG": str(self.log), "MOCK_OS": "Linux",
                    "MOCK_ARCH": "x86_64", "MOCK_FAIL": "0", "BOOT_COMMIT": COMMIT,
                    "HOME": str(self.root / "home"), "TMPDIR": str(self.root)}
        self.write_executable(self.bin / "uname", '#!/bin/sh\ncase "$1" in -s) echo "$MOCK_OS";; -m) echo "$MOCK_ARCH";; esac\n')
        self.write_executable(self.bin / "curl", '''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
args = sys.argv[1:]
url = next(arg for arg in args if arg.startswith("https://"))
output = pathlib.Path(args[args.index("--output") + 1])
with open(os.environ["CALL_LOG"], "a") as stream:
    stream.write(json.dumps({"url": url, "args": args}) + "\\n")
if os.environ["MOCK_FAIL"] == "download":
    sys.exit(22)
if url.startswith("https://api.github.com/"):
    output.write_text(json.dumps({"sha": os.environ["BOOT_COMMIT"]}))
else:
    relative = url.split("/" + os.environ["BOOT_COMMIT"] + "/", 1)[1]
    shutil.copyfile(pathlib.Path(os.environ["FIXTURE"]) / relative, output)
''')
        self.write_executable(self.fixture / "deploy-github-release.sh", '''#!/usr/bin/env bash
set -eu
printf '%s\\n' "$*" > "$FIXTURE/deploy-args"
printf '%s\\n' "$APP_RELEASE_ALLOWED_SIGNERS_FILE" > "$FIXTURE/signer"
[[ "$MOCK_FAIL" != verify ]] || exit 1
[[ "$*" != *--check-only* ]] || exit 0
root="$2"
mkdir -p "$root/configs"
cp "$FIXTURE/configs/product_identity.toml" "$root/configs/product_identity.toml"
cp "$FIXTURE/installer" "$root/install-agent-cmd.sh"
''')
        self.write_executable(self.fixture / "installer", '#!/usr/bin/env bash\nprintf "%s\\n" "$*" > "$FIXTURE/install-args"\n')

    def write_executable(self, path, content):
        path.write_text(content)
        path.chmod(0o755)

    def run_bootstrap(self, *options):
        return subprocess.run(["bash", str(ROOT / "install-latest-release.sh"),
                               "--repo", self.repository, "--root", str(self.install), *options],
                              env=self.env, capture_output=True, text=True)

    def test_fresh_install_platforms_pin_bootstrap_and_never_build(self):
        for system, arch in (("Linux", "x86_64"), ("Linux", "aarch64"),
                             ("Darwin", "x86_64"), ("Darwin", "arm64")):
            with self.subTest(system=system, arch=arch):
                self.env.update(MOCK_OS=system, MOCK_ARCH=arch)
                result = self.run_bootstrap()
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("--restart", (self.fixture / "deploy-args").read_text())
                self.assertEqual((self.fixture / "install-args").read_text().strip(), "--user --no-deploy-ui")
        for call in map(json.loads, self.log.read_text().splitlines()):
            self.assertIn("--proto-redir", call["args"])
            if "raw.githubusercontent.com" in call["url"]:
                self.assertIn(f"/{COMMIT}/", call["url"])
        source = (ROOT / "install-latest-release.sh").read_text()
        for forbidden in ("cargo build", "npm install", "npm run build", "git clone", "sudo"):
            self.assertNotIn(forbidden, source)

    def test_existing_trust_anchor_and_no_start(self):
        signer = self.install / "configs/release_allowed_signers"
        signer.parent.mkdir(parents=True)
        signer.write_text("existing trusted signer")
        result = self.run_bootstrap("--no-start")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.fixture / "signer").read_text().strip(), str(signer))
        self.assertEqual(signer.read_text(), "existing trusted signer")
        self.assertIn("--no-restart", (self.fixture / "deploy-args").read_text())

    def test_check_only_does_not_install(self):
        result = self.run_bootstrap("--check-only")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.install.exists())
        self.assertFalse((self.fixture / "install-args").exists())

    def test_failures_stop_before_install(self):
        for failure in ("download", "verify"):
            self.env["MOCK_FAIL"] = failure
            result = self.run_bootstrap()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.install.exists())
            self.assertFalse((self.fixture / "install-args").exists())
        self.assertFalse(list(self.root.glob("agent-release-bootstrap.*")))

    def test_invalid_platform_commit_and_arguments(self):
        self.env["MOCK_ARCH"] = "armv7l"
        self.assertNotEqual(self.run_bootstrap().returncode, 0)
        self.assertFalse(self.log.exists())
        self.env.update(MOCK_ARCH="x86_64", BOOT_COMMIT="../bad")
        self.assertNotEqual(self.run_bootstrap().returncode, 0)
        self.assertFalse(self.install.exists())
        self.assertEqual(self.run_bootstrap("--ref", "../../bad").returncode, 2)
        self.assertNotEqual(self.run_bootstrap("--root", "/").returncode, 0)


if __name__ == "__main__":
    unittest.main()
