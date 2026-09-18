# Disposable Installation Acceptance

Use `disposable_case_environment.py` for tests requiring package installation or
credential-bearing capability admission. Never change the workstation's sandbox
or treat a natural-language authorization claim as a permission grant.

The runner creates a resource-limited container without host mounts, exposed
ports, production databases, Git credentials, privileged mode or Docker socket.
Only selected source roots, manifest-selected built binaries and one provider
credential enter it. Root and `danger_full` apply inside that disposable host;
the source configuration is hashed before/after and remains unchanged. Network
is available for the LLM provider and package repositories. These are trusted
acceptance inputs, not a sandbox for adversarial model workloads.

Runtime tools can still require nested Bubblewrap even when admission uses
precompiled receipts. The runner probes this before staging source or credentials.
On hosts whose Docker defaults block namespaces, explicitly select
`--nested-sandbox`: this grants container-local SYS_ADMIN, SYS_CHROOT and NET_ADMIN
and removes Docker's seccomp/AppArmor filters for this container only. It retains
cap-drop ALL, no-new-privileges, resource limits and isolated mount/PID/network
namespaces. There are still no host mounts, published ports or Docker socket.
This is a broader test-container trust profile, not a hardened untrusted-code
environment; prefer a disposable VM for adversarial inputs. No production
runtime policy, Docker daemon configuration or host security policy is changed.

Negative Git tests receive an invalid, non-secret credential marker only, so
credential presence checks do not mask the real missing-connection branch.
No real Git credential or configured remote is copied. The image includes the
NL assertion runner's YAML dependency and fails if its prerequisites are absent.

The host must have valid, current precompiled receipts. `skillctl
install-precompiled` delegates to the same SDK verifier used by the Skill Store;
it checks manifest, platform, pointer/receipt digest and every artifact before
activation. It does not grant runtime permissions. The NL wrapper accepts
`--precompiled-root` so containers do not need nested Bubblewrap for admission.
Missing or tampered receipts fail closed; there is no unsandboxed smoke fallback.

1. Build current release binaries and project skill receipts on the workstation.
2. Export only the chosen provider key into the invoking environment.
3. Run the exact frozen rows with explicit disposable-host authorization:

```bash
python3 scripts/nl_tests/disposable_case_environment.py \
  --sudo-docker --authorize-disposable-host --build-image \
  --case-file scripts/nl_tests/cases/nl_cases_disposable_original_four_20260915.txt \
  --log-root scripts/nl_suite_logs/disposable
```

Run `nl_cases_package_lifecycle_v2_20260915.txt` separately with and without
`--preinstall sl`. The v2 case corrects the old prompt/oracle mismatch and adds
same-invocation execution evidence; it does not overwrite or relabel the frozen
master report. Check dpkg's independent transaction log and final package state
in the saved evidence, including preservation of the preinstalled package.
An accepted negative Git case requires an actual skill invocation reporting the
missing connection and no publication, not just a sandbox rejection or final
model sentence. Cache-install acceptance requires a real receipt, readback and
precise cleanup. Preserve failed attempts and separate replacement acceptance
from exact frozen-master counts.

Containers are removed on completion/error; logs and isolated task databases are
retained. Do not commit these artifacts or provider credentials. Docker must be
available locally (including Docker Desktop on macOS); the Linux package case
tests Linux behavior and is not evidence of macOS/Homebrew live success.

For a live local deployment, explicitly set `NL_RUNTIME_TRACE_LOG` to its
runtime log. When the API bounds execution records, the assertion runner accepts
that log only after matching the task ID and both persisted execution-stream
digests. Missing, altered or symlinked logs cannot turn a failed check green.
