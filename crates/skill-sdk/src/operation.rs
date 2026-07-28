use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::manifest::validate_safe_name;
use crate::{SkillSdkError, SkillSdkResult};

pub const OPERATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationAction {
    Install,
    Update,
    Repair,
    Rollback,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Queued,
    Running,
    Success,
    Failure,
    Cancelled,
}

impl OperationStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Success | Self::Failure | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStage {
    Queued,
    Preflight,
    Dependencies,
    Build,
    Smoke,
    Activate,
    Configure,
    Remove,
    Rollback,
    Success,
    Failure,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationStageRecord {
    pub stage: OperationStage,
    pub recorded_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationFailure {
    pub error_code: String,
    pub message_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillOperation {
    pub schema_version: u32,
    pub operation_id: String,
    pub skill_name: String,
    pub action: OperationAction,
    pub status: OperationStatus,
    pub stage: OperationStage,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub heartbeat_at_unix: u64,
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(default)]
    pub stages: Vec<OperationStageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<OperationFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
}

impl SkillOperation {
    pub fn validate(&self) -> SkillSdkResult<()> {
        if self.schema_version != OPERATION_SCHEMA_VERSION {
            return Err(SkillSdkError::new(
                "operation_schema_unsupported",
                format!("schema_version={}", self.schema_version),
            ));
        }
        Uuid::parse_str(&self.operation_id)
            .map_err(|error| SkillSdkError::new("operation_id_invalid", error.to_string()))?;
        validate_safe_name(&self.skill_name, "operation.skill_name")?;
        if self.updated_at_unix < self.created_at_unix
            || self.heartbeat_at_unix < self.created_at_unix
            || self.stages.is_empty()
        {
            return Err(SkillSdkError::new(
                "operation_timeline_invalid",
                format!("operation_id={}", self.operation_id),
            ));
        }
        if self.status == OperationStatus::Failure && self.failure.is_none() {
            return Err(SkillSdkError::new(
                "operation_failure_missing",
                format!("operation_id={}", self.operation_id),
            ));
        }
        if let Some(failure) = &self.failure {
            if failure.error_code.trim().is_empty() || failure.message_key.trim().is_empty() {
                return Err(SkillSdkError::new(
                    "operation_failure_invalid",
                    format!("operation_id={}", self.operation_id),
                ));
            }
            if failure.diagnostic.as_deref().is_some_and(|diagnostic| {
                diagnostic.len() > 8 * 1024
                    || crate::secret_scan::redact_diagnostics(diagnostic) != diagnostic
            }) {
                return Err(SkillSdkError::new(
                    "operation_diagnostic_unsafe",
                    format!("operation_id={}", self.operation_id),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SkillOperationStore {
    root: PathBuf,
}

impl SkillOperationStore {
    pub fn new(package_root: impl Into<PathBuf>) -> Self {
        Self {
            root: package_root.into().join("operations"),
        }
    }

    pub fn create(
        &self,
        skill_name: &str,
        action: OperationAction,
    ) -> SkillSdkResult<SkillOperation> {
        let _guard = operation_io_guard();
        validate_safe_name(skill_name, "operation.skill_name")?;
        let now = now_unix()?;
        let operation = SkillOperation {
            schema_version: OPERATION_SCHEMA_VERSION,
            operation_id: Uuid::new_v4().to_string(),
            skill_name: skill_name.to_string(),
            action,
            status: OperationStatus::Queued,
            stage: OperationStage::Queued,
            created_at_unix: now,
            updated_at_unix: now,
            heartbeat_at_unix: now,
            cancel_requested: false,
            stages: vec![OperationStageRecord {
                stage: OperationStage::Queued,
                recorded_at_unix: now,
            }],
            failure: None,
            result: None,
        };
        self.write_unlocked(&operation)?;
        Ok(operation)
    }

    pub fn get(&self, operation_id: &str) -> SkillSdkResult<SkillOperation> {
        let _guard = operation_io_guard();
        self.get_unlocked(operation_id)
    }

    fn get_unlocked(&self, operation_id: &str) -> SkillSdkResult<SkillOperation> {
        let path = self.operation_path(operation_id)?;
        let operation: SkillOperation = serde_json::from_slice(&fs::read(path)?)?;
        operation.validate()?;
        Ok(operation)
    }

    pub fn transition(
        &self,
        operation_id: &str,
        status: OperationStatus,
        stage: OperationStage,
        failure: Option<OperationFailure>,
        result: Option<serde_json::Value>,
    ) -> SkillSdkResult<SkillOperation> {
        let _guard = operation_io_guard();
        self.transition_unlocked(operation_id, status, stage, failure, result)
    }

    fn transition_unlocked(
        &self,
        operation_id: &str,
        status: OperationStatus,
        stage: OperationStage,
        failure: Option<OperationFailure>,
        result: Option<serde_json::Value>,
    ) -> SkillSdkResult<SkillOperation> {
        let mut operation = self.get_unlocked(operation_id)?;
        if operation.status.is_terminal() {
            return Err(SkillSdkError::new(
                "operation_already_terminal",
                format!("operation_id={operation_id}"),
            ));
        }
        let now = now_unix()?;
        operation.status = status;
        operation.stage = stage;
        operation.updated_at_unix = now;
        operation.heartbeat_at_unix = now;
        operation.failure = failure;
        operation.result = result;
        operation.stages.push(OperationStageRecord {
            stage,
            recorded_at_unix: now,
        });
        self.write_unlocked(&operation)?;
        Ok(operation)
    }

    pub fn heartbeat(&self, operation_id: &str) -> SkillSdkResult<SkillOperation> {
        let _guard = operation_io_guard();
        let mut operation = self.get_unlocked(operation_id)?;
        if !operation.status.is_terminal() {
            operation.heartbeat_at_unix = now_unix()?;
            operation.updated_at_unix = operation.heartbeat_at_unix;
            self.write_unlocked(&operation)?;
        }
        Ok(operation)
    }

    pub fn request_cancel(&self, operation_id: &str) -> SkillSdkResult<SkillOperation> {
        let _guard = operation_io_guard();
        let mut operation = self.get_unlocked(operation_id)?;
        if !operation.status.is_terminal() {
            operation.cancel_requested = true;
            operation.updated_at_unix = now_unix()?;
            self.write_unlocked(&operation)?;
        }
        Ok(operation)
    }

    pub fn list(&self) -> SkillSdkResult<Vec<SkillOperation>> {
        let _guard = operation_io_guard();
        self.list_unlocked()
    }

    fn list_unlocked(&self) -> SkillSdkResult<Vec<SkillOperation>> {
        if !self.root.is_dir() {
            return Ok(Vec::new());
        }
        let mut operations = fs::read_dir(&self.root)?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "json")
            })
            .filter_map(|entry| {
                serde_json::from_slice::<SkillOperation>(&fs::read(entry.path()).ok()?).ok()
            })
            .filter(|operation| operation.validate().is_ok())
            .collect::<Vec<_>>();
        operations
            .sort_by_key(|operation| (operation.created_at_unix, operation.operation_id.clone()));
        Ok(operations)
    }

    pub fn latest_active(&self) -> SkillSdkResult<Option<SkillOperation>> {
        let _guard = operation_io_guard();
        Ok(self
            .list_unlocked()?
            .into_iter()
            .rev()
            .find(|operation| !operation.status.is_terminal()))
    }

    pub fn recover_interrupted(&self) -> SkillSdkResult<Vec<SkillOperation>> {
        let _guard = operation_io_guard();
        let mut recovered = Vec::new();
        for operation in self.list_unlocked()? {
            if operation.status.is_terminal() {
                continue;
            }
            recovered.push(self.transition_unlocked(
                &operation.operation_id,
                OperationStatus::Failure,
                OperationStage::Failure,
                Some(OperationFailure {
                    error_code: "operation_interrupted".to_string(),
                    message_key: "skill_store.operation_interrupted".to_string(),
                    phase: Some(format!("{:?}", operation.stage).to_ascii_lowercase()),
                    retryable: true,
                    diagnostic: None,
                }),
                None,
            )?);
        }
        Ok(recovered)
    }

    fn write_unlocked(&self, operation: &SkillOperation) -> SkillSdkResult<()> {
        operation.validate()?;
        fs::create_dir_all(&self.root)?;
        let destination = self.operation_path(&operation.operation_id)?;
        let temporary = self.root.join(format!(".{}.tmp", operation.operation_id));
        fs::write(&temporary, serde_json::to_vec_pretty(operation)?)?;
        fs::rename(temporary, destination)?;
        Ok(())
    }

    fn operation_path(&self, operation_id: &str) -> SkillSdkResult<PathBuf> {
        Uuid::parse_str(operation_id)
            .map_err(|error| SkillSdkError::new("operation_id_invalid", error.to_string()))?;
        Ok(self.root.join(format!("{operation_id}.json")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn operation_io_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_unix() -> SkillSdkResult<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| SkillSdkError::new("system_time_invalid", error.to_string()))
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
