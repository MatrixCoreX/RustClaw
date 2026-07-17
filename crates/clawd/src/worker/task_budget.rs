use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerTaskBudgetClass {
    Adaptive,
    Interactive,
    Standard,
    LongTail,
}

impl WorkerTaskBudgetClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Interactive => "interactive",
            Self::Standard => "standard",
            Self::LongTail => "long_tail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerTaskBudget {
    pub(crate) class: WorkerTaskBudgetClass,
    pub(crate) requested_profile: Option<String>,
    pub(crate) profile_valid: bool,
    pub(crate) timeout_seconds: u64,
    pub(crate) admin_max_seconds: u64,
}

pub(crate) fn task_execution_budget(
    administrator_max_seconds: u64,
    _task_kind: &str,
    payload: &Value,
) -> WorkerTaskBudget {
    let admin_max_seconds = administrator_max_seconds.max(1);
    let requested_profile = payload
        .get("budget_profile")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let (class, profile_valid) = requested_profile
        .as_deref()
        .map(classify_profile)
        .unwrap_or((WorkerTaskBudgetClass::Adaptive, true));
    let timeout_seconds = match class {
        WorkerTaskBudgetClass::Interactive => bounded_fraction(admin_max_seconds, 1, 4, 30),
        WorkerTaskBudgetClass::Standard => bounded_fraction(admin_max_seconds, 1, 2, 60),
        WorkerTaskBudgetClass::Adaptive | WorkerTaskBudgetClass::LongTail => admin_max_seconds,
    };

    WorkerTaskBudget {
        class,
        requested_profile,
        profile_valid,
        timeout_seconds,
        admin_max_seconds,
    }
}

pub(crate) fn task_execution_timeout_seconds(
    administrator_max_seconds: u64,
    task_kind: &str,
    payload_json: &str,
) -> u64 {
    let payload = serde_json::from_str::<Value>(payload_json).unwrap_or(Value::Null);
    task_execution_budget(administrator_max_seconds, task_kind, &payload).timeout_seconds
}

fn classify_profile(profile: &str) -> (WorkerTaskBudgetClass, bool) {
    match profile {
        "interactive" | "short" | "fast_read" => (WorkerTaskBudgetClass::Interactive, true),
        "standard" | "grounded_summary" => (WorkerTaskBudgetClass::Standard, true),
        "long_tail" | "background_async_job" | "multi_step_workspace" | "ops_closed_loop" => {
            (WorkerTaskBudgetClass::LongTail, true)
        }
        "adaptive" | "general" => (WorkerTaskBudgetClass::Adaptive, true),
        _ => (WorkerTaskBudgetClass::Adaptive, false),
    }
}

fn bounded_fraction(admin_max_seconds: u64, numerator: u64, denominator: u64, floor: u64) -> u64 {
    let fraction = admin_max_seconds
        .saturating_mul(numerator)
        .checked_div(denominator.max(1))
        .unwrap_or(admin_max_seconds);
    fraction
        .max(floor.min(admin_max_seconds))
        .min(admin_max_seconds)
}

#[cfg(test)]
#[path = "task_budget_tests.rs"]
mod tests;
