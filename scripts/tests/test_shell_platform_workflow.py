"""Keep script-only verification scoped and exercise its rejection behavior."""
from contextlib import redirect_stdout
import io
from pathlib import Path
import textwrap
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / '.github/workflows/shell-platform-ci.yml'


def verify_scope(paths):
    source = WORKFLOW.read_text()
    body = source.split("python3 - <<'PY'\n", 1)[1].split('\n          PY', 1)[0]
    changed = b'\0'.join(path.encode() for path in paths) + b'\0'
    with patch('subprocess.check_output', return_value=changed), redirect_stdout(io.StringIO()):
        exec(compile(textwrap.dedent(body), str(WORKFLOW), 'exec'), {})


class ShellPlatformWorkflowTests(unittest.TestCase):
    def test_native_platform_matrix_and_identity_build_are_preserved(self):
        source = WORKFLOW.read_text()
        self.assertIn('os: [ubuntu-22.04, macos-15]', source)
        self.assertIn("github.event_name != 'push' || github.ref != 'refs/heads/main'", source)
        self.assertIn('branches: [main, "ci/build-capacity-*"]', source)
        self.assertIn('bash scripts/shell_compat_tests.sh', source)
        self.assertIn('bash scripts/product_identity_tests.sh --with-ui', source)
        self.assertIn('python3 scripts/check_on_demand_skill_builds.py', source)
        self.assertNotIn('cargo build', source)
        self.assertNotIn('cargo test', source)

    def test_only_the_reviewed_script_and_test_paths_are_accepted(self):
        verify_scope([])
        verify_scope([
            'scripts/shell_compat.sh', 'scripts/shell_compat_tests.sh',
            'scripts/shell_capacity_tests.sh', 'scripts/tests/test_shell_platform_workflow.py',
            '.github/workflows/shell-platform-ci.yml',
        ])

    def test_runtime_configuration_ui_and_uncovered_scripts_require_full_verification(self):
        for path in ['crates/clawd/src/main.rs', 'Cargo.toml', 'configs/config.toml',
                     'UI/src/App.tsx', 'scripts/uncovered.sh', 'README.md',
                     'scripts/shell_compat.sh\ncrates/clawd/src/main.rs']:
            with self.subTest(path=path), self.assertRaises(SystemExit):
                verify_scope(['scripts/shell_compat.sh', path])


if __name__ == '__main__':
    unittest.main()
