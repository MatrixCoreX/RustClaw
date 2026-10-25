#!/usr/bin/env python3
"""Reject Linux release Python runtimes that cannot move or back copied venvs.

This is a build-time gate, not a runtime installer. The child mount namespace
hides both the original runtime and /opt (including Actions' toolcache). Tests
run with an empty environment and verify the same ELF-copy/configuration used
by the SDK's precompiled Python rebind, without weakening receipt verification.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


PROBE = r'''
import ctypes, json, pathlib, shutil, sqlite3, ssl, subprocess, sys, tomllib, venv, zlib

root, work, expected = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
assert pathlib.Path('/opt').is_dir() and not list(pathlib.Path('/opt').iterdir())
original = pathlib.Path(sys.argv[4])
assert not original.exists() or not list(original.iterdir()), 'original runtime remains visible'

check = r"""
import ctypes, json, pathlib, sqlite3, ssl, sys, tomllib, venv, zlib
root, expected, copied = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3] == 'true'
assert sys.version.split()[0] == expected, (sys.version, expected)
assert pathlib.Path(sys.base_prefix).resolve() == root, sys.base_prefix
assert (sys.prefix != sys.base_prefix) == copied, (sys.prefix, sys.base_prefix)
assert not pathlib.Path(sys.executable).is_symlink(), sys.executable
assert sqlite3.connect(':memory:').execute('select 1').fetchone() == (1,)
assert zlib.decompress(zlib.compress(b'portable')) == b'portable'
assert tomllib.loads('portable = true')['portable']
ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
ctypes.CDLL(None)
for line in pathlib.Path('/proc/self/maps').read_text().splitlines():
    fields = line.split(maxsplit=5)
    if len(fields) != 6:
        continue
    path = fields[5].removesuffix(' (deleted)')
    if path.startswith('/') and pathlib.Path(path).name.startswith('libpython'):
        assert pathlib.Path(path).is_relative_to(root), 'host libpython loaded'
print(json.dumps({'version': sys.version.split()[0], 'copied': copied, 'stdlib': True}))
"""

def run(program, copied):
    version = subprocess.check_output([str(program), '--version'], env={}, text=True).strip()
    assert version == 'Python ' + expected, version
    result = subprocess.run(
        [str(program), '-I', '-B', '-c', check, str(root), expected, str(copied).lower()],
        env={}, check=True, text=True, capture_output=True,
    )
    return json.loads(result.stdout)

base = root / 'bin/python3'
results = {'base': run(base, False)}
copied = work / 'venv with copies'
venv.EnvBuilder(with_pip=False, symlinks=False).create(copied)
results['copied_venv'] = run(copied / 'bin/python3', True)

# Match installer.rs rebind_precompiled_python: copy only the interpreter and
# write its home/executable, without copying shared libraries into the venv.
rebound = work / 'sdk rebound venv'
(rebound / 'bin').mkdir(parents=True)
program = rebound / 'bin/python3'
shutil.copy2(base, program)
(rebound / 'pyvenv.cfg').write_text(
    f'home = {base.parent}\ninclude-system-site-packages = false\n'
    f'version = {expected}\nexecutable = {base}\n'
)
results['sdk_rebound_venv'] = run(program, True)
print(json.dumps({'schema_version': 1, 'version': expected, 'source_hidden': True,
                  'toolcache_hidden': True, 'results': results}))
'''


def verify(runtime_root, expected_version=None):
    if sys.platform != "linux":
        raise ValueError("Linux Python portability verification requires Linux")
    runtime_root = Path(runtime_root).resolve(strict=True)
    program = runtime_root / "bin/python3"
    if not program.is_file() or not os.access(program, os.X_OK):
        raise ValueError("Linux Python runtime is missing bin/python3")
    version = subprocess.check_output([str(program), "--version"], env={}, text=True).strip()
    match = re.fullmatch(r"Python (\d+\.\d+\.\d+)", version)
    if not match or expected_version and match[1] != expected_version:
        raise ValueError(f"Linux Python version mismatch: {version}")
    expected_version = expected_version or match[1]
    bwrap = shutil.which("bwrap")
    if not bwrap:
        raise ValueError("Linux Python portability verification requires bubblewrap")
    with tempfile.TemporaryDirectory(prefix="agent-python-relocation-") as directory:
        work = Path(directory).resolve()
        relocated = work / "different release root with spaces" / "python"
        shutil.copytree(runtime_root, relocated, symlinks=False)
        command = [
            bwrap, "--die-with-parent", "--unshare-user", "--unshare-pid", "--unshare-net",
            "--ro-bind", "/", "/", "--tmpfs", "/opt",
        ]
        if not runtime_root.is_relative_to("/opt"):
            command += ["--tmpfs", str(runtime_root)]
        command += [
            "--bind", str(work), str(work), "--proc", "/proc", "--dev", "/dev",
            "--chdir", str(work), "--clearenv", "--", str(relocated / "bin/python3"),
            "-I", "-B", "-c", PROBE, str(relocated), str(work), expected_version,
            str(runtime_root),
        ]
        completed = subprocess.run(command, env={}, text=True, capture_output=True, timeout=90)
        if completed.returncode:
            raise ValueError("Linux Python relocation/copied-venv verification failed:\n" + completed.stderr)
        return json.loads(completed.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime_root", type=Path)
    parser.add_argument("--expected-version")
    args = parser.parse_args()
    try:
        result = verify(args.runtime_root, args.expected_version)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
