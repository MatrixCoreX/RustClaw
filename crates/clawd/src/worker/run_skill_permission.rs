use serde_json::{json, Value};

use crate::execution_recipe::ExecutionRecipeRuntimeState;
use crate::verifier::{verify_plan, VerifyInput, VerifyMode, VerifyResult};
use crate::{AppState, ClaimedTask, PlanKind, PlanResult, PlanStep};

pub(super) struct DirectRunSkillVerification {
    pub(super) request_envelope: String,
    pub(super) plan: PlanResult,
    pub(super) verify: VerifyResult,
}

impl DirectRunSkillVerification {
    pub(super) fn allowed(&self) -> bool {
        self.verify.approved && !self.verify.needs_confirmation
    }

    pub(super) fn needs_confirmation(&self) -> bool {
        self.verify.approved && self.verify.needs_confirmation
    }

    pub(super) fn denial_error(&self, skill_name: &str) -> String {
        let issue = self.verify.issues.first();
        let reason_code = issue
            .map(|issue| issue.kind.reason_code())
            .unwrap_or("verify_rejected");
        let message_key = issue
            .map(|issue| issue.kind.message_key())
            .unwrap_or("clawd.verify.rejected");
        crate::skills::structured_skill_error_from_parts(
            skill_name,
            "permission_denied",
            "permission_denied",
            Some(std::env::consts::OS),
            Some(json!({
                "reason_code": reason_code,
                "message_key": message_key,
                "decision": crate::policy_decision::PolicyDecision::Deny.as_token(),
                "permission_decision": self.verify.permission_decision,
            })),
        )
    }
}

pub(super) fn verify_direct_run_skill(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
) -> DirectRunSkillVerification {
    let canonical_skill = state.resolve_canonical_skill_name(skill_name);
    let request_envelope = json!({
        "request_kind": "direct_skill",
        "skill": canonical_skill,
        "args_keys": args
            .as_object()
            .map(|args| args.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
    })
    .to_string();
    let plan = PlanResult {
        goal: format!("skill:{canonical_skill}"),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps: vec![PlanStep {
            step_id: "direct_skill".to_string(),
            action_type: "call_skill".to_string(),
            skill: canonical_skill,
            args,
            depends_on: Vec::new(),
            why: String::new(),
        }],
        planner_notes: "direct_skill".to_string(),
        plan_kind: PlanKind::Single,
        raw_plan_text: json!({"plan_source": "direct_skill"}).to_string(),
    };
    let verify = verify_plan(
        state,
        task,
        VerifyInput {
            output_contract: None,
            request_text: Some(&request_envelope),
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    DirectRunSkillVerification {
        request_envelope,
        plan,
        verify,
    }
}

#[cfg(test)]
#[path = "run_skill_permission_tests.rs"]
mod tests;
