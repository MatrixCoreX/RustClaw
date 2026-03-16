#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CARGO_TOML = REPO_ROOT / "Cargo.toml"
REGISTRY_TOML = REPO_ROOT / "configs" / "skills_registry.toml"
SKILLS_DIR = REPO_ROOT / "crates" / "skills"


MAIN_RS_TEMPLATE = """use std::io::{{self, BufRead, Write}};

use serde::{{Deserialize, Serialize}};
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct Req {{
    request_id: String,
    args: Value,
}}

#[derive(Debug, Serialize)]
struct Resp {{
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
}}

fn main() -> anyhow::Result<()> {{
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {{
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {{
            Ok(req) => match execute(req.args) {{
                Ok(text) => Resp {{
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    error_text: None,
                }},
                Err(err) => Resp {{
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                }},
            }},
            Err(err) => Resp {{
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {{err}}")),
            }},
        }};
        writeln!(stdout, "{{}}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }}

    Ok(())
}}

fn execute(args: Value) -> Result<String, String> {{
    let _obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    Err("TODO: implement skill logic".to_string())
}}
"""


INTERFACE_TEMPLATE = """# {skill} Interface Spec

> 本技能接口草稿由 `skill_develop/create_skill.py` 生成，请在接入前补全。

## Capability Summary
- TODO: 描述 `{skill}` 的能力边界。

## Actions
- TODO: 列出支持的 `action`。

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| TODO | TODO | TODO | TODO | TODO | TODO |

## Error Contract
- TODO: 描述 `error_text` 约定。

## Request/Response Examples

### Example 1
Request:
```json
{{"request_id":"demo-1","args":{{}}}}
```
Response:
```json
{{"request_id":"demo-1","status":"ok","text":"TODO","error_text":null}}
```
"""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Scaffold a new RustClaw runner skill.")
    parser.add_argument("skill_name", help="snake_case skill name, e.g. stock_quote")
    parser.add_argument(
        "--aliases",
        default="",
        help="comma-separated aliases for configs/skills_registry.toml",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=30,
        help="default timeout_seconds for registry entry",
    )
    parser.add_argument(
        "--output-kind",
        default="text",
        choices=["text", "file", "image", "mixed"],
        help="registry output_kind",
    )
    parser.add_argument(
        "--disabled",
        action="store_true",
        help="create registry entry with enabled = false",
    )
    parser.add_argument(
        "--runner-name",
        default="",
        help="optional custom runner name if binary does not follow convention",
    )
    return parser.parse_args()


def ensure_valid_skill_name(skill_name: str) -> None:
    if not re.fullmatch(r"[a-z0-9_]+", skill_name):
        raise SystemExit("skill_name must match [a-z0-9_]+")


def cargo_toml_text(skill_name: str) -> str:
    bin_name = skill_name.replace("_", "-") + "-skill"
    return f"""[package]
name = "{bin_name}"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "{bin_name}"
path = "src/main.rs"

[dependencies]
anyhow.workspace = true
serde.workspace = true
serde_json.workspace = true
"""


def registry_entry_text(
    skill_name: str,
    aliases: list[str],
    timeout: int,
    output_kind: str,
    enabled: bool,
    runner_name: str,
) -> str:
    alias_text = ", ".join(f'"{alias}"' for alias in aliases)
    lines = [
        "",
        "[[skills]]",
        f'name = "{skill_name}"',
        f'enabled = {"false" if not enabled else "true"}',
        'kind = "runner"',
        f"aliases = [{alias_text}]",
        f"timeout_seconds = {timeout}",
        f'prompt_file = "prompts/skills/{skill_name}.md"',
        f'output_kind = "{output_kind}"',
    ]
    if runner_name.strip():
        lines.append(f'runner_name = "{runner_name.strip()}"')
    lines.append("")
    return "\n".join(lines)


def add_workspace_member(skill_name: str) -> bool:
    target = f'    "crates/skills/{skill_name}",'
    content = CARGO_TOML.read_text(encoding="utf-8")
    if target in content:
        return False

    marker = "\n]"
    insert_at = content.find(marker)
    if insert_at < 0:
        raise SystemExit("cannot find workspace members block in Cargo.toml")

    updated = content[:insert_at] + f'{target}' + content[insert_at:]
    CARGO_TOML.write_text(updated, encoding="utf-8")
    return True


def add_registry_entry(
    skill_name: str,
    aliases: list[str],
    timeout: int,
    output_kind: str,
    enabled: bool,
    runner_name: str,
) -> bool:
    content = REGISTRY_TOML.read_text(encoding="utf-8")
    if f'name = "{skill_name}"' in content:
        return False

    updated = content.rstrip() + registry_entry_text(
        skill_name, aliases, timeout, output_kind, enabled, runner_name
    )
    REGISTRY_TOML.write_text(updated + "\n", encoding="utf-8")
    return True


def write_if_missing(path: Path, content: str) -> bool:
    if path.exists():
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    return True


def main() -> int:
    args = parse_args()
    skill_name = args.skill_name.strip()
    ensure_valid_skill_name(skill_name)

    aliases = [v.strip() for v in args.aliases.split(",") if v.strip()]
    skill_dir = SKILLS_DIR / skill_name

    created = []
    updated = []

    if write_if_missing(skill_dir / "Cargo.toml", cargo_toml_text(skill_name)):
        created.append(skill_dir / "Cargo.toml")
    if write_if_missing(skill_dir / "src" / "main.rs", MAIN_RS_TEMPLATE):
        created.append(skill_dir / "src" / "main.rs")
    if write_if_missing(
        skill_dir / "INTERFACE.md", INTERFACE_TEMPLATE.format(skill=skill_name)
    ):
        created.append(skill_dir / "INTERFACE.md")

    if add_workspace_member(skill_name):
        updated.append(CARGO_TOML)
    if add_registry_entry(
        skill_name=skill_name,
        aliases=aliases,
        timeout=max(args.timeout, 1),
        output_kind=args.output_kind,
        enabled=not args.disabled,
        runner_name=args.runner_name,
    ):
        updated.append(REGISTRY_TOML)

    print(f"[skill] {skill_name}")
    print(f"[dir] {skill_dir.relative_to(REPO_ROOT)}")
    for path in created:
        print(f"[create] {path.relative_to(REPO_ROOT)}")
    for path in updated:
        print(f"[update] {path.relative_to(REPO_ROOT)}")

    print("[next] 运行 python3 scripts/sync_skill_docs.py")
    print("[next] 补充 prompts/agent_tool_spec.md 的技能契约")
    print(f"[next] 运行 cargo check -p clawd -p skill-runner -p {skill_name.replace('_', '-')}-skill")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
