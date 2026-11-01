use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;

use super::super::host_filesystem::HostFilesystemPolicy;
use super::ToolsPolicy;

#[derive(Clone)]
pub(crate) struct SkillRuntime {
    pub(crate) skill_timeout_seconds: u64,
    pub(crate) skill_runner_path: PathBuf,
    pub(crate) skill_global_max_concurrency: usize,
    pub(crate) skill_semaphore: Arc<Semaphore>,
    pub(crate) skill_concurrency_gates: Arc<SkillConcurrencyGates>,
    pub(crate) runner_pool: Arc<crate::skills::runner_pool::WarmRunnerPool>,
    pub(crate) tools_policy: Arc<ToolsPolicy>,
    pub(crate) host_filesystem: Arc<HostFilesystemPolicy>,
    pub(crate) cmd_timeout_seconds: u64,
    pub(crate) cmd_idle_timeout_seconds: u64,
    pub(crate) cmd_async_retention_seconds: u64,
    pub(crate) cmd_terminate_grace_seconds: u64,
    pub(crate) cmd_max_output_bytes: usize,
    pub(crate) max_cmd_length: usize,
    pub(crate) workspace_root: PathBuf,
    pub(crate) default_locator_search_dir: PathBuf,
}

pub(crate) struct SkillConcurrencyGates {
    gates: Mutex<HashMap<String, SkillConcurrencyGate>>,
    resource_broker: crate::resource_scheduler::ResourceBroker,
}

impl Default for SkillConcurrencyGates {
    fn default() -> Self {
        Self {
            gates: Mutex::new(HashMap::new()),
            resource_broker: crate::resource_scheduler::ResourceBroker::default(),
        }
    }
}

struct SkillConcurrencyGate {
    limit: usize,
    semaphore: Arc<Semaphore>,
}

impl SkillConcurrencyGates {
    pub(crate) fn new(resource_broker: crate::resource_scheduler::ResourceBroker) -> Self {
        Self {
            gates: Mutex::new(HashMap::new()),
            resource_broker,
        }
    }

    pub(crate) fn resource_broker(&self) -> &crate::resource_scheduler::ResourceBroker {
        &self.resource_broker
    }

    pub(crate) fn semaphore(&self, skill_name: &str, limit: usize) -> Arc<Semaphore> {
        let limit = limit.max(1);
        let mut gates = self.gates.lock().unwrap();
        if let Some(gate) = gates.get(skill_name) {
            if gate.limit != limit {
                tracing::warn!(
                    skill = skill_name,
                    active_limit = gate.limit,
                    requested_limit = limit,
                    "skill_concurrency_limit_change_requires_restart"
                );
            }
            return gate.semaphore.clone();
        }
        let semaphore = Arc::new(Semaphore::new(limit));
        gates.insert(
            skill_name.to_string(),
            SkillConcurrencyGate {
                limit,
                semaphore: semaphore.clone(),
            },
        );
        semaphore
    }
}

#[cfg(test)]
#[path = "skill_execution_test_support.rs"]
mod test_support;
