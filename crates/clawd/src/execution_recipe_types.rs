#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExecutionRecipeKind {
    #[default]
    None,
    OpsClosedLoop,
}

impl ExecutionRecipeKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OpsClosedLoop => "ops_closed_loop",
        }
    }
}

pub(crate) fn parse_execution_recipe_kind_text(value: &str) -> ExecutionRecipeKind {
    match value.trim().to_ascii_lowercase().as_str() {
        "ops_closed_loop" | "ops" | "closed_loop" => ExecutionRecipeKind::OpsClosedLoop,
        _ => ExecutionRecipeKind::None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExecutionRecipeProfile {
    #[default]
    None,
    OpsService,
    ConfigChange,
    CodeChange,
    SkillAuthoring,
    PackageChange,
    DatabaseChange,
}

impl ExecutionRecipeProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OpsService => "ops_service",
            Self::ConfigChange => "config_change",
            Self::CodeChange => "code_change",
            Self::SkillAuthoring => "skill_authoring",
            Self::PackageChange => "package_change",
            Self::DatabaseChange => "database_change",
        }
    }
}

pub(crate) fn parse_execution_recipe_profile_text(value: &str) -> ExecutionRecipeProfile {
    match value.trim().to_ascii_lowercase().as_str() {
        "ops_service" | "ops" | "service_ops" => ExecutionRecipeProfile::OpsService,
        "config_change" | "config" => ExecutionRecipeProfile::ConfigChange,
        "code_change" | "code" | "project_change" | "problem_resolution" => {
            ExecutionRecipeProfile::CodeChange
        }
        "skill_authoring" | "skill" | "extension_build" => ExecutionRecipeProfile::SkillAuthoring,
        "package_change" | "package" | "package_manager" | "dependency_change" => {
            ExecutionRecipeProfile::PackageChange
        }
        "database_change" | "database" | "schema_change" | "migration" => {
            ExecutionRecipeProfile::DatabaseChange
        }
        _ => ExecutionRecipeProfile::None,
    }
}

pub(crate) fn profile_requires_specific_validation(profile: ExecutionRecipeProfile) -> bool {
    matches!(
        profile,
        ExecutionRecipeProfile::ConfigChange
            | ExecutionRecipeProfile::CodeChange
            | ExecutionRecipeProfile::SkillAuthoring
            | ExecutionRecipeProfile::PackageChange
            | ExecutionRecipeProfile::DatabaseChange
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExecutionRecipeTargetScope {
    #[default]
    Unknown,
    System,
    CurrentRepo,
    ExternalWorkspace,
    Greenfield,
}

impl ExecutionRecipeTargetScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::System => "system",
            Self::CurrentRepo => "current_repo",
            Self::ExternalWorkspace => "external_workspace",
            Self::Greenfield => "greenfield",
        }
    }
}

