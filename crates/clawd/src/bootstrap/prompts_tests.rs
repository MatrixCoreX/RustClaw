use super::*;
use crate::{load_schedule_runtime, PolicyConfig};
use claw_core::config::ScheduleConfig;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_workspace(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("rustclaw_prompts_hot_reload_{name}_{unique}"));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let abs = root.join(rel);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).expect("mkdir parent");
    }
    std::fs::write(&abs, content).expect("write file");
}

fn write_minimal_config(root: &Path, persona_text: &str) {
    write_file(
        root,
        "configs/config.toml",
        r#"
[server]
listen = "127.0.0.1:0"
request_timeout_seconds = 30

[telegram]

[database]
sqlite_path = ":memory:"
busy_timeout_ms = 5000

[worker]

[persona]
profile = "executor"
dir = "prompts/personas"

[schedule]
timezone = "Asia/Shanghai"
intent_prompt_path = "prompts/schedule_intent_prompt.md"
intent_rules_path  = "prompts/schedule_intent_rules.md"
locale = "zh-CN"
i18n_dir = "configs/i18n"

[llm]
selected_vendor = "openai"
selected_model  = "gpt-4o-mini"
"#,
    );
    // persona file
    write_file(root, "prompts/personas/executor.md", persona_text);
    // schedule prompt + rules
    write_file(
        root,
        "prompts/schedule_intent_prompt.md",
        "<!-- Version: test.1 -->\nTEMPLATE_V1 __REQUEST__",
    );
    write_file(root, "prompts/schedule_intent_rules.md", "RULES_V1");
    // schedule i18n (load_schedule_runtime tries it; missing falls back to defaults)
    write_file(
        root,
        "configs/i18n/schedule.zh-CN.toml",
        "[dict]\n\"schedule.desc.daily\" = \"daily {time}\"\n",
    );
}

#[test]
fn reload_runtime_prompts_swaps_persona_and_schedule() {
    let root = temp_workspace("swap");
    write_minimal_config(&root, "PERSONA_V1");

    let policy = PolicyConfig::test_default();
    policy.replace_persona_prompt("PERSONA_V0".to_string());
    policy
        .schedule
        .replace_intent_prompt_template("OLD_TEMPLATE".to_string());
    policy
        .schedule
        .replace_intent_rules_template("OLD_RULES".to_string());

    let cfg_path = root.join("configs/config.toml");
    let report = reload_runtime_prompts_impl(&root, &policy, cfg_path.to_str().unwrap());

    assert!(report.config_reread_ok);
    assert_eq!(policy.persona_prompt_string(), "PERSONA_V1");
    assert!(policy
        .schedule
        .intent_prompt_template_string()
        .contains("TEMPLATE_V1"));
    assert_eq!(policy.schedule.intent_rules_template_string(), "RULES_V1");

    write_file(
        &root,
        "prompts/personas/executor.md",
        "PERSONA_V2_AFTER_EDIT",
    );
    write_file(
        &root,
        "prompts/schedule_intent_prompt.md",
        "<!-- Version: test.2 -->\nTEMPLATE_V2 __REQUEST__",
    );
    write_file(&root, "prompts/schedule_intent_rules.md", "RULES_V2");

    let report2 = reload_runtime_prompts_impl(&root, &policy, cfg_path.to_str().unwrap());
    assert!(report2.config_reread_ok);
    assert_eq!(policy.persona_prompt_string(), "PERSONA_V2_AFTER_EDIT");
    assert!(policy
        .schedule
        .intent_prompt_template_string()
        .contains("TEMPLATE_V2"));
    assert_eq!(policy.schedule.intent_rules_template_string(), "RULES_V2");
}

#[test]
fn load_schedule_runtime_merges_locale_i18n_files() {
    let root = temp_workspace("merged_i18n");
    write_file(
        &root,
        "prompts/schedule_intent_prompt.md",
        "<!-- Version: test.1 -->\nTEMPLATE __REQUEST__",
    );
    write_file(&root, "prompts/schedule_intent_rules.md", "RULES");
    write_file(
        &root,
        "configs/i18n/schedule.zh-CN.toml",
        "[dict]\n\"schedule.desc.daily\" = \"SCHEDULE_DAILY_ZH\"\n",
    );
    write_file(
        &root,
        "configs/i18n/crypto.zh-CN.toml",
        "[dict]\ncrypto.err.account_access_failed = \"CRYPTO_ACCOUNT_ACCESS_ZH\"\n",
    );

    let cfg = ScheduleConfig {
        intent_prompt_path: "prompts/schedule_intent_prompt.md".to_string(),
        intent_rules_path: "prompts/schedule_intent_rules.md".to_string(),
        locale: "zh-CN".to_string(),
        i18n_dir: "configs/i18n".to_string(),
        ..ScheduleConfig::default()
    };
    let runtime = load_schedule_runtime(&root, &cfg, None).expect("schedule runtime");

    assert_eq!(
        runtime.i18n_dict.get("schedule.desc.daily"),
        Some(&"SCHEDULE_DAILY_ZH".to_string())
    );
    assert_eq!(
        runtime.i18n_dict.get("crypto.err.account_access_failed"),
        Some(&"CRYPTO_ACCOUNT_ACCESS_ZH".to_string())
    );
}

