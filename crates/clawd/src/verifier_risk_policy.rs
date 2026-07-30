use claw_core::skill_registry::SkillRiskLevel;

use crate::execution_recipe::ActionEffect;

pub(super) fn high_risk_side_effect_requires_confirmation(
    effect: ActionEffect,
    risk_level: SkillRiskLevel,
) -> bool {
    matches!(risk_level, SkillRiskLevel::High) && effect.mutates
}