pub(crate) fn parse_execution_recipe_target_scope_text(value: &str) -> ExecutionRecipeTargetScope {
    match value.trim().to_ascii_lowercase().as_str() {
        "system" | "host" => ExecutionRecipeTargetScope::System,
        "current_repo" | "repo" | "current_workspace" | "workspace" => {
            ExecutionRecipeTargetScope::CurrentRepo
        }
        "external_workspace" | "external" | "other_workspace" => {
            ExecutionRecipeTargetScope::ExternalWorkspace
        }
        "greenfield" | "new_project" | "new_script" => ExecutionRecipeTargetScope::Greenfield,
        _ => ExecutionRecipeTargetScope::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ExecutionRecipePhase {
    #[default]
    Inspect,
    Apply,
    Validate,
    Repair,
    Done,
}

impl ExecutionRecipePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Apply => "apply",
            Self::Validate => "validate",
            Self::Repair => "repair",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ExecutionRecipeSpec {
    pub(crate) kind: ExecutionRecipeKind,
    pub(crate) profile: ExecutionRecipeProfile,
    pub(crate) target_scope: ExecutionRecipeTargetScope,
    pub(crate) inspect_first: bool,
    pub(crate) validation_required: bool,
    pub(crate) max_repairs: usize,
}

pub(crate) fn explicit_execution_recipe_spec(
    kind: ExecutionRecipeKind,
    profile: ExecutionRecipeProfile,
    target_scope: ExecutionRecipeTargetScope,
) -> Option<ExecutionRecipeSpec> {
    if matches!(kind, ExecutionRecipeKind::None) || matches!(profile, ExecutionRecipeProfile::None)
    {
        return None;
    }
    Some(ExecutionRecipeSpec {
        kind,
        profile,
        target_scope,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ExecutionRecipeRuntimeState {
    pub(crate) kind: ExecutionRecipeKind,
    pub(crate) profile: ExecutionRecipeProfile,
    pub(crate) target_scope: ExecutionRecipeTargetScope,
    pub(crate) phase: ExecutionRecipePhase,
    pub(crate) inspect_first: bool,
    pub(crate) validation_required: bool,
    pub(crate) max_repairs: usize,
    pub(crate) repair_count: usize,
    pub(crate) saw_inspect: bool,
    pub(crate) saw_mutation: bool,
    pub(crate) saw_validation: bool,
    pub(crate) saw_external_target: bool,
    pub(crate) saw_greenfield_creation: bool,
}

impl ExecutionRecipeRuntimeState {
    fn profile_guidance(self) -> &'static str {
        match self.profile {
            ExecutionRecipeProfile::None => "Treat this as a general closed-loop execution task.",
            ExecutionRecipeProfile::OpsService => {
                "Focus on service/system state: inspect status, logs, ports, and runtime config before mutating; validate with machine-verifiable service or network signals."
            }
            ExecutionRecipeProfile::ConfigChange => {
                "Focus on configuration safety: inspect the current file and effective values first, prefer minimal targeted edits, and validate parse/reload/effective-state after changes."
            }
            ExecutionRecipeProfile::CodeChange => {
                "Focus on solving the requested code or script problem: inspect relevant files and current failures first, keep edits scoped, and validate with compile/test/lint/runtime evidence."
            }
            ExecutionRecipeProfile::SkillAuthoring => {
                "Focus on building or updating a reusable skill/extension: inspect existing interface, registration, prompts, and docs first, then validate structure, integration points, and targeted tests."
            }
            ExecutionRecipeProfile::PackageChange => {
                "Focus on package/dependency state: inspect the current manager and dependency context first, make the minimal install/update/remove change, and validate with package manager state, build/test, or runtime command evidence."
            }
            ExecutionRecipeProfile::DatabaseChange => {
                "Focus on database/schema state: inspect schema or target rows first, apply only confirmed structured mutations, and validate with schema/version/table/query evidence after the change."
            }
        }
    }

    fn target_scope_guidance(self) -> &'static str {
        match self.target_scope {
            ExecutionRecipeTargetScope::Unknown => {
                "Scope is not explicit. Infer the smallest safe working area from the request before mutating."
            }
            ExecutionRecipeTargetScope::System => {
                "Target scope is the host system or service environment. Prefer system/service evidence over repository-local assumptions."
            }
            ExecutionRecipeTargetScope::CurrentRepo => {
                "Target scope is the current repository/workspace. Prefer local project files, tests, and configs in this repo."
            }
            ExecutionRecipeTargetScope::ExternalWorkspace => {
                "Target scope is outside the current repo. Confirm paths and avoid assuming the current workspace contains the target files."
            }
            ExecutionRecipeTargetScope::Greenfield => {
                "Target scope is greenfield creation. Create the minimal new files or scaffold needed, then validate the new artifact works."
            }
        }
    }

    pub(crate) fn from_spec(spec: ExecutionRecipeSpec) -> Self {
        Self {
            kind: spec.kind,
            profile: spec.profile,
            target_scope: spec.target_scope,
            phase: if matches!(spec.kind, ExecutionRecipeKind::None) {
                ExecutionRecipePhase::Done
            } else {
                ExecutionRecipePhase::Inspect
            },
            inspect_first: spec.inspect_first,
            validation_required: spec.validation_required,
            max_repairs: spec.max_repairs,
            repair_count: 0,
            saw_inspect: false,
            saw_mutation: false,
            saw_validation: false,
            saw_external_target: false,
            saw_greenfield_creation: false,
        }
    }

    pub(crate) fn is_active(self) -> bool {
        !matches!(self.kind, ExecutionRecipeKind::None)
    }

    pub(crate) fn needs_validation(self) -> bool {
        self.is_active() && self.validation_required && self.saw_mutation && !self.saw_validation
    }

    pub(crate) fn remaining_repairs(self) -> usize {
        self.max_repairs.saturating_sub(self.repair_count)
    }

    pub(crate) fn phase_summary_line(self) -> String {
        format!(
            "kind={} profile={} target_scope={} phase={} inspect_first={} validation_required={} repair_count={} max_repairs={} saw_inspect={} saw_mutation={} saw_validation={} saw_external_target={} saw_greenfield_creation={}",
            self.kind.as_str(),
            self.profile.as_str(),
            self.target_scope.as_str(),
            self.phase.as_str(),
            self.inspect_first,
            self.validation_required,
            self.repair_count,
            self.max_repairs,
            self.saw_inspect,
            self.saw_mutation,
            self.saw_validation,
            self.saw_external_target,
            self.saw_greenfield_creation
        )
    }

    pub(crate) fn goal_overlay(self) -> Option<String> {
        if !self.is_active() {
            return None;
        }
        Some(format!(
            "[EXECUTION_RECIPE]\nkind={}\nprofile={}\ntarget_scope={}\ncurrent_phase={}\ninspect_first={}\nvalidation_required={}\nrepair_count={}\nmax_repairs={}\nremaining_repairs={}\nobserved_state=saw_inspect:{} saw_mutation:{} saw_validation:{} saw_external_target:{} saw_greenfield_creation:{}\nProfileGuidance:\n- {}\nScopeGuidance:\n- {}\nRules:\n- Collect current state/config evidence before mutating.\n- After any mutating step, include machine-verifiable validation steps.\n- If validation fails after a mutation, use the next round for repair instead of looping blindly.\n",
            self.kind.as_str(),
            self.profile.as_str(),
            self.target_scope.as_str(),
            self.phase.as_str(),
            self.inspect_first,
            self.validation_required,
            self.repair_count,
            self.max_repairs,
            self.remaining_repairs(),
            self.saw_inspect,
            self.saw_mutation,
            self.saw_validation,
            self.saw_external_target,
            self.saw_greenfield_creation,
            self.profile_guidance(),
            self.target_scope_guidance()
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ActionEffect {
    pub(crate) observes: bool,
    pub(crate) mutates: bool,
    pub(crate) validates: bool,
}

impl ActionEffect {
    pub(crate) const fn observe() -> Self {
        Self {
            observes: true,
            mutates: false,
            validates: false,
        }
    }

    pub(crate) const fn mutate() -> Self {
        Self {
            observes: false,
            mutates: true,
            validates: false,
        }
    }

    pub(crate) const fn validate() -> Self {
        Self {
            observes: true,
            mutates: false,
            validates: true,
        }
    }
}
