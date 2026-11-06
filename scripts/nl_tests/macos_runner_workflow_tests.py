"""Keep native Mac verification on supported runners without changing target OS floors."""
from copy import deepcopy
from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
EXPECTED = (
    ("macos-polyglot-skill-ci.yml", "polyglot-skill-macos", "macos-15", None),
    ("macos-arm64-release-artifact.yml", "build-macos",
     "${{ inputs.architecture == 'x86_64' && 'macos-15-intel' || 'macos-15' }}", "13.0"),
    ("release-python-wheels.yml", "macos-wheels",
     "${{ startsWith(inputs.target, 'aarch64') && 'macos-15' || 'macos-15-intel' }}", "13.0"),
    ("desktop-native.yml", "desktop",
     "${{ matrix.platform == 'windows-x64' && 'windows-2022' || matrix.platform == 'macos-arm64' && 'macos-15' || 'macos-15-intel' }}", "13.0"),
)


def load(name):
    return yaml.safe_load((ROOT / ".github/workflows" / name).read_text(encoding="utf-8"))


def valid_job(job, runner, target):
    return job.get("runs-on") == runner and (
        target is None or str(job.get("env", {}).get("MACOSX_DEPLOYMENT_TARGET")) == target
    )


def required_reload_gate(job, selector="skill_reload_tests"):
    command = f"cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1 --nocapture"
    return required_command_gate(job, command)


def required_command_gate(job, command):
    selected = [step for step in job["steps"] if command in step.get("run", "")]
    return len(selected) == 1 and (
        "if" not in selected[0] and not selected[0].get("continue-on-error", False)
        and "set -euo pipefail" in selected[0]["run"]
        and sum(line.strip() == command for line in selected[0]["run"].splitlines()) == 1
    )


