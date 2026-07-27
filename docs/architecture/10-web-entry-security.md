# Web Entry And Core Isolation

<!-- ai-learning-stage: safety-operations -->
<!-- ai-learning-audience: operator -->

<!-- ai-learning-navigation:start -->
Previous: [Interactive coding and presentation](09-interactive-coding.md) |
[Architecture index](README.md)

<!-- ai-learning-navigation:end -->

RustClaw keeps browser-facing security in `webd` and keeps `clawd` as an
internal core API. This prevents a second public UI/API entry from bypassing
the browser session gateway.

## Current Access Topology

```mermaid
flowchart LR
    B[Browser]
    N[nginx<br/>optional TLS + static UI]
    W[webd :8788<br/>UI + login + session + proxy]
    C[clawd 127.0.0.1:8787<br/>internal /v1 API only]
    U[UI/dist]
    D[Local channel daemons / clawcli]

    B -->|local deployment| W
    B -->|domain or TLS| N
    N -->|static files| U
    N -->|/v1 and /webd| W
    W -->|static files without nginx| U
    W -->|authenticated /v1| C
    D -->|local authenticated API| C
```

`clawd` does not serve `UI/dist` and cannot bind a non-loopback address. Its
address is not a user configuration field. The internal test override accepts
only loopback socket addresses, so parallel local test servers cannot reopen
the public interface.

## Ownership

- `webd` owns browser login, persisted sessions, credential injection,
  login-failure lockout, body limits, forwarded-client handling, and API proxying.
- `clawd` owns authenticated task and administration APIs, agent execution,
  tools, skills, and persistence.
- nginx is optional. It owns TLS/domain termination and static asset delivery,
  then forwards `/v1` and `/webd` to `webd`, never to `clawd`.
- Local channel daemons and `clawcli` may call the loopback core API directly;
  this is machine-local component communication, not a browser entry.

## Dashboard Operations

The dashboard reports whether nginx is installed, running, configured for
RustClaw, and serving a deployed UI. Admin-only actions use the existing
background workspace-operation state:

- **Enable/Repair nginx** installs or starts nginx, writes the RustClaw site,
  deploys existing `UI/dist`, validates configuration, and starts or reloads it.
- **Build and Deploy Latest UI** builds current UI source in a source checkout,
  or uses packaged UI assets in a Release installation, then deploys to nginx.
- **Open nginx entry** appears only when process, site, and UI checks all pass.

Operations return machine status and error keys. Human-facing Chinese or
English guidance is rendered by the UI rather than parsed by runtime logic.

## Deployment Rules

- Native Linux/macOS: `clawd` is loopback-only; `webd` is the browser entry.
- Docker: publish `8788`, not `8787`; `webd` and `clawd` communicate through
  loopback inside the same container.
- Local use does not require nginx. Open `webd` directly.
- Server/domain use should place nginx in front of `webd` and use trusted TLS.
- Never expose or port-forward `8787`.

## Verification

```bash
cargo test -p webd
cargo test -p clawd internal_listener_tests::
cargo test -p clawd workspace_nginx_tests::
python3 scripts/check_long_files.py
```

The webd tests prove that UI assets are served locally while `/v1` remains a
proxy path. The clawd listener test rejects wildcard and LAN addresses.
