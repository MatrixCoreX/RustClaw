use claw_core::skill_registry::SkillRiskLevel;
use serde_json::Value;

use crate::execution_recipe::ActionEffect;

fn dry_run_evidence_present(args: &Value) -> bool {
    args.get("dry_run").and_then(Value::as_bool) == Some(true)
}

pub(super) fn high_risk_side_effect_requires_confirmation(
    effect: ActionEffect,
    risk_level: SkillRiskLevel,
    args: &Value,
) -> bool {
    matches!(risk_level, SkillRiskLevel::High) && effect.mutates && !dry_run_evidence_present(args)
}
