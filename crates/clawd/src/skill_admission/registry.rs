use serde::Serialize;
use skill_sdk::{
    CapabilityActionRequest, HostPolicyGrant, HostRiskLevel, PackageManifest, RequestedEffect,
    RequestedExecutionMode,
};

use super::ExternalSkillMetadata;

#[derive(Debug, Serialize)]
struct RegistryFragment {
    skills: Vec<RegistryEntry>,
}

#[derive(Debug, Serialize)]
struct RegistryEntry {
    name: String,
    enabled: bool,
    planner_visible: bool,
    planner_eager_load: bool,
    kind: &'static str,
    planner_kind: &'static str,
    group: String,
    install_mode: &'static str,
    package_manifest: String,
    config_files: Vec<String>,
    aliases: Vec<String>,
    timeout_seconds: u64,
    prompt_file: String,
    output_kind: &'static str,
    description: String,
    semantic_tags: Vec<String>,
    risk_level: &'static str,
    auto_invocable: bool,
    requires_confirmation: bool,
    side_effect: bool,
    retryable: bool,
    supported_os: Vec<String>,
    capabilities: Vec<String>,
    planner_capabilities: Vec<RegistryCapability>,
    input_schema: toml::Value,
    output_schema: toml::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage: Option<RegistryStorage>,
}

#[derive(Debug, Serialize)]
struct RegistryStorage {
    kind: String,
    schema_version: u32,
    migration_owner: String,
}

#[derive(Debug, Serialize)]
struct RegistryCapability {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    effect: &'static str,
    required: Vec<String>,
    optional: Vec<String>,
    risk_level: &'static str,
    preferred: bool,
    once_per_task: bool,
    idempotent: bool,
    dedup_scope: &'static str,
    execution_mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u64>,
    network_access: bool,
    filesystem_write: bool,
    external_publish: bool,
    credential_access: bool,
    subprocess: bool,
    package_install: bool,
    privilege_escalation: bool,
}

