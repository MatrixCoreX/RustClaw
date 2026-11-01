use super::*;

impl SkillRuntime {
    pub(crate) fn test_default() -> Self {
        let tools_policy = ToolsPolicy::from_config(&claw_core::config::ToolsConfig::default())
            .expect("tools policy");
        Self {
            skill_timeout_seconds: 30,
            skill_runner_path: PathBuf::new(),
            skill_global_max_concurrency: 1,
            skill_semaphore: Arc::new(Semaphore::new(1)),
            skill_concurrency_gates: Arc::new(SkillConcurrencyGates::default()),
            runner_pool: Arc::new(crate::skills::runner_pool::WarmRunnerPool::default()),
            tools_policy: Arc::new(tools_policy),
            host_filesystem: Arc::new(HostFilesystemPolicy::without_private_authority()),
            cmd_timeout_seconds: 60,
            cmd_idle_timeout_seconds: 60,
            cmd_async_retention_seconds: 86_400,
            cmd_terminate_grace_seconds: 5,
            cmd_max_output_bytes: 8000,
            max_cmd_length: 4096,
            workspace_root: std::env::temp_dir(),
            default_locator_search_dir: std::env::temp_dir(),
        }
    }
}
