pub const PROVIDER_WAIT_RECOVERY_ACTION: &str = "wait_background";
pub const PROVIDER_WAIT_RESUME_ENTRYPOINT: &str = "next_planner_round";
pub const PROVIDER_WAIT_RESUME_REASON: &str = "provider_blocker_wait_background";

#[cfg(test)]
#[path = "provider_failure_policy_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureClass {
    Timeout,
    TransportRetryable,
    ProviderRetryableResponse,
    RateLimited,
    QuotaExhausted,
    ProviderNonRetryableBusiness,
    LocalNonRetryable,
}

impl ProviderFailureClass {
    pub const ALL: [Self; 7] = [
        Self::Timeout,
        Self::TransportRetryable,
        Self::ProviderRetryableResponse,
        Self::RateLimited,
        Self::QuotaExhausted,
        Self::ProviderNonRetryableBusiness,
        Self::LocalNonRetryable,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::TransportRetryable => "transport_retryable",
            Self::ProviderRetryableResponse => "provider_retryable_response",
            Self::RateLimited => "rate_limited",
            Self::QuotaExhausted => "quota_exhausted",
            Self::ProviderNonRetryableBusiness => "provider_non_retryable_business",
            Self::LocalNonRetryable => "local_non_retryable",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim() {
            "timeout" => Some(Self::Timeout),
            "transport_retryable" => Some(Self::TransportRetryable),
            "provider_retryable_response" => Some(Self::ProviderRetryableResponse),
            "rate_limited" => Some(Self::RateLimited),
            "quota_exhausted" => Some(Self::QuotaExhausted),
            "provider_non_retryable_business" => Some(Self::ProviderNonRetryableBusiness),
            "local_non_retryable" => Some(Self::LocalNonRetryable),
            _ => None,
        }
    }

    pub const fn policy(self) -> ProviderFailurePolicy {
        match self {
            Self::QuotaExhausted => ProviderFailurePolicy::background_wait(
                self,
                false,
                3 * 60 * 60,
                "provider.quota_exhausted",
            ),
            Self::RateLimited => {
                ProviderFailurePolicy::background_wait(self, true, 60, "provider.rate_limited")
            }
            Self::Timeout | Self::TransportRetryable | Self::ProviderRetryableResponse => {
                ProviderFailurePolicy::background_wait(
                    self,
                    true,
                    30,
                    "provider.temporarily_unavailable",
                )
            }
            Self::ProviderNonRetryableBusiness => {
                ProviderFailurePolicy::terminal(self, "provider.non_retryable_business")
            }
            Self::LocalNonRetryable => {
                ProviderFailurePolicy::terminal(self, "provider.local_non_retryable")
            }
        }
    }

    pub const fn background_wait_seconds(self) -> Option<u64> {
        self.policy().retry_after_seconds
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderFailurePolicy {
    pub failure_class: ProviderFailureClass,
    pub provider_retryable: bool,
    pub provider_blocker: bool,
    pub retry_policy: &'static str,
    pub retry_after_seconds: Option<u64>,
    pub waiting_state: &'static str,
    pub checkpoint_required: bool,
    pub recovery_action: Option<&'static str>,
    pub resume_reason: Option<&'static str>,
    pub resume_entrypoint: Option<&'static str>,
    pub message_key: &'static str,
}

impl ProviderFailurePolicy {
    const fn background_wait(
        failure_class: ProviderFailureClass,
        provider_retryable: bool,
        retry_after_seconds: u64,
        message_key: &'static str,
    ) -> Self {
        Self {
            failure_class,
            provider_retryable,
            provider_blocker: true,
            retry_policy: "background_wait",
            retry_after_seconds: Some(retry_after_seconds),
            waiting_state: "waiting",
            checkpoint_required: true,
            recovery_action: Some(PROVIDER_WAIT_RECOVERY_ACTION),
            resume_reason: Some(PROVIDER_WAIT_RESUME_REASON),
            resume_entrypoint: Some(PROVIDER_WAIT_RESUME_ENTRYPOINT),
            message_key,
        }
    }

    const fn terminal(failure_class: ProviderFailureClass, message_key: &'static str) -> Self {
        Self {
            failure_class,
            provider_retryable: false,
            provider_blocker: false,
            retry_policy: "none",
            retry_after_seconds: None,
            waiting_state: "terminal",
            checkpoint_required: false,
            recovery_action: None,
            resume_reason: None,
            resume_entrypoint: None,
            message_key,
        }
    }
}
