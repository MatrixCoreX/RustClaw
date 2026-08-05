use claw_core::types::AuthIdentity;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemorySettingMode {
    Inherit,
    Enabled,
    Disabled,
}

impl MemorySettingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "inherit" => Ok(Self::Inherit),
            "enabled" => Ok(Self::Enabled),
            "disabled" => Ok(Self::Disabled),
            _ => anyhow::bail!("memory_settings_mode_invalid"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExternalContextPolicy {
    Inherit,
    Exclude,
    EvidenceOnly,
    Allow,
}

impl ExternalContextPolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Exclude => "exclude",
            Self::EvidenceOnly => "evidence_only",
            Self::Allow => "allow",
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "inherit" => Ok(Self::Inherit),
            "exclude" => Ok(Self::Exclude),
            "evidence_only" => Ok(Self::EvidenceOnly),
            "allow" => Ok(Self::Allow),
            _ => anyhow::bail!("memory_external_context_policy_invalid"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemorySettingScope {
    Admin,
    Principal,
    Conversation,
}

impl Default for MemorySettingScope {
    fn default() -> Self {
        Self::Principal
    }
}

impl MemorySettingScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Principal => "principal",
            Self::Conversation => "conversation",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct MemorySettingsUpdateRequest {
    #[serde(default)]
    pub(crate) scope: MemorySettingScope,
    pub(crate) target_principal_id: Option<String>,
    pub(crate) conversation_id: Option<String>,
    pub(crate) use_mode: Option<MemorySettingMode>,
    pub(crate) generate_mode: Option<MemorySettingMode>,
    pub(crate) external_context_policy: Option<ExternalContextPolicy>,
    pub(crate) expected_revision: Option<i64>,
    pub(crate) long_term_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct MemoryRequestedSettings {
    pub(crate) use_mode: MemorySettingMode,
    pub(crate) generate_mode: MemorySettingMode,
    pub(crate) external_context_policy: ExternalContextPolicy,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct MemoryEffectiveSettings {
    pub(crate) schema_version: u32,
    pub(crate) scope: MemorySettingScope,
    pub(crate) target_principal_id: String,
    pub(crate) conversation_id: Option<String>,
    pub(crate) requested: MemoryRequestedSettings,
    pub(crate) use_memory: bool,
    pub(crate) generate_memory: bool,
    pub(crate) external_context_policy: ExternalContextPolicy,
    pub(crate) use_source: String,
    pub(crate) generate_source: String,
    pub(crate) external_context_source: String,
    pub(crate) managed_deny_reason: Option<String>,
    pub(crate) revision: i64,
    pub(crate) policy_digest: String,
    pub(crate) restart_required: bool,
}

#[derive(Debug, Clone)]
struct SettingsRow {
    use_mode: MemorySettingMode,
    generate_mode: MemorySettingMode,
    external_context_policy: ExternalContextPolicy,
    managed_deny_use: bool,
    managed_deny_generate: bool,
    revision: i64,
}

pub(crate) fn resolve_memory_settings(
    db: &Connection,
    principal_id: &str,
    conversation_id: Option<&str>,
    release_default: bool,
) -> anyhow::Result<MemoryEffectiveSettings> {
    resolve_for_scope(
        db,
        MemorySettingScope::Conversation,
        principal_id,
        conversation_id,
        release_default,
    )
}

pub(crate) fn resolve_principal_memory_settings(
    db: &Connection,
    principal_id: &str,
    release_default: bool,
) -> anyhow::Result<MemoryEffectiveSettings> {
    resolve_for_scope(
        db,
        MemorySettingScope::Principal,
        principal_id,
        None,
        release_default,
    )
}

pub(crate) fn resolve_task_memory_settings(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
) -> anyhow::Result<Option<MemoryEffectiveSettings>> {
    let Some(user_key) = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("memory_settings_db_pool_failed:{error}"))?;
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, user_key)?
        .ok_or_else(|| anyhow::anyhow!("memory_settings_principal_not_found"))?;
    let conversation_id = serde_json::from_str::<serde_json::Value>(&task.payload_json)
        .ok()
        .and_then(|payload| {
            payload
                .get("conversation_id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        });
    resolve_memory_settings(
        &db,
        &principal_id,
        conversation_id.as_deref(),
        state.policy.memory.long_term_enabled,
    )
    .map(Some)
}

pub(crate) fn task_memory_use_enabled(state: &crate::AppState, task: &crate::ClaimedTask) -> bool {
    match resolve_task_memory_settings(state, task) {
        Ok(Some(settings)) => settings.use_memory,
        Ok(None) => state.policy.memory.long_term_enabled,
        Err(_) => false,
    }
}

pub(crate) fn task_memory_generation_enabled(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
) -> bool {
    match resolve_task_memory_settings(state, task) {
        Ok(Some(settings)) => settings.generate_memory,
        Ok(None) => state.policy.memory.long_term_enabled,
        Err(_) => false,
    }
}

pub(crate) fn revocation_fenced_task_memory_settings(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    pinned: Option<&MemoryEffectiveSettings>,
) -> Option<MemoryEffectiveSettings> {
    let mut snapshot = pinned
        .cloned()
        .or_else(|| resolve_task_memory_settings(state, task).ok().flatten())?;
    let Ok(Some(current)) = resolve_task_memory_settings(state, task) else {
        snapshot.use_memory = false;
        snapshot.generate_memory = false;
        snapshot.use_source = "revocation_fence_resolution_failed".to_string();
        snapshot.generate_source = "revocation_fence_resolution_failed".to_string();
        return Some(snapshot);
    };
    apply_revocation_fence(&mut snapshot, &current);
    Some(snapshot)
}

fn apply_revocation_fence(
    snapshot: &mut MemoryEffectiveSettings,
    current: &MemoryEffectiveSettings,
) {
    if !current.use_memory {
        snapshot.use_memory = false;
        snapshot.use_source = "revocation_fence".to_string();
    }
    if !current.generate_memory {
        snapshot.generate_memory = false;
        snapshot.generate_source = "revocation_fence".to_string();
    }
    if external_policy_rank(current.external_context_policy)
        < external_policy_rank(snapshot.external_context_policy)
    {
        snapshot.external_context_policy = current.external_context_policy;
        snapshot.external_context_source = "revocation_fence".to_string();
    }
    if current.managed_deny_reason.is_some() {
        snapshot.managed_deny_reason = current.managed_deny_reason.clone();
    }
}

fn external_policy_rank(policy: ExternalContextPolicy) -> u8 {
    match policy {
        ExternalContextPolicy::Exclude | ExternalContextPolicy::Inherit => 0,
        ExternalContextPolicy::EvidenceOnly => 1,
        ExternalContextPolicy::Allow => 2,
    }
}

pub(crate) fn update_memory_settings(
    db: &Connection,
    actor: &AuthIdentity,
    request: &MemorySettingsUpdateRequest,
    release_default: bool,
) -> anyhow::Result<MemoryEffectiveSettings> {
    let expected_revision = request
        .expected_revision
        .ok_or_else(|| anyhow::anyhow!("memory_settings_revision_required"))?;
    anyhow::ensure!(expected_revision >= 0, "memory_settings_revision_invalid");
    let target_principal_id = target_principal(actor, request)?;
    let conversation_id =
        normalize_conversation_id(request.scope, request.conversation_id.as_deref())?;
    let setting_key = setting_key(
        request.scope,
        &target_principal_id,
        conversation_id.as_deref(),
    );
    let tx = db.unchecked_transaction()?;
    ensure_target_principal(&tx, request.scope, &target_principal_id)?;
    let existing = load_settings_row(&tx, &setting_key)?;
    let current_revision = existing.as_ref().map_or(0, |row| row.revision);
    anyhow::ensure!(
        current_revision == expected_revision,
        "memory_settings_revision_conflict"
    );
    let legacy_mode = request.long_term_enabled.map(|enabled| {
        if enabled {
            MemorySettingMode::Enabled
        } else {
            MemorySettingMode::Disabled
        }
    });
    let use_mode = request
        .use_mode
        .or(legacy_mode)
        .or_else(|| existing.as_ref().map(|row| row.use_mode))
        .unwrap_or(MemorySettingMode::Inherit);
    let generate_mode = request
        .generate_mode
        .or(legacy_mode)
        .or_else(|| existing.as_ref().map(|row| row.generate_mode))
        .unwrap_or(MemorySettingMode::Inherit);
    let external_context_policy = request
        .external_context_policy
        .or_else(|| existing.as_ref().map(|row| row.external_context_policy))
        .unwrap_or(ExternalContextPolicy::Inherit);
    let next_revision = current_revision + 1;
    let now = crate::now_ts();
    tx.execute(
        "INSERT INTO memory_runtime_settings (
            setting_key, setting_scope, principal_id, conversation_id,
            use_mode, generate_mode, external_context_policy,
            managed_deny_use, managed_deny_generate, revision, policy_digest,
            updated_at, updated_by_principal_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, ?8, 'pending', ?9, ?10)
         ON CONFLICT(setting_key) DO UPDATE SET
            use_mode = excluded.use_mode,
            generate_mode = excluded.generate_mode,
            external_context_policy = excluded.external_context_policy,
            revision = excluded.revision,
            updated_at = excluded.updated_at,
            updated_by_principal_id = excluded.updated_by_principal_id",
        params![
            setting_key,
            request.scope.as_str(),
            if matches!(request.scope, MemorySettingScope::Admin) {
                None::<&str>
            } else {
                Some(target_principal_id.as_str())
            },
            conversation_id,
            use_mode.as_str(),
            generate_mode.as_str(),
            external_context_policy.as_str(),
            next_revision,
            now,
            actor.principal_id,
        ],
    )?;
    let effective = resolve_for_scope(
        &tx,
        request.scope,
        &target_principal_id,
        conversation_id.as_deref(),
        release_default,
    )?;
    tx.execute(
        "UPDATE memory_runtime_settings SET policy_digest = ?2 WHERE setting_key = ?1",
        params![setting_key, effective.policy_digest],
    )?;
    if !matches!(request.scope, MemorySettingScope::Admin) {
        tx.execute(
            "UPDATE memory_onboarding_state
             SET status = 'completed', updated_at = ?1
             WHERE singleton_id = 1 AND status = 'pending_choice'",
            [crate::now_ts()],
        )?;
    }
    tx.commit()?;
    Ok(effective)
}

fn resolve_for_scope(
    db: &Connection,
    scope: MemorySettingScope,
    principal_id: &str,
    conversation_id: Option<&str>,
    release_default: bool,
) -> anyhow::Result<MemoryEffectiveSettings> {
    let admin = load_settings_row(db, "admin:default")?;
    let principal_key = setting_key(MemorySettingScope::Principal, principal_id, None);
    let principal = load_settings_row(db, &principal_key)?;
    let conversation_key = conversation_id.map(|conversation_id| {
        setting_key(
            MemorySettingScope::Conversation,
            principal_id,
            Some(conversation_id),
        )
    });
    let conversation = conversation_key
        .as_deref()
        .map(|key| load_settings_row(db, key))
        .transpose()?
        .flatten();
    let target_key = setting_key(scope, principal_id, conversation_id);
    let target = load_settings_row(db, &target_key)?;
    let (mut use_memory, mut use_source) = resolve_mode(
        release_default,
        admin.as_ref().map(|row| row.use_mode),
        principal.as_ref().map(|row| row.use_mode),
        conversation.as_ref().map(|row| row.use_mode),
    );
    let (mut generate_memory, mut generate_source) = resolve_mode(
        release_default,
        admin.as_ref().map(|row| row.generate_mode),
        principal.as_ref().map(|row| row.generate_mode),
        conversation.as_ref().map(|row| row.generate_mode),
    );
    let mut deny_reasons = Vec::new();
    if admin.as_ref().is_some_and(|row| row.managed_deny_use) {
        use_memory = false;
        use_source = "admin_managed_deny".to_string();
        deny_reasons.push("memory_use_managed_denied");
    }
    if admin.as_ref().is_some_and(|row| row.managed_deny_generate) {
        generate_memory = false;
        generate_source = "admin_managed_deny".to_string();
        deny_reasons.push("memory_generate_managed_denied");
    }
    let (external_context_policy, external_context_source) = resolve_external_policy(
        admin.as_ref().map(|row| row.external_context_policy),
        principal.as_ref().map(|row| row.external_context_policy),
        conversation.as_ref().map(|row| row.external_context_policy),
    );
    let requested = target
        .as_ref()
        .map(|row| MemoryRequestedSettings {
            use_mode: row.use_mode,
            generate_mode: row.generate_mode,
            external_context_policy: row.external_context_policy,
        })
        .unwrap_or(MemoryRequestedSettings {
            use_mode: MemorySettingMode::Inherit,
            generate_mode: MemorySettingMode::Inherit,
            external_context_policy: ExternalContextPolicy::Inherit,
        });
    let revision = target.as_ref().map_or(0, |row| row.revision);
    let managed_deny_reason = (!deny_reasons.is_empty()).then(|| deny_reasons.join(","));
    let digest_input = serde_json::json!({
        "schema_version": 1,
        "principal_id": principal_id,
        "conversation_id": conversation_id,
        "use_memory": use_memory,
        "generate_memory": generate_memory,
        "external_context_policy": external_context_policy,
        "use_source": use_source,
        "generate_source": generate_source,
        "external_context_source": external_context_source,
        "managed_deny_reason": managed_deny_reason,
        "admin_revision": admin.as_ref().map_or(0, |row| row.revision),
        "principal_revision": principal.as_ref().map_or(0, |row| row.revision),
        "conversation_revision": conversation.as_ref().map_or(0, |row| row.revision),
    });
    let policy_digest = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&digest_input)?)
    );
    Ok(MemoryEffectiveSettings {
        schema_version: 1,
        scope,
        target_principal_id: principal_id.to_string(),
        conversation_id: conversation_id.map(ToString::to_string),
        requested,
        use_memory,
        generate_memory,
        external_context_policy,
        use_source,
        generate_source,
        external_context_source,
        managed_deny_reason,
        revision,
        policy_digest,
        restart_required: false,
    })
}

fn resolve_mode(
    release_default: bool,
    admin: Option<MemorySettingMode>,
    principal: Option<MemorySettingMode>,
    conversation: Option<MemorySettingMode>,
) -> (bool, String) {
    let mut value = release_default;
    let mut source = "release_default";
    for (layer, mode) in [
        ("admin_runtime_default", admin),
        ("principal_runtime_default", principal),
        ("conversation_override", conversation),
    ] {
        match mode {
            Some(MemorySettingMode::Enabled) => {
                value = true;
                source = layer;
            }
            Some(MemorySettingMode::Disabled) => {
                value = false;
                source = layer;
            }
            Some(MemorySettingMode::Inherit) | None => {}
        }
    }
    (value, source.to_string())
}

fn resolve_external_policy(
    admin: Option<ExternalContextPolicy>,
    principal: Option<ExternalContextPolicy>,
    conversation: Option<ExternalContextPolicy>,
) -> (ExternalContextPolicy, String) {
    for (source, policy) in [
        ("conversation_override", conversation),
        ("principal_runtime_default", principal),
        ("admin_runtime_default", admin),
    ] {
        if let Some(policy) = policy.filter(|policy| *policy != ExternalContextPolicy::Inherit) {
            return (policy, source.to_string());
        }
    }
    (
        ExternalContextPolicy::Exclude,
        "release_default".to_string(),
    )
}

fn load_settings_row(db: &Connection, key: &str) -> anyhow::Result<Option<SettingsRow>> {
    db.query_row(
        "SELECT use_mode, generate_mode, external_context_policy,
                managed_deny_use, managed_deny_generate, revision
         FROM memory_runtime_settings WHERE setting_key = ?1",
        [key],
        |row| {
            let use_mode = row.get::<_, String>(0)?;
            let generate_mode = row.get::<_, String>(1)?;
            let external_context_policy = row.get::<_, String>(2)?;
            Ok((
                use_mode,
                generate_mode,
                external_context_policy,
                row.get::<_, i64>(3)? != 0,
                row.get::<_, i64>(4)? != 0,
                row.get::<_, i64>(5)?,
            ))
        },
    )
    .optional()?
    .map(
        |(use_mode, generate_mode, external_context_policy, deny_use, deny_generate, revision)| {
            Ok(SettingsRow {
                use_mode: MemorySettingMode::parse(&use_mode)?,
                generate_mode: MemorySettingMode::parse(&generate_mode)?,
                external_context_policy: ExternalContextPolicy::parse(&external_context_policy)?,
                managed_deny_use: deny_use,
                managed_deny_generate: deny_generate,
                revision,
            })
        },
    )
    .transpose()
}

fn target_principal(
    actor: &AuthIdentity,
    request: &MemorySettingsUpdateRequest,
) -> anyhow::Result<String> {
    if matches!(request.scope, MemorySettingScope::Admin) {
        anyhow::ensure!(actor.role == "admin", "memory_settings_admin_required");
        anyhow::ensure!(
            request.target_principal_id.is_none(),
            "memory_settings_admin_target_invalid"
        );
        return Ok(actor.principal_id.clone());
    }
    let explicit_target = request
        .target_principal_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(target) = explicit_target {
        anyhow::ensure!(actor.role == "admin", "memory_settings_admin_required");
        return Ok(target.to_string());
    }
    Ok(actor.principal_id.clone())
}

fn normalize_conversation_id(
    scope: MemorySettingScope,
    conversation_id: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let normalized = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    if matches!(scope, MemorySettingScope::Conversation) {
        let conversation_id =
            normalized.ok_or_else(|| anyhow::anyhow!("memory_settings_conversation_required"))?;
        anyhow::ensure!(
            conversation_id.len() <= 256,
            "memory_settings_conversation_invalid"
        );
        return Ok(Some(conversation_id));
    }
    anyhow::ensure!(
        normalized.is_none(),
        "memory_settings_conversation_not_allowed"
    );
    Ok(None)
}

fn setting_key(
    scope: MemorySettingScope,
    principal_id: &str,
    conversation_id: Option<&str>,
) -> String {
    match scope {
        MemorySettingScope::Admin => "admin:default".to_string(),
        MemorySettingScope::Principal => format!("principal:{principal_id}"),
        MemorySettingScope::Conversation => format!(
            "conversation:{principal_id}:{}",
            conversation_id.unwrap_or_default()
        ),
    }
}

fn ensure_target_principal(
    tx: &Transaction<'_>,
    scope: MemorySettingScope,
    principal_id: &str,
) -> anyhow::Result<()> {
    if matches!(scope, MemorySettingScope::Admin) {
        return Ok(());
    }
    let exists = tx
        .query_row(
            "SELECT 1 FROM principals WHERE principal_id = ?1 AND status = 'active'",
            [principal_id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    anyhow::ensure!(exists, "memory_settings_principal_not_found");
    Ok(())
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
