# Release Installation for Users and Deployment Agents

[简体中文](release_installation.zh-CN.md)

Install and update from signed GitHub Release packages. Do not compile on the
destination, install a Rust toolchain, or fall back to a source build when an
asset is missing. Source development is a separate, explicitly requested
[developer workflow](developer_build.md).

## Select a Package

Read the publisher and artifact prefix from `configs/product_identity.toml`.
Select the newest non-draft, non-prerelease **for the destination platform**;
GitHub's global latest release can belong to a different platform.

| Destination | Release tag prefix | Package target |
| --- | --- | --- |
| Linux x86_64 | `ubuntu-x86_64-` | `x86_64-unknown-linux-gnu` |
| Linux aarch64 / 64-bit Raspberry Pi | `pi-aarch64-` | `aarch64-unknown-linux-gnu` |
| macOS Intel | `macos-x86_64-` | `x86_64-apple-darwin` |
| macOS Apple Silicon | `macos-aarch64-` | `aarch64-apple-darwin` |

Check `uname -s`, `uname -m`, free disk space, and the release's operating-system
requirements. A 32-bit Pi OS cannot run the aarch64 package. There is no implicit
cross-platform or source-build fallback. Install Bash, Python 3.11+, curl, tar,
and OpenSSH's `ssh-keygen`; individual skills can require additional runtimes.

The publisher stages optional chip libraries against the selected target using
the same native-header verifier as installation. Matching libraries retain their
contents; recognized libraries for other architectures are excluded, while invalid
headers or unsafe aliases stop packaging. This does not remove source libraries
or install a substitute on the device. A system/configured native library can
still be used; without one, chip operations report the existing unavailable
status and non-hardware features remain available.

## First Installation

Download `install-latest-release.sh` from the trusted release repository, then run
the following command with the repository from `configs/product_identity.toml`:

```bash
bash install-latest-release.sh --repo OWNER/REPO
```

This installs the newest matching signed Release into `~/agent-runtime` and starts
it. It also registers automatic startup through systemd on Linux/Raspberry Pi or
a per-user LaunchAgent on macOS. No Git checkout, Rust toolchain, source build,
or new nginx site is needed.
Use `--root DIR` for another location, `--check-only` to check without installing,
or `--no-start` to install without starting services or registering automatic
startup. Bootstrap files are fetched
over HTTPS from one pinned commit in the explicitly trusted repository; verify
the repository and script origin first. Existing installations keep their trusted
release signer. The manual verification procedure follows.

Inspect or remove the generated startup registration with
`bash scripts/configure-autostart.sh --status` or
`bash scripts/configure-autostart.sh --disable`.

The Ubuntu x86_64 and macOS full Releases preinstall and enable every platform-compatible
bundled Skill Store package and companion AiAPP from the base registry without
compiling on the user's host. The signed package also carries the reviewed Python
and Node runtimes required by those skills, so users do not install Homebrew or
developer toolchains. The media package includes its pinned FunASR, CPU PyTorch,
SenseVoiceSmall, and FSMN VAD assets. Deployment initializes only
missing private skill directories and never overwrites existing media data,
browser sessions, or configuration. Reinstalling replaces release-managed programs
and bundled skill versions while preserving accounts, sessions, configuration,
credentials, external skills, and private skill data. Whisper.cpp remains an independent optional
local engine and its binary/model are not included. Third-party external skills
still require host admission and a policy grant. Raspberry Pi and macOS use their
own platform packages and cannot reuse another architecture or operating system's
Python/FunASR runtime.

1. Download the matching archive and its `.sha256`, `.spdx.json`,
   `.manifest.json`, and `.manifest.json.sig` assets from the same release.
2. Obtain `configs/release_allowed_signers` and
   `scripts/security/release_manifest.py` from an independently trusted publisher
   source. Do not establish trust using a key supplied only by the unverified
   archive. Verify before extracting or executing its scripts:

```bash
archive=/absolute/path/to/downloaded.tar.gz
trusted=/absolute/path/to/trusted-source
target=x86_64-unknown-linux-gnu # select from the table
source "$trusted/scripts/product_identity.sh"
ssh-keygen -Y verify -f "$trusted/configs/release_allowed_signers" \
  -I release -n agent-runtime-release \
  -s "$archive.manifest.json.sig" < "$archive.manifest.json"
python3 "$trusted/scripts/security/release_manifest.py" verify \
  --artifact "$archive" --sbom "$archive.spdx.json" \
  --manifest "$archive.manifest.json" --expected-target "$target" \
  --expected-package-root "$APP_RELEASE_ARTIFACT_ID"
```

