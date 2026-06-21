# Raspberry Pi aarch64 Release Package

RustClaw can publish a prebuilt Raspberry Pi package through GitHub Actions so
the Pi does not need to run `cargo build`.

## Build And Publish

1. Open GitHub Actions.
2. Run `Build Pi aarch64 Release`.
3. Optional: set a release tag such as `pi-aarch64-20260621`.
4. Keep `prerelease` enabled for test packages.

The workflow builds:

- Rust workspace binaries for `aarch64-unknown-linux-gnu`
- `UI/dist`
- a runtime archive named `RustClaw-pi-aarch64-<tag>.tar.gz`
  (`RustClaw-<tag>.tar.gz` when the tag already starts with `pi-aarch64-`)
- a matching `.sha256` checksum file

The archive is uploaded both as a workflow artifact and as a GitHub Release
asset.

## Pi Update Notes

On the Pi, keep runtime state from the existing install:

- `configs/`
- `data/`
- `logs/`
- `.pids/`

Use the package as a source of updated binaries, scripts, prompts, migrations,
and `UI/dist`. Do not overwrite live secrets or channel settings with packaged
defaults.

After replacing files, restart the needed services. For a backend-only update,
restart `clawd`; for a full runtime update, restart the selected channel
adapters too.
