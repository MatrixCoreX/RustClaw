use super::{
    collect_whitelisted_env_pairs, crypto_recoverable_i18n_error_key, extract_task_request_text,
    is_crypto_account_access_error, is_missing_target_skill_error, is_recoverable_skill_error,
    parse_policy_block_error, parse_structured_skill_error, policy_block_default_text,
    policy_block_error, request_reply_language, run_safe_command, skill_error_machine_observation,
    skill_runner_env_strict_enabled, structured_skill_error_from_parts,
    structured_skill_error_string, task_allows_path_outside_workspace, task_allows_sudo,
    task_request_locale_tag, RequestReplyLanguage, CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX,
    READ_FILE_NOT_FOUND_PREFIX, SKILL_RUNNER_ENV_WHITELIST, STRUCTURED_SKILL_ERROR_PREFIX,
};
use crate::{
    runtime::state::ClaimedTask, AgentRuntimeConfig, AppState, CommandIntentRuntime,
    ScheduleRuntime, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
};
use claw_core::config::{AgentConfig, ToolsConfig};
use rusqlite::params;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, RwLock};

static STRICT_ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let path = std::env::current_dir()
            .expect("current test directory")
            .join("target/clawd-skills-tests")
            .join(format!(
                "rustclaw_{prefix}_{}",
                uuid::Uuid::new_v4().simple()
            ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn test_state(locale: &str) -> AppState {
    let agents_by_id = HashMap::from([(
        DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig {
            command_intent: CommandIntentRuntime {
                default_locale: locale.to_string(),
            },
            schedule: ScheduleRuntime {
                timezone: "Asia/Shanghai".to_string(),
                intent_prompt_template: Arc::new(RwLock::new(String::new())),
                intent_prompt_source: String::new(),
                intent_rules_template: Arc::new(RwLock::new(String::new())),
                locale: locale.to_string(),
                i18n_dir: "configs/i18n".to_string(),
                i18n_dict: HashMap::new(),
            },
            ..crate::PolicyConfig::test_default()
        },
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

fn install_real_registry(state: &mut AppState) {
    let registry_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/skills_registry.toml")
        .canonicalize()
        .expect("canonicalize registry path");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load real skills registry");
    let enabled: HashSet<String> = registry.enabled_names().into_iter().collect();
    *state.core.skill_views_snapshot.write().unwrap() = Arc::new(SkillViewsSnapshot {
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(enabled),
    });
}

fn install_registry_from_toml(state: &mut AppState, root: &Path, toml: &str, enabled: &[&str]) {
    let registry_path = root.join("skills_registry.toml");
    fs::write(&registry_path, toml).expect("write registry fixture");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load registry fixture");
    let enabled = enabled.iter().map(|skill| (*skill).to_string()).collect();
    *state.core.skill_views_snapshot.write().unwrap() = Arc::new(SkillViewsSnapshot {
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(enabled),
    });
}

fn allow_test_skills(state: &mut AppState, skills: &[&str]) {
    let mut config = ToolsConfig::default();
    config.allow = skills
        .iter()
        .map(|skill| format!("skill:{skill}"))
        .collect();
    state.skill_rt.tools_policy =
        Arc::new(ToolsPolicy::from_config(&config).expect("test tools policy"));
}

fn make_echo_skill_runner(root: &Path) -> PathBuf {
    let path = root.join("echo-skill-runner");
    fs::write(
            &path,
            r#"#!/usr/bin/env bash
python3 -c 'import json, sys; req=json.loads(sys.stdin.readline()); print(json.dumps({"request_id": req.get("request_id", ""), "status": "ok", "text": json.dumps(req.get("args", {}), ensure_ascii=False), "error_text": None}, ensure_ascii=False))'
"#,
        )
        .expect("write fake skill runner");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)
            .expect("fake runner metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod fake runner");
    }
    path
}

fn make_sandbox_probe_skill_runner(root: &Path) -> PathBuf {
    let path = root.join("sandbox-probe-skill-runner");
    fs::write(
        &path,
        r#"#!/usr/bin/env python3
import json
import os
import sys

request = json.loads(sys.stdin.readline())
args = request.get("args", {})

def try_write(path):
    try:
        with open(path, "w", encoding="utf-8") as handle:
            handle.write("probe")
        return True
    except OSError:
        return False

result = {
    "workspace_write": try_write(os.path.join(os.environ["WORKSPACE_ROOT"], "workspace-probe.txt")),
    "outside_write": try_write(args["outside_path"]),
}
print(json.dumps({
    "request_id": request.get("request_id", ""),
    "status": "ok",
    "text": json.dumps(result),
    "error_text": None,
}))
"#,
    )
    .expect("write sandbox probe skill runner");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)
            .expect("sandbox probe metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod sandbox probe");
    }
    path
}

fn init_git_fixture_repo(root: &Path) {
    let run = |args: &[&str]| {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("run git fixture command");
        assert!(status.success(), "git fixture command failed: {args:?}");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "RustClaw Test"]);
    fs::write(root.join("README.md"), "base\n").expect("write git fixture README");
    run(&["add", "README.md"]);
    run(&["commit", "-q", "-m", "init"]);
}

fn insert_kb_doc_row(
    db: &rusqlite::Connection,
    user_key: &str,
    source_ref: &str,
    text: &str,
    ts: i64,
) {
    db.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref, user_id, chat_id, user_key,
                memory_kind, role, search_text, trigger_text, topic_tags, vector_json, metadata_json,
                salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             )
             VALUES (?1, NULL, NULL, ?2, 0, 0, ?3, ?4, NULL, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            rusqlite::params![
                crate::memory::RETRIEVAL_SOURCE_KB_DOC,
                source_ref,
                user_key,
                crate::memory::RETRIEVAL_KIND_KNOWLEDGE_DOC,
                text,
                crate::memory::retrieval::build_topic_tags(text),
                crate::memory::retrieval::vector_to_json(
                    &crate::memory::embedding::embed_text_locally(text),
                ),
                r#"{"scope_kind":"user","namespace":"photo_docs","path":"photo_rules.md"}"#,
                0.78_f32,
                crate::memory::RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
                crate::memory::RETRIEVAL_PRODUCER_KB,
                ts,
            ],
        )
        .expect("insert kb doc row");
}

