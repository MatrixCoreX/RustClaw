use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn bool_is_false(value: &bool) -> bool {
    !*value
}

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
    CallCapability {
        capability: String,
        args: Value,
    },
    SynthesizeAnswer {
        #[serde(default)]
        evidence_refs: Vec<String>,
    },
    Respond {
        content: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FirstLayerDecision {
    Clarify,
    DirectAnswer,
    PlannerExecute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteGateKind {
    Chat,
    Clarify,
    Execute,
}

impl FirstLayerDecision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Clarify => "clarify",
            Self::DirectAnswer => "direct_answer",
            Self::PlannerExecute => "planner_execute",
        }
    }
}

impl RouteGateKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Clarify => "clarify",
            Self::Execute => "execute",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct CommandIntentRules {
    #[serde(default)]
    pub(crate) execute_prefixes: Vec<String>,
    #[serde(default)]
    pub(crate) standalone_commands: Vec<String>,
    #[serde(default)]
    pub(crate) result_suffixes: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct CommandIntentRuntime {
    pub(crate) all_result_suffixes: Vec<String>,
    pub(crate) execute_prefixes: Vec<String>,
    pub(crate) standalone_commands: Vec<String>,
    pub(crate) default_locale: String,
    pub(crate) verify_enforce_enabled: bool,
}

#[derive(Clone)]
pub(crate) struct ScheduleRuntime {
    pub(crate) timezone: String,
    /// §3.5d: prompt 模板字段封装为 `Arc<RwLock<String>>`，使 SIGHUP 触发的
    /// hot reload 能 swap 内部内容；所有 `AppState` clone（axum router 分发）
    /// 共享同一份内部存储。读取请用 `intent_prompt_template_string()` helper。
    pub(crate) intent_prompt_template: Arc<RwLock<String>>,
    pub(crate) intent_prompt_source: String,
    /// §3.5d: 同上。
    pub(crate) intent_rules_template: Arc<RwLock<String>>,
    pub(crate) locale: String,
    pub(crate) i18n_dir: String,
    pub(crate) i18n_dict: HashMap<String, String>,
}

impl ScheduleRuntime {
    pub(crate) fn intent_prompt_template_string(&self) -> String {
        self.intent_prompt_template
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(crate) fn intent_rules_template_string(&self) -> String {
        self.intent_rules_template
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// §3.5d: 用新串覆盖现有 prompt 模板内容（写锁；poison 时静默回退）。
    pub(crate) fn replace_intent_prompt_template(&self, new_template: String) {
        if let Ok(mut guard) = self.intent_prompt_template.write() {
            *guard = new_template;
        }
    }

    pub(crate) fn replace_intent_rules_template(&self, new_rules: String) {
        if let Ok(mut guard) = self.intent_rules_template.write() {
            *guard = new_rules;
        }
    }
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
    pub(crate) mode: String,
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub(crate) dry_run: bool,
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub(crate) preview_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) create_real: Option<bool>,
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
    #[serde(default, alias = "trigger_at")]
    pub(crate) run_at: String,
    #[serde(default)]
    pub(crate) time: String,
    #[serde(default)]
    pub(crate) weekday: i64,
    #[serde(default)]
    pub(crate) every_minutes: i64,
    #[serde(default)]
    pub(crate) cron: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) timezone: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) content: String,
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
    pub(crate) isolation_profile: String,
    pub(crate) permission_policy_json: String,
    pub(crate) thread_resume_enabled: bool,
    pub(crate) last_thread_task_id: Option<String>,
}
