## schedule — schedule semantic compiler

Compile natural-language scheduling requests into structured schedule plans.

## Capability
- Converts user schedule requests into a normalized JSON plan.
- Used by clawd schedule service as the workflow owner for scheduled jobs.
- Create/list/delete/pause/resume actions persist through RustClaw `scheduled_jobs`; do not use shell, crontab, systemd timers, or ad-hoc command scheduling for ordinary reminders.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `action` | yes | string | - | One of `compile`, `create`, `preview`, `list`, `delete`, `pause`, `resume`. |
| `text` | for compile/create/preview unless `intent` is complete | string | - | Original user schedule request. |
| `intent` | no | object | - | Complete `ScheduleIntentOutput` when already available. |

## Planner capabilities
- `schedule.create`: create a scheduled job. Prefer args `{ "text": "<original user request>" }` unless a complete `intent` object is already available.
- `schedule.preview`: parse or preview a schedule without mutating state. Runtime forces `compile_only` / `dry_run`.
- `schedule.list`: list scheduled jobs.
- `schedule.delete`: delete scheduled jobs; use `target_job_id` when the user names one.
- `schedule.pause`: pause scheduled jobs; use `target_job_id` when the user names one.
- `schedule.resume`: resume scheduled jobs; use `target_job_id` when the user names one.
- `schedule.compile`: only compile natural language into `ScheduleIntentOutput`.

## Output
- JSON string with fields matching `ScheduleIntentOutput`:
  - `kind`, `timezone`, `schedule`, `task`, `target_job_id`, `confidence`
- If request is not a schedule intent, return an error.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