fn seed_photo_organize_policy_memory(state: &AppState, user_id: i64, chat_id: i64, user_key: &str) {
    let db = state.core.db.get().expect("db pool");
    db.execute_batch(crate::INIT_SQL).expect("init base schema");
    crate::ensure_memory_schema(&db).expect("ensure memory schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("ensure retrieval schema");
    let ts = 1_775_301_800_i64;

    db.execute(
            "INSERT INTO user_preferences (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, updated_at, updated_at_ts)
             VALUES (?1, ?2, ?3, 'photo_grouping', 'PHOTO_ALLOWED_PREF_by_year_month', 0.95, 'test', '1775301800', ?4)",
            rusqlite::params![user_id, chat_id, user_key, ts],
        )
        .expect("insert preference");
    db.execute(
            "INSERT INTO long_term_memories (user_id, chat_id, user_key, summary, source_memory_id, created_at, updated_at, created_at_ts, updated_at_ts)
             VALUES (?1, ?2, ?3, 'PHOTO_BLOCKED_LONG_TERM_SUMMARY', 1, '1775301800', '1775301800', ?4, ?4)",
            rusqlite::params![user_id, chat_id, user_key, ts],
        )
        .expect("insert long term summary");

    crate::memory::indexing::upsert_knowledge_fact(
        &db,
        user_id,
        user_key,
        "photo_profile",
        crate::memory::RETRIEVAL_KIND_SEMANTIC_FACT,
        "test:photo:allowed-fact",
        "PHOTO_ALLOWED_FACT prefer grouping travel photos by capture date",
        ts,
    )
    .expect("insert knowledge fact");
    insert_kb_doc_row(
        &db,
        user_key,
        "kb:test:photo:allowed-doc",
        "PHOTO_ALLOWED_KB_DOC preserve original EXIF timestamp during organization",
        ts,
    );
    crate::memory::indexing::index_memory_row(
        &db,
        user_id,
        chat_id,
        user_key,
        101,
        crate::memory::MEMORY_ROLE_USER,
        "PHOTO_BLOCKED_RECENT_EVENT previous photo operation command",
        crate::memory::MEMORY_TYPE_GENERIC,
        0.9,
        true,
        ts + 1,
    )
    .expect("insert recent event memory");
    crate::memory::indexing::index_memory_row(
        &db,
        user_id,
        chat_id,
        user_key,
        102,
        crate::memory::MEMORY_ROLE_ASSISTANT,
        "PHOTO_BLOCKED_ASSISTANT_RESULT previous classified folder result",
        crate::memory::MEMORY_TYPE_ASSISTANT_OUTCOME,
        0.9,
        false,
        ts + 2,
    )
    .expect("insert assistant result memory");
    crate::memory::indexing::index_memory_row(
        &db,
        user_id,
        chat_id,
        user_key,
        103,
        crate::memory::MEMORY_ROLE_USER,
        "PHOTO_BLOCKED_UNFINISHED_GOAL continue moving all photos now",
        crate::memory::MEMORY_TYPE_UNFINISHED_GOAL,
        0.9,
        false,
        ts + 3,
    )
    .expect("insert unfinished goal memory");
}

fn test_task(payload: serde_json::Value) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: "task-test".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: Some("rk-test".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    }
}

#[tokio::test]
async fn disabled_skill_preflight_returns_policy_decision_payload() {
    let state = test_state("zh-CN");
    let task = test_task(json!({"kind": "run_skill"}));

    let err = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "tmp/out.txt", "content": "alpha\n"}),
    )
    .await
    .expect_err("disabled skill should fail before execution");
    let parsed = parse_policy_block_error(&err).expect("policy block error");
    let normalized: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("write_file", &err).unwrap())
            .unwrap();

    assert_eq!(parsed.reason_code, "skill_disabled");
    assert_eq!(parsed.decision, "deny");
    assert!(parsed.policy_boundary.iter().all(|item| {
        item.contains('=')
            && item
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '=' | '-'))
    }));
    assert_eq!(
        normalized
            .pointer("/permission_decision/decision")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
}

#[tokio::test]
async fn external_prompt_bundle_error_uses_machine_payload() {
    let root = TempDirGuard::new("external_prompt_bundle_machine_error");
    let mut state = test_state("en");
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "external_prompt_fixture"
enabled = true
kind = "external"
external_kind = "prompt_bundle"
"#,
        &["external_prompt_fixture"],
    );
    allow_test_skills(&mut state, &["external_prompt_fixture"]);
    let task = test_task(json!({"kind": "run_skill"}));

    let err = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "external_prompt_fixture",
        json!({"action": "preview"}),
    )
    .await
    .expect_err("prompt_bundle runtime preview should be a structured adapter error");
    let parsed = parse_structured_skill_error(&err).expect("structured external adapter error");
    let extra = parsed.extra.expect("external adapter extra");

    assert_eq!(parsed.error_kind, "external_kind_not_enabled");
    assert_eq!(parsed.error_text, "external_kind_not_enabled");
    assert_eq!(
        extra.get("owner_layer").and_then(serde_json::Value::as_str),
        Some("external_skill_adapter")
    );
    assert_eq!(
        extra.get("message_key").and_then(serde_json::Value::as_str),
        Some("clawd.msg.external_skill.external_kind_not_enabled")
    );
    assert_eq!(
        extra
            .get("external_kind")
            .and_then(serde_json::Value::as_str),
        Some("prompt_bundle")
    );
    assert_eq!(
        extra
            .get("unsupported_reason")
            .and_then(serde_json::Value::as_str),
        Some("external_kind_not_enabled")
    );
    assert_eq!(
        extra
            .get("provider_supported")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

#[tokio::test]
async fn high_risk_skill_dispatch_start_is_audited() {
    let mut state = test_state("en");
    install_real_registry(&mut state);
    let task = test_task(json!({"kind": "run_skill"}));

    super::run_skill_with_runner_outcome(
        &state,
        &task,
        "run_cmd",
        json!({
            "command": "true",
            "timeout_seconds": 5,
            "idle_timeout_seconds": 5,
            "max_output_bytes": 8000
        }),
    )
    .await
    .expect("safe high-risk command should run");

    let conn = state.core.audit_db.get().expect("audit db");
    let (action, detail_json, user_id): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT action, detail_json, user_id FROM audit_logs ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("latest audit row");
    assert_eq!(action, "skill_dispatch.high_risk_start");
    assert_eq!(user_id, Some(task.user_id));
    let detail: serde_json::Value =
        serde_json::from_str(detail_json.as_deref().expect("audit detail json")).unwrap();
    assert_eq!(detail["task_id"], task.task_id);
    assert_eq!(detail["skill"], "run_cmd");
    assert_eq!(detail["risk_level"], "high");
    assert_eq!(detail["requires_confirmation"], true);
}

#[tokio::test]
async fn builtin_write_file_outcome_exposes_structured_extra() {
    let root = TempDirGuard::new("builtin_write_file_structured_extra");
    let mut state = test_state("zh-CN");
    install_real_registry(&mut state);
    state.skill_rt.workspace_root = root.path().to_path_buf();
    let task = test_task(json!({"kind": "run_skill"}));

    let write = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "tmp/out.txt", "content": "alpha\n"}),
    )
    .await
    .expect("write_file outcome");
    let write_extra = write.extra.expect("write_file extra");
    assert_eq!(
        write_extra.get("action").and_then(|value| value.as_str()),
        Some("write_text")
    );
    assert_eq!(
        write_extra.get("path").and_then(|value| value.as_str()),
        Some("tmp/out.txt")
    );
    assert_eq!(
        write_extra
            .get("content_bytes")
            .and_then(|value| value.as_u64()),
        Some(6)
    );
    assert_eq!(
        write_extra
            .get("preexisting")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        write_extra.get("changed").and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        write_extra.get("noop").and_then(|value| value.as_bool()),
        Some(false)
    );
    assert!(
        write_extra
            .get("resolved_path")
            .and_then(|value| value.as_str())
            .is_some_and(|path| path.ends_with("tmp/out.txt")),
        "extra: {write_extra}"
    );

    let rewrite_same = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "tmp/out.txt", "content": "alpha\n"}),
    )
    .await
    .expect("same-content write outcome");
    let rewrite_same_extra = rewrite_same.extra.expect("same-content write extra");
    assert_eq!(
        rewrite_same_extra
            .get("preexisting")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        rewrite_same_extra
            .get("changed")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        rewrite_same_extra
            .get("noop")
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    let append = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "tmp/out.txt", "content": "beta\n", "append": true}),
    )
    .await
    .expect("append outcome");
    let append_extra = append.extra.expect("append extra");
    assert_eq!(
        append_extra.get("action").and_then(|value| value.as_str()),
        Some("append_text")
    );
    assert_eq!(
        append_extra.get("append").and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[tokio::test]
async fn builtin_write_file_local_temp_workspace_executes_in_isolation_root() {
    let root = TempDirGuard::new("builtin_write_file_local_temp_isolation");
    let mut state = test_state("en");
    state.skill_rt.workspace_root = root.path().to_path_buf();
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_temp_workspace", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["write_file"],
    );
    let task = test_task(json!({"kind": "run_skill"}));

    let outcome = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "tmp/out.txt", "content": "isolated\n"}),
    )
    .await
    .expect("isolated write_file outcome");

    assert!(
        !root.path().join("tmp/out.txt").exists(),
        "write_file should not modify the primary workspace"
    );
    let extra = outcome.extra.expect("write_file extra");
    let refs = extra
        .get("artifact_refs")
        .and_then(serde_json::Value::as_array)
        .expect("isolation artifact refs");
    let execution_root = refs[0]
        .get("execution_root")
        .and_then(serde_json::Value::as_str)
        .expect("execution root");
    assert_eq!(refs[0]["profile"], "local_temp_workspace");
    assert_eq!(refs[0]["requires_cleanup"], true);
    assert!(
        Path::new(execution_root).join("tmp/out.txt").exists(),
        "write_file should write inside the isolation root"
    );
    assert!(
        extra
            .get("resolved_path")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|path| path.starts_with(execution_root)),
        "extra: {extra}"
    );
}

