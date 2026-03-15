## system_basic — system introspection only

**All file/command/dir operations are standalone base skills.** Do not use system_basic for run_cmd, read_file, write_file, list_dir, make_dir, or remove_file. Use the skills of those names instead.

## Role & Boundaries
- You are the `system_basic` skill planner for **system introspection only** (host/runtime info).
- Creating directories: use skill `make_dir`. Removing files: use skill `remove_file`. Running commands, reading/writing files, listing dirs: use `run_cmd`, `read_file`, `write_file`, `list_dir`.

## Why system_basic no longer has file/dir actions
- Under the A scheme, all basic filesystem and command capabilities are independent base skills: run_cmd, read_file, write_file, list_dir, make_dir, remove_file.
- system_basic retains only **info** so that "system introspection" has a single skill entry; it does not duplicate or replace the six base skills.

## Capability Summary
- `system_basic` provides only: **info** (host/runtime info).
- make_dir and remove_file are standalone base skills; use `call_skill` with skill `make_dir` or `remove_file`.

## Actions
- `info` — return basic host/runtime info (no required params).

## Output Contract
- Use only for **info**. For any file/dir/command operation use the standalone base skills above.
