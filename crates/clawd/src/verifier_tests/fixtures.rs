use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;

use crate::{
    AgentRuntimeConfig, AppState, ClaimedTask, IntentOutputContract, PlanKind, PlanResult,
    PlanStep, SkillViewsSnapshot, ToolsPolicy,
};

fn test_registry() -> SkillsRegistry {
    let toml = r#"
[[skills]]
name = "read_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "nullable_skill"
enabled = true
kind = "runner"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["expected_sha"], properties = { expected_sha = { anyOf = [ { type = "string", pattern = "^[0-9a-f]{40}$" }, { type = "null" } ] } } }

[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["command"], properties = { action = { type = "string" }, command = { type = "string" }, cwd = { type = "string" }, timeout_seconds = { type = "integer" }, idle_timeout_seconds = { type = "integer" }, max_output_bytes = { type = "integer" } } }
planner_capabilities = [
  { name = "system.preview_command_permission", action = "preview_command_permission", effect = "observe", required = ["command"], risk_level = "low", idempotent = true, dedup_scope = "args" },
  { name = "system.inspect_cli_help", action = "inspect_cli_help", effect = "observe", required = ["command"], risk_level = "low", idempotent = true, dedup_scope = "args" },
  { name = "system.run_command", effect = "external", required = ["command"], risk_level = "high", preferred = true, once_per_task = true, idempotent = false, dedup_scope = "action" },
]

[[skills]]
name = "list_dir"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path", "content"], properties = { path = { type = "string" }, content = { type = "string" } } }

[[skills]]
name = "make_dir"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "remove_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "fs_basic"
enabled = true
kind = "builtin"
planner_kind = "tool"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["action"], properties = { action = { type = "string", enum = ["stat_paths", "count_entries", "read_text_range", "write_text", "make_dir", "remove_path", "apply_patch"] }, path = { type = "string" }, paths = { type = "array", items = { type = "string" } }, content = { type = "string" }, patch = { type = "string" }, precondition_hashes = { type = "object" }, recursive = { type = "boolean" }, include_hidden = { type = "boolean" }, files_only = { type = "boolean" }, dirs_only = { type = "boolean" }, kind_filter = { type = "string", enum = ["any", "file", "dir"] }, count_files = { type = "boolean" }, count_dirs = { type = "boolean" }, ext_filter = { anyOf = [ { type = "string" }, { type = "array", items = { type = "string" } } ] }, start_line = { type = "integer" }, end_line = { type = "integer" } } }
planner_capabilities = [
  { name = "filesystem.stat_paths", action = "stat_paths", effect = "observe", required = ["path|paths"] },
  { name = "filesystem.count_entries", action = "count_entries", effect = "observe", required = ["path"], optional = ["recursive", "include_hidden", "files_only", "dirs_only", "kind_filter", "count_files", "count_dirs", "ext_filter"] },
  { name = "filesystem.read_text_range", action = "read_text_range", effect = "observe", required = ["path"] },
  { name = "filesystem.write_file", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", network_access = false, filesystem_write = true, external_publish = false, credential_access = false, package_install = false, privilege_escalation = false },
  { name = "filesystem.make_dir", action = "make_dir", effect = "mutate", required = ["path"], risk_level = "high" },
  { name = "filesystem.remove_path", action = "remove_path", effect = "mutate", required = ["path"], risk_level = "high" },
  { name = "workspace.apply_patch", action = "apply_patch", effect = "mutate", required = ["patch"], optional = ["precondition_hashes"], risk_level = "high", preferred = true, once_per_task = true, idempotent = false, dedup_scope = "args", isolation_profile = "local_current_workspace", network_access = false, filesystem_write = true, external_publish = false, credential_access = false, subprocess = true, package_install = false, privilege_escalation = false },
  { name = "filesystem.apply_patch", action = "apply_patch", effect = "mutate", required = ["patch"], optional = ["precondition_hashes"], risk_level = "high", once_per_task = true, idempotent = false, dedup_scope = "args", isolation_profile = "local_current_workspace", network_access = false, filesystem_write = true, external_publish = false, credential_access = false, subprocess = true, package_install = false, privilege_escalation = false },
]

[[skills]]
name = "system_basic"
enabled = true
kind = "runner"
planner_kind = "tool"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", properties = { action = { type = "string" }, kind = { type = "string", enum = ["current_user", "host_name", "kernel_release", "current_time", "current_working_directory"] } } }
planner_capabilities = [
  { name = "system.runtime_status", action = "runtime_status", effect = "observe", required = ["kind"], risk_level = "low", preferred = true },
]

[[skills]]
name = "config_basic"
enabled = true
kind = "builtin"
planner_kind = "tool"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["action"], properties = { action = { type = "string", enum = ["read_field", "read_fields"] }, path = { type = "string" }, field_path = { type = "string" }, field_paths = { type = "array", items = { type = "string" } } } }
planner_capabilities = [
  { name = "config.read_field", action = "read_field", effect = "observe", required = ["path", "field_path"], risk_level = "low", preferred = true, idempotent = true, dedup_scope = "args" },
  { name = "config.read_fields", action = "read_fields", effect = "observe", required = ["path", "field_paths"], risk_level = "low", preferred = true, idempotent = true, dedup_scope = "args" },
]

