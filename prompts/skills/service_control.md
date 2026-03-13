# service_control

## Purpose

Use this skill when the user asks to inspect, control, verify, or troubleshoot a named service, daemon, or service-like runtime target.

This skill is specialized for service lifecycle operations and service-level diagnosis.

Typical responsibilities include:

- checking service status
- starting a service
- stopping a service
- restarting a service
- reloading a service
- checking whether a service recovered
- viewing recent service logs
- diagnosing why a service failed to start or stay healthy

This is not a general shell skill, not a code editing skill, not a package installation skill, and not a deployment orchestration skill.

## Primary Goal

Help the user operate a service safely and report the real observed result with evidence.

Prefer verifiable operational facts over assumptions.

## Service Manager Awareness

A target may belong to different service managers or runtime forms.

Possible manager types include:

- `systemd`
- `service`
- `docker_compose`
- `docker_container`
- `supervisor`
- `process_only`
- `unknown`

Always prefer the control model that best matches the target.

Examples:

- `nginx`, `redis`, `sshd` are often `systemd` or `service`
- `web`, `api`, `db` may be `docker compose` services
- a named container may be `docker_container`
- some app processes may only support process-level inspection and not true service lifecycle control

Do not assume every target is a `systemd` service.
Do not treat a container name as a host service name unless evidence supports it.

If manager type is unclear, prefer low-risk inspection first.

## Read-Only First Rule

If the user did not explicitly request a state-changing action, default to read-only diagnosis.

Read-only actions include:

- `status`
- `logs`
- `verify`

Do not proactively `start`, `stop`, `restart`, or `reload` a target unless the user explicitly requested a change or the workflow clearly requires it.

Examples:

- "看看 nginx 在不在" -> read-only
- "查下为什么没起来" -> read-only diagnosis
- "看日志" -> read-only
- "恢复了吗" -> read-only verify
- "重启一下" -> state-changing action is explicitly requested

## Risk Levels

Treat actions with different risk levels.

### Low Risk

- `status`
- `logs`
- `verify`

### Medium Risk

- `start`
- `reload`

### High Risk

- `restart`
- `stop`

Rules:

- Low-risk actions may be used freely when they match user intent.
- Medium-risk actions should match clear user intent or a very obvious recovery workflow.
- High-risk actions require explicit user intent.
- Do not perform `stop` or `restart` on a critical or unspecified service without a clear request.
- Do not chain repeated high-risk actions automatically.

## Supported Actions

Map requests to one of these actions when possible:

- `status`
- `start`
- `stop`
- `restart`
- `reload`
- `logs`
- `verify`
- `diagnose_start_failure` (status + logs, for "why did start fail")
- `diagnose_unhealthy_state` (status + logs, for "why is it unhealthy")

If the underlying runtime only supports a subset, choose the closest valid action and keep behavior predictable.

**Implementation note**: The skill returns **structured JSON** (status, service_name, manager_type, requested_action, executed_actions, pre_state, post_state, verified, key_evidence, failure_reason, next_step, summary). Implemented managers: **rustclaw** (RustClaw daemons via clawd API), **systemd**, **service**. docker_compose / docker_container / supervisor return "not implemented". After start/restart/reload the skill **auto-runs verify**; on failure it **auto-fetches recent logs**. High-risk actions (stop/restart) are **refused for ambiguous targets** (e.g. "后端", "服务们") unless explicitly allowed. Use `target` or `service` for the service name; optional `manager_type`, `tail_lines`, `verify`, `allow_risky`.

## Input Understanding Rules

Try to identify:

- `service_name`
- `action`
- optional `manager_type`
- optional `tail_lines`
- optional `since`
- optional `environment`
- optional `follow = false`

Do not invent missing critical inputs.

If the service target is missing, vague, or ambiguous, do not guess.
Return a short bounded response indicating the service name or target is unclear.

Examples of ambiguity:

- "把服务拉起来"
- "重启后端"
- "看看服务状态"
- "把几个服务都重启一下"

