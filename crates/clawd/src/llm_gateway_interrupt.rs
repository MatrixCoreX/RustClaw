use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelInterrupt {
    TaskCancelled,
    ConversationInput,
}

impl ModelInterrupt {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::TaskCancelled => "task_llm_cancelled",
            Self::ConversationInput => super::CONVERSATION_INPUT_INTERRUPTED_ERR,
        }
    }

    pub(super) fn message_key(self) -> &'static str {
        match self {
            Self::TaskCancelled => "assistant.output.task_cancelled",
            Self::ConversationInput => "assistant.output.interrupted_by_new_input",
        }
    }

    pub(super) fn cost_status(self) -> &'static str {
        match self {
            Self::TaskCancelled => "cancelled",
            Self::ConversationInput => "interrupted",
        }
    }
}

pub(super) async fn wait(
    task: Option<CancellationToken>,
    conversation: &CancellationToken,
) -> ModelInterrupt {
    let task = task.unwrap_or_default();
    tokio::select! {
        biased;
        _ = task.cancelled() => ModelInterrupt::TaskCancelled,
        _ = conversation.cancelled() => ModelInterrupt::ConversationInput,
    }
}

#[cfg(test)]
#[path = "llm_gateway_interrupt_tests.rs"]
mod tests;
