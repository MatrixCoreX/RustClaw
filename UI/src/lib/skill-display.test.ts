import test from "node:test";
import assert from "node:assert/strict";

import {
  baseSkillNamesWithFallback,
  filterSkillNamesBySearch,
  formatCapabilityToken,
  groupSkillNames,
  hasCuratedSkillUsageExamples,
  isUiHiddenSkill,
  isVisibleSkillName,
  normalizeSkillSearchQuery,
  skillCapabilityLabel,
  skillDescription,
  skillIsolationLabels,
  skillPlannerCapabilityLabel,
  skillRiskLabel,
  skillRuntimeIssue,
  skillUsageExamples,
  visibleSkillNames,
} from "./skill-display.ts";

const CURRENT_REGISTRY_SKILLS = [
  "run_cmd", "read_file", "write_file", "workspace_patch", "list_dir", "make_dir", "remove_file", "fs_basic",
  "code_index", "config_basic", "config_edit", "schedule", "x", "system_basic", "http_basic", "git_basic",
  "install_module", "process_basic", "package_manager", "archive_basic", "db_basic", "docker_basic", "fs_search",
  "rss_fetch", "image_vision", "image_generate", "image_edit", "audio_transcribe", "audio_synthesize", "video_generate",
  "music_generate", "health_check", "log_analyze", "service_control", "task_control", "config_guard", "crypto", "stock",
  "weather", "map_merchant", "doc_parse", "transform", "invest_copy", "web_search_extract", "kb", "browser_web",
  "photo_organize", "extension_manager",
];

test("filters hidden UI-only skills", () => {
  assert.equal(isUiHiddenSkill("chat"), true);
  assert.equal(isVisibleSkillName("chat"), false);
  assert.deepEqual(visibleSkillNames(["chat", "run_cmd", "image_generate"]), ["run_cmd", "image_generate"]);
});

test("uses fallback base skill names when backend data is empty", () => {
  const fallback = baseSkillNamesWithFallback([]);
  assert.ok(fallback.includes("run_cmd"));
  assert.ok(fallback.includes("fs_basic"));
  assert.ok(fallback.includes("schedule"));
  assert.ok(fallback.includes("extension_manager"));
  assert.ok(fallback.includes("kb"));
  assert.deepEqual(baseSkillNamesWithFallback(["custom_base", "chat"]), ["custom_base"]);
});

test("groups managed skills by runtime metadata", () => {
  const groups = groupSkillNames(
    ["image_generate", "audio_synthesize", "video_generate", "music_generate", "run_cmd", "crypto", "fs_basic"],
    new Set(["fs_basic"]),
    new Set(["run_cmd"]),
  );
  assert.deepEqual(groups.tool, ["run_cmd"]);
  assert.deepEqual(groups.image, ["image_generate"]);
  assert.deepEqual(groups.audio, ["audio_synthesize"]);
  assert.deepEqual(groups.multimedia, ["music_generate", "video_generate"]);
  assert.deepEqual(groups.base, ["fs_basic"]);
  assert.deepEqual(groups.other, ["crypto"]);
});

test("keeps future video and music machine names in multimedia", () => {
  const groups = groupSkillNames(
    ["video_edit", "music_remix", "image_edit", "audio_transcribe", "plain_skill"],
    new Set(),
    new Set(),
  );
  assert.deepEqual(groups.multimedia, ["music_remix", "video_edit"]);
  assert.deepEqual(groups.other, ["plain_skill"]);
});

test("groups default workflow and knowledge skills as always-on base skills", () => {
  const groups = groupSkillNames(
    ["schedule", "extension_manager", "kb", "crypto"],
    new Set(baseSkillNamesWithFallback([])),
    new Set(),
  );
  assert.deepEqual(groups.base, ["extension_manager", "kb", "schedule"]);
  assert.deepEqual(groups.other, ["crypto"]);
});

test("normalizes and applies skill search text", () => {
  const query = normalizeSkillSearchQuery("  IMAGE ");
  assert.equal(query, "image");
  assert.deepEqual(filterSkillNamesBySearch(["image_generate", "audio_synthesize"], query), ["image_generate"]);
  assert.deepEqual(filterSkillNamesBySearch(["run_cmd"], ""), ["run_cmd"]);
});