## Chinese Intent Interpretation

Interpret common Chinese expressions carefully.

### Likely `status`

- "在不在"
- "起了没"
- "有没有起来"
- "还活着吗"
- "正常吗"
- "是不是好的"
- "看看服务状态"

### Likely `start`

- "拉起来"
- "启动一下"
- "开起来"

### Likely `stop`

- "停掉"
- "关掉"
- "停一下"

### Likely `restart`

- "重启一下"
- "重开一下"
- "重来一下"
- "刷一下服务"

### Likely `reload`

- "重载一下"
- "reload 一下"

### Likely `logs`

- "看日志"
- "查日志"
- "看报错"
- "最近日志"
- "最近 100 行"

### Likely diagnosis flow

- "没起来"
- "挂了"
- "起不来"
- "启动失败"
- "看是不是假死"

### Likely `verify`

- "恢复了吗"
- "现在好了吗"
- "确认一下正常没"
- "确认服务生效没"

When the user says "看看" without a specific action, prefer `status` first.

## Execution Strategy

Prefer a structured workflow instead of single-step guessing.

### 1. Status-style requests

For requests asking whether the service is running, healthy, present, or recovered:

- use `status`
- use `verify` if the runtime supports a stronger post-check concept
- return the observed state and a short supporting detail

Examples:

- "看看 nginx 在不在"
- "redis 起了吗"
- "rustclaw 现在正常吗"

### 2. Start requests

If the user asks to start a service:

1. identify target
2. inspect current status first if feasible
3. run `start`
4. verify actual state afterward
5. if verification fails, inspect recent logs

Do not claim success based only on command acceptance.

### 3. Stop requests

If the user asks to stop a service:

1. identify target
2. run `stop`
3. verify it stopped if feasible
4. return the observed final state

### 4. Restart requests

If the user asks to restart:

1. identify target
2. inspect status first if current state is unknown
3. run `restart`
4. verify afterward
5. if verification fails, inspect logs
6. do not retry restart in a loop

### 5. Reload requests

If the user asks to reload:

1. identify target
2. inspect state if useful
3. run `reload`
4. verify service health after reload
5. inspect logs if reload failed or service became unhealthy

### 6. Diagnosis requests

If the user says the service is down, failed, unhealthy, crashed, or cannot start:

1. inspect `status`
2. inspect recent `logs`
3. summarize the observed issue
4. state uncertainty explicitly if evidence is incomplete
5. do not fabricate a root cause

### 7. Log-only requests

If the user asks for logs:

- use bounded log inspection
- prefer last 50 to 200 lines unless the user specified another value
- summarize key errors, warnings, and state transitions
- do not dump excessive raw logs by default

## Pre-Check Rules

Before medium-risk or high-risk actions, prefer these checks when feasible:

- current service state
- whether the service is already in the requested state
- whether the target appears unstable or repeatedly failing
- whether recent logs already show a clear blocking issue

Examples:

- before `restart`, check whether the service is already failed or flapping
- before `start`, check whether it is already running
- before `reload`, check whether the service exists and is active

## Post-Check Rules

After `start`, `restart`, `reload`, and when feasible after `stop`, perform a post-check.

Post-checks may include:

- service state
- active/running flag
- failed/degraded flag
- recent log signals
- whether the main process stayed up
- whether the target immediately exited again

Do not report success if the target immediately returned to failed, stopped, or unhealthy state.

## Failure Escalation Rules

If a state-changing action fails:

1. return the direct failure reason if available
2. inspect recent logs when feasible
3. summarize the most relevant evidence
4. avoid guessing beyond observed evidence
5. if evidence is inconclusive, say so plainly

Useful categories to mention when supported by evidence:

- port already in use
- config parse error
- missing file
- missing environment variable
- permission denied
- dependency service unavailable
- binary missing
- crash after start
- health check failed

Do not present these as facts unless supported by output or logs.

## Batch and Ambiguity Guardrails

Be extra cautious with vague or broad requests.

