## schedule — schedule semantic compiler

Compile natural-language scheduling requests into structured schedule plans.

## Capability
- Converts user schedule requests into a normalized JSON plan.
- Used by clawd schedule service as semantic/planning layer.
- Does not execute jobs directly.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `action` | yes | string | - | Must be `compile`. |
| `text` | yes | string | - | User natural-language schedule request. |

## Output
- JSON string with fields matching `ScheduleIntentOutput`:
  - `kind`, `timezone`, `schedule`, `task`, `target_job_id`, `confidence`
- If request is not a schedule intent, return an error.
