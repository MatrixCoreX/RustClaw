use super::{Capability, SkillRiskLevel, SkillsRegistry};

impl SkillsRegistry {
    /// Audits capability declarations against the host-owned execution shape.
    /// The sorted machine-readable violations are stable for CI and startup.
    pub fn validate_shape_consistency(&self) -> Vec<String> {
        let mut violations = Vec::new();
        for (name, entry) in &self.by_name {
            let caps = &entry.resolved_capabilities;
            let has = |capability: &Capability| caps.contains(capability);
            if has(&Capability::ExecSudo) {
                if entry.requires_confirmation != Some(true) {
                    violations.push(format!(
                        "skill `{name}` declares `exec.sudo` but `requires_confirmation` is not `true` (R1)"
                    ));
                }
                if entry.risk_level != Some(SkillRiskLevel::High) {
                    violations.push(format!(
                        "skill `{name}` declares `exec.sudo` but `risk_level` is not `high` (R2)"
                    ));
                }
            }
            for capability in caps {
                let Capability::LlmCredentialFallback(secret_name) = capability else {
                    continue;
                };
                if !has(&Capability::Llm) || !has(&Capability::Secrets(secret_name.clone())) {
                    violations.push(format!(
                        "skill `{name}` declares `{}` without matching `llm` and `secrets.{secret_name}`",
                        capability.as_token()
                    ));
                }
            }
            if (has(&Capability::FsWrite) || has(&Capability::Exec) || has(&Capability::ExecSudo))
                && entry.side_effect == Some(false)
            {
                violations.push(format!(
                    "skill `{name}` declares fs.write/exec/exec.sudo but `side_effect = false` is set explicitly (R3)"
                ));
            }
        }
        violations.sort();
        violations
    }
}
