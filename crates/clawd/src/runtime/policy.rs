use std::collections::{HashMap, VecDeque};

use claw_core::config::{ToolApprovalPolicy, ToolSandboxBackend, ToolSandboxMode, ToolsConfig};

use super::state::LlmProviderRuntime;

pub(crate) struct RateLimiter {
    pub(crate) global_rpm: usize,
    pub(crate) user_rpm: usize,
    pub(crate) global: VecDeque<u64>,
    pub(crate) per_user: HashMap<i64, VecDeque<u64>>,
}

pub(crate) struct ToolsPolicy {
    pub(crate) access_profile: String,
    pub(crate) sandbox_mode: ToolSandboxMode,
    pub(crate) sandbox_backend: ToolSandboxBackend,
    pub(crate) approval_policy: ToolApprovalPolicy,
    pub(crate) allow: Vec<String>,
    pub(crate) deny: Vec<String>,
    pub(crate) by_provider: HashMap<String, ProviderScopedPolicy>,
}

pub(crate) struct ProviderScopedPolicy {
    pub(crate) allow: Vec<String>,
    pub(crate) deny: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SandboxRequirements<'a> {
    pub(crate) mutates: bool,
    pub(crate) network_access: bool,
    pub(crate) filesystem_write: bool,
    pub(crate) external_publish: bool,
    pub(crate) credential_access: bool,
    pub(crate) subprocess: bool,
    pub(crate) package_install: bool,
    pub(crate) privilege_escalation: bool,
    pub(crate) isolation_profile: Option<&'a str>,
}

impl RateLimiter {
    pub(crate) fn new(global_rpm: usize, user_rpm: usize) -> Self {
        Self {
            global_rpm: global_rpm.max(1),
            user_rpm: user_rpm.max(1),
            global: VecDeque::new(),
            per_user: HashMap::new(),
        }
    }

    pub(crate) fn check_and_record(&mut self, user_id: i64) -> Result<(), &'static str> {
        let now = crate::now_ts_u64();
        let min_ts = now.saturating_sub(60);

        while self.global.front().is_some_and(|v| *v < min_ts) {
            self.global.pop_front();
        }

        let (limit_ok, user_q_empty_after_pop) = {
            let user_q = self.per_user.entry(user_id).or_default();
            while user_q.front().is_some_and(|v| *v < min_ts) {
                user_q.pop_front();
            }
            let empty = user_q.is_empty();
            if self.global.len() >= self.global_rpm {
                (Err("global_rpm"), empty)
            } else if user_q.len() >= self.user_rpm {
                (Err("user_rpm"), false)
            } else {
                (Ok(()), empty)
            }
        };

        if let Err("global_rpm") = limit_ok {
            if user_q_empty_after_pop {
                self.per_user.remove(&user_id);
            }
            return Err("global_rpm");
        }
        if limit_ok.is_err() {
            return limit_ok;
        }

        self.global.push_back(now);
        self.per_user.entry(user_id).or_default().push_back(now);
        Ok(())
    }
}

impl ToolsPolicy {
    pub(crate) fn from_config(cfg: &ToolsConfig) -> Result<Self, String> {
        let access_profile = cfg.access_profile.trim().to_ascii_lowercase();
        if !matches!(
            access_profile.as_str(),
            "full" | "coding" | "minimal" | "messaging"
        ) {
            return Err(format!(
                "{}:{}",
                "invalid_tools_access_profile", cfg.access_profile
            ));
        }
        let allow: Vec<String> = cfg
            .allow
            .iter()
            .map(|v| normalize_capability_pattern(v.trim()))
            .filter(|v| !v.is_empty())
            .collect();
        let deny: Vec<String> = cfg
            .deny
            .iter()
            .map(|v| normalize_capability_pattern(v.trim()))
            .filter(|v| !v.is_empty())
            .collect();

        for p in allow.iter().chain(deny.iter()) {
            if p != "*" && !p.starts_with("skill:") && !p.starts_with("capability:") {
                return Err(format!(
                    "invalid tools pattern: {p}; expected '*' or prefix 'skill:'/'capability:' (legacy 'tool:' is auto-converted to 'skill:')"
                ));
            }
        }

        let mut by_provider = HashMap::new();
        for (provider_key, scoped) in &cfg.by_provider {
            let key = provider_key.trim().to_ascii_lowercase();
            if key.is_empty() {
                return Err("tools.by_provider contains empty key".to_string());
            }
            let allow_scoped: Vec<String> = scoped
                .allow
                .iter()
                .map(|v| normalize_capability_pattern(v.trim()))
                .filter(|v| !v.is_empty())
                .collect();
            let deny_scoped: Vec<String> = scoped
                .deny
                .iter()
                .map(|v| normalize_capability_pattern(v.trim()))
                .filter(|v| !v.is_empty())
                .collect();

            for p in allow_scoped.iter().chain(deny_scoped.iter()) {
                if p != "*" && !p.starts_with("skill:") && !p.starts_with("capability:") {
                    return Err(format!(
                        "invalid tools.by_provider.{key} pattern: {p}; expected '*' or prefix 'skill:'/'capability:' (legacy 'tool:' is auto-converted to 'skill:')"
                    ));
                }
            }

            by_provider.insert(
                key,
                ProviderScopedPolicy {
                    allow: allow_scoped,
                    deny: deny_scoped,
                },
            );
        }

        Ok(Self {
            access_profile,
            sandbox_mode: cfg.sandbox_mode,
            sandbox_backend: cfg.sandbox_backend,
            approval_policy: cfg.approval_policy,
            allow,
            deny,
            by_provider,
        })
    }

