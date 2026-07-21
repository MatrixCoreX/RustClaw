# Cross-Platform Runtime Contract

RustClaw shared production code targets Linux and macOS. Platform-specific
capabilities remain explicit: they use a platform adapter, return a structured
unsupported result, or are excluded with `cfg`. A missing dependency never
authorizes a less restrictive fallback.

## Process sandbox

`tools.sandbox_backend = "auto"` resolves to Bubblewrap on Linux and Seatbelt
(`/usr/bin/sandbox-exec`) on macOS. Both local backends fail closed when their
executable is unavailable. `danger_full` is the only direct process mode and
must be selected explicitly. `remote_container` is a contract placeholder and
returns `sandbox_remote_backend_not_configured` until a remote executor is
configured; it is not an implicit fallback.

Sandbox diagnostics expose the requested and resolved backend, availability,
fail-closed state, reason code, platform, and controls for filesystem,
network, process, credentials, resources, and environment. These fields are
machine contracts, not localized user replies.

## Platform services and tools

- Linux service discovery and lifecycle operations use systemd or SysV only
  through the Linux platform adapter.
- macOS service discovery uses Homebrew services, launchd, or process
  observation. A requested Linux manager returns `unsupported_platform`
  without launching a Linux command.
- Package, system-health, process, and filesystem skills either use a native
  macOS implementation or return structured unavailable data for an
  unsupported measurement.
- Long-running command jobs use GNU `timeout`, Homebrew `gtimeout`, or a
  Python process-group watchdog. If none is available, execution fails closed
  instead of running without a deadline.

## Development scripts

Scripts use `scripts/shell_compat.sh` for timeouts, file metadata, host/target
detection, and low-memory build settings. Release and NL scripts must avoid
GNU-only `stat -c`, `date -d`, `find -printf`, and Bash 4-only arrays or case
conversion so the default macOS shell remains usable.

Run the permanent guard after modifying platform-sensitive code:

```bash
python3 scripts/check_cross_platform_contracts.py --self-test
python3 scripts/check_cross_platform_contracts.py
```

On a macOS host, run the native workspace tests. On another host, an Apple
target check may be attempted when the target and Darwin C toolchain/SDK are
installed. A Rust target alone is insufficient for crates with native C or
assembly dependencies such as `ring`; release evidence must distinguish a
source failure from an unavailable cross compiler or SDK.