Rules:

- Do not execute destructive or high-risk operations against multiple services unless the request clearly names them.
- Do not treat group labels like "后端", "服务们", "那几个", "生产服务" as sufficient precision for restart/stop.
- Do not use wildcards or broad matching for stop/restart.
- If multiple services match, prefer a bounded clarification-style response instead of guessing.

Examples that should not trigger direct execution:

- "把服务都重启一下"
- "重启后端"
- "把那几个停掉"
- "看下服务"

## Output Contract

Return concise but structured operational results.

Whenever possible, the result should internally preserve these fields, even if the final user-facing text is natural language:

- `service_name`
- `manager_type`
- `requested_action`
- `executed_actions`
- `pre_state`
- `post_state`
- `verified`
- `key_evidence`
- `failure_reason`
- `next_step`

## User-Facing Output Style

Prefer short operational summaries.

Good success examples:

- "Service `nginx` is running."
- "Restarted `rustclaw` successfully. Verification passed."
- "Started `redis` successfully. The service is now active."

Good failure examples:

- "Failed to restart `clawd`. The service did not become active after restart."
- "Failed to start `redis`. Recent logs show port 6379 is already in use."
- "Service `nginx` is stopped. Logs indicate a configuration error."

Good ambiguity examples:

- "The request implies a service action, but the target service is unclear."
- "I can inspect the service, but no specific service name was identified."

## Evidence Rules

Prefer evidence from:

- actual status output
- post-action verification
- recent logs
- manager-specific observed state

Do not:

- claim recovery without verification
- claim root cause without supporting evidence
- summarize a failure as success just because the control command returned
- hide uncertainty when evidence is partial

## Scope Boundaries

Use this skill only for service lifecycle and closely related service logs.

Do not silently shift into:

- source code edits
- config rewrites
- dependency installation
- infrastructure provisioning
- deployment rollout logic
- unrelated shell diagnosis

If the service issue appears to come from code, config, or environment problems, report that clearly and stop at the service evidence boundary.

## Coordination With Other Skills

This skill should usually be the first choice when the user asks about a named service's state or lifecycle.

If service evidence suggests another domain, report the evidence so another skill can take over later.

Examples:

- config syntax invalid -> config or code editing skill may be needed
- binary missing -> build or deploy workflow may be needed
- package not installed -> package management skill may be needed
- no service exists but process-level behavior matters -> process skill may be needed

## Do Not

- Do not invent service names.
- Do not invent manager types.
- Do not guess root cause without evidence.
- Do not overuse restart as a default fix.
- Do not stop or restart unspecified critical services.
- Do not retry the same failed high-risk action multiple times.
- Do not flood the user with full logs unless explicitly requested.
- Do not report success before verification.

## High-Quality Behavior Examples

### Example 1

User: "重启一下 nginx"

Expected behavior:

- identify `nginx`
- likely manager `systemd` or `service`
- inspect status if helpful
- run restart
- verify service is active afterward
- inspect logs if verification fails
- return concise result

### Example 2

User: "看看 rustclaw 在不在跑"

Expected behavior:

- map to `status`
- use read-only inspection
- return observed service state
- avoid unnecessary restart or logs unless needed

### Example 3

User: "clawd 没起来，查一下"

Expected behavior:

- use diagnosis flow
- inspect status
- inspect recent logs
- summarize concrete observed failure
- avoid guessing beyond evidence

### Example 4

User: "看 redis 最近 100 行日志"

Expected behavior:

- identify `redis`
- run bounded log inspection with 100 lines
- summarize key errors or warnings first
- avoid dumping excessive raw output unless requested

### Example 5

User: "把服务拉起来"

Expected behavior:

- infer likely `start`
- detect missing service target
- do not invent a service name
- return a short bounded explanation that the target is unclear

### Example 6

User: "重启后端"

Expected behavior:

- detect vague target
- recognize this is a potentially high-risk batch-style request
- do not guess which service to restart
- return a short bounded response indicating the target service list is unclear
