use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::policy_decision::PolicyDecision;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SubagentRoleDefinition {
    pub(crate) token: String,
    pub(crate) family: String,
    pub(crate) default_scope: String,
    pub(crate) default_permission_profile: String,
    pub(crate) allowed_permission_profiles: Vec<String>,
    pub(crate) result_contract_required: bool,
    pub(crate) model_policy: ModelPolicy,
    pub(crate) tool_policy: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ModelPolicy {
    pub(crate) model_class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_output_tokens: Option<u64>,
}

impl Default for ModelPolicy {
    fn default() -> Self {
        Self {
            model_class: "default".to_string(),
            reasoning_effort: None,
            max_output_tokens: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ResolvedModelPolicy {
    pub(crate) schema_version: u8,
    pub(crate) model_class: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_output_tokens: Option<u64>,
    pub(crate) model_policy_fallback: bool,
    pub(crate) reasoning_effort_status: String,
}

pub(crate) fn parse_model_policy(value: Value) -> Result<ModelPolicy, &'static str> {
    let policy: ModelPolicy =
        serde_json::from_value(value).map_err(|_| "subagent_model_policy_schema_invalid")?;
    if !matches!(
        policy.model_class.as_str(),
        "default" | "fast" | "reasoning"
    ) {
        return Err("subagent_model_class_invalid");
    }
    if policy
        .reasoning_effort
        .as_deref()
        .is_some_and(|value| !matches!(value, "low" | "medium" | "high"))
    {
        return Err("subagent_reasoning_effort_invalid");
    }
    if policy
        .max_output_tokens
        .is_some_and(|value| value == 0 || value > 1_000_000)
    {
        return Err("subagent_max_output_tokens_invalid");
    }
    Ok(policy)
}

pub(crate) fn resolve_model_policy(
    llm: &claw_core::config::LlmConfig,
    policy: &ModelPolicy,
) -> Result<ResolvedModelPolicy, &'static str> {
    let requested = llm.model_classes.get(&policy.model_class);
    let fallback_mapping = llm.model_classes.get("default");
    let mapping = requested.or(fallback_mapping);
    let (provider, model, fallback) = if let Some(mapping) = mapping {
        (
            mapping.provider.trim().to_string(),
            mapping.model.trim().to_string(),
            requested.is_none(),
        )
    } else {
        (
            llm.selected_vendor
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or("subagent_default_model_provider_missing")?
                .to_string(),
            llm.selected_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or("subagent_default_model_id_missing")?
                .to_string(),
            true,
        )
    };
    if provider.is_empty() || model.is_empty() {
        return Err("subagent_model_class_mapping_invalid");
    }
    let reasoning_effort_status = match (policy.reasoning_effort.as_deref(), provider.as_str()) {
        (None, _) => "not_requested",
        (Some(_), "google") => "ignored_unsupported",
        (Some(_), _) => "applied",
    };
    Ok(ResolvedModelPolicy {
        schema_version: 1,
        model_class: policy.model_class.clone(),
        provider,
        model,
        reasoning_effort: policy.reasoning_effort.clone(),
        max_output_tokens: policy.max_output_tokens,
        model_policy_fallback: fallback,
        reasoning_effort_status: reasoning_effort_status.to_string(),
    })
}

impl SubagentRoleDefinition {
    pub(crate) fn allows_permission_profile(&self, profile: &str) -> bool {
        self.allowed_permission_profiles
            .iter()
            .any(|allowed| allowed == profile)
    }
}

pub(crate) fn default_subagent_role_definitions() -> Vec<SubagentRoleDefinition> {
    [
        ("observe", "explorer", "read_only_discovery", false),
        ("explorer", "explorer", "read_only_discovery", false),
        ("worker", "worker", "read_only_worker", false),
        ("writer", "worker", "isolated_worktree_write", true),
        ("review", "reviewer", "read_only_review", true),
        ("reviewer", "reviewer", "read_only_review", true),
        ("test", "verifier", "read_only_verification", true),
        ("verifier", "verifier", "read_only_verification", true),
    ]
    .into_iter()
    .map(|(token, family, default_scope, result_contract_required)| {
        let writer = token == "writer";
        SubagentRoleDefinition {
            token: token.to_string(),
            family: family.to_string(),
            default_scope: default_scope.to_string(),
            default_permission_profile: if writer {
                "local_worktree".to_string()
            } else {
                "read_only".to_string()
            },
            allowed_permission_profiles: if writer {
                vec!["local_worktree".to_string()]
            } else if matches!(token, "worker" | "test") {
                vec!["read_only".to_string(), "local_worktree".to_string()]
            } else {
                vec!["read_only".to_string()]
            },
            result_contract_required,
            model_policy: ModelPolicy::default(),
            tool_policy: json!({}),
        }
    })
    .collect()
}

pub(crate) fn load_subagent_role_definitions(path: &Path) -> Vec<SubagentRoleDefinition> {
    let defaults = default_subagent_role_definitions();
    let Some(root) = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<toml::Value>(&raw).ok())
    else {
        return defaults;
    };
    let Some(role_table) = root
        .get("agent")
        .and_then(|agent| agent.get("subagents"))
        .and_then(|subagents| subagents.get("role_definitions"))
        .and_then(toml::Value::as_table)
    else {
        return defaults;
    };
    let mut definitions = Vec::new();
    for (raw_token, raw_definition) in role_table {
        let token = normalize_machine_token(raw_token);
        let Some(table) = raw_definition.as_table() else {
            continue;
        };
        if !valid_machine_token(&token) {
            continue;
        }
        let Some(family) = table
            .get("family")
            .and_then(toml::Value::as_str)
            .map(normalize_machine_token)
            .filter(|value| valid_machine_token(value))
        else {
            continue;
        };
        let Some(default_scope) = table
            .get("default_scope")
            .and_then(toml::Value::as_str)
            .map(normalize_machine_token)
            .filter(|value| valid_machine_token(value))
        else {
            continue;
        };
        let Some(default_permission_profile) = table
            .get("default_permission_profile")
            .and_then(toml::Value::as_str)
            .map(normalize_machine_token)
            .filter(|value| valid_permission_profile(value))
        else {
            continue;
        };
        let mut allowed_permission_profiles = table
            .get("allowed_permission_profiles")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .map(normalize_machine_token)
            .filter(|value| valid_permission_profile(value))
            .collect::<Vec<_>>();
        if allowed_permission_profiles.is_empty() {
            allowed_permission_profiles.push(default_permission_profile.clone());
        }
        if !allowed_permission_profiles
            .iter()
            .any(|profile| profile == &default_permission_profile)
        {
            continue;
        }
        definitions.push(SubagentRoleDefinition {
            token,
            family,
            default_scope,
            default_permission_profile,
            allowed_permission_profiles,
            result_contract_required: table
                .get("result_contract_required")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false),
            model_policy: match table.get("model_policy") {
                Some(value) => match toml_to_json_object(value)
                    .and_then(|value| parse_model_policy(value).ok())
                {
                    Some(policy) => policy,
                    None => continue,
                },
                None => ModelPolicy::default(),
            },
            tool_policy: table
                .get("tool_policy")
                .and_then(toml_to_json_object)
                .unwrap_or_else(|| json!({})),
        });
    }
    definitions.sort_by(|left, right| left.token.cmp(&right.token));
    definitions.dedup_by(|left, right| left.token == right.token);
    if definitions.is_empty() {
        defaults
    } else {
        definitions
    }
}

