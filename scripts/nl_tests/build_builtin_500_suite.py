#!/usr/bin/env python3
"""Compose reviewed regression and new bounded parameter cases for live NL."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CASES = ROOT / "scripts/nl_tests/cases"
OUTPUT = CASES / "generated/builtin_500_20260914/suite.txt"
REPORT = OUTPUT.with_suffix(".coverage.json")


def build() -> tuple[str, dict]:
    registry = tomllib.loads((ROOT / "configs/skills_registry.toml").read_text())
    fixed = {s["name"] for s in registry["skills"] if s.get("install_mode") != "on_demand"}
    optional = {s["name"] for s in registry["skills"] if s.get("install_mode") == "on_demand"}
    rows, provenance, seen = [], [], set()

    def add(name, tags, prompt, source, expectation=""):
        normalized = " ".join(prompt.split()).casefold()
        if normalized in seen:
            return
        if "|" in prompt or "\n" in prompt:
            raise ValueError("case_format_invalid")
        seen.add(normalized)
        if "requires_tool_call=" not in tags:
            tags += ";requires_tool_call=true"
        rows.append("|".join(["builtin_500_20260914", name, tags, prompt] + ([expectation] if expectation else [])))
        provenance.append({"name": name, "source": source, "prompt_sha256": hashlib.sha256(prompt.encode()).hexdigest()})

    for filename in ["nl_cases_builtin_tool_skill_fresh_20260830.txt", "nl_cases_basic_skill_100_coverage_20260629.txt"]:
        for line_number, line in enumerate((CASES / filename).read_text().splitlines(), 1):
            if not line or line.startswith("#"):
                continue
            parts = line.split("|", 4)
            if len(parts) < 4:
                continue
            _, name, tags, prompt = parts[:4]
            match = re.search(r"(?:^|;)covers:([^;]+)", tags)
            covered = set(match.group(1).split(",")) if match else set()
            if covered & optional or "optional_skill_deferred" in tags:
                continue
            # These two historical rows depend on another case's namespace.
            if name in {"b100_069_kb_search_fixture_en", "b100_070_kb_stats_fixture_zh"}:
                continue
            covered &= fixed
            if not covered:
                continue
            tags = re.sub(r"(?:^|;)covers:[^;]+", "covers:" + ",".join(sorted(covered)), tags, count=1)
            # Each media family gets one live regression; repeated media coverage
            # is provided by the reviewed preview/lifecycle rows, not paid loops.
            if filename.startswith("nl_cases_basic") and covered & {"image_generate", "image_edit", "audio_synthesize", "audio_transcribe", "video_generate", "music_generate"}:
                continue
            expectation = parts[4] if len(parts) == 5 else ""
            if name == "fresh_image_vision_structured_extract_en":
                prompt = "Analyze scripts/nl_tests/fixtures/photo_organize/fixture_2024_05.jpg and extract a JSON object with keys `scene_type`, `dominant_color`, and `text_present`. Use the image capability, do not infer from the filename. If the provider fails, return a JSON object with status=error and the observed error_code and failure_phase, plus provider and status_code when available. Do not invent a description, resize the image, or repeatedly retry the failed provider."
                expectation = 'expect=skill_outcome_json:{"skill":"image_vision","success_fields":["scene_type","dominant_color","text_present"]}'
            if name == "b100_089_image_vision_local_zh":
                prompt = "用图片技能看一下本地图片 scripts/nl_tests/fixtures/photo_organize/fixture_2024_05.jpg，成功时返回包含 image_path 和 description（一句中文短描述）的 JSON。若服务商调用失败，返回 JSON：status=error，加上实际观测的 error_code、failure_phase，以及可获得的 provider 和 status_code；不要编造图片内容，不缩放图片，不反复重试。"
                tags += ",allow_terminal_failure"
                expectation = 'expect=skill_outcome_json:{"skill":"image_vision","success_fields":["image_path","description"]}'
            if name == "fresh_write_file_atomic_cleanup_en":
                expectation = "expect=result_text_json_eq:/bytes_written=24;result_text_json_eq:/line_count=2;result_text_json_eq:/content_verified=true;contains:cleanup_status"
            if name == "fresh_workspace_patch_preview_zh":
                prompt = prompt.replace("listener ready", "Service Notes").replace("listener verified", "Service Notes Reviewed")
                expectation = "expect=result_text_json_eq:/occurrence_count=1;result_text_json_eq:/would_change=true;result_text_json_eq:/writes_performed=0"
            if name == "fresh_memory_lifecycle_en":
                prompt = prompt.replace("task-scoped memory", "current_principal memory in this isolated test account").replace("kind=note", "kind=fact")
                expectation = "expect=result_text_json_eq:/corrected=true;result_text_json_eq:/forgotten=true;contains:cleanup_status"
            if name == "fresh_fs_compare_docs_zh":
                tags = tags.replace("capability:filesystem.compare_paths", "any_successful_capability:filesystem.compare_paths;any_successful_capability:filesystem.stat_paths")
            if name == "fresh_process_clawd_observation_zh":
                expectation = "expect=contains:pid"
            if name == "fresh_subagent_readonly_sections_en":
                expectation = "expect=contains:headings;contains:line"
            if name == "fresh_doc_parse_agents_section_en":
                prompt = prompt.replace("AGENTS.md", "docs/base_skill_response_contract.md")
                prompt += " Return a JSON object using format, title, section_count and headings keys."
                expectation = "expect=result_text_json_eq:/section_count=6;contains:Base Skill Response Contract"
            if name == "fresh_fs_search_recovery_line_en":
                prompt = prompt.replace("`*.log`", "`**/*.log`")
                prompt += " Return a JSON object with matches as an array of observed path, line_number and line records."
                expectation = "expect=result_text_json_eq:/matches/0/line_number=7;observed_eq:line=7;contains:upstream request recovered"
                tags += ";min_successful_capability_calls:filesystem.grep_text=1;observed_field:text"
            if name == "b100_task_plan_read_en":
                tags += ";min_successful_capability_calls:task.plan_read=1;observed_field:steps"
                expectation = "expect=observed_eq:plan_revision=0"
            if name == "b100_026_system_workspace_glance_zh":
                tags += ";observed_field:path;observed_field:files;observed_field:branch"
                expectation = ""
            if name == "b100_028_run_cmd_pwd_en":
                prompt = prompt.replace("exit_code and stdout_path", "exit_code and stdout (the text printed by the command, not an output-artifact path)")
                tags += ";min_successful_capability_calls:system.run_command=1;final_observed_field:exit_code;final_observed_field:stdout"
                expectation = "expect=observed_eq:exit_code=0"
            if name == "b100_066_transform_dedupe_zh":
                prompt = "请使用数据转换工具完成这个去重任务，不要直接心算返回。" + prompt
                tags += ";min_successful_capability_calls:data.transform_records=1"
                expectation = 'expect=result_text_json_eq:=["a","b","c"]'
            if name in {"b100_063_transform_filter_json_en", "b100_064_transform_group_sum_zh", "b100_065_transform_csv_project_en"}:
                prompt = "Use the data transformation tool for this task, not a model-only calculation. " + prompt
                tags += ";min_successful_capability_calls:data.transform_records=1"
            if name == "b100_063_transform_filter_json_en":
                expectation = 'expect=result_text_json_eq:=[{"name":"a","ok":true},{"name":"c","ok":true}]'
            if name == "fresh_config_edit_locale_fixture_en":
                prompt += " Use config.apply_change, config.validate_after_change and config.read_back for the edit/validation/readback steps."
                tags = tags.replace("config_edit,config_guard,run_cmd", "config_edit,config_guard").replace(";capability:system.run_command", "")
                expectation = "expect=result_text_json_eq:/applied=true;result_text_json_eq:/valid=true;result_text_json_eq:/observed_value=fr-FR;contains:cleanup_status"
            if name == "b100_003_fs_count_logs_zh":
                prompt += " Return exactly a JSON object with the count key."
                count = len(list((CASES.parent / "fixtures/device_local/logs").iterdir()))
                expectation = f"expect=result_text_json_eq:/count={count}"
            if name == "fresh_video_generate_live_zh":
                prompt += " 删除前计算文件的 SHA-256；最终返回真实 task_id、size_bytes、sha256 和 cleanup_status。"
                tags += ";min_successful_capability_calls:filesystem.remove_path=1;observed_field:outputs[0].path"
                expectation = "expect=contains:task_id;contains:sha256"
            add(name, tags, prompt, f"{filename}:{line_number}", expectation)
    reused = len(rows)
    fixture = "scripts/nl_tests/fixtures/device_local"
    for n in range(1, 100):
        limit = [1, 2, 3, 5, 8, 13, 20][(n - 1) % 7]
        start = (n - 1) % 12 + 1
        cases = [
            ("read", "covers:read_file,fs_basic;capability:filesystem.read_text_range;local_readonly",
             f"请读取 {fixture}/docs/service_notes.md 的第 {start} 到 {start + limit - 1} 行，最多保留 {800 + n * 17} 个字符。告诉我实际读到了哪些行；超过文件末尾时不要补造内容。"),
            ("database", "covers:db_basic;capability:database.query;local_readonly",
             f"Use the database query tool on {fixture}/data/test_contract.sqlite, read-only. Run SELECT id,status,amount FROM orders WHERE amount >= {n} ORDER BY amount DESC,id LIMIT {limit}. Report the observed columns and rows, including an empty result when appropriate; do not edit the database."),
            ("transform", "covers:transform;capability:data.transform_records;local_readonly",
             f"请用数据转换工具处理数组 [{{\"label\":\"amber\",\"score\":{n}}},{{\"label\":\"jade\",\"score\":{n+7}}},{{\"label\":\"pearl\",\"score\":{n-3}}}]，先按 score 大于等于 {n} 筛选，再按 score 降序排序，最终只保留 label 与 score。不要由模型心算代替工具，返回实际 JSON 结果。"),
            ("schedule", "covers:schedule;capability:schedule.preview;local_readonly;no_external_side_effect",
             f"Preview, without creating anything, a one-time reminder at 2098-{(n-1)%12+1:02d}-{(n-1)%27+1:02d} {n%24:02d}:{n%60:02d} in {['Asia/Shanghai','Asia/Tokyo','Europe/London','America/New_York','UTC'][n%5]} to review archive batch {n}. Return the normalized timestamp and timezone and make clear whether anything was stored."),
            ("file_cycle", "covers:write_file,read_file,remove_file,fs_basic;capability:filesystem.write_file;capability:filesystem.read_text_range;capability:filesystem.remove_path;local_side_effect;cleanup",
             f"在本次隔离工作区 tmp/nl_500_note_{n}.txt 写入两行内容：第一行 audit-{n}，第二行 value-{n*13}，均以换行结尾。读回并验证行数与内容，再删除这个文件。汇报每一步实际结果，任何一步失败都要明确指出。"),
            ("document", "covers:doc_parse;capability:document.parse;local_readonly",
             f"Parse {fixture}/docs/{['service_notes.md','release_checklist.md','archive/README.txt'][n%3]} with the document tool, include_metadata=true and max_text_chars={500+n*29}. Return the format, available metadata and a brief extract without inventing missing sections."),
            ("missing", "covers:fs_basic;capability:filesystem.stat_paths;local_readonly",
             f"检查 tmp/nl_500_absent_{n}.md 是否存在，只检查这个精确路径。不创建、不搜索替代文件；据实返回 path 与 exists，如果不存在就停止。"),
            ("multi", "covers:fs_basic,run_cmd;capability:filesystem.write_file;capability:system.run_command;local_side_effect;cleanup",
             f"在 tmp/nl_500_code_{n}.py 创建一个真实 Python 程序：定义 double(x) 返回 x*2，并断言 double({n}) == {n*2}，成功时打印 verified-{n}。用 python3 执行它，根据退出码确认断言是否通过，最后只删除这个测试程序。不要只给我代码。"),
        ]
        for kind, tags, prompt in cases:
            if len(rows) == 500:
                break
            capabilities = re.findall(r"(?:^|;)capability:([^;]+)", tags)
            tags += "".join(f";min_successful_capability_calls:{capability}=1" for capability in capabilities)
            expectation = ""
            if kind == "file_cycle":
                content = f"audit-{n}\nvalue-{n*13}\n".encode()
                expectation = "expect=workspace_file_cycle:" + json.dumps({
                    "schema_version": 1, "path": f"tmp/nl_500_note_{n}.txt",
                    "sha256": hashlib.sha256(content).hexdigest(),
                    "size_bytes": len(content), "line_count": 2,
                }, sort_keys=True, separators=(",", ":"))
            elif kind == "transform":
                expected = [{"label": "jade", "score": n + 7}, {"label": "amber", "score": n}]
                expectation = "expect=result_text_json_records_eq:" + json.dumps(
                    expected, sort_keys=True, separators=(",", ":"))
            add(f"new_{kind}_{n:03d}", tags, prompt, "parameter_matrix_20260914", expectation)
        if len(rows) == 500:
            break
    assert len(rows) == len(seen) == 500
    coverage = {skill: [] for skill in sorted(fixed)}
    for row in rows:
        _, name, tags, *_ = row.split("|")
        match = re.search(r"(?:^|;)covers:([^;]+)", tags)
        for skill in match.group(1).split(","):
            coverage[skill].append(name)
    assert all(coverage.values()), [k for k, v in coverage.items() if not v]
    header = "# 500 distinct live NL inputs; structured invocation evidence is required.\n# Historical regressions plus new parameter/boundary/multi-step cases.\n# Fixed built-ins only. No live X, transfer, market trade, or remote publishing.\n# Run only with isolated workspace/database; preserve model logs for review.\n"
    body = header + "\n".join(rows) + "\n"
    return body, {"schema_version": 1, "case_count": 500, "reused_regressions": reused,
                  "new_parameter_cases": 500-reused, "fixed_entries": len(fixed),
                  "coverage_kind": "intended_only_verify_against_task_journal",
                  "sha256": hashlib.sha256(body.encode()).hexdigest(),
                  "intended_coverage": coverage, "cases": provenance}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    body, report = build()
    report_text = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.check:
        assert OUTPUT.read_text() == body
        assert REPORT.read_text() == report_text
    else:
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        OUTPUT.write_text(body)
        REPORT.write_text(report_text)
    print(json.dumps({k: report[k] for k in ("case_count", "reused_regressions", "new_parameter_cases", "fixed_entries", "sha256")}))


if __name__ == "__main__":
    main()