#[tokio::test]
async fn builtin_write_file_local_worktree_executes_in_isolated_worktree() {
    let root = TempDirGuard::new("builtin_write_file_local_worktree_isolation");
    init_git_fixture_repo(root.path());
    let mut state = test_state("en");
    state.skill_rt.workspace_root = root.path().to_path_buf();
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_worktree", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["write_file"],
    );
    let task = test_task(json!({"kind": "run_skill"}));

    let outcome = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "src/generated.txt", "content": "worktree\n"}),
    )
    .await
    .expect("isolated worktree write_file outcome");

    assert!(
        !root.path().join("src/generated.txt").exists(),
        "local_worktree should not modify the primary workspace"
    );
    let extra = outcome.extra.expect("write_file extra");
    let refs = extra
        .get("artifact_refs")
        .and_then(serde_json::Value::as_array)
        .expect("isolation artifact refs");
    let execution_root = refs[0]
        .get("execution_root")
        .and_then(serde_json::Value::as_str)
        .expect("execution root");
    assert_eq!(refs[0]["profile"], "local_worktree");
    assert_eq!(refs[0]["creation_kind"], "create_local_git_worktree");
    assert_eq!(refs[0]["requires_cleanup"], true);
    assert_eq!(
        fs::read_to_string(Path::new(execution_root).join("src/generated.txt"))
            .expect("read isolated worktree output"),
        "worktree\n"
    );
}

#[tokio::test]
async fn builtin_write_file_local_worktree_preserves_dirty_primary_workspace() {
    let root = TempDirGuard::new("builtin_write_file_local_worktree_dirty_primary");
    init_git_fixture_repo(root.path());
    fs::write(root.path().join("README.md"), "dirty primary\n").expect("dirty primary readme");
    let mut state = test_state("en");
    state.skill_rt.workspace_root = root.path().to_path_buf();
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_worktree", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["write_file"],
    );
    let task = test_task(json!({"kind": "run_skill"}));

    let outcome = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "README.md", "content": "isolated edit\n"}),
    )
    .await
    .expect("isolated worktree write_file outcome");

    assert_eq!(
        fs::read_to_string(root.path().join("README.md")).expect("read primary README"),
        "dirty primary\n",
        "local_worktree must preserve pre-existing primary workspace edits"
    );
    let extra = outcome.extra.expect("write_file extra");
    let refs = extra
        .get("artifact_refs")
        .and_then(serde_json::Value::as_array)
        .expect("isolation artifact refs");
    let execution_root = refs[0]
        .get("execution_root")
        .and_then(serde_json::Value::as_str)
        .expect("execution root");
    assert_eq!(refs[0]["profile"], "local_worktree");
    let resolved_path = extra
        .get("resolved_path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<missing>");
    assert!(
        resolved_path.starts_with(execution_root),
        "resolved path must stay under isolated worktree: {extra}"
    );
    assert_eq!(
        fs::read_to_string(resolved_path).expect("read isolated worktree output"),
        "isolated edit\n",
        "execution_root={execution_root} resolved_path={resolved_path} extra={extra}"
    );
}

#[tokio::test]
async fn builtin_write_file_local_worktree_blocks_absolute_primary_workspace_path() {
    let root = TempDirGuard::new("builtin_write_file_local_worktree_absolute_primary");
    init_git_fixture_repo(root.path());
    fs::write(root.path().join("README.md"), "dirty primary\n").expect("dirty primary readme");
    let mut state = test_state("en");
    state.skill_rt.workspace_root = root.path().to_path_buf();
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_worktree", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["write_file"],
    );
    let task = test_task(json!({"kind": "run_skill"}));

    let err = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": root.path().join("README.md"), "content": "escaped edit\n"}),
    )
    .await
    .expect_err("absolute primary workspace path should be blocked in local_worktree isolation");

    assert!(
        err.contains("path_outside_workspace"),
        "expected path_outside_workspace policy block, got: {err}"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("README.md")).expect("read primary README"),
        "dirty primary\n",
        "local_worktree must not write absolute paths back into the primary workspace"
    );
}