3. Stop if either verification fails. Extract into a new, user-owned installation
   directory, never directly over an existing installation. Enter the extracted
   package and run:

```bash
bash install-agent-cmd.sh --user --no-deploy-ui
export APP_RUNTIME_ENV_SCRIPT=/absolute/path/runtime_env_filled.sh
agentctl start -q
agentctl -status
agentctl -health
```

The installer validates native binaries and bundled UI assets; it does not
build missing files.

In Model Settings, select a provider and enter its API key. Save writes the key
to `.agent-runtime/credentials/models.env` with directory mode `0700` and file
mode `0600` on Linux/macOS. Keys are not returned to the browser or written to
`config.toml`; an empty key field preserves the existing credential. Test
Connection uses the draft key without saving it.

The launchers load the external environment script (`APP_RUNTIME_ENV_SCRIPT`,
default `$HOME/runtime_env_filled.sh`) and then this managed model environment
file on every startup. UI-saved keys override that provider's external key;
providers without a saved override keep their existing credential source.
The managed file contains literal `NAME=value` assignments, parsed without
shell execution; do not `source` it yourself. It is plaintext protected by file
permissions, so include it only in private backups, never in Git or Release
packages. Runtime upgrades preserve it. Hosted relay credentials remain
device-managed and do not require manual entry. Use HTTPS for remote key entry.
Open webd on its configured port (normally `http://127.0.0.1:8788`). Local use
does not need nginx. Configure channels and third-party skills through the UI.
Bundled skills receive their release-reviewed grants during installation; external
skills do not receive permissions or become enabled merely by being copied locally.

On the first start with an empty database, the web login is `admin` with the
initial password `654321`. Change it immediately in account management to a
strong password of at least 12 bytes before exposing the service externally.
Updates and restarts preserve existing accounts, passwords, and disabled states.
Backfilling a login for an existing admin key and factory resets still generate
random passwords. Bootstrap credentials are recorded in
`data/bootstrap-credentials.txt` under the installation directory; store them
safely, then delete that credentials file.

## Update an Existing Installation

Use the UI's Release update or the existing trusted deployment script:

```bash
bash deploy-github-release.sh --check-only
bash deploy-github-release.sh --restart
```

The script selects the platform, verifies signature/checksum/manifest/SBOM and
native binaries, preserves local configuration and runtime data, and restarts
the runtime. Existing nginx deployments receive the bundled UI; local installs
do not create an nginx deployment. Use `--tag TAG` to pin a specific compatible
release. Do not run `git pull`, `cargo build`, `npm run build`, or
`install-agent-cmd.sh --build` as an installation fallback.

Before updating, confirm backups and enough space for the download, staging
directory, and rollback copy. Afterwards verify health, UI, configured channels,
installed skills, and the displayed release version. Keep configuration, wallet
keys, databases, skill data, and the newest rollback copy. Do not clear caches
or business data just to make an installation pass.

Operator NL records under `scripts/nl_suite_logs` are preserved during updates,
including an existing link to a retained data directory, and excluded from
distributable scripts. Keep these records private. Staging needs both free bytes
and sufficient per-user quota: `release_staging_quota_exceeded` and
`release_staging_space_exhausted` stop extraction before runtime replacement.
Choose a user-owned native filesystem with enough staging space through `TMPDIR`;
do not remove business data or weaken signature checks to bypass these errors.
Rollback roots and per-update directories are owner-private (`0700`); compressed
rollback files are created as `0600`. A symlinked rollback root is rejected before
runtime replacement, and each update receives a unique directory.

A developer Git checkout must be clean in release-managed paths and match the
signed package's source commit. Align it through the separate developer workflow
before deployment; the installer does not reset, merge, or discard source edits.
Tracked development files omitted from a runtime package are preserved. Release-only
installations do not require Git. Device-local screen settings are preserved;
existing chip libraries are retained when the signed package does not replace them.

## Publisher Retention

Every platform Release includes all compatible optional Skill Store packages.
UI installation uses verified precompiled native skills, Node packages with
lifecycle scripts disabled, and Python wheels. Dependencies lacking upstream
wheels are built on matching native publisher runners and included with pinned hashes.
Target devices do not run Cargo, Go, native dependency builds, or npm lifecycle
build scripts. Missing/tampered/incompatible native packages fail explicitly;
the UI never falls back to compiling. Python 3.13 and 3.14 wheels are bundled;
other interpreters require a compatible dependency bundle, never a source build.
Host dependencies, browser binaries and model weights may still need a network
download. Optional packages remain uninstalled/disabled until explicitly admitted.

Publish and validate the replacement before deleting older releases. Retain
one stable release per supported platform, not one global release. In-progress
tags and other platforms are not cleanup candidates.
