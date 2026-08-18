## schedule — schedule semantic compiler

Compile natural-language scheduling requests into structured schedule plans.

## Capability
- Converts user schedule requests into a normalized JSON plan.
- Used by clawd schedule service as the workflow owner for scheduled jobs.
- Create/list/delete/pause/resume actions persist through Agent Runtime `scheduled_jobs`; do not use shell, crontab, systemd timers, or ad-hoc command scheduling for ordinary reminders.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `action` | yes | string | - | One of `compile`, `create`, `create_structured`, `preview`, `list`, `delete`, `delete_matching`, `pause`, `resume`. |
| `text` | for natural-language compile/create/preview unless `intent` is complete | string | - | Original user schedule request. |
| `intent` | no | object | - | Complete `ScheduleIntentOutput` when already available. |
| `intent_json` | for create_structured | string | - | Exact serialized machine intent copied from a current capability observation. |

## Planner capabilities
- `schedule.create_structured`: create a scheduled job from an observed `intent_json` string. Copy the string unchanged in the parent loop; do not reconstruct nested values, turn it back into natural-language intent, or delegate this mutating call.
- `schedule.create`: create a scheduled job. Prefer args `{ "text": "<original user request>" }` unless a complete `intent` object is already available.
- `schedule.preview`: parse or preview a schedule without mutating state. Call this capability with only
  `{ "text": "<original user request>" }`; runtime forces `compile_only` / `dry_run`, so do not add
  `action`, `dry_run`, or `preview_only` to capability args.
- `schedule.list`: list scheduled jobs.
- `schedule.delete`: delete scheduled jobs; use `target_job_id` when the user names one.
- `schedule.delete_matching`: delete only jobs matching structured task ownership fields. Pass `match_task_kind`, `match_skill_name`, `match_task_action`, and `match_platforms`; never derive these fields from localized prose. A shared multi-platform job is retained unless every platform in that job is included in `match_platforms`.
- `schedule.pause`: pause scheduled jobs; use `target_job_id` when the user names one.
- `schedule.resume`: resume scheduled jobs; use `target_job_id` when the user names one.

The builtin `compile` action is an internal workflow protocol operation, not a
planner capability. Use `schedule.preview` for every user-requested parse-only
or dry-run schedule operation.

## Output
- JSON string with fields matching `ScheduleIntentOutput`:
  - `kind`, `timezone`, `schedule`, `task`, `target_job_id`, `confidence`
- Preview output exposes `dry_run=true`, `preview_only=true`, `would_mutate=false`, stable
  `datetime` (for a parsed one-time `run_at`), and `title` (the parsed task content), while
  preserving the canonical schedule/task fields.
- For an ordinary preview, set `requires_content_evidence=true` and let the
  model synthesize the answer from observed preview data.
- Only when the user explicitly requests the exact `datetime`, `timezone`, and
  `title` machine fields, set `response_shape="strict"` and
  `structured_field_selector="datetime,timezone,title"`. Return only observed
  values for those fields.
- If request is not a schedule intent, return an error. An incomplete create returns `schedule_needs_more_info` with `failure_phase=pre_dispatch`, `side_effect_applied=false`, and `recovery_action=replan_arguments`; correct it in one bounded retry.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