#[tokio::test]
async fn builtin_run_cmd_local_temp_workspace_uses_isolated_cwd() {
    let root = TempDirGuard::new("builtin_run_cmd_local_temp_isolation");
    let mut state = test_state("en");
    state.skill_rt.workspace_root = root.path().to_path_buf();
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "system.run_command", effect = "external", required = ["command"], optional = ["cwd"], risk_level = "high", isolation_profile = "local_temp_workspace", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["run_cmd"],
    );
    let task = test_task(json!({"kind": "run_skill"}));

    let outcome = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "run_cmd",
        json!({
            "command": "printf isolated > marker.txt",
            "cwd": ".",
            "timeout_seconds": 5,
            "idle_timeout_seconds": 5,
            "max_output_bytes": 8000
        }),
    )
    .await
    .expect("isolated run_cmd outcome");

    assert!(
        !root.path().join("marker.txt").exists(),
        "run_cmd should not modify the primary workspace"
    );
    let extra = outcome.extra.expect("run_cmd isolation extra");
    let refs = extra
        .get("artifact_refs")
        .and_then(serde_json::Value::as_array)
        .expect("isolation artifact refs");
    let execution_root = refs[0]
        .get("execution_root")
        .and_then(serde_json::Value::as_str)
        .expect("execution root");
    assert_eq!(refs[0]["profile"], "local_temp_workspace");
    let marker = fs::read_to_string(Path::new(execution_root).join("marker.txt"))
        .expect("read isolated marker");
    assert_eq!(marker, "isolated");
}

#[tokio::test]
async fn builtin_run_cmd_async_start_outcome_exposes_pending_async_job_extra() {
    let root = TempDirGuard::new("builtin_run_cmd_async_start_extra");
    let mut state = test_state("zh-CN");
    install_real_registry(&mut state);
    state.skill_rt.workspace_root = root.path().to_path_buf();
    let task = test_task(json!({"kind": "run_skill"}));

    let outcome = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "run_cmd",
        json!({
            "command": "sleep 0.05; echo async-ok",
            "cwd": ".",
            "async_start": true,
            "poll_after_seconds": 1,
            "expires_in_seconds": 30
        }),
    )
    .await
    .expect("run_cmd async start outcome");

    let extra = outcome.extra.expect("async start extra");
    let job = extra
        .get("pending_async_job")
        .expect("pending async job extra");
    assert_eq!(job["status"], "accepted");
    assert_eq!(job["poll_after_seconds"], 1);
    assert_eq!(job["poll_after_ms"], 1_000);
    assert_eq!(job["provider"], "local_process");
    assert_eq!(job["result_ref"], job["job_id"]);
    assert_eq!(job["cancel_token"], job["cancel_ref"]);
    assert_eq!(job["retryable"], true);
    assert_eq!(job["message_key"], "clawd.task.async_job_pending");
    assert!(
        job["job_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("local_process:")),
        "job: {job}"
    );
    assert!(
        job["cancel_ref"]
            .as_str()
            .is_some_and(|value| value.starts_with("local_process:")),
        "job: {job}"
    );
    assert!(outcome.text.contains("\"status\":\"accepted\""));
}

fn insert_auth_key(state: &AppState, user_key: &str, role: &str) {
    let db = state.core.db.get().expect("db pool");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("create auth schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, ?2, 1, '123', NULL)",
        params![user_key, role],
    )
    .expect("insert auth key");
}

#[test]
fn request_reply_language_prefers_english_for_ascii_requests() {
    assert_eq!(
        request_reply_language("open the config page"),
        RequestReplyLanguage::En
    );
}

#[test]
fn request_reply_language_prefers_chinese_for_cjk_requests() {
    assert_eq!(
        request_reply_language("去改配置"),
        RequestReplyLanguage::ZhCn
    );
}

#[test]
fn request_reply_language_falls_back_for_mixed_requests() {
    assert_eq!(
        request_reply_language("用 English 改配置"),
        RequestReplyLanguage::ConfigDefault
    );
}

#[test]
fn extract_task_request_text_reads_top_level_text() {
    let payload = json!({
        "text": "please update the config"
    });
    assert_eq!(
        extract_task_request_text(&payload.to_string()).as_deref(),
        Some("please update the config")
    );
}

#[test]
fn extract_task_request_text_reads_nested_request_text() {
    let payload = json!({
        "skill_name": "run_cmd",
        "args": {
            "request_text": "set the config flag"
        }
    });
    assert_eq!(
        extract_task_request_text(&payload.to_string()).as_deref(),
        Some("set the config flag")
    );
}

