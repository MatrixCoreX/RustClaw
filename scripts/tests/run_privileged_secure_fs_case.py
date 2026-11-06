"""Run one exact, Cargo-described filesystem fixture under CI sudo, never Cargo itself."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import subprocess

TEST_NAME = 'secure_workspace_fs::tests::secure_workspace_atomic_replacement_preserves_mode_and_ownership'
SUCCESS_MARKER = 'secure_workspace_metadata_preserved=true privileged_owner_check=true'
SDK_TEST_NAME = 'receipt::publication_tests::sdk_publication_preserves_existing_owner_and_group'
SDK_SUCCESS_MARKER = 'sdk_publication_metadata_preserved=true privileged_owner_check=true unpreservable_owner_rejected=true'


def resolve_executable(text: str, workspace: Path, *, target_name='clawd', target_kind='bin') -> Path:
    candidates = set()
    for line in text.splitlines():
        if not line.strip():
            continue
        item = json.loads(line)
        if (item.get('reason') == 'compiler-artifact' and item.get('target', {}).get('name') == target_name
                and item.get('target', {}).get('kind') == [target_kind]
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


def run_case(executable: Path, run=None, *, test_name=TEST_NAME, success_marker=SUCCESS_MARKER) -> str:
    if run is None:
        run = subprocess.run
    result = run(['sudo', '-n', str(executable), test_name, '--exact', '--nocapture'],
                 text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    print(result.stdout, end='')
    if (result.returncode != 0 or 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in result.stdout
            or success_marker not in result.stdout):
        raise ValueError('secure_fs_privileged_ownership_fixture_failed')
    return result.stdout


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cargo_artifacts', type=Path)
    parser.add_argument('--sdk-publication', action='store_true')
    args = parser.parse_args()
    if args.sdk_publication:
        run_case(resolve_executable(args.cargo_artifacts.read_text(), Path.cwd(), target_name='skill_sdk', target_kind='lib'),
                 test_name=SDK_TEST_NAME, success_marker=SDK_SUCCESS_MARKER)
    else:
        run_case(resolve_executable(args.cargo_artifacts.read_text(), Path.cwd()))
