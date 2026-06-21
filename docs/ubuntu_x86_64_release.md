# Ubuntu x86_64 Release Package

RustClaw can publish a prebuilt Ubuntu x86_64 runtime package through GitHub
Actions. This package is for regular 64-bit Ubuntu servers and PCs.

## Build And Publish

1. Open GitHub Actions.
2. Run `Build Ubuntu x86_64 Release`.
3. Optional: set a release tag such as `ubuntu-x86_64-20260621`.
4. Keep `prerelease` enabled for test packages.

The workflow builds:

- Rust workspace binaries for `x86_64-unknown-linux-gnu`
- `UI/dist`
- a runtime archive named `RustClaw-ubuntu-x86_64-<tag>.tar.gz`
  (`RustClaw-<tag>.tar.gz` when the tag already starts with `ubuntu-x86_64-`)
- a matching `.sha256` checksum file

The archive is uploaded both as a workflow artifact and as a GitHub Release
asset.

## Update Notes

When updating an existing install, keep runtime state:

- `configs/`
- `data/`
- `logs/`
- `.pids/`

Use the package as a source of updated binaries, scripts, prompts, migrations,
and `UI/dist`. Do not overwrite live secrets or channel settings with packaged
defaults.
