use std::future::Future;

use crate::AgentAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepExecutionStatus {
    Ok,
    Error,
}

impl Default for StepExecutionStatus {
    fn default() -> Self {
        Self::Error
    }
}

impl StepExecutionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StepExecutionResult {
    pub(crate) step_id: String,
    pub(crate) skill: String,
    pub(crate) status: StepExecutionStatus,
    pub(crate) output: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) started_at: u64,
    pub(crate) finished_at: u64,
}

impl StepExecutionResult {
    pub(crate) fn is_ok(&self) -> bool {
        self.status == StepExecutionStatus::Ok
    }
}

fn action_subject(action: &AgentAction) -> String {
    match action {
        AgentAction::CallSkill { skill, .. } => skill.clone(),
        AgentAction::CallTool { tool, .. } => tool.clone(),
        AgentAction::SynthesizeAnswer { .. } => "synthesize_answer".to_string(),
        AgentAction::Respond { .. } => "respond".to_string(),
        AgentAction::Think { .. } => "think".to_string(),
    }
}

pub(crate) async fn execute_step<F, Fut>(
    step_id: &str,
    action: &AgentAction,
    exec: F,
) -> StepExecutionResult
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let started_at = crate::now_ts_u64();
    let result = exec().await;
    let finished_at = crate::now_ts_u64();
    match result {
        Ok(output) => StepExecutionResult {
            step_id: step_id.to_string(),
            skill: action_subject(action),
            status: StepExecutionStatus::Ok,
            output: Some(output),
            error: None,
            started_at,
            finished_at,
        },
        Err(error) => StepExecutionResult {
            step_id: step_id.to_string(),
            skill: action_subject(action),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(error),
            started_at,
            finished_at,
        },
    }
}