#[tokio::test]
async fn run_skill_photo_organize_injects_registry_cropped_memory_args() {
    let temp = TempDirGuard::new("skill_memory_echo_runner");
    let mut state = test_state("zh-CN");
    install_real_registry(&mut state);
    allow_test_skills(&mut state, &["photo_organize"]);
    state.skill_rt.skill_runner_path = make_echo_skill_runner(temp.path());
    state.skill_rt.workspace_root = temp.path().to_path_buf();
    state.skill_rt.skill_timeout_seconds = 5;

    let user_id = 91;
    let chat_id = 92;
    let user_key = "user:photo-policy";
    seed_photo_organize_policy_memory(&state, user_id, chat_id, user_key);
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-photo-memory-policy".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "run_skill".to_string(),
        payload_json: json!({
            "skill_name": "photo_organize",
            "args": {"action": "prepare", "text": "请准备照片整理候选目录"}
        })
        .to_string(),
    };

    let outcome = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "photo_organize",
        json!({"action": "prepare", "text": "请准备照片整理候选目录"}),
    )
    .await
    .expect("run fake photo_organize");
    let echoed_args: serde_json::Value =
        serde_json::from_str(&outcome.text).expect("echoed args json");
    let memory = echoed_args
        .get("_memory")
        .expect("memory args should be injected");
    let context = memory
        .get("context")
        .and_then(|value| value.as_str())
        .expect("memory context string");

    assert_eq!(
        memory
            .get("use_policy")
            .and_then(|value| value.get("profile"))
            .and_then(|value| value.as_str()),
        Some("skill_scoped")
    );
    assert_eq!(
        memory.get("lang_hint").and_then(|value| value.as_str()),
        Some("zh-CN")
    );
    assert!(context.contains("PHOTO_ALLOWED_FACT"), "context: {context}");
    assert!(
        context.contains("PHOTO_ALLOWED_KB_DOC"),
        "context: {context}"
    );
    assert_eq!(
        memory
            .get("preferences")
            .and_then(|value| value.get("photo_grouping"))
            .and_then(|value| value.as_str()),
        Some("PHOTO_ALLOWED_PREF_by_year_month")
    );
    assert!(memory
        .get("long_term_summary")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .is_empty());
    let serialized = serde_json::to_string(memory).expect("serialize memory");
    for blocked in [
        "PHOTO_BLOCKED_LONG_TERM_SUMMARY",
        "PHOTO_BLOCKED_RECENT_EVENT",
        "PHOTO_BLOCKED_ASSISTANT_RESULT",
        "PHOTO_BLOCKED_UNFINISHED_GOAL",
    ] {
        assert!(
            !serialized.contains(blocked),
            "blocked memory leaked: {blocked}; memory={serialized}"
        );
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn runner_process_can_write_workspace_but_not_adjacent_host_path() {
    if !Path::new("/usr/bin/bwrap").is_file() && !Path::new("/bin/bwrap").is_file() {
        return;
    }
    let root = TempDirGuard::new("runner_process_sandbox");
    let outside_path = root
        .path()
        .parent()
        .expect("sandbox fixture parent")
        .join(format!(
            "outside-probe-{}.txt",
            uuid::Uuid::new_v4().simple()
        ));
    let mut state = test_state("en");
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "sandbox_probe"
enabled = true
kind = "runner"
runner_name = "sandbox-probe"
capabilities = ["fs.write"]
"#,
        &["sandbox_probe"],
    );
    allow_test_skills(&mut state, &["sandbox_probe"]);
    state.skill_rt.skill_runner_path = make_sandbox_probe_skill_runner(root.path());
    state.skill_rt.workspace_root = root.path().to_path_buf();
    state.skill_rt.skill_timeout_seconds = 5;
    let task = test_task(json!({"kind": "run_skill"}));

    let outcome = super::run_skill_with_runner_outcome(
        &state,
        &task,
        "sandbox_probe",
        json!({"outside_path": outside_path}),
    )
    .await
    .expect("run sandbox probe");
    let probe: serde_json::Value = serde_json::from_str(&outcome.text).expect("probe output");

    assert_eq!(
        probe.get("workspace_write").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        probe.get("outside_write").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert!(root.path().join("workspace-probe.txt").is_file());
    assert!(!outside_path.exists());
}

#[test]
fn task_request_locale_tag_prefers_english_request_text() {
    let state = test_state("zh-CN");
    let task = test_task(json!({
        "text": "check my binance spot balances"
    }));
    assert_eq!(task_request_locale_tag(&state, &task), "en-US");
}

#[test]
fn task_request_locale_tag_falls_back_to_schedule_locale() {
    let state = test_state("en-US");
    let task = test_task(json!({
        "text": "12345"
    }));
    assert_eq!(task_request_locale_tag(&state, &task), "en-US");
}

#[test]
fn task_allows_privileged_tools_for_admin_only() {
    let mut state = test_state("zh-CN");
    state.policy.allow_sudo = true;
    state.policy.allow_path_outside_workspace = true;

    insert_auth_key(&state, "rk-admin", "admin");
    insert_auth_key(&state, "rk-user", "user");

    let mut admin_task = test_task(json!({ "text": "run privileged command" }));
    admin_task.user_key = Some("rk-admin".to_string());
    assert!(task_allows_sudo(&state, Some(&admin_task)));
    assert!(task_allows_path_outside_workspace(
        &state,
        Some(&admin_task)
    ));

    let mut user_task = test_task(json!({ "text": "run privileged command" }));
    user_task.user_key = Some("rk-user".to_string());
    assert!(!task_allows_sudo(&state, Some(&user_task)));
    assert!(!task_allows_path_outside_workspace(
        &state,
        Some(&user_task)
    ));
}

#[test]
fn read_file_not_found_is_recoverable() {
    let err = format!("{}/etc/missing", READ_FILE_NOT_FOUND_PREFIX);
    assert!(is_recoverable_skill_error("read_file", &err));
    assert!(is_recoverable_skill_error("READ_FILE", &err));
    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("read_file", &err).unwrap()).unwrap();
    assert_eq!(
        observation
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("read_file_not_found")
    );
    assert_eq!(
        observation
            .pointer("/path")
            .and_then(serde_json::Value::as_str),
        Some("/etc/missing")
    );
    assert!(is_missing_target_skill_error("read_file", &err));
}

#[test]
fn builtin_read_only_structured_file_errors_are_recoverable() {
    let read_err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "read_file",
            "error_kind": "is_directory",
            "error_text": "read_file requires a file",
            "platform": "linux",
            "extra": { "requested_path": "docs" }
        })
    );
    let list_err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "list_dir",
            "error_kind": "ambiguous_target",
            "error_text": "directory locator matched multiple candidates",
            "platform": "linux",
            "extra": { "candidates": ["/tmp/a", "/tmp/b"] }
        })
    );
    let remove_err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "remove_file",
            "error_kind": "not_found",
            "error_text": "remove_file failed",
            "platform": "linux",
            "extra": { "requested_path": "missing.txt" }
        })
    );

    assert!(is_recoverable_skill_error("read_file", &read_err));
    assert!(is_recoverable_skill_error("list_dir", &list_err));
    assert!(!is_recoverable_skill_error("remove_file", &remove_err));
    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("list_dir", &list_err).unwrap())
            .unwrap();
    assert_eq!(
        observation
            .pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("ambiguous_target")
    );
    assert!(observation.pointer("/error_text").is_none());
}