    pub(crate) fn is_allowed(&self, token: &str, provider_type: Option<&str>) -> bool {
        self.is_any_allowed(&[token], provider_type)
    }

    pub(crate) fn is_any_allowed(&self, tokens: &[&str], provider_type: Option<&str>) -> bool {
        if tokens.is_empty()
            || self
                .deny
                .iter()
                .any(|pattern| tokens.iter().any(|token| wildcard_match(pattern, token)))
        {
            return false;
        }

        if !self.allow.is_empty() {
            return self
                .allow
                .iter()
                .any(|pattern| tokens.iter().any(|token| wildcard_match(pattern, token)));
        }

        let mut allowed = tokens.iter().any(|token| self.default_allowed(token));

        if !allowed {
            return false;
        }

        if let Some(provider) = provider_type {
            let keys = provider_policy_keys(provider);
            for key in keys {
                if let Some(scoped) = self.by_provider.get(&key) {
                    if scoped
                        .deny
                        .iter()
                        .any(|pattern| tokens.iter().any(|token| wildcard_match(pattern, token)))
                    {
                        return false;
                    }
                    if !scoped.allow.is_empty()
                        && !scoped.allow.iter().any(|pattern| {
                            tokens.iter().any(|token| wildcard_match(pattern, token))
                        })
                    {
                        return false;
                    }
                    allowed = true;
                    break;
                }
            }
        }

        allowed
    }