pub(super) fn render_registry_fragment(
    manifest: &PackageManifest,
    metadata: &ExternalSkillMetadata,
    grant: &HostPolicyGrant,
    enabled: bool,
    planner_visible: bool,
    package_manifest_path: String,
    prompt_path: String,
) -> Result<String, String> {
    grant
        .validate_against(manifest)
        .map_err(|error| error.to_string())?;
    let request = manifest
        .effective_capability_request()
        .map_err(|error| error.to_string())?;
    let planner_capabilities = grant
        .capabilities
        .iter()
        .map(|granted| {
            let requested = request
                .capabilities
                .iter()
                .find(|candidate| {
                    candidate.name == granted.name && candidate.action == granted.action
                })
                .ok_or_else(|| {
                    format!(
                        "granted capability is absent from request: {} {:?}",
                        granted.name, granted.action
                    )
                })?;
            Ok(render_capability(requested, grant))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let timeout_seconds = request
        .capabilities
        .iter()
        .filter_map(|capability| capability.timeout_seconds)
        .max()
        .unwrap_or(manifest.run.timeout_seconds)
        .max(1);
    let capabilities = permission_tokens(grant);
    let semantic_tags = grant
        .capabilities
        .iter()
        .map(|capability| capability.name.clone())
        .collect();
    let storage = (manifest.storage.kind != "none").then(|| RegistryStorage {
        kind: manifest.storage.kind.clone(),
        schema_version: manifest.storage.schema_version,
        migration_owner: manifest.storage.migration_owner.clone(),
    });
    let entry = RegistryEntry {
        name: metadata.name.clone(),
        enabled,
        planner_visible,
        planner_eager_load: false,
        kind: "external",
        planner_kind: "skill",
        group: metadata.group.clone(),
        install_mode: "on_demand",
        package_manifest: package_manifest_path,
        config_files: manifest.lifecycle.config_files.clone(),
        aliases: metadata.aliases.clone(),
        timeout_seconds,
        prompt_file: prompt_path,
        output_kind: "text",
        description: metadata.description.clone(),
        semantic_tags,
        risk_level: risk_token(grant.risk_level),
        auto_invocable: grant.auto_invocable,
        requires_confirmation: grant.requires_confirmation,
        side_effect: request.capabilities.iter().any(|capability| {
            matches!(
                capability.effect,
                RequestedEffect::Mutate | RequestedEffect::External
            )
        }),
        retryable: true,
        supported_os: manifest.package.supported_os.clone(),
        capabilities,
        planner_capabilities,
        input_schema: json_schema_to_toml(&request.input_schema)?,
        output_schema: json_schema_to_toml(&request.output_schema)?,
        storage,
    };
    toml::to_string_pretty(&RegistryFragment {
        skills: vec![entry],
    })
    .map_err(|error| format!("serialize registry fragment failed: {error}"))
}

fn render_capability(
    request: &CapabilityActionRequest,
    grant: &HostPolicyGrant,
) -> RegistryCapability {
    RegistryCapability {
        name: request.name.clone(),
        action: request.action.clone(),
        description: request.description.clone(),
        effect: effect_token(request.effect),
        required: request.required.clone(),
        optional: request.optional.clone(),
        risk_level: risk_token(grant.risk_level),
        preferred: true,
        once_per_task: matches!(
            request.effect,
            RequestedEffect::Mutate | RequestedEffect::External
        ),
        idempotent: matches!(
            request.effect,
            RequestedEffect::Observe | RequestedEffect::Validate
        ),
        dedup_scope: "args",
        execution_mode: execution_mode_token(request.execution_mode),
        timeout_seconds: request.timeout_seconds,
        network_access: grant.permissions.network,
        filesystem_write: grant.permissions.filesystem_write,
        external_publish: grant.permissions.external_publish,
        credential_access: !grant.permissions.credential_refs.is_empty(),
        subprocess: grant.permissions.subprocess,
        package_install: grant.permissions.package_install,
        privilege_escalation: grant.permissions.privilege_escalation,
    }
}

fn permission_tokens(grant: &HostPolicyGrant) -> Vec<String> {
    let permissions = &grant.permissions;
    let mut tokens = Vec::new();
    if permissions.llm_gateway {
        tokens.push("llm".to_string());
    }
    if permissions.network {
        tokens.push("net".to_string());
    }
    if permissions.filesystem_read {
        tokens.push("fs.read".to_string());
    }
    if permissions.filesystem_write {
        tokens.push("fs.write".to_string());
    }
    if permissions.subprocess || permissions.package_install {
        tokens.push("exec".to_string());
    }
    if permissions.privilege_escalation {
        tokens.push("exec.sudo".to_string());
    }
    tokens.extend(
        permissions
            .credential_refs
            .iter()
            .map(|name| format!("secrets.{name}")),
    );
    tokens.sort();
    tokens.dedup();
    tokens
}

fn json_schema_to_toml(value: &serde_json::Value) -> Result<toml::Value, String> {
    toml::Value::try_from(value)
        .map_err(|error| format!("convert JSON schema to TOML failed: {error}"))
}

fn risk_token(risk: HostRiskLevel) -> &'static str {
    match risk {
        HostRiskLevel::Low => "low",
        HostRiskLevel::Medium => "medium",
        HostRiskLevel::High => "high",
    }
}

fn effect_token(effect: RequestedEffect) -> &'static str {
    match effect {
        RequestedEffect::Observe => "observe",
        RequestedEffect::Mutate => "mutate",
        RequestedEffect::Validate => "validate",
        RequestedEffect::External => "external",
    }
}

fn execution_mode_token(mode: RequestedExecutionMode) -> &'static str {
    match mode {
        RequestedExecutionMode::SyncShort => "sync_short",
        RequestedExecutionMode::AsyncPreferred => "async_preferred",
        RequestedExecutionMode::AsyncRequired => "async_required",
    }
}