[[skills]]
name = "package_manager"
enabled = true
kind = "builtin"
planner_kind = "skill"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", properties = { action = { type = "string" }, package = { type = "string" }, dry_run = { type = "boolean" } } }
planner_capabilities = [
  { name = "package.detect_manager", action = "detect", effect = "observe" },
  { name = "package.install", action = "install", effect = "mutate", required = ["package"], risk_level = "high", subprocess = true, package_install = true, privilege_escalation = false },
]

[[skills]]
name = "db_basic"
enabled = true
kind = "builtin"
planner_kind = "skill"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", properties = { action = { type = "string" }, db_path = { type = "string" }, sql = { type = "string" }, confirm = { type = "boolean" } } }
planner_capabilities = [
  { name = "database.query", action = "sqlite_query", effect = "observe", required = ["sql"] },
  { name = "database.schema_version", action = "schema_version", effect = "observe" },
  { name = "database.execute", action = "sqlite_execute", effect = "mutate", required = ["sql", "confirm"], risk_level = "high" },
]

[[skills]]
name = "config_edit"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "text"
side_effect = true
requires_confirmation = true
auto_invocable = true
input_schema = { type = "object", properties = { action = { type = "string" }, path = { type = "string" }, field_path = { type = "string" }, value = { type = "string" } } }
planner_capabilities = [
  { name = "config.plan_change", action = "plan_config_change", effect = "observe", required = ["field_path", "value"], risk_level = "low" },
  { name = "config.apply_change", action = "apply_config_change", effect = "mutate", required = ["field_path", "value"], risk_level = "high" },
]

[[skills]]
name = "audio_synthesize"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["text"], properties = { text = { type = "string" }, output_path = { type = "string" } } }
planner_capabilities = [
  { name = "audio.synthesize", action = "synthesize", effect = "external", required = ["text"], optional = ["output_path"], risk_level = "high" },
]

[[skills]]
name = "image_generate"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "image"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["prompt"], properties = { prompt = { type = "string" }, output_path = { type = "string" }, dry_run = { type = "boolean" } } }
planner_capabilities = [
  { name = "image.generate", action = "generate", effect = "external", required = ["prompt"], optional = ["output_path", "dry_run"], risk_level = "high" },
]

[[skills]]
name = "image_edit"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "image"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["instruction"], properties = { instruction = { type = "string" }, image = { type = "string" }, output_path = { type = "string" } } }
planner_capabilities = [
  { name = "image.edit", action = "edit", effect = "external", required = ["instruction"], optional = ["image", "output_path"], risk_level = "high" },
]

[[skills]]
name = "weather"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "text"
input_schema = { type = "object", properties = { city = { type = "string" }, latitude = { type = "number" }, longitude = { type = "number" } } }
planner_capabilities = [
  { name = "weather.current", action = "query", effect = "observe", required = ["city|latitude+longitude"], risk_level = "low" },
]