#[test]
fn system_basic_read_failures_are_recoverable() {
    let perm_err = "read file failed: Permission denied (os error 13)";
    let dir_err = "read file failed: Is a directory (os error 21)";
    let nf_err = "read file failed: No such file or directory (os error 2)";
    let structured_perm_err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "system_basic",
            "error_kind": "permission_denied",
            "error_text": "read_range failed for /tmp/demo",
            "platform": "linux"
        })
    );
    let structured_nf_err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "system_basic",
            "error_kind": "not_found",
            "error_text": "path was not found: /tmp/demo",
            "platform": "linux"
        })
    );
    let structured_dir_err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "system_basic",
            "error_kind": "is_directory",
            "error_text": "read_range requires a file, but target is a directory: /tmp/demo",
            "platform": "linux"
        })
    );

    assert!(!is_recoverable_skill_error("system_basic", perm_err));
    assert!(!is_recoverable_skill_error("system_basic", dir_err));
    assert!(!is_recoverable_skill_error("system_basic", nf_err));
    assert!(!is_recoverable_skill_error("SYSTEM_BASIC", perm_err));
    assert!(is_recoverable_skill_error(
        "system_basic",
        &structured_perm_err
    ));
    assert!(is_recoverable_skill_error(
        "system_basic",
        &structured_nf_err
    ));
    assert!(is_missing_target_skill_error(
        "system_basic",
        &structured_nf_err
    ));
    assert!(!is_missing_target_skill_error(
        "system_basic",
        &structured_perm_err
    ));
    assert!(is_recoverable_skill_error(
        "system_basic",
        &structured_dir_err
    ));

    let n1: serde_json::Value = serde_json::from_str(
        &skill_error_machine_observation("system_basic", &structured_perm_err).unwrap(),
    )
    .unwrap();
    assert_eq!(
        n1.pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("permission_denied")
    );
    let n2: serde_json::Value = serde_json::from_str(
        &skill_error_machine_observation("system_basic", &structured_dir_err).unwrap(),
    )
    .unwrap();
    assert_eq!(
        n2.pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("is_directory")
    );
    let n3: serde_json::Value = serde_json::from_str(
        &skill_error_machine_observation("system_basic", &structured_nf_err).unwrap(),
    )
    .unwrap();
    assert_eq!(
        n3.pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("not_found")
    );
}

#[test]
fn structured_skill_error_ignores_json_hidden_in_visible_text() {
    let encoded = structured_skill_error_string(
        "system_basic",
        &json!({
            "status": "error",
            "error_text": "skill failed",
            "text": "{\"error_kind\":\"not_found\",\"failure_reason\":\"path was not found\",\"platform\":\"linux\",\"manager_type\":\"systemd\",\"service_name\":\"clawd\"}"
        }),
    );

    let parsed = parse_structured_skill_error(&encoded).expect("structured skill error");

    assert_eq!(parsed.error_kind, "unknown");
    assert_eq!(parsed.error_text, "skill failed");
    assert_eq!(parsed.platform, None);
    assert_eq!(parsed.manager_type, None);
    assert_eq!(parsed.service_name, None);
    assert!(!is_recoverable_skill_error("system_basic", &encoded));
}

#[test]
fn structured_skill_error_accepts_extra_machine_fields() {
    let encoded = structured_skill_error_string(
        "system_basic",
        &json!({
            "status": "error",
            "error_text": "skill failed",
            "extra": {
                "error_kind": "not_found",
                "failure_reason": "path was not found",
                "platform": "linux",
                "manager_type": "systemd",
                "service_name": "clawd"
            },
            "text": "display only"
        }),
    );

    let parsed = parse_structured_skill_error(&encoded).expect("structured skill error");

    assert_eq!(parsed.error_kind, "not_found");
    assert_eq!(parsed.error_text, "skill failed");
    assert_eq!(parsed.platform.as_deref(), Some("linux"));
    assert_eq!(parsed.manager_type.as_deref(), Some("systemd"));
    assert_eq!(parsed.service_name.as_deref(), Some("clawd"));
    assert!(is_recoverable_skill_error("system_basic", &encoded));
}

#[test]
fn run_cmd_structured_error_machine_observation_uses_extra_streams() {
    let err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 7",
            "platform": "linux",
            "extra": {
                "exit_code": 7,
                "stderr": "problem",
                "stdout": "progress",
                "output_truncated": false
            }
        })
    );

    let structured = parse_structured_skill_error(&err).expect("structured run_cmd error");
    assert_eq!(structured.error_kind, "nonzero_exit");
    assert_eq!(
        structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("stderr"))
            .and_then(|value| value.as_str()),
        Some("problem")
    );

    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("run_cmd", &err).unwrap()).unwrap();
    assert_eq!(
        observation
            .pointer("/extra/exit_code")
            .and_then(serde_json::Value::as_i64),
        Some(7)
    );
    assert_eq!(
        observation
            .pointer("/extra/stderr")
            .and_then(serde_json::Value::as_str),
        Some("problem")
    );
    assert_eq!(
        observation
            .pointer("/extra/stdout")
            .and_then(serde_json::Value::as_str),
        Some("progress")
    );
}

#[test]
fn run_cmd_structured_error_machine_observation_uses_exit_category() {
    let err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "platform": "linux",
            "extra": {
                "exit_code": 127,
                "exit_category": "command_not_found",
                "exit_classification_source": "exit_code",
                "stderr": "shell-specific message",
                "output_truncated": false
            }
        })
    );

    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("run_cmd", &err).unwrap()).unwrap();

    assert_eq!(
        observation
            .pointer("/extra/exit_category")
            .and_then(serde_json::Value::as_str),
        Some("command_not_found")
    );
    assert_eq!(
        observation
            .pointer("/extra/exit_code")
            .and_then(serde_json::Value::as_i64),
        Some(127)
    );
    assert!(!observation
        .to_string()
        .contains("Command failed with exit code"));
}

#[test]
fn crypto_account_access_errors_are_recoverable() {
    let payload = json!({
        "exchange": "binance",
        "detail": "binance api error code=-2015: Invalid API-key, IP, or permissions for action"
    });
    let err = format!("{CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX}{payload}");

    assert!(is_recoverable_skill_error("crypto", &err));
    assert!(is_crypto_account_access_error("crypto", &err));
    assert!(is_recoverable_skill_error("CRYPTO", &err));
    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("crypto", &err).unwrap()).unwrap();
    assert_eq!(
        observation
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("crypto.err.account_access_failed")
    );
    assert_eq!(
        observation
            .pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("account_access_failed")
    );
    assert_eq!(
        observation
            .pointer("/exchange")
            .and_then(serde_json::Value::as_str),
        Some("binance")
    );
}

#[test]
fn wrapped_crypto_account_access_errors_are_recoverable() {
    let marker = format!(
        "{CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX}{}",
        json!({
            "exchange": "binance",
            "detail": "binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"
        })
    );
    let err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "crypto",
            "error_kind": "unknown",
            "error_text": marker,
            "extra": null
        })
    );

    assert!(is_recoverable_skill_error("crypto", &err));
    assert!(is_crypto_account_access_error("crypto", &err));
    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("crypto", &err).unwrap()).unwrap();
    assert_eq!(
        observation
            .pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("unknown")
    );
    assert!(!observation
        .to_string()
        .contains(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX));
}

