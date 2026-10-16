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
        self.assertIn('$("$APP_PYTHON_BIN" --version', source)
        self.assertNotIn('$($APP_PYTHON_BIN --version', source)

    def test_release_deployment_is_prebuilt_and_keeps_one_rollback(self):
        deploy_source = (ROOT / "deploy-github-release.sh").read_text()
        bootstrap_source = (ROOT / "install-latest-release.sh").read_text()
        for source in (deploy_source, bootstrap_source):
            for forbidden in ("cargo build", "npm install", "npm run build", "vite build"):
                self.assertNotIn(forbidden, source)
        self.assertIn("KEEP_BACKUPS=1", deploy_source)
        self.assertIn('rm -rf "$WORK_DIR"', deploy_source)
        self.assertIn('rm -rf "$PACKAGE_STAGE_DIR"', deploy_source)

    def test_fresh_install_skips_unconfigured_telegram(self):
        import tomllib

        telegram = tomllib.loads(
            (ROOT / "configs/channels/telegram.toml").read_text(encoding="utf-8")
        )
        self.assertFalse(telegram["telegram_bot"]["enabled"])
        launcher = (ROOT / "start-all-bin.sh").read_text()
        self.assertIn("telegram_credentials_available()", launcher)
        self.assertIn('elif [[ "$TELEGRAM_ENABLED" != "1" ]]; then', launcher)
        self.assertIn("elif ! telegram_credentials_available; then", launcher)
        self.assertIn("Telegram is not configured yet; skipping telegramd startup.", launcher)
        self.assertIn('print("1" if configured else "0")', launcher)
        self.assertNotIn("raise SystemExit(0 if configured else 1)", launcher)
        self.assertIn('STARTUP_FAILURES=""', launcher)
        self.assertNotIn("STARTUP_FAILURES=()", launcher)
        self.assertNotIn("${#STARTUP_FAILURES[@]}", launcher)

    def test_bundled_skill_bootstrap_is_safe_on_macos_bash_3(self):
        deploy_source = (ROOT / "deploy-github-release.sh").read_text()
        bootstrap_start = deploy_source.index("bootstrap_bundled_release_skills()")
        bootstrap_end = deploy_source.index("converge_installed_bundled_skills()")
        bootstrap = deploy_source[bootstrap_start:bootstrap_end]
        self.assertIn("local -a bootstrap_args=(", bootstrap)
        self.assertIn("bootstrap_args+=(--skip-bundled-skill", bootstrap)
        self.assertNotIn("resource_skipped_skills", bootstrap)

    def test_release_package_contains_runtime_assets_without_build_caches(self):
        package_source = (ROOT / "package-release.sh").read_text()
        self.assertIn('cp -R "$SCRIPT_DIR/UI/dist" "$STAGE_PROJECT_DIR/UI/dist"', package_source)
        self.assertIn('mkdir -p "$STAGE_PROJECT_DIR/target/release"', package_source)
        self.assertIn('mkdir -p "$STAGE_PROJECT_DIR/prebuilt/skill-packages"', package_source)
        self.assertIn("bundled-skill-bootstrap-v1.json", package_source)
        for forbidden in (
            'copy_if_exists "UI/src"',
            'copy_if_exists "UI/node_modules"',
            'copy_if_exists "target"',
            'copy_if_exists "crates"',
        ):
            self.assertNotIn(forbidden, package_source)

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

    def test_macos_release_is_full_and_embedded_in_desktop_installer(self):
        workflow = (ROOT / ".github/workflows/macos-arm64-release-artifact.yml").read_text()
        self.assertIn('APP_PRECOMPILE_ALL_BUNDLED_SKILLS: "1"', workflow)
        self.assertIn("APP_RELEASE_INSTALL_ALL_BUNDLED_SKILLS=1", workflow)
        self.assertIn("APP_RELEASE_PYTHON_RUNTIME_ROOT", workflow)
        self.assertIn("APP_RELEASE_NODE_RUNTIME_ROOT", workflow)
        self.assertIn("APP_RELEASE_HOST_TOOLS_ROOT", workflow)
        self.assertIn("bundle_macos_skill_dependencies.py", workflow)
        dependency_bundler = (
            ROOT / "scripts/bundle_macos_skill_dependencies.py"
        ).read_text()
        self.assertIn('"git": "git/bin/git"', dependency_bundler)
        self.assertIn('(tesseract, "tesseract-real")', dependency_bundler)
        self.assertIn("write_tesseract_command_shim(root)", dependency_bundler)
        self.assertIn('--tessdata-dir "$TESSDATA_PREFIX"', dependency_bundler)
        self.assertIn('root / "chromium-executable-relative"', dependency_bundler)
        self.assertIn('root / "bin/chromium"', dependency_bundler)
        self.assertIn('"chromium_command": "bin/chromium"', dependency_bundler)
        self.assertNotIn("compile_tool_launchers", dependency_bundler)
        self.assertNotIn("tool_launcher_source", dependency_bundler)
        self.assertIn('release host tools/${RUST_TARGET}', workflow)
        self.assertIn('tesseract "${tesseract_fixture}" stdout -l chi_sim', workflow)
        self.assertIn('Path(sys.argv[1]).write_bytes(b"P5\\n100 40\\n255\\n"', workflow)
        self.assertNotIn("tesseract --list-langs | grep -q", workflow)
        self.assertIn('[[ "$(command -v git)" == "${host_tools}/git/bin/git" ]]', workflow)
        self.assertIn(
            '[[ "$(command -v tesseract)" == "${host_tools}/bin/tesseract" ]]',
            workflow,
        )
        self.assertIn('[[ -x "${host_tools}/bin/tesseract-real" ]]', workflow)
        self.assertIn(
            '[[ "$(command -v whisper-cli)" == "${host_tools}/bin/whisper-cli" ]]',
            workflow,
        )
        self.assertIn("APP_DESKTOP_BOOTSTRAP_PYTHON_ROOT", workflow)
        self.assertIn("APP_DESKTOP_RUNTIME_BUNDLE_DIR", workflow)
        self.assertIn("embedded-runtime/bundle.json", workflow)
        self.assertGreaterEqual(
            workflow.count('cat "${app_data}/bundled-runtime-install.log" >&2 || true'),
            2,
        )
        self.assertIn(
            'local cleanup_runtime_root="${runtime_root:-${app_data}/runtime}"',
            workflow,
        )
        self.assertIn('sudo rm -rf "${app_data}"', workflow)
        package = (ROOT / "package-release.sh").read_text()
        self.assertIn('*-linux-*|*-apple-darwin', package)
        self.assertNotIn("supported only by Linux Release packages", package)
        self.assertIn("prebuilt/toolchains/python", package)
        self.assertIn("prebuilt/toolchains/node", package)
        self.assertIn("prebuilt/toolchains/host", package)
        launcher = (ROOT / "start-all-bin.sh").read_text()
        self.assertIn("prebuilt/toolchains/python/bin/python3", launcher)
        self.assertIn("prebuilt/toolchains/node/bin", launcher)
        self.assertIn('configure_bundled_host_tools "$SCRIPT_DIR"', launcher)
        shell_compat = (ROOT / "scripts/shell_compat.sh").read_text()
        self.assertIn("configure_bundled_host_tools()", shell_compat)
        self.assertIn("chromium-executable-relative", shell_compat)
        self.assertIn('"$host_root/bin/chromium"', shell_compat)
        self.assertIn('GIT_EXEC_PATH="$host_root/git/libexec/git-core"', shell_compat)
        self.assertIn('TESSDATA_PREFIX="$host_root/share/tessdata"', shell_compat)
        self.assertIn('$("$SKILL_RECEIPT_CLI" receipt-verify', launcher)
        self.assertNotIn('$($SKILL_RECEIPT_CLI receipt-verify', launcher)
        source_launcher = (ROOT / "start-all.sh").read_text()
        self.assertIn('$("$receipt_cli" receipt-verify', source_launcher)
        self.assertNotIn('$($receipt_cli receipt-verify', source_launcher)
        desktop_runtime = (ROOT / "desktop/src/bundled_runtime.rs").read_text()
        self.assertIn("executable_fingerprint", desktop_runtime)
        self.assertIn('.arg("--force")', desktop_runtime)

    def test_public_bootstrap_uses_overwrite_reconciliation(self):
        source = (ROOT / "install-latest-release.sh").read_text()
        invocation = 'bash "$BOOTSTRAP/deploy-github-release.sh" --root "$ROOT_DIR" --force "$restart_option"'
        self.assertIn(invocation, source)

    def test_macos_pkg_uses_a_fixed_overwrite_payload(self):
        source = (ROOT / "desktop/scripts/build-macos-pkg.sh").read_text()
        self.assertIn('--root "${PAYLOAD_ROOT}"', source)
        self.assertIn('"${PAYLOAD_ROOT}/Applications/${APP_BUNDLE_NAME}"', source)
        self.assertIn('--component-plist "${COMPONENT_PLIST}"', source)
        self.assertIn("<key>BundleIsRelocatable</key>", source)
        self.assertIn("<false/>", source)
        self.assertIn("<string>Applications/${APP_BUNDLE_NAME}</string>", source)
        self.assertIn('/bin/rm -rf "\\${managed_app}"', source)
        self.assertNotIn('--component "${APP_BUNDLE}"', source)
        self.assertNotIn("Application Support", source)

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

    def test_package_excludes_skill_build_cache_before_receipt_pruning(self):
        package_source = (ROOT / "package-release.sh").read_text()
        cache_cleanup = 'rm -rf "$STAGE_PROJECT_DIR/prebuilt/skill-packages/cache"'
        receipt_prune = 'prune_staged_receipt_versions "$STAGE_PROJECT_DIR/prebuilt/skill-packages"'
        self.assertIn(cache_cleanup, package_source)
        self.assertLess(package_source.index(cache_cleanup), package_source.index(receipt_prune))

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