test("formats skill descriptions and risk labels", () => {
  assert.equal(skillDescription("image_generate", "en"), "Generate images from prompts.");
  assert.equal(skillDescription("image_generate", "zh"), "根据描述生成图片。");
  assert.equal(skillDescription("code_index", "zh"), "索引并搜索代码结构和符号。");
  assert.equal(skillDescription("config_edit", "en"), "Preview, update, and validate configuration.");
  assert.equal(
    skillDescription("workspace_patch", "zh"),
    "用可检查、可回退的补丁修改工作区文件。",
  );
  assert.equal(
    skillDescription("workspace_patch", "zh", "English registry description"),
    "用可检查、可回退的补丁修改工作区文件。",
  );
  assert.equal(skillDescription("unknown_skill", "en"), "No short description for this skill.");
  assert.equal(skillDescription("unknown_skill", "en", " Registry text "), "Registry text");
  assert.equal(skillRiskLabel("high", "en"), "High risk");
  assert.equal(skillRiskLabel(null, "zh"), "风险未声明");
});

test("provides three to five curated usage examples for every registry skill", () => {
  for (const name of CURRENT_REGISTRY_SKILLS) {
    assert.equal(hasCuratedSkillUsageExamples(name), true, `${name} should have curated examples`);
    for (const lang of ["zh", "en"] as const) {
      const examples = skillUsageExamples(name, lang);
      assert.ok(examples.length >= 3 && examples.length <= 5, `${name}/${lang} should have 3-5 examples`);
      assert.ok(examples.every((example) => example.trim().length > 0));
    }
  }
});

test("uses localized curated examples and external-skill fallbacks", () => {
  assert.equal(skillUsageExamples("schedule", "zh")[0], "先解析‘每周一上午九点提醒我开周会’，不要创建任务。");
  assert.equal(skillUsageExamples("schedule", "en")[0], "Parse 'remind me every Monday at 9 AM' without creating it.");
  assert.deepEqual(skillUsageExamples("custom_skill", "en", "Process custom records"), [
    "Help me with this request: Process custom records",
    "Check whether this can be completed without performing side effects: Process custom records",
    "Complete this task and tell me the result and next step: Process custom records",
  ]);
});

test("formats runtime and planner capabilities", () => {
  assert.equal(skillCapabilityLabel("fs.read", "en"), "Reads files");
  assert.equal(skillCapabilityLabel("secrets.api_key", "zh"), "需要密钥");
  assert.equal(skillCapabilityLabel("custom.capability", "en"), "custom.capability");
  assert.equal(formatCapabilityToken("read_file.by_path"), "read file / by path");
  assert.equal(skillPlannerCapabilityLabel("filesystem.read_file", "en"), "Files: read file");
  assert.equal(skillPlannerCapabilityLabel("database.query_table", "zh"), "数据库: query table");
});

test("formats isolation policy labels from structured fields", () => {
  const labels = skillIsolationLabels(
    {
      name: "http_basic",
      planner_capability_policies: [
        {
          capability: "http.post_json",
          isolation_profile: "remote_executor",
          network_access: true,
          filesystem_write: false,
          external_publish: true,
          credential_access: true,
        },
      ],
    },
    "en",
  );

  assert.deepEqual(labels, ["External execution", "Network", "Can publish", "Uses keys"]);
});

test("formats runtime availability issues from structured fields", () => {
  assert.equal(
    skillRuntimeIssue({ name: "image_generate", runtime_available: false, unavailable_reason: "skill_disabled" }, "en"),
    "This skill is currently disabled",
  );
  assert.equal(
    skillRuntimeIssue({ name: "docker_basic", runtime_available: false, current_os: "darwin", unsupported_os: ["darwin"] }, "en"),
    "Current OS darwin is not supported: darwin",
  );
  assert.equal(
    skillRuntimeIssue({ name: "git_basic", runtime_available: false, missing_required_bins: ["git"] }, "zh"),
    "缺少本地工具：git",
  );
  assert.equal(skillRuntimeIssue({ name: "run_cmd", runtime_available: true }, "en"), null);
});
