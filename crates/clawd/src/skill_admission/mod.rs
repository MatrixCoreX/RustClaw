mod model;
mod registry;
mod store;

pub(crate) use model::{
    AdmissionExecutionBinding, AdmissionMutation, ExternalSkillMetadata, OverlaySnapshot,
    SkillAdmissionSource,
};
pub(crate) use store::SkillAdmissionService;

#[cfg(test)]
#[path = "skill_admission_tests.rs"]
mod tests;
