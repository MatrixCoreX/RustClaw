use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeChannel {
    Telegram,
    Whatsapp,
    Wechat,
    Feishu,
    Lark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhatsappDeliveryRoute {
    Cloud,
    WebBridge,
}

pub(crate) struct AskReply {
    pub(crate) text: String,
    pub(crate) messages: Vec<String>,
    pub(crate) is_llm_reply: bool,
    pub(crate) task_journal: Option<crate::task_journal::TaskJournal>,
    pub(crate) should_fail_task: bool,
    pub(crate) error_text: Option<String>,
    pub(crate) resume_context: Option<Value>,
}

impl AskReply {
    pub(crate) fn llm(text: String) -> Self {
        Self {
            text,
            messages: Vec::new(),
            is_llm_reply: true,
            task_journal: None,
            should_fail_task: false,
            error_text: None,
            resume_context: None,
        }
    }

    pub(crate) fn non_llm(text: String) -> Self {
        Self {
            text,
            messages: Vec::new(),
            is_llm_reply: false,
            task_journal: None,
            should_fail_task: false,
            error_text: None,
            resume_context: None,
        }
    }

    pub(crate) fn with_messages(mut self, messages: Vec<String>) -> Self {
        self.messages = messages;
        self
    }

    pub(crate) fn with_task_journal(
        mut self,
        task_journal: crate::task_journal::TaskJournal,
    ) -> Self {
        self.task_journal = Some(task_journal);
        self
    }

    pub(crate) fn with_failure(mut self, error_text: impl Into<String>) -> Self {
        self.should_fail_task = true;
        self.error_text = Some(error_text.into());
        self
    }

    pub(crate) fn with_resume_context(mut self, resume_context: Value) -> Self {
        self.resume_context = Some(resume_context);
        self
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AgentAction {
    Think {
        #[allow(dead_code)]
        content: String,
    },
    CallTool {
        tool: String,
        args: Value,
    },
    CallSkill {
        skill: String,
        args: Value,
    },
    Respond {
        content: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoutedMode {
    Chat,
    Act,
    ChatAct,
    AskClarify,
}

impl RoutedMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Act => "Act",
            Self::ChatAct => "ChatAct",
            Self::AskClarify => "AskClarify",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CommandIntentRules {}

#[derive(Clone)]
pub(crate) struct CommandIntentRuntime {
    pub(crate) all_result_suffixes: Vec<String>,
    pub(crate) default_locale: String,
    pub(crate) verify_enforce_enabled: bool,
}

#[derive(Clone)]
pub(crate) struct ScheduleRuntime {
    pub(crate) timezone: String,
    pub(crate) intent_prompt_template: String,
    pub(crate) intent_prompt_source: String,
    pub(crate) intent_rules_template: String,
    pub(crate) locale: String,
    pub(crate) i18n_dict: HashMap<String, String>,
}

#[derive(Serialize)]
pub(crate) struct LocalInteractionContext {
    pub(crate) user_id: i64,
    pub(crate) chat_id: i64,
    pub(crate) role: String,
}

#[derive(Deserialize)]
pub(crate) struct MemoryConfigFileWrapper {
    pub(crate) memory: claw_core::config::MemoryConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(crate) struct ScheduleIntentOutput {
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) timezone: String,
    #[serde(default)]
    pub(crate) schedule: ScheduleIntentSchedule,
    #[serde(default)]
    pub(crate) task: ScheduleIntentTask,
    #[serde(default)]
    pub(crate) target_job_id: String,
    #[serde(default)]
    pub(crate) raw: String,
    #[serde(default)]
    pub(crate) reason: String,
    #[serde(default)]
    pub(crate) needs_clarify: bool,
    #[serde(default)]
    pub(crate) clarify_question: String,
    #[serde(default)]
    pub(crate) confidence: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(crate) struct ScheduleIntentSchedule {
    #[serde(default)]
    pub(crate) r#type: String,
    #[serde(default)]
    pub(crate) run_at: String,
    #[serde(default)]
    pub(crate) time: String,
    #[serde(default)]
    pub(crate) weekday: i64,
    #[serde(default)]
    pub(crate) every_minutes: i64,
    #[serde(default)]
    pub(crate) cron: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(crate) struct ScheduleIntentTask {
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) payload: Value,
}

pub(crate) struct ScheduledJobDue {
    pub(crate) job_id: String,
    pub(crate) user_id: i64,
    pub(crate) chat_id: i64,
    pub(crate) user_key: Option<String>,
    pub(crate) channel: String,
    pub(crate) external_user_id: Option<String>,
    pub(crate) external_chat_id: Option<String>,
    pub(crate) task_kind: String,
    pub(crate) task_payload_json: String,
    pub(crate) next_run_at: i64,
    pub(crate) schedule_type: String,
    pub(crate) time_of_day: Option<String>,
    pub(crate) weekday: Option<i64>,
    pub(crate) every_minutes: Option<i64>,
    pub(crate) timezone: String,
}
