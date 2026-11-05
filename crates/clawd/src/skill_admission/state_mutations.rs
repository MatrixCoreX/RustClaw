use super::*;
use crate::skill_admission::AdmissionStateChange;

impl SkillAdmissionService {
    pub(crate) fn set_state(
        &self,
        skill_name: &str,
        state: AdmissionState,
        grant: Option<HostPolicyGrant>,
    ) -> Result<OverlaySnapshot> {
        self.set_states(
            vec![AdmissionStateChange {
                skill_name: skill_name.to_string(),
                state,
                grant,
            }],
            None,
        )
    }

    /// Prepare state-only changes from one validated generation while holding
    /// the existing admission lock. No stale metadata, prompt or grant is
    /// carried across the lock boundary, and publication happens only once.
    pub(crate) fn set_states(
        &self,
        changes: Vec<AdmissionStateChange>,
        expected_generation: Option<u64>,
    ) -> Result<OverlaySnapshot> {
        self.with_mutation_lock(|| {
            let current = self.read_current_generation()?;
            let generation = current
                .as_ref()
                .map_or(0, |(pointer, _, _)| pointer.generation);
            if expected_generation.is_some_and(|expected| expected != generation) {
                return Err(error(
                    "skill_admission_generation_changed",
                    format!("expected={expected_generation:?} actual={generation}"),
                ));
            }
            if changes.is_empty() {
                return self.snapshot();
            }
            let (pointer, record, root) = current
                .ok_or_else(|| error("skill_admission_missing", "skill_admission_missing"))?;
            // A normal state change is not a repair operation: never bless
            // tampered metadata or a revoked/invalid receipt by rewriting it.
            self.validate_generation(&pointer, &record, &root, true)?;
            let mut names = BTreeSet::new();
            let mut mutations = Vec::with_capacity(changes.len());
            for change in changes {
                let name = change.skill_name;
                skill_sdk::validate_safe_name(&name, "state_change.skill_name")
                    .map_err(|source| error("skill_admission_name_invalid", source.to_string()))?;
                if !names.insert(name.clone()) {
                    return Err(error(
                        "skill_admission_name_invalid",
                        format!("duplicate={name}"),
                    ));
                }
                if !record.skills.contains_key(&name) {
                    return Err(error("skill_admission_missing", format!("skill={name}")));
                }
                let metadata = read_json(&root.join("metadata").join(format!("{name}.json")))?;
                let prompt = fs::read_to_string(root.join("prompts").join(format!("{name}.md")))
                    .map_err(|source| {
                        error(
                            "skill_admission_prompt_read_failed",
                            format!("skill={name} error={source}"),
                        )
                    })?;
                mutations.push(AdmissionMutation {
                    metadata,
                    prompt,
                    state: change.state,
                    grant: change.grant,
                });
            }
            self.commit_mutations_locked(mutations, false, &BTreeSet::new())
        })
    }
}
