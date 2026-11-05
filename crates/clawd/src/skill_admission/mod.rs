mod model;
mod registry;
mod store;

pub(crate) use model::{
    AdmissionExecutionBinding, AdmissionMutation, AdmissionStateChange, AippViewState,
    ExternalSkillMetadata, OverlaySnapshot, SkillAdmissionSource,
};
pub(crate) use store::SkillAdmissionService;

#[cfg(test)]
#[path = "skill_admission_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) use tests::{fixture_grant as test_policy_grant, install_fixture as install_test_fixture};

#[cfg(test)]
#[path = "aipp_view_state_tests.rs"]
mod aipp_view_state_tests;
