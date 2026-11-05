"""Run one exact, Cargo-described filesystem fixture under CI sudo, never Cargo itself."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import subprocess

TEST_NAME = 'secure_workspace_fs::tests::secure_workspace_atomic_replacement_preserves_mode_and_ownership'


def resolve_executable(text: str, workspace: Path) -> Path:
    candidates = set()
    for line in text.splitlines():
        if not line.strip():
            continue
        item = json.loads(line)
        if (item.get('reason') == 'compiler-artifact' and item.get('target', {}).get('name') == 'clawd'
                and item.get('target', {}).get('kind') == ['bin']
                and item.get('profile', {}).get('test') is True and item.get('executable')):
            candidates.add(item['executable'])
    if len(candidates) != 1:
        raise ValueError('secure_fs_test_executable_not_unique')
    path = Path(candidates.pop())
    if not path.is_absolute() or path.is_symlink() or not path.is_file():
        raise ValueError('secure_fs_test_executable_invalid')
    resolved = path.resolve(strict=True)
    if not resolved.is_relative_to((workspace / 'target').resolve()) or not os.access(resolved, os.X_OK):
        raise ValueError('secure_fs_test_executable_outside_target_or_not_executable')
    return resolved


def run_case(executable: Path, run=None) -> str:
    if run is None:
        run = subprocess.run
    result = run(['sudo', '-n', str(executable), TEST_NAME, '--exact', '--nocapture'],
                 text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    print(result.stdout, end='')
    if (result.returncode != 0 or 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in result.stdout
            or 'secure_workspace_metadata_preserved=true privileged_owner_check=true' not in result.stdout):
        raise ValueError('secure_fs_privileged_ownership_fixture_failed')
    return result.stdout


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cargo_artifacts', type=Path)
    args = parser.parse_args()
    run_case(resolve_executable(args.cargo_artifacts.read_text(), Path.cwd()))
