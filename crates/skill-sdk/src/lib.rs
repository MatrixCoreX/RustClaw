//! Language-neutral package, installation, protocol, and launch contracts for
//! RustClaw skills.

pub mod adapter;
pub mod error;
pub mod installer;
pub mod manifest;
pub mod operation;
pub mod platform;
mod prebuilt;
mod process;
pub mod protocol;
pub mod receipt;
pub mod runtime;
pub mod sandbox;
mod secret_scan;
pub mod templates;

pub use error::{SkillSdkError, SkillSdkResult};
pub use installer::{
    AdoptBuiltRequest, InstallControl, InstallOutcome, InstallRequest, SkillInstaller,
};
pub use manifest::{
    ArchiveFormat, BuildAdapter, BuildNetworkPolicy, LauncherKind, PackageManifest, SandboxProfile,
    RUSTCLAW_JSONL_PROTOCOL, SKILL_MANIFEST_SCHEMA_VERSION,
};
pub use operation::{
    OperationAction, OperationFailure, OperationStage, OperationStageRecord, OperationStatus,
    SkillOperation, SkillOperationStore,
};
pub use platform::HostPlatform;
pub use protocol::{
    validate_response_line, ProtocolRequest, ProtocolResponse, ProtocolStatus,
    MAX_PROTOCOL_LINE_BYTES,
};
pub use receipt::{
    ArtifactReceipt, CurrentInstallPointer, InstallReceipt, InstallReceiptStore,
    ProtocolSmokeReceipt, INSTALL_RECEIPT_SCHEMA_VERSION,
};
pub use runtime::{SkillLaunchSpec, SkillRuntimeResolver, SKILL_LAUNCH_SCHEMA_VERSION};
pub use sandbox::{
    prepare_sandboxed_command, PreparedSandboxCommand, SandboxNetwork, PARENT_SANDBOX_BACKEND_ENV,
    SKILL_STORAGE_WRITABLE_DIRECTORY_ENV,
};
pub use secret_scan::redact_diagnostics;
pub use templates::{scaffold_skill, ImplementationLanguage, ScaffoldOutcome, ScaffoldRequest};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "reference_conformance_tests.rs"]
mod reference_conformance_tests;