#[test]
fn structured_crypto_account_access_extra_is_recoverable_without_sentinel() {
    let err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "crypto",
            "error_kind": "account_access_failed",
            "error_text": "private exchange account access failed",
            "extra": {
                "error_kind": "account_access_failed",
                "message_key": "crypto.err.account_access_failed",
                "exchange": "binance",
                "detail": "binance api error code=-2015: Invalid API-key"
            }
        })
    );

    assert!(is_recoverable_skill_error("crypto", &err));
    assert!(is_crypto_account_access_error("crypto", &err));
    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("crypto", &err).unwrap()).unwrap();
    assert_eq!(
        observation
            .pointer("/extra/message_key")
            .and_then(serde_json::Value::as_str),
        Some("crypto.err.account_access_failed")
    );
    assert_eq!(
        observation
            .pointer("/extra/exchange")
            .and_then(serde_json::Value::as_str),
        Some("binance")
    );
    assert!(!observation
        .to_string()
        .contains(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX));
}

#[test]
fn structured_crypto_credential_errors_are_recoverable_i18n() {
    let err = format!(
        "{STRUCTURED_SKILL_ERROR_PREFIX}{}",
        json!({
            "skill": "crypto",
            "error_kind": "credential_not_bound",
            "error_text": "credential binding unavailable",
            "extra": {
                "error_kind": "credential_not_bound",
                "message_key": "crypto.err.okx_not_bound",
                "exchange": "okx",
                "action": "cancel_all_orders",
                "recoverable": true,
                "status_code": "credential_not_bound"
            }
        })
    );

    assert!(is_recoverable_skill_error("crypto", &err));
    assert_eq!(
        crypto_recoverable_i18n_error_key("crypto", &err).as_deref(),
        Some("crypto.err.okx_not_bound")
    );
    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("crypto", &err).unwrap()).unwrap();
    assert_eq!(
        observation
            .pointer("/extra/message_key")
            .and_then(serde_json::Value::as_str),
        Some("crypto.err.okx_not_bound")
    );
    assert_eq!(
        observation
            .pointer("/extra/action")
            .and_then(serde_json::Value::as_str),
        Some("cancel_all_orders")
    );
}

#[test]
fn contract_structured_errors_project_machine_observation() {
    let err = structured_skill_error_from_parts(
        "system_basic",
        "contract_action_rejected",
        "action `system_basic.inventory_dir` is rejected by contract `generic_path_content`",
        None,
        Some(json!({
            "action": "system_basic.inventory_dir",
            "contract_match": "generic_path_content"
        })),
    );

    let observation: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("system_basic", &err).unwrap())
            .unwrap();

    assert_eq!(
        observation
            .pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("contract_action_rejected")
    );
    assert_eq!(
        observation
            .pointer("/extra/contract_match")
            .and_then(serde_json::Value::as_str),
        Some("generic_path_content")
    );
    assert!(!observation.to_string().contains("__RC_SKILL_ERROR__"));
}

#[test]
fn other_skill_errors_are_not_recoverable() {
    assert!(!is_recoverable_skill_error(
        "git_basic",
        "fatal: not a git repository"
    ));
    assert!(!is_recoverable_skill_error(
        "system_basic",
        "command not found"
    ));
    assert!(!is_recoverable_skill_error(
        "read_file",
        "some random error"
    ));
    assert!(!is_recoverable_skill_error(
        "crypto",
        "binance api error code=-2015: Invalid API-key, IP, or permissions for action"
    ));
}

#[test]
fn policy_block_error_roundtrips_structured_payload() {
    let encoded = policy_block_error(
        "path_outside_workspace",
        vec!["denied_path: /etc/shadow".to_string()],
        vec!["Do not access the denied path.".to_string()],
    );
    let parsed = parse_policy_block_error(&encoded).expect("policy block payload");
    assert_eq!(parsed.decision, "deny");
    assert_eq!(parsed.reason_code, "path_outside_workspace");
    assert_eq!(parsed.observed_facts, vec!["denied_path: /etc/shadow"]);
    assert_eq!(
        parsed.policy_boundary,
        vec!["Do not access the denied path."]
    );
    let normalized: serde_json::Value =
        serde_json::from_str(&skill_error_machine_observation("read_file", &encoded).unwrap())
            .unwrap();
    assert_eq!(
        normalized
            .pointer("/decision")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
    assert_eq!(
        normalized
            .pointer("/permission_decision/decision")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
    assert_eq!(
        normalized
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.policy.path_outside_workspace")
    );
    assert_eq!(
        normalized
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("path_outside_workspace")
    );
}

#[test]
fn skill_error_machine_observation_strips_user_visible_fields() {
    let encoded = format!(
        "{}{}",
        STRUCTURED_SKILL_ERROR_PREFIX,
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "text": "visible prose",
            "extra": {
                "command": "missing-bin",
                "exit_code": 127,
                "stderr": "missing-bin: command not found",
                "nested": {
                    "text": "nested visible prose",
                    "error_text": "nested error prose",
                    "error_code": "command_not_found"
                }
            }
        })
    );

    let observation =
        skill_error_machine_observation("run_cmd", &encoded).expect("machine observation");
    let value: serde_json::Value = serde_json::from_str(&observation).expect("json observation");

    assert_eq!(
        value
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("structured_skill_error")
    );
    assert_eq!(
        value
            .pointer("/extra/exit_code")
            .and_then(serde_json::Value::as_i64),
        Some(127)
    );
    assert!(value.pointer("/error_text").is_none());
    assert!(value.pointer("/text").is_none());
    assert!(value.pointer("/extra/nested/error_text").is_none());
    assert!(value.pointer("/extra/nested/text").is_none());
    assert!(!observation.contains("Command failed with exit code"));
    assert!(!observation.contains("visible prose"));
}

#[test]
fn skill_error_machine_observation_projects_read_file_not_found_marker() {
    let marker = format!("{READ_FILE_NOT_FOUND_PREFIX}/tmp/missing.md");
    let observation =
        skill_error_machine_observation("read_file", &marker).expect("machine observation");
    let value: serde_json::Value = serde_json::from_str(&observation).expect("json observation");

    assert_eq!(
        value
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("read_file_not_found")
    );
    assert_eq!(
        value
            .pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("not_found")
    );
    assert_eq!(
        value.pointer("/path").and_then(serde_json::Value::as_str),
        Some("/tmp/missing.md")
    );
    assert!(!observation.contains("file not found"));
}

#[test]
fn legacy_policy_block_payload_defaults_to_deny_decision() {
    let encoded = format!(
        "__RC_POLICY_BLOCK__:{}",
        json!({
            "reason_code": "legacy_policy_block",
            "observed_facts": ["policy_token: skill:demo"],
            "policy_boundary": []
        })
    );

    let parsed = parse_policy_block_error(&encoded).expect("legacy policy block payload");

    assert_eq!(parsed.decision, "deny");
    assert_eq!(parsed.reason_code, "legacy_policy_block");
}

