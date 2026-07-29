# RustClaw Polyglot Skill SDK Contract

New RustClaw skill packages use `skill.toml` schema version 2. Version 1 remains
a centralized read-only compatibility input and is normalized to v2 before a
new source installation is activated. Package/build/run metadata and a typed
capability/permission request live in the manifest. The request includes input
and output schemas, effect, execution mode, artifacts, evidence, timeout,
configuration entry points, and requested runtime resources. It is never a
grant: risk, auto-invocation, confirmation, actual credentials, and effective
permissions remain host-owned policy.

The supported build adapters are Cargo, Python, Node, Go, prebuilt native,
generic process, and typed `http_json`. Manifests contain typed fields only;
they never contain a shell command. Build network is denied by default,
dependencies are private to the package install root, and missing sandbox
support must fail closed.

Every local process uses `rustclaw-jsonl-v1`: one JSON request record on stdin
and exactly one JSON response record on stdout. Diagnostics belong on stderr.
The response must echo `request_id`; errors require `error_text` plus stable
`extra.error_code` and `extra.message_key`. Each record is limited to 1 MiB.

Successful installation writes an immutable v2 receipt containing manifest,
semantic-contract, source, lockfile and artifact digests, the exact platform
and adapter version, the protocol-smoke result, and trusted launch metadata.
Host admission separately binds that receipt to a policy-grant digest and
registry generation. Receipts contain credential references only, never secret
values. Activation atomically updates `current.json`, preserving
`previous.json` for bounded rollback. Business data is never stored in the
package receipt tree.

At execution time `SkillRuntimeResolver` verifies the current pointer,
receipt, manifest and every artifact, then produces `SkillLaunchSpec`. Planner
arguments cannot change the program, entrypoint, working directory,
environment, sandbox profile or receipt identity.

Machine schemas live under `docs/schemas/`. The `rustclaw-skill` binary emits
one JSON result for CI-friendly validation and receipt inspection.

## Package layout and canonical ownership

Every process package contains `skill.toml` and `INTERFACE.md`. The manifest
owns package version, implementation adapter, source/lockfile paths, supported
platforms, typed launch metadata, build-network policy, sandbox, storage, and
lifecycle files. The package requests planner capabilities and runtime
resources; the host registry/policy validates, narrows, and grants them. The
host exclusively owns risk, confirmation, auto-invocation, credential
resolution, aliases, and prompt admission. The registry references the
manifest through `package_manifest`.

```text
my_skill/
├── skill.toml
├── INTERFACE.md
├── README.md
├── source and lockfile selected by the adapter
└── tests/ (or the language's separate test directory)
```

Bundled core packages live under `crates/skills/`, on-demand bundled packages
under `optional_skills/`, and submitted packages under `external_skills/`.
Only repository-maintained core/bundled Cargo packages become root Cargo
workspace members. External Cargo packages remain standalone workspaces.

## CLI quick start

Build the SDK CLI once, then choose a language explicitly:

```bash
CARGO_BUILD_JOBS=1 cargo build -p rustclaw-skill-sdk --bin rustclaw-skill
target/debug/rustclaw-skill init python demo_skill external_skills/demo_skill --human
target/debug/rustclaw-skill validate external_skills/demo_skill/skill.toml --human
target/debug/rustclaw-skill build external_skills/demo_skill/skill.toml . data/skill-packages --human
target/debug/rustclaw-skill receipt-verify data/skill-packages demo_skill --human
```

`build`, `protocol-test`, and `install-local` run the same verified install
pipeline. Add `--network` only after an explicit review when the manifest says
`network = "approval_required"`. JSON is the default output; `--human` is a
concise developer view. Packaging jobs may add `--target <triple>`; cross-target
protocol smoke must have a supported emulator and otherwise fails closed.

## Language quick starts

- Rust: `init rust`; keep `Cargo.lock`, add the package to the workspace only
  when it is repository-bundled, and set exact Cargo package/binary identities.
- Python: `init python`; keep a declared `requirements.lock`. Installation
  creates a private virtual environment and never uses user/global site packages.
- Node: `init node`; commit `package-lock.json`. Installation uses the private
  package root. Dependency lifecycle scripts are disabled and rejected by
  every supported schema version.
- Go: `init go`; commit `go.mod` and `go.sum`. The adapter uses isolated module
  and build caches and emits one target-specific executable; it never performs
  a global `go install`.
- Prebuilt: `init prebuilt`; declare an exact OS/architecture artifact, SHA-256,
  optional size, archive type, and entrypoint. Selection must match exactly.
- Generic process: use a local, already-built JVM/.NET/native artifact with a
  typed launcher and argument vector. Arbitrary shell strings are forbidden.
- `http_json`: declare a credential-free HTTPS endpoint, build-network approval,
  and runtime network permission. Redirects are rejected; secrets come only
  through registry-scoped runtime capabilities.

All implementations read the same request fields (`request_id`, `args`,
`context`, `user_id`, `chat_id`) and return one response record. A failure must
use `status="error"`, readable `error_text`, and stable
`extra.error_code`/`extra.message_key`.

## Publishing and registration

1. Complete `INTERFACE.md` with actions, typed parameters, errors, and examples.
2. Run manifest validation and the adapter build/protocol smoke.
3. Run `python3 scripts/check_polyglot_skill_contracts.py --require-all` and,
   after prompt changes, `python3 scripts/check_skill_prompts.py`.
4. Register `package_manifest`, planner metadata, storage/config ownership, and
   the generated prompt. Do not add a per-skill branch to `clawd`.
5. Treat compile/protocol success as admissible evidence only. Enable after the
   verified receipt, explicit host grant, and registry-generation activation
   all succeed. External/imported packages cannot infer entrypoints from file
   extensions or self-grant permissions.

## Lifecycle and diagnostics

Skill Store operations are durable and expose queued, preflight, dependency,
build, smoke, activate, configure, success/failure/cancel stages. Disable keeps
the installed package and owned data. Uninstall removes only the selected
versioned runtime/receipts, preserving configuration and private data by
default. Update activates a new verified directory atomically; rollback first
verifies the previous pointer, receipt, manifest, and every artifact digest.
Uninstall never removes shared Rust/Cargo, Python, Node, Go, JVM/.NET runtimes,
or reusable build caches. Toolchain cleanup is a separate administrator action.

Beginner-facing errors use stable phase/code/message keys. Redacted bounded
diagnostics are secondary details. Never copy credentials, raw provider
responses, environment dumps, or hidden reasoning into manifests, receipts,
operation records, or protocol fixtures.

## Platforms

Shared code supports Linux and macOS. Native and prebuilt packages declare
both OS and architecture. Python, Node, and generic-process cross-target builds
fail with a structured unsupported result; Go uses explicit GOOS/GOARCH with
CGO disabled; Cargo and prebuilt use their declared target/artifact rules.
Low-memory hosts and Raspberry Pi builds serialize heavyweight work by default.
Missing sandbox, emulator, runtime, or toolchain fails closed rather than
launching without isolation.

## Reference conformance

`crates/skill-sdk/tests/reference/` contains equivalent Cargo, Python, Node,
Go, and prebuilt fixtures. They prove the same calculation, structured error,
artifact, waiting, needs-user, timeout, malformed/multiple/oversized stdout,
and stderr-only diagnostic behavior. The test also performs an atomic update,
verified rollback, failed-update preservation, build-network denial, and
source-tree mutation check for every available adapter. Ubuntu and macOS CI set
`RUSTCLAW_REQUIRE_REFERENCE_ADAPTERS=1`, so all five adapters and the required
sandbox must run instead of being skipped for a missing toolchain.