#[test]
fn reload_runtime_prompts_keeps_state_when_config_missing() {
    let root = temp_workspace("missing_config");
    let policy = PolicyConfig::test_default();
    policy.replace_persona_prompt("KEEP_ME".to_string());

    let bad_path = root.join("configs/does_not_exist.toml");
    let report = reload_runtime_prompts_impl(&root, &policy, bad_path.to_str().unwrap());

    assert!(!report.config_reread_ok);
    assert_eq!(policy.persona_prompt_string(), "KEEP_ME");
    assert_eq!(report.persona_chars_before, report.persona_chars_after);
}

#[test]
fn reload_runtime_prompts_propagates_to_clones_via_arc_rwlock() {
    let root = temp_workspace("clone_propagate");
    write_minimal_config(&root, "SHARED_V1");

    let policy = PolicyConfig::test_default();
    // §3.5d: PolicyConfig is `#[derive(Clone)]`; the persona_prompt /
    // schedule.intent_prompt_template / intent_rules_template fields are
    // `Arc<RwLock<String>>` so a clone should observe writes through the
    // original handle (this is the property axum's AppState clone path
    // depends on).
    let cloned_policy = policy.clone();

    let cfg_path = root.join("configs/config.toml");
    reload_runtime_prompts_impl(&root, &policy, cfg_path.to_str().unwrap());

    assert_eq!(cloned_policy.persona_prompt_string(), "SHARED_V1");
    assert!(cloned_policy
        .schedule
        .intent_prompt_template_string()
        .contains("TEMPLATE_V1"));
    assert_eq!(
        cloned_policy.schedule.intent_rules_template_string(),
        "RULES_V1"
    );
}

#[test]
fn reload_runtime_prompts_keeps_schedule_when_schedule_prompt_missing() {
    let root = temp_workspace("schedule_missing");
    write_minimal_config(&root, "PERSONA_V1");

    let policy = PolicyConfig::test_default();
    policy.replace_persona_prompt("PERSONA_KEEP".to_string());
    policy
        .schedule
        .replace_intent_prompt_template("SCHEDULE_KEEP".to_string());
    policy
        .schedule
        .replace_intent_rules_template("RULES_KEEP".to_string());

    let cfg_path = root.join("configs/config.toml");
    std::fs::remove_file(root.join("prompts/schedule_intent_prompt.md"))
        .expect("remove schedule prompt");

    let report = reload_runtime_prompts_impl(&root, &policy, cfg_path.to_str().unwrap());
    assert!(!report.config_reread_ok);
    assert_eq!(policy.persona_prompt_string(), "PERSONA_KEEP");
    assert_eq!(
        policy.schedule.intent_prompt_template_string(),
        "SCHEDULE_KEEP"
    );
    assert_eq!(policy.schedule.intent_rules_template_string(), "RULES_KEEP");
}

#[test]
fn strict_prompt_validation_error_lists_missing_prompts() {
    let report = PromptValidationReport {
        checked: 17,
        active_llm_vendor: Some("mimo".to_string()),
        vendor: "minimax".to_string(),
        missing: vec![PromptValidationIssue {
            logical_path: "prompts/loop_incremental_plan_prompt.md".to_string(),
            label: "loop_incremental_plan (agent_engine.planning)".to_string(),
            resolved_disk_path: "prompts/loop_incremental_plan_prompt.md".to_string(),
        }],
    };

    let message = strict_prompt_validation_error(&report)
        .expect("strict mode should return an error message when prompts are missing");
    assert!(message.contains("strict mode blocked startup"));
    assert!(message.contains("active_llm_vendor=mimo"));
    assert!(message.contains("prompt_vendor_patch=minimax"));
    assert!(message.contains("loop_incremental_plan (agent_engine.planning)"));
    assert!(message.contains("prompts/loop_incremental_plan_prompt.md"));
}

#[test]
fn visible_response_prompts_keep_agent_identity_boundary() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for rel in [
        "chat_response_prompt.md",
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt = std::fs::read_to_string(overlays.join(rel)).expect("read prompt overlay");
        assert!(
            prompt.contains("agent/runtime identity"),
            "{rel} should keep the agent identity boundary"
        );
        assert!(
            prompt.contains("provider/model"),
            "{rel} should keep provider/model separated from identity"
        );
        assert!(
            prompt.contains("backend metadata"),
            "{rel} should describe provider/model as backend metadata"
        );
        assert!(
            prompt.contains("__AGENT_RUNTIME_IDENTITY__"),
            "{rel} should render the runtime identity machine fact"
        );
    }
}

#[test]
fn single_plan_prompt_keeps_low_risk_drafting_nonblocking_rule() {
    let prompt = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../prompts/layers/overlays/single_plan_execution_prompt.md"),
    )
    .expect("read single plan prompt overlay");

    assert!(prompt.contains("low-risk chat-only drafting"));
    assert!(prompt.contains("do not use a blocking clarification"));
    assert!(prompt.contains("neutral assumptions"));
    assert!(prompt.contains("requested output shape"));
}