[[skills]]
name = "media_download"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "file"
risk_level = "high"
auto_invocable = true
requires_confirmation = true
side_effect = true
confirmation_exempt_when = [
  { action = "resolve" },
  { action = "download" },
]
input_schema = { type = "object", required = ["action"], additionalProperties = false, properties = { action = { type = "string", enum = ["resolve", "download", "ocr"] }, share = { type = "string" }, text_conversion_scope = { type = "string", enum = ["images_only", "audio_only", "images_and_audio", "none"] }, input_paths = { type = "array", items = { type = "string" } } } }
planner_capabilities = [
  { name = "media_download.resolve", action = "resolve", effect = "observe", required = ["share"], risk_level = "low", isolation_profile = "local_current_workspace", network_access = true, filesystem_write = true, external_publish = false, credential_access = false, subprocess = true, package_install = false, privilege_escalation = false },
  { name = "media_download.download", action = "download", effect = "mutate", required = ["share"], optional = ["text_conversion_scope"], current_request_evidence = ["text_conversion_scope"], risk_level = "medium", isolation_profile = "local_current_workspace", network_access = true, filesystem_write = true, external_publish = false, credential_access = false, subprocess = true, package_install = false, privilege_escalation = false },
  { name = "media_download.ocr", action = "ocr", effect = "mutate", required = ["input_paths"], current_request_evidence = ["input_paths"], risk_level = "medium", isolation_profile = "local_current_workspace", network_access = true, filesystem_write = true, external_publish = false, credential_access = false, subprocess = true, package_install = false, privilege_escalation = false },
]

[[skills]]
name = "primary_reader"
enabled = true
kind = "runner"
output_kind = "text"
group = "reader"

	[[skills]]
	name = "fallback_reader"
	enabled = true
	kind = "runner"
	output_kind = "text"
	group = "reader"

	[[skills]]
	name = "photo_organize"
	enabled = true
	kind = "runner"
	output_kind = "text"
	risk_level = "high"
	auto_invocable = false
	requires_confirmation = true
	side_effect = true
	confirmation_exempt_when = [
	  { action = "prepare" },
	  { action = "organize", mode = "plan" },
	]

	[[skills]]
	name = "admin_hidden_probe"
	enabled = true
	planner_visible = false
	kind = "runner"
	output_kind = "text"
	side_effect = false
	"#;
    let path = std::env::temp_dir().join(format!(
        "verifier_registry_{}_{}_{}.toml",
        std::process::id(),
        crate::now_ts_u64(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, toml).expect("write registry");
    let registry = SkillsRegistry::load_from_path(&path).expect("load registry");
    let _ = std::fs::remove_file(path);
    registry
}

pub(in crate::verifier) fn test_state() -> AppState {
    let registry = Arc::new(test_registry());
    let skills_list = Arc::new(
        [
            "read_file",
            "nullable_skill",
            "run_cmd",
            "list_dir",
            "write_file",
            "make_dir",
            "fs_basic",
            "system_basic",
            "package_manager",
            "db_basic",
            "audio_synthesize",
            "image_generate",
            "image_edit",
            "weather",
            "media_download",
            "primary_reader",
            "fallback_reader",
            "photo_organize",
            "admin_hidden_probe",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>(),
    );
    let agents_by_id = HashMap::from([(
        crate::DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(std::sync::RwLock::new(Arc::new(agents_by_id))),
            agent_runtime_leases: Arc::new(std::sync::RwLock::new(HashMap::new())),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                binding: Default::default(),
                registry: Some(registry),
                skills_list,
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

pub(in crate::verifier) fn test_task() -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: "task-verify".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

pub(in crate::verifier) fn route_result() -> IntentOutputContract {
    crate::IntentOutputContract::default()
}

pub(in crate::verifier) fn route_result_with_contract(
    exact_command_output: bool,
) -> IntentOutputContract {
    let mut route = crate::IntentOutputContract {
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    if exact_command_output {
        route.configure_exact_command_output();
    }
    route
}

pub(in crate::verifier) fn plan_result(steps: Vec<PlanStep>) -> PlanResult {
    PlanResult {
        goal: "test".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps,
        planner_notes: String::new(),
        plan_kind: PlanKind::Single,
        raw_plan_text: String::new(),
    }
}
