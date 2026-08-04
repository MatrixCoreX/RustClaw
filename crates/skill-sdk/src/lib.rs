//! Language-neutral package, installation, protocol, and launch contracts for
//! Agent runtime skill development and installation contracts.

pub mod adapter;
pub mod admission;
pub mod bounded_result;
pub mod capability_request;
pub mod error;
pub mod installer;
pub mod manifest;
pub mod operation;
pub mod path_policy;
pub mod platform;
mod prebuilt;
mod process;
pub mod progress;
pub mod protocol;
pub mod receipt;
pub mod runtime;
pub mod safe_archive;
pub mod sandbox;
mod secret_scan;
pub mod templates;

pub use admission::{
    AdmissionReceipt, AdmissionState, ApprovalSource, GrantedCapability, HostPolicyGrant,
    HostRiskLevel, ADMISSION_RECEIPT_SCHEMA_VERSION, HOST_POLICY_GRANT_SCHEMA_VERSION,
};
pub use bounded_result::{
    ArtifactDescriptor, ArtifactSpill, BoundedResult, ContinuationDescriptor, FieldTruncation,
};
pub use capability_request::{
    ArtifactContractRequest, ArtifactKindRequest, CapabilityActionRequest, CapabilityRequestSet,
    ConfigEntryPointKind, ConfigEntryPointRequest, EvidenceContractRequest, InputSemanticRole,
    RequestedEffect, RequestedExecutionMode, RuntimePermissionRequest,
    CAPABILITY_REQUEST_SCHEMA_VERSION,
};
pub use error::{SkillSdkError, SkillSdkResult};
pub use installer::{
    validate_install_resources, AdoptBuiltRequest, InstallControl, InstallOrigin, InstallOutcome,
    InstallRequest, PrecompiledInstallRequest, SkillInstaller,
};
pub use manifest::{
    ArchiveFormat, BuildAdapter, BuildNetworkPolicy, ExecutionProfile, InstallResourceRequirements,
    InstallSpec, LauncherKind, PackageManifest, SandboxProfile, AGENT_JSONL_PROTOCOL,
    LEGACY_SKILL_MANIFEST_SCHEMA_VERSION, SKILL_MANIFEST_SCHEMA_VERSION,
};
pub use operation::{
    OperationAction, OperationFailure, OperationStage, OperationStageRecord, OperationStatus,
    SkillOperation, SkillOperationStore,
};
pub use path_policy::{ExpectedPathKind, PathAuthority, SkillPathPolicy};
pub use platform::HostPlatform;
pub use progress::SkillProgressEmitter;
pub use protocol::{
    validate_progress_frame_line, validate_protocol_output, validate_response_line,
    ProtocolRequest, ProtocolResponse, ProtocolStatus, SkillProgressFrame, SkillProgressKind,
    SkillProgressReference, MAX_PROGRESS_FRAMES_PER_INVOCATION, MAX_PROGRESS_FRAMES_PER_SECOND,
    MAX_PROGRESS_FRAME_LINE_BYTES, MAX_PROGRESS_FRAME_PARAMS, MAX_PROTOCOL_LINE_BYTES,
    SKILL_PROGRESS_FRAME_RECORD_TYPE, SKILL_PROGRESS_FRAME_SCHEMA_VERSION,
};
pub use receipt::{
    digest_file, is_sensitive_runtime_environment_name, ArtifactReceipt, CurrentInstallPointer,
    InstallReceipt, InstallReceiptStore, ProtocolSmokeReceipt, VerifiedInstall,
    CURRENT_INSTALL_POINTER_SCHEMA_VERSION, INSTALL_RECEIPT_SCHEMA_VERSION,
    LEGACY_INSTALL_RECEIPT_SCHEMA_VERSION,
};
pub use runtime::{
    SkillLaunchSpec, SkillRuntimeResolver, SkillVersionPin, SKILL_LAUNCH_SCHEMA_VERSION,
};
pub use safe_archive::{
    extract_safe_archive, inspect_safe_archive, read_safe_archive_member, SafeArchiveEntry,
    SafeArchiveInspection, SafeArchiveLimits,
};
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
#[path = "capability_request_tests.rs"]
mod capability_request_tests;

#[cfg(test)]
#[path = "admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "reference_conformance_tests.rs"]
mod reference_conformance_tests;
