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

## First Installation

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
build missing files. Keep model credentials outside the installation directory.
Open webd on its configured port (normally `http://127.0.0.1:8788`). Local use
does not need nginx. Configure channels and optional skills through the UI.
Installation alone does not grant a skill permissions or enable it.

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

## Publisher Retention

Publish and validate the replacement before deleting older releases. Retain
one stable release per supported platform, not one global release. In-progress
tags and other platforms are not cleanup candidates.