pub(crate) fn runtime_protocol_hint_line(definitions: &[SubagentRoleDefinition]) -> String {
    let roles = definitions
        .iter()
        .map(|definition| definition.token.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let hooks = crate::agent_hooks::HookStage::all()
        .iter()
        .map(|stage| stage.as_token())
        .collect::<Vec<_>>()
        .join("|");
    let policy_decisions = PolicyDecision::all_tokens().join("|");
    format!(
        "agent_runtime_protocols=subagent_roles:{roles};subagent_inline_write_enabled:false;subagent_persistent_worktree_write_enabled:true;subagent_external_publish_enabled:false;hook_stages:{hooks};hook_decisions:{policy_decisions}"
    )
}

fn normalize_machine_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn valid_machine_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn valid_permission_profile(value: &str) -> bool {
    matches!(
        value,
        "read_only"
            | "local_current_workspace"
            | "local_worktree"
            | "local_temp_workspace"
            | "remote_executor"
    )
}

fn toml_to_json_object(value: &toml::Value) -> Option<Value> {
    let value = serde_json::to_value(value).ok()?;
    value.is_object().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_protocol_hint_exposes_trusted_roles_and_hook_stages() {
        let definitions = default_subagent_role_definitions();
        let hint = runtime_protocol_hint_line(&definitions);
        let expected_roles = format!(
            "subagent_roles:{}",
            definitions
                .iter()
                .map(|definition| definition.token.as_str())
                .collect::<Vec<_>>()
                .join("|")
        );
        assert!(hint.contains(&expected_roles));
        assert!(hint.contains("subagent_inline_write_enabled:false"));
        assert!(hint.contains("subagent_persistent_worktree_write_enabled:true"));
        assert!(hint.contains("subagent_external_publish_enabled:false"));
        let expected_hook_stages = format!(
            "hook_stages:{}",
            crate::agent_hooks::HookStage::all()
                .iter()
                .map(|stage| stage.as_token())
                .collect::<Vec<_>>()
                .join("|")
        );
        assert!(hint.contains(&expected_hook_stages));
        let expected_hook_decisions =
            format!("hook_decisions:{}", PolicyDecision::all_tokens().join("|"));
        assert!(hint.contains(&expected_hook_decisions));
    }

    #[test]
    fn trusted_config_can_define_role_without_rust_branch() {
        let path =
            std::env::temp_dir().join(format!("roles_{}.toml", uuid::Uuid::new_v4().simple()));
        std::fs::write(
            &path,
            r#"
[agent.subagents.role_definitions.architect]
family = "planner"
default_scope = "read_only_architecture"
default_permission_profile = "read_only"
allowed_permission_profiles = ["read_only"]
result_contract_required = true
[agent.subagents.role_definitions.architect.model_policy]
model_class = "reasoning"
[agent.subagents.role_definitions.architect.tool_policy]
policy_class = "repository_read"
"#,
        )
        .expect("write role config");
        let definitions = load_subagent_role_definitions(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].token, "architect");
        assert_eq!(definitions[0].family, "planner");
        assert_eq!(definitions[0].default_scope, "read_only_architecture");
        assert_eq!(definitions[0].model_policy.model_class, "reasoning");
        assert_eq!(
            definitions[0].tool_policy["policy_class"],
            "repository_read"
        );
    }

    #[test]
    fn model_policy_schema_is_closed_and_validated() {
        assert_eq!(
            parse_model_policy(json!({"model_class":"reasoning","reasoning_effort":"high","max_output_tokens":8192}))
                .expect("valid policy")
                .reasoning_effort
                .as_deref(),
            Some("high")
        );
        assert_eq!(
            parse_model_policy(json!({"model_class":"reasoning","unexpected":true})),
            Err("subagent_model_policy_schema_invalid")
        );
        assert_eq!(
            parse_model_policy(json!({"reasoning_effort":"extreme"})),
            Err("subagent_reasoning_effort_invalid")
        );
    }

    #[test]
    fn repository_review_role_resolves_the_reasoning_model_and_effort() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let definitions =
            load_subagent_role_definitions(&repository_root.join("configs/agent_guard.toml"));
        let review = definitions
            .iter()
            .find(|definition| definition.token == "review")
            .expect("review role");
        let config_path = repository_root
            .join("configs/config.toml")
            .to_string_lossy()
            .into_owned();
        let config = claw_core::config::AppConfig::load(&config_path)
            .expect("repository_runtime_config_missing");
        let resolved = resolve_model_policy(&config.llm, &review.model_policy)
            .expect("review_model_policy_resolution_failed");
        assert_eq!(resolved.model_class, "reasoning");
        assert_eq!(resolved.provider, "minimax");
        assert_eq!(resolved.model, "MiniMax-M3");
        assert_eq!(resolved.reasoning_effort.as_deref(), Some("high"));
        assert!(!resolved.model_policy_fallback);
    }
}