    pub(crate) fn sandbox_mode_token(&self) -> &'static str {
        self.sandbox_mode.as_token()
    }

    pub(crate) fn sandbox_backend_token(&self) -> &'static str {
        self.sandbox_backend.as_token()
    }

    pub(crate) fn approval_policy_token(&self) -> &'static str {
        self.approval_policy.as_token()
    }

    #[cfg(test)]
    pub(crate) fn sandbox_denial(
        &self,
        requirements: SandboxRequirements<'_>,
    ) -> Option<&'static str> {
        Self::sandbox_denial_for_mode(self.sandbox_mode, requirements)
    }

    pub(crate) fn sandbox_denial_for_mode(
        sandbox_mode: ToolSandboxMode,
        requirements: SandboxRequirements<'_>,
    ) -> Option<&'static str> {
        match sandbox_mode {
            ToolSandboxMode::DangerFull => None,
            ToolSandboxMode::ReadOnly => {
                if requirements.mutates || requirements.filesystem_write {
                    Some("sandbox_read_only_write_denied")
                } else if requirements.subprocess
                    && (requirements.network_access || requirements.external_publish)
                {
                    Some("sandbox_read_only_external_denied")
                } else if requirements.subprocess && requirements.credential_access {
                    Some("sandbox_read_only_credential_denied")
                } else if requirements.package_install || requirements.privilege_escalation {
                    Some("sandbox_read_only_privilege_denied")
                } else if requirements.subprocess
                    && requirements.isolation_profile != Some("read_only")
                {
                    Some("sandbox_read_only_subprocess_denied")
                } else {
                    None
                }
            }
            ToolSandboxMode::WorkspaceWrite => {
                if requirements.subprocess
                    && (requirements.external_publish || requirements.network_access)
                {
                    Some("sandbox_workspace_external_denied")
                } else if requirements.subprocess && requirements.credential_access {
                    Some("sandbox_workspace_credential_denied")
                } else if requirements.package_install || requirements.privilege_escalation {
                    Some("sandbox_workspace_privilege_denied")
                } else {
                    None
                }
            }
            ToolSandboxMode::IsolatedWorktree => {
                if requirements.subprocess
                    && (requirements.external_publish || requirements.network_access)
                {
                    Some("sandbox_worktree_external_denied")
                } else if requirements.subprocess && requirements.credential_access {
                    Some("sandbox_worktree_credential_denied")
                } else if requirements.package_install || requirements.privilege_escalation {
                    Some("sandbox_worktree_privilege_denied")
                } else if requirements.subprocess
                    && !matches!(
                        requirements.isolation_profile,
                        Some("read_only" | "local_worktree")
                    )
                {
                    Some("sandbox_worktree_subprocess_isolation_required")
                } else if requirements.mutates
                    && requirements.isolation_profile != Some("local_worktree")
                {
                    Some("sandbox_worktree_isolation_required")
                } else {
                    None
                }
            }
        }
    }

    pub(crate) fn approval_required(
        &self,
        risk_requires_approval: bool,
        planner_requested_approval: bool,
        mutates_or_external: bool,
    ) -> bool {
        Self::approval_required_for_policy(
            self.approval_policy,
            risk_requires_approval,
            planner_requested_approval,
            mutates_or_external,
        )
    }

    pub(crate) fn approval_required_for_policy(
        approval_policy: ToolApprovalPolicy,
        risk_requires_approval: bool,
        planner_requested_approval: bool,
        mutates_or_external: bool,
    ) -> bool {
        match approval_policy {
            ToolApprovalPolicy::Never => false,
            ToolApprovalPolicy::OnRisk => risk_requires_approval,
            ToolApprovalPolicy::OnRequest => risk_requires_approval || planner_requested_approval,
            ToolApprovalPolicy::Always => mutates_or_external || risk_requires_approval,
        }
    }

    fn default_allowed(&self, token: &str) -> bool {
        let defaults = match self.access_profile.as_str() {
            "full" => vec!["*"],
            "coding" => vec![
                "skill:run_cmd",
                "skill:code_index",
                "skill:fs_basic",
                "skill:config_basic",
                "skill:config_edit",
                "skill:read_file",
                "skill:write_file",
                "skill:list_dir",
                "skill:make_dir",
                "skill:remove_file",
                "skill:workspace_patch",
                "skill:system_basic",
                "skill:git_basic",
                "skill:process_basic",
                "skill:archive_basic",
                "skill:fs_search",
                "skill:health_check",
                "skill:log_analyze",
                "capability:service_control",
                "skill:task_control",
                "skill:doc_parse",
                "skill:transform",
                "skill:kb",
                "capability:image.preview_generate",
                "capability:audio.preview_synthesize",
                "capability:video.preview_generate",
                "capability:music.preview_generate",
                "capability:schedule.preview",
                "capability:schedule.list",
            ],
            "minimal" => vec![
                "skill:run_cmd",
                "skill:read_file",
                "skill:write_file",
                "skill:list_dir",
                "skill:make_dir",
                "skill:remove_file",
                "skill:system_basic",
            ],
            "messaging" => vec!["skill:system_basic"],
            _ => vec!["*"],
        };
        defaults.into_iter().any(|p| wildcard_match(p, token))
    }
}

pub(crate) fn provider_policy_keys(provider_type: &str) -> Vec<String> {
    let p = provider_type.trim().to_ascii_lowercase();
    let mut keys = vec![p.clone()];
    match p.as_str() {
        "openai_compat" => keys.push("openai".to_string()),
        "google_gemini" => keys.push("google".to_string()),
        "anthropic_claude" => keys.push("anthropic".to_string()),
        _ => {}
    }
    keys
}

pub(crate) fn llm_vendor_name(provider: &LlmProviderRuntime) -> &str {
    provider
        .config
        .name
        .strip_prefix("vendor-")
        .unwrap_or(provider.config.name.as_str())
}

pub(crate) fn llm_model_kind(provider: &LlmProviderRuntime) -> &'static str {
    match provider.config.provider_type.as_str() {
        "openai_compat" => "compat",
        "google_gemini" => "gemini_native",
        "anthropic_claude" => "claude_native",
        _ => "unknown",
    }
}

pub(crate) fn normalize_capability_pattern(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("tool:") {
        format!("skill:{}", &s[5..])
    } else {
        s.to_string()
    }
}

pub(crate) fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }

    let mut idx = 0usize;
    let mut first = true;
    for part in &parts {
        if part.is_empty() {
            continue;
        }
        if first && !pattern.starts_with('*') {
            if !text[idx..].starts_with(part) {
                return false;
            }
            idx += part.len();
            first = false;
            continue;
        }
        if let Some(found) = text[idx..].find(part) {
            idx += found + part.len();
        } else {
            return false;
        }
        first = false;
    }
    if !pattern.ends_with('*') {
        if let Some(last) = parts.last() {
            return text.ends_with(last);
        }
    }
    true
}