class MacosRunnerWorkflowTests(unittest.TestCase):
    def test_catalogue_selection_and_cli_contracts_gate_both_systems_and_signed_artifact(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        commands = [
            'cargo test --locked -p claw-core llm_provider_descriptors::tests -- --test-threads=1 --nocapture',
            'cargo test --locked -p clawd --bin clawd llm_gateway::tests -- --test-threads=1 --nocapture',
            'cargo test --locked -p clawd --bin clawd llm_provider_runtime_tests -- --test-threads=1 --nocapture',
            "cargo test --locked -p clawd --bin clawd model_catalog -- --test-threads=1 --nocapture",
            "cargo test --locked -p clawd --bin clawd task_model_selection -- --test-threads=1 --nocapture",
            "cargo test --locked -p clawcli commands::models_tests -- --test-threads=1 --nocapture",
        ]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for command in commands:
                self.assertTrue(required_command_gate(jobs[name], command))
                for mutation in ["remove", "ignore", "continue", "conditional"]:
                    with self.subTest(job=name, command=command, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job["steps"] if command in step.get("run", ""))
                        if mutation == "remove": step["run"] = step["run"].replace(command, "true")
                        elif mutation == "ignore": step["run"] = step["run"].replace(command, command + " || true")
                        elif mutation == "continue": step["continue-on-error"] = True
                        else: step["if"] = "${{ always() }}"
                        self.assertFalse(required_command_gate(job, command))
        build = next(step for step in jobs["runtime-read-linux"]["steps"]
                     if step.get("name") == "Build the exact verified Linux core without publishing a release")
        for fragment in [
            "cargo build --locked --release -p clawd --bin clawd -p clawcli --bin clawcli",
            'install -m 755 target/release/clawcli "$artifact_root/runtime-read/clawcli"',
            "sha256sum clawd clawcli source-commit verified-gates",
            "model_catalog_truthfulness task_model_selection cli_model_readiness",
            "provider_descriptors",
        ]:
            self.assertIn(fragment, build["run"])

    def test_effective_provider_credentials_gate_both_systems_without_skipping_or_ignoring_failures(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        selector = "provider_credential_tests"
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            self.assertTrue(required_reload_gate(jobs[name], selector))
            for mutation in ["remove", "ignore", "continue", "conditional"]:
                with self.subTest(job=name, mutation=mutation):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job["steps"] if f"--bin clawd {selector} --" in step.get("run", ""))
                    if mutation == "remove": step["run"] = step["run"].replace(f"--bin clawd {selector} --", "--bin clawd absent_fixture --")
                    elif mutation == "ignore": step["run"] = step["run"].replace(f"{selector} -- --test-threads=1", f"{selector} -- --ignored --test-threads=1")
                    elif mutation == "continue": step["continue-on-error"] = True
                    else: step["if"] = "${{ always() }}"
                    self.assertFalse(required_reload_gate(job, selector))
        build = next(step for step in jobs["runtime-read-linux"]["steps"]
                     if step.get("name") == "Build the exact verified Linux core without publishing a release")
        self.assertIn("provider_credentials", build["run"])

    def test_configuration_lock_and_credential_transaction_suites_gate_both_systems(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for command in [
                "cargo test --locked -p clawd --bin clawd configuration_files::tests -- --test-threads=1 --nocapture",
                "cargo test --locked -p clawd --bin clawd channel_registration_race_tests::application_tests -- --test-threads=1 --nocapture",
                "cargo test --locked -p claw-core secrets::model_environment::tests -- --test-threads=1 --nocapture",
            ]:
                self.assertTrue(required_command_gate(jobs[name], command))
                for mutation in ["remove", "ignore", "continue", "conditional"]:
                    with self.subTest(job=name, command=command, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job["steps"] if command in step.get("run", ""))
                        if mutation == "remove": step["run"] = step["run"].replace(command, "true")
                        elif mutation == "ignore": step["run"] = step["run"].replace(command, command + " || true")
                        elif mutation == "continue": step["continue-on-error"] = True
                        else: step["if"] = "${{ always() }}"
                        self.assertFalse(required_command_gate(job, command))

    def test_shared_configuration_and_retained_credentials_gate_both_native_systems(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for selector in ["configuration_write_tests", "runtime_config_write_", "llm_credentials_"]:
                self.assertTrue(required_reload_gate(jobs[name], selector), (name, selector))
        build = next(step for step in jobs["runtime-read-linux"]["steps"]
                     if step.get("name") == "Build the exact verified Linux core without publishing a release")
        self.assertIn("configuration_publication", build["run"])

    def test_configuration_write_gate_cannot_skip_or_discard_a_failure(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for selector in ["configuration_write_tests", "runtime_config_write_", "llm_credentials_"]:
                for mutation in ["remove", "ignore", "continue", "conditional"]:
                    with self.subTest(job=name, selector=selector, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job["steps"] if f"--bin clawd {selector} --" in step.get("run", ""))
                        if mutation == "remove": step["run"] = step["run"].replace(f"--bin clawd {selector} --", "--bin clawd absent_fixture --")
                        elif mutation == "ignore": step["run"] = step["run"].replace(f"{selector} -- --test-threads=1", f"{selector} -- --ignored --test-threads=1")
                        elif mutation == "continue": step["continue-on-error"] = True
                        else: step["if"] = "${{ always() }}"
                        self.assertFalse(required_reload_gate(job, selector))

    def test_runtime_reload_cases_gate_both_native_systems_and_the_signed_core(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            self.assertTrue(required_reload_gate(jobs[name]), name)
        build = next(step for step in jobs["runtime-read-linux"]["steps"]
                     if step.get("name") == "Build the exact verified Linux core without publishing a release")
        self.assertIn("skill_reload_publication", build["run"])

    def test_runtime_reload_gate_cannot_skip_ignore_or_discard_failures(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for mode in ["remove", "ignore", "continue", "conditional"]:
                with self.subTest(job=name, mutation=mode):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job["steps"] if "--bin clawd skill_reload_tests" in step.get("run", ""))
                    if mode == "remove": step["run"] = step["run"].replace("--bin clawd skill_reload_tests", "--bin clawd absent_fixture")
                    elif mode == "ignore": step["run"] = step["run"].replace("skill_reload_tests -- --test-threads=1 --nocapture", "skill_reload_tests -- --ignored --test-threads=1 --nocapture")
                    elif mode == "continue": step["continue-on-error"] = True
                    else: step["if"] = "${{ always() }}"
                    self.assertFalse(required_reload_gate(job))

    def test_supported_runners_preserve_architecture_and_deployment_floor(self):
        for name, job_id, runner, target in EXPECTED:
            with self.subTest(workflow=name, job=job_id):
                self.assertTrue(valid_job(load(name)["jobs"][job_id], runner, target))

    def test_retired_runner_wrong_architecture_and_newer_floor_are_rejected(self):
        for name, job_id, runner, target in EXPECTED:
            job = load(name)["jobs"][job_id]
            for changed_runner in [runner.replace("macos-15", "macos-14"), "macos-15-intel"]:
                changed = deepcopy(job)
                changed["runs-on"] = changed_runner
                with self.subTest(workflow=name, runner=changed_runner):
                    self.assertFalse(valid_job(changed, runner, target))
            if target:
                changed = deepcopy(job)
                changed["env"]["MACOSX_DEPLOYMENT_TARGET"] = "15.0"
                with self.subTest(workflow=name, mutation="deployment_floor"):
                    self.assertFalse(valid_job(changed, runner, target))

    def test_runner_contract_is_in_native_ci(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        command = "python3 scripts/nl_tests/macos_runner_workflow_tests.py"
        self.assertEqual(sum(command in step.get("run", "") for step in steps), 1)

    def test_focused_ui_mode_cannot_disable_automatic_full_runtime_checks(self):
        workflow = load("macos-polyglot-skill-ci.yml")
        option = workflow["on"]["workflow_dispatch"]["inputs"]["verification_scope"]
        self.assertEqual(option["default"], "all")
        self.assertEqual(option["options"], ["all", "ui"])
        self.assertEqual(workflow["jobs"]["polyglot-skill-macos"]["if"],
                         "${{ github.event_name != 'workflow_dispatch' || inputs.verification_scope != 'ui' }}")
        focused = workflow["jobs"]["ui-browser-reliability"]
        self.assertEqual(focused["if"],
                         "${{ github.event_name == 'workflow_dispatch' && inputs.verification_scope == 'ui' }}")
        self.assertEqual(focused["uses"], "./.github/workflows/ui-browser-reliability-ci.yml")
        lightweight = load("ui-browser-reliability-ci.yml")["jobs"]["ui-and-browser"]["steps"]
        guard = next(step for step in lightweight if step.get("name") == "Require unchanged runtime sources for manual UI-only verification")
        self.assertEqual(guard["if"], "${{ github.event_name == 'workflow_dispatch' && !inputs.full_runtime_validation }}")
        self.assertIn("git diff --quiet origin/main HEAD -- Cargo.toml Cargo.lock .cargo crates optional_skills external_skills configs docker prompts pi_app services", guard["run"])
        self.assertIn("exit 1", guard["run"])

    def test_full_mode_runs_both_ui_platforms_alongside_every_runtime_gate(self):
        workflow = load("macos-polyglot-skill-ci.yml")
        jobs = workflow["jobs"]
        full = jobs["full-ui-browser-reliability"]
        self.assertEqual(full["uses"], "./.github/workflows/ui-browser-reliability-ci.yml")
        self.assertEqual(full["with"], {"full_runtime_validation": True})
        for job_id in ["skill-sdk-linux", "runtime-read-linux", "media-session-portability", "polyglot-skill-macos"]:
            self.assertEqual(full["if"], jobs[job_id]["if"])
        self.assertNotIn("continue-on-error", full)
        native = load("ui-browser-reliability-ci.yml")["jobs"]["ui-and-browser"]
        self.assertEqual(native["strategy"]["matrix"]["os"], ["ubuntu-22.04", "macos-15"])
        self.assertFalse(native["strategy"]["fail-fast"])

    def test_lightweight_entry_points_cannot_select_the_internal_full_runtime_mode(self):
        lightweight = load("ui-browser-reliability-ci.yml")
        self.assertIsNone(lightweight["on"]["workflow_dispatch"])
        option = lightweight["on"]["workflow_call"]["inputs"]["full_runtime_validation"]
        self.assertEqual(option["type"], "boolean")
        self.assertIs(option["default"], False)
        self.assertIs(option["required"], False)
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        self.assertNotIn("with", jobs["ui-browser-reliability"])
        self.assertEqual([name for name, job in jobs.items()
                          if job.get("with", {}).get("full_runtime_validation") is True],
                         ["full-ui-browser-reliability"])

    def test_browser_preparation_is_followed_by_the_runtime_signature_selector(self):
        steps = load("macos-arm64-release-artifact.yml")["jobs"]["build-macos"]["steps"]
        script = next(step["run"] for step in steps if step.get("name") == "Test signed system browser and isolated skill profiles")
        prepare = script.index("python3 scripts/prepare_macos_ci_browser.py")
        self.assertLess(prepare, script.index('executable="$(bash scripts/macos-system-browser.sh --resolve-executable)"'))

    def test_runtime_reads_have_independent_cache_and_attested_core_artifacts(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        sdk = jobs["skill-sdk-linux"]
        runtime = jobs["runtime-read-linux"]
        self.assertEqual(runtime["if"], jobs["polyglot-skill-macos"]["if"])
        self.assertEqual(runtime["runs-on"], "ubuntu-22.04")
        self.assertNotIn("cargo test --locked -p clawd", "\n".join(step.get("run", "") for step in sdk["steps"]))
        steps = runtime["steps"]
        named = {step.get("name"): step for step in steps}
        gate = named["Verify Linux indexed history reads and startup upgrades"]
        for selector in ["task_event_transport", "auth_contract_tests", "model_catalog", "model_settings_integrity_tests", "configuration_read_tests", "secure_workspace_fs", "aipp", "run_skill", "skill_admission::tests::state_mutations"]:
            self.assertIn(f"cargo test --locked -p clawd --bin clawd {selector}", gate["run"])
        native_steps = jobs["polyglot-skill-macos"]["steps"]
        stress_command = "cargo test --locked -p claw-core model_catalog::tests -- --test-threads=16"
        self.assertIn(stress_command, gate["run"])
        self.assertIn("for attempt in 1 2 3 4 5; do", gate["run"])
        native_stress = [step for step in native_steps if stress_command in step.get("run", "")]
        self.assertEqual(len(native_stress), 1)
        self.assertIn("for attempt in 1 2 3 4 5; do", native_stress[0]["run"])
        auth_command = "cargo test --locked -p clawd --bin clawd auth_contract_tests"
        self.assertEqual(sum(auth_command in step.get("run", "") for step in native_steps), 1)
        catalog_command = "cargo test --locked -p clawd --bin clawd model_catalog -- --test-threads=1 --nocapture"
        self.assertIn(catalog_command, gate["run"])
        self.assertEqual(sum(catalog_command in step.get("run", "") for step in native_steps), 1)
        self.assertNotIn("--bin clawd task_event_archive", gate["run"])
        self.assertIn("ripgrep", named["Prepare native build and sandbox dependencies"]["run"])
        self.assertIn("python3 scripts/check_aipp_decoupling.py --self-test", gate["run"])
        self.assertIn("python3 scripts/check_aipp_decoupling.py\n", gate["run"])
        build = named["Build the exact verified Linux core without publishing a release"]
        attest = named["Attest the exact Linux core provenance"]
        upload = named["Preserve the exact verified Linux core artifact"]
        self.assertLess(steps.index(gate), steps.index(build))
        self.assertLess(steps.index(named["Verify both product identities with the shared UI"]), steps.index(build))
        self.assertLess(steps.index(build), steps.index(attest))
        self.assertLess(steps.index(attest), steps.index(upload))
        self.assertIn("cargo build --locked --release -p clawd --bin clawd", build["run"])
        self.assertIn("$GITHUB_SHA", build["run"])
        self.assertIn("source-commit", build["run"])
        self.assertIn("aipp_decoupling", build["run"])
        self.assertIn("ui_auth_error_contract", build["run"])
        self.assertIn("model_catalog_fixture_isolation", build["run"])
        self.assertIn("aipp_read_surfaces", build["run"])
        self.assertIn("aipp_generation", build["run"])
        self.assertIn("model_catalog_read", build["run"])
        for selector in ["model_settings_integrity_tests", "configuration_read_tests", "secure_workspace_fs"]:
            command = f"cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1 --nocapture"
            self.assertIn(command, gate["run"])
            self.assertEqual(sum(command in step.get("run", "") for step in native_steps), 1)
        self.assertIn("--no-run --message-format=json", gate["run"])
        self.assertIn("scripts/tests/run_privileged_secure_fs_case.py", gate["run"])
        self.assertIn("model_settings_integrity", build["run"])
        self.assertIn("secure_workspace_metadata", build["run"])
        self.assertIn("configuration_reads", build["run"])
        mutation_command = "cargo test --locked -p clawd --bin clawd skill_admission::tests::state_mutations -- --test-threads=1 --nocapture"
        self.assertIn(mutation_command, gate["run"])
        self.assertEqual(sum(mutation_command in step.get("run", "") for step in native_steps), 1)
        self.assertIn("skill_state_mutations", build["run"])
        for selector in ["skill_admission::tests", "skill_switch_update_tests"]:
            command = f"cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1"
            self.assertIn(command, gate["run"])
            self.assertEqual(sum(command in step.get("run", "") for step in native_steps), 1)
        self.assertIn("skill_switch_updates", build["run"])
        self.assertIn("attest-build-provenance@", attest["uses"])
        self.assertIn("${{ github.sha }}", upload["with"]["name"])
        self.assertEqual(runtime["permissions"]["id-token"], "write")
        caches = [next(step["with"] for step in job["steps"] if "Swatinem/rust-cache@" in step.get("uses", "")) for job in [sdk, runtime]]
        self.assertNotEqual(caches[0]["key"], caches[1]["key"])
        for cache in caches:
            self.assertEqual(str(cache["cache-on-failure"]).lower(), "false")
            self.assertEqual(cache["save-if"], "${{ github.event_name != 'pull_request' }}")

    def test_deployable_ui_is_captured_before_desktop_build_and_uploaded_only_after_checks(self):
        steps = load("ui-browser-reliability-ci.yml")["jobs"]["ui-and-browser"]["steps"]
        named = {step.get("name"): step for step in steps}
        stage = named["Stage the production web UI artifact"]
        upload = named["Upload the verified production web UI"]
        self.assertLess(steps.index(named["Test, typecheck and build UI"]), steps.index(stage))
        self.assertLess(steps.index(stage), steps.index(named["Verify shared desktop UI transport and build"]))
        self.assertLess(steps.index(named["Stress QR scope commits without provider calls"]), steps.index(upload))
        self.assertEqual(stage["if"], "${{ runner.os == 'Linux' }}")
        self.assertEqual(upload["if"], stage["if"])
        self.assertIn("node UI/build-fingerprint.mjs UI configs/product_identity.toml", stage["run"])
        self.assertIn('"$GITHUB_SHA" > UI/dist/.agent-ui-source-commit', stage["run"])
        self.assertIn('-C UI dist', stage["run"])
        self.assertIn('sha256sum "$archive_name"', stage["run"])
        self.assertEqual(upload["with"]["name"], "verified-web-ui-${{ github.sha }}")
        self.assertEqual(upload["with"]["if-no-files-found"], "error")
        self.assertIn("python3 scripts/nl_tests/macos_runner_workflow_tests.py",
                      named["Check browser preparation boundaries"]["run"])
        self.assertIn("--only-binary=:all: --requirement scripts/nl_tests/requirements.txt",
                      named["Check browser preparation boundaries"]["run"])

    def test_ui_authentication_browser_regressions_gate_both_native_artifacts(self):
        job = load("ui-browser-reliability-ci.yml")["jobs"]["ui-and-browser"]
        self.assertEqual(job["strategy"]["matrix"]["os"], ["ubuntu-22.04", "macos-15"])
        steps = job["steps"]
        gate = next(step for step in steps if step.get("name") == "Verify authentication ownership and transient recovery")
        self.assertNotIn("if", gate)
        self.assertIn("set -euo pipefail", gate["run"])
        for fixture in ["ownership", "concurrency", "recovery"]:
            self.assertIn(f"node UI/test/authentication-{fixture}.browser.mjs", gate["run"])
        upload = next(step for step in steps if step.get("name") == "Upload the verified production web UI")
        self.assertLess(steps.index(gate), steps.index(upload))


if __name__ == "__main__":
    unittest.main()
