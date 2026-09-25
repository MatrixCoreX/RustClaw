use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{ChannelKind, SubmitTaskRequest};

pub const CONVERSATION_INPUT_SCHEMA_VERSION: u32 = 1;
pub const CONVERSATION_INPUT_MAX_CONTENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInputScopeRef {
    pub conversation_id: String,
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    pub channel: String,
    #[serde(default)]
    pub channel_account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedConversationInputScope {
    pub owner_principal_id: String,
    pub conversation: ConversationInputScopeRef,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationInputDeliveryMode {
    #[default]
    Auto,
    Defer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationInputContent {
    Text {
        text: String,
    },
    Attachment {
        attachment_id: String,
        #[serde(default)]
        media_type: Option<String>,
        #[serde(default)]
        display_name: Option<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInputSource {
    #[serde(default)]
    pub provider_message_id: Option<String>,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub received_at_ts: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInputSubmission {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub client_message_id: String,
    pub scope: ConversationInputScopeRef,
    pub content: Vec<ConversationInputContent>,
    #[serde(default)]
    pub delivery_mode: ConversationInputDeliveryMode,
    #[serde(default)]
    pub expected_task_id: Option<Uuid>,
    #[serde(default)]
    pub expected_instruction_revision: Option<u64>,
    #[serde(default)]
    pub source: ConversationInputSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationInputPreparationState {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationInputDisposition {
    Pending,
    Deferred,
    NeedsClarification,
    Applied,
    Rejected,
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInputReceipt {
    pub schema_version: u32,
    pub input_id: Uuid,
    pub client_message_id: String,
    pub input_seq: u64,
    pub scope: ConversationInputScopeRef,
    pub preparation_state: ConversationInputPreparationState,
    pub disposition: ConversationInputDisposition,
    #[serde(default)]
    pub target_task_id: Option<Uuid>,
    #[serde(default)]
    pub decision_ref: Option<String>,
    pub instruction_revision: u64,
    pub execution_epoch: u64,
    pub accepted_at_ts: u64,
    pub updated_at_ts: u64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInputRecord {
    pub receipt: ConversationInputReceipt,
    pub content: Vec<ConversationInputContent>,
    pub delivery_mode: ConversationInputDeliveryMode,
    #[serde(default)]
    pub expected_task_id: Option<Uuid>,
    #[serde(default)]
    pub expected_instruction_revision: Option<u64>,
    #[serde(default)]
    pub source: ConversationInputSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationInputClientTaskRequest {
    pub input: ConversationInputSubmission,
    pub task: SubmitTaskRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationInputTaskHandoffState {
    TaskCreated,
    BoundExistingTask,
    WaitingForTask,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInputClientTaskReceipt {
    pub schema_version: u32,
    pub input: ConversationInputReceipt,
    pub handoff_state: ConversationInputTaskHandoffState,
}

impl ConversationInputClientTaskRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn channel_text(
        task: SubmitTaskRequest,
        client_message_id: impl Into<String>,
        conversation_id: impl Into<String>,
        agent_id: impl Into<String>,
        channel: ChannelKind,
        channel_account_id: impl Into<String>,
        text: impl Into<String>,
        source: ConversationInputSource,
    ) -> Self {
        Self {
            input: ConversationInputSubmission {
                schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
                client_message_id: client_message_id.into(),
                scope: ConversationInputScopeRef {
                    conversation_id: conversation_id.into(),
                    agent_id: agent_id.into(),
                    channel: channel_token(channel).to_string(),
                    channel_account_id: channel_account_id.into(),
                },
                content: vec![ConversationInputContent::Text { text: text.into() }],
                delivery_mode: ConversationInputDeliveryMode::Auto,
                expected_task_id: None,
                expected_instruction_revision: None,
                source,
            },
            task,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationInputErrorCode {
    InvalidRequest,
    Unauthorized,
    TargetConflict,
    IdempotencyConflict,
    PreparationFailed,
    CapacityExceeded,
    ModelUnavailable,
    NotFound,
    DatabaseFailed,
}

impl ConversationInputSubmission {
    pub fn validate(&self) -> Result<(), ConversationInputErrorCode> {
        if self.schema_version != CONVERSATION_INPUT_SCHEMA_VERSION
            || !valid_machine_token(&self.client_message_id, 512)
            || !self.scope.validate()
            || self.content.is_empty()
        {
            return Err(ConversationInputErrorCode::InvalidRequest);
        }
        let encoded = serde_json::to_vec(&self.content)
            .map_err(|_| ConversationInputErrorCode::InvalidRequest)?;
        if encoded.len() > CONVERSATION_INPUT_MAX_CONTENT_BYTES
            || self.content.iter().any(|item| !item.validate())
            || !optional_machine_token(self.source.provider_message_id.as_deref(), 512)
            || !optional_machine_token(self.source.reply_to_message_id.as_deref(), 512)
        {
            return Err(ConversationInputErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

impl ConversationInputScopeRef {
    pub fn validate(&self) -> bool {
        valid_machine_token(&self.conversation_id, 512)
            && valid_machine_token(&self.agent_id, 160)
            && valid_machine_token(&self.channel, 64)
            && (self.channel_account_id.is_empty()
                || valid_machine_token(&self.channel_account_id, 256))
    }
}

impl OwnedConversationInputScope {
    pub fn validate(&self) -> bool {
        valid_machine_token(&self.owner_principal_id, 256) && self.conversation.validate()
    }
}

impl ConversationInputContent {
    fn validate(&self) -> bool {
        match self {
            Self::Text { text } => !text.trim().is_empty(),
            Self::Attachment {
                attachment_id,
                media_type,
                display_name,
            } => {
                valid_machine_token(attachment_id, 512)
                    && optional_display_value(media_type.as_deref(), 256)
                    && optional_display_value(display_name.as_deref(), 512)
            }
        }
    }
}

fn optional_machine_token(value: Option<&str>, max_chars: usize) -> bool {
    value.is_none_or(|value| valid_machine_token(value, max_chars))
}

fn optional_display_value(value: Option<&str>, max_chars: usize) -> bool {
    value.is_none_or(|value| {
        let value = value.trim();
        !value.is_empty() && value.chars().count() <= max_chars && !value.contains('\0')
    })
}

fn valid_machine_token(value: &str, max_chars: usize) -> bool {
    let trimmed = value.trim();
    value == trimmed
        && !value.is_empty()
        && value.chars().count() <= max_chars
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '@'))
}

fn default_agent_id() -> String {
    "main".to_string()
}

const fn schema_version() -> u32 {
    CONVERSATION_INPUT_SCHEMA_VERSION
}

const fn channel_token(channel: ChannelKind) -> &'static str {
    match channel {
        ChannelKind::Telegram => "telegram",
        ChannelKind::Whatsapp => "whatsapp",
        ChannelKind::Ui => "ui",
        ChannelKind::Wechat => "wechat",
        ChannelKind::Feishu => "feishu",
        ChannelKind::Lark => "lark",
    }
}

#[cfg(test)]
#[path = "conversation_input_tests.rs"]
mod tests;
