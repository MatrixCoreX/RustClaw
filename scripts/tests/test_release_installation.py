"""Release-only installation and native binary admission regressions."""

import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("release_binary", ROOT / "scripts/verify_release_binary.py")
binary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(binary)


class ReleaseInstallationTests(unittest.TestCase):
    def test_native_architectures_and_cross_target_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "program"
            for target, (kind, machine) in binary.TARGETS.items():
                header = bytearray(32)
                if kind == "elf":
                    header[:6] = b"\x7fELF\x02\x01"
                    header[18:20] = machine.to_bytes(2, "little")
                else:
                    header[:4] = b"\xcf\xfa\xed\xfe"
                    struct.pack_into("<I", header, 4, machine)
                path.write_bytes(header)
                binary.verify_binary(path, target)
                for other in binary.TARGETS:
                    if other != target:
                        with self.assertRaises(ValueError):
                            binary.verify_binary(path, other)
            for invalid in (b"", b"#!/bin/sh\nexit 0", b"\x7fELF\x01\x01" + bytes(26)):
                path.write_bytes(invalid)
                with self.assertRaises(ValueError):
                    binary.verify_binary(path, "x86_64-unknown-linux-gnu")

    def test_installer_has_no_build_entrypoint(self):
        source = (ROOT / "install-agent-cmd.sh").read_text()
        for forbidden in ("cargo build", "npm run build", "npm install", "ensure_build()", "install_pinned_rustup.sh"):
            self.assertNotIn(forbidden, source)
        for flag in ("--build", "--force-build"):
            result = subprocess.run(["bash", str(ROOT / "install-agent-cmd.sh"), flag], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("Compilation is not an installation step", result.stdout)

    def test_missing_assets_fail_before_installing_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            for name in ("shell_compat.sh", "product_identity.sh", "verify_release_binary.py"):
                shutil.copyfile(ROOT / "scripts" / name, root / "scripts" / name)
            shutil.copyfile(ROOT / "install-agent-cmd.sh", root / "install-agent-cmd.sh")
            shutil.copyfile(ROOT / "agentctl", root / "agentctl")
            env = {**os.environ, "APP_PRODUCT_IDENTITY_CONFIG": str(ROOT / "configs/product_identity.toml")}
            destination = root / "commands"
            command = ["bash", str(root / "install-agent-cmd.sh"), "--user", "--dir", str(destination)]
            result = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Release binaries are missing", result.stderr)
            self.assertFalse(destination.exists())
            release = root / "target/release"
            release.mkdir(parents=True)
            target = subprocess.check_output(
                ["bash", "-c", 'source scripts/shell_compat.sh; host_rust_target'],
                cwd=ROOT, env=env, text=True,
            ).strip()
            kind, machine = binary.TARGETS[target]
            header = bytearray(32)
            if kind == "elf":
                header[:6] = b"\x7fELF\x02\x01"
                header[18:20] = machine.to_bytes(2, "little")
            else:
                header[:4] = b"\xcf\xfa\xed\xfe"
                struct.pack_into("<I", header, 4, machine)
            for name in ("clawd", "webd", "clawcli"):
                (release / name).write_bytes(header)
                (release / name).chmod(0o755)
            result = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Release UI assets are missing", result.stderr)
            self.assertFalse(destination.exists())
            (root / "UI/dist").mkdir(parents=True)
            (root / "UI/dist/index.html").write_text("<!doctype html><title>fixture</title>")
            result = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((destination / "agentctl").is_symlink())
            self.assertTrue((destination / "clawcli").is_symlink())

    def test_release_workflows_prepare_packaged_browser_assets(self):
        for name in ("ubuntu-x86_64-release.yml", "pi-aarch64-release.yml", "macos-arm64-release-artifact.yml"):
            source = (ROOT / ".github/workflows" / name).read_text()
            self.assertIn("npm ci --ignore-scripts --prefix crates/skills/browser_web", source)
        wheels = (ROOT / ".github/workflows/release-python-wheels.yml").read_text()
        self.assertIn("AGENT_TOOLSDIRECTORY: /opt/agent-python-toolcache", wheels)
        self.assertIn('git config --global --add safe.directory "$GITHUB_WORKSPACE"', wheels)

    def test_package_includes_bridge_local_runtime_dependencies(self):
        package_source = (ROOT / "package-release.sh").read_text()
        bridge = ROOT / "services/wa-web-bridge"
        pending = [bridge / "index.js"]
        visited = set()
        while pending:
            module = pending.pop()
            if module in visited:
                continue
            visited.add(module)
            relative = module.relative_to(ROOT).as_posix()
            self.assertTrue(module.is_file(), relative)
            self.assertIn(f'copy_if_exists "{relative}"', package_source)
            for dependency in re.findall(r'require\(["\'](\./[^"\']+)["\']\)', module.read_text()):
                pending.append(module.parent / dependency)
        self.assertIn(bridge / "media-preflight.js", visited)

    def test_updater_detects_all_native_platforms_without_building(self):
        import tomllib
        identity = tomllib.loads((ROOT / "configs/product_identity.toml").read_text())
        artifact_id = identity["release_artifact_id"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            uname = root / "bin/uname"
            uname.write_text('#!/bin/sh\ncase "$1" in -s) echo "$MOCK_OS";; -m) echo "$MOCK_ARCH";; esac\n')
            uname.chmod(0o755)
            cases = (("Linux", "x86_64", "ubuntu-x86_64"), ("Linux", "aarch64", "pi-aarch64"),
                     ("Darwin", "x86_64", "macos-x86_64"), ("Darwin", "arm64", "macos-aarch64"))
            metadata = root / "releases.json"
            for system, arch, flavor in cases:
                archive = f"{artifact_id}-{flavor}-test.tar.gz"
                metadata.write_text(json.dumps([{"tag_name": f"{flavor}-test", "draft": False,
                    "prerelease": False, "assets": [{"name": archive + suffix,
                        "browser_download_url": "https://example.invalid/" + archive + suffix}
                        for suffix in ("", ".sha256", ".spdx.json", ".manifest.json", ".manifest.json.sig")]}]))
                env = {**os.environ, "PATH": str(root / "bin") + os.pathsep + os.environ["PATH"],
                       "MOCK_OS": system, "MOCK_ARCH": arch, "APP_RELEASES_JSON_FILE": str(metadata)}
                result = subprocess.run(["bash", str(ROOT / "deploy-github-release.sh"),
                    "--root", str(root / "runtime"), "--check-only"], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn(f"release_tag={flavor}-test", result.stdout)


if __name__ == "__main__":
    unittest.main()