#[test]
fn policy_block_default_text_returns_machine_payload() {
    let state = test_state("zh-CN");
    let task = test_task(json!({
        "text": "读取 /etc/shadow 第一行"
    }));
    let encoded = policy_block_error(
        "path_outside_workspace",
        vec!["denied_path: /etc/shadow".to_string()],
        Vec::new(),
    );
    let parsed = parse_policy_block_error(&encoded).expect("policy block payload");
    let text = policy_block_default_text(&state, &task, "读取 /etc/shadow 第一行", &parsed);
    let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        payload
            .pointer("/permission_decision/decision")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
    assert_eq!(
        payload
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.policy.path_outside_workspace")
    );
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("path_outside_workspace")
    );
    assert_eq!(
        payload
            .pointer("/observed_facts/denied_path")
            .and_then(serde_json::Value::as_str),
        Some("/etc/shadow")
    );
}

// §E2 step1 ===============================================================
// 抽象 helper 才能稳定测：apply_skill_runner_env_isolation 直接读 std::env::vars()
// 在并发测试里读到的是 cargo runner 的环境，没法稳定断言；所以靠 collect 函数 +
// 显式 source map 验证白名单语义本身。

#[test]
fn skill_env_strict_defaults_on_and_accepts_explicit_opt_out() {
    let _guard = STRICT_ENV_TEST_LOCK.lock().expect("strict env test lock");
    // 暂存 + 清掉避免邻测污染
    let prev = std::env::var_os("RUSTCLAW_SKILL_ENV_STRICT");
    std::env::remove_var("RUSTCLAW_SKILL_ENV_STRICT");
    assert!(skill_runner_env_strict_enabled(), "default must be ON");

    std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", "");
    assert!(
        skill_runner_env_strict_enabled(),
        "empty value keeps default ON"
    );

    for val in ["0", "false", "FALSE", "off", "no"] {
        std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", val);
        assert!(
            !skill_runner_env_strict_enabled(),
            "RUSTCLAW_SKILL_ENV_STRICT={val:?} must opt out"
        );
    }

    // 恢复
    match prev {
        Some(v) => std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", v),
        None => std::env::remove_var("RUSTCLAW_SKILL_ENV_STRICT"),
    }
}

#[test]
fn skill_env_strict_on_for_truthy_values() {
    let _guard = STRICT_ENV_TEST_LOCK.lock().expect("strict env test lock");
    let prev = std::env::var_os("RUSTCLAW_SKILL_ENV_STRICT");
    for val in ["1", "true", "TRUE", "True", "on", "yes"] {
        std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", val);
        assert!(
            skill_runner_env_strict_enabled(),
            "RUSTCLAW_SKILL_ENV_STRICT={val:?} 应被识别为 ON"
        );
    }
    match prev {
        Some(v) => std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", v),
        None => std::env::remove_var("RUSTCLAW_SKILL_ENV_STRICT"),
    }
}

#[test]
fn whitelist_keeps_only_listed_keys_and_drops_secrets_or_unknown() {
    let source = vec![
        ("PATH", "/usr/bin:/bin"),
        ("HOME", "/home/u"),
        ("LANG", "en_US.UTF-8"),
        // 以下都不在白名单，必须被剥离
        ("OPENAI_API_KEY", "sk-fake-leak"),
        ("MINIMAX_API_KEY", "sk-fake-leak2"),
        ("MIMO_API_KEY", "sk-fake-leak3"),
        ("XIAOMI_API_KEY", "sk-fake-leak4"),
        ("RUSTCLAW_USER_KEY", "rk-leak"),
        ("DATABASE_URL", "postgres://leak"),
        ("AWS_ACCESS_KEY_ID", "AKIA..."),
    ];
    let kept = collect_whitelisted_env_pairs(source);
    let kept_keys: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kept_keys, vec!["HOME", "LANG", "PATH"], "字典序 + 仅白名单");
    for (k, _) in &kept {
        assert!(SKILL_RUNNER_ENV_WHITELIST.contains(&k.as_str()));
    }
}

#[test]
fn whitelist_drops_empty_value_to_avoid_silent_propagation() {
    let source = vec![
        ("PATH", "/usr/bin"),
        ("HOME", ""), // 空值不传，避免 skill 拿到 "" 又 fail-loud
        ("LC_ALL", "C"),
    ];
    let kept = collect_whitelisted_env_pairs(source);
    let kept_keys: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kept_keys, vec!["LC_ALL", "PATH"]);
}

#[test]
fn whitelist_does_not_invent_keys_for_missing_source() {
    let source: Vec<(&str, &str)> = vec![("UNRELATED", "x")];
    let kept = collect_whitelisted_env_pairs(source);
    assert!(kept.is_empty(), "没有白名单匹配时不应注入任何 env");
}

#[test]
fn whitelist_constant_does_not_include_obvious_secrets_or_clawd_specific_keys() {
    // §E2 step1 防回归：白名单不能不小心放进 API key / RustClaw 专属变量。
    let banned = [
        "OPENAI_API_KEY",
        "MINIMAX_API_KEY",
        "MIMO_API_KEY",
        "XIAOMI_API_KEY",
        "QWEN_API_KEY",
        "ANTHROPIC_API_KEY",
        "RUSTCLAW_USER_KEY",
        "RUSTCLAW_ADMIN_KEY",
        "DATABASE_URL",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
    ];
    for needle in banned {
        assert!(
            !SKILL_RUNNER_ENV_WHITELIST.contains(&needle),
            "{needle} 不能进白名单 —— 必须走 secrets broker 或 clawd 显式 env 注入"
        );
    }
}

#[tokio::test]
async fn run_cmd_does_not_inherit_undeclared_parent_secret() {
    let _guard = STRICT_ENV_TEST_LOCK.lock().expect("strict env test lock");
    let strict_before = std::env::var_os("RUSTCLAW_SKILL_ENV_STRICT");
    let secret_before = std::env::var_os("RUSTCLAW_TEST_PARENT_SECRET");
    std::env::remove_var("RUSTCLAW_SKILL_ENV_STRICT");
    std::env::set_var("RUSTCLAW_TEST_PARENT_SECRET", "must-not-reach-child");

    let output = run_safe_command(
        Path::new("."),
        "printf '%s' \"${RUSTCLAW_TEST_PARENT_SECRET-unset}\"",
        256,
        5,
        5,
        1024,
        false,
    )
    .await
    .expect("bounded run_cmd");
    assert_eq!(output, "unset");

    match strict_before {
        Some(value) => std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", value),
        None => std::env::remove_var("RUSTCLAW_SKILL_ENV_STRICT"),
    }
    match secret_before {
        Some(value) => std::env::set_var("RUSTCLAW_TEST_PARENT_SECRET", value),
        None => std::env::remove_var("RUSTCLAW_TEST_PARENT_SECRET"),
    }
}

#[path = "skills_tests/task_scoped_worktree.rs"]
mod task_scoped_worktree;
