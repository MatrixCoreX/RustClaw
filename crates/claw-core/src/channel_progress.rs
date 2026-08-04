use crate::types::ChannelKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelProgressCapabilities {
    pub typing: bool,
    pub editable_message: bool,
    pub slow_notice_limit: u8,
}

impl ChannelProgressCapabilities {
    pub fn for_channel(channel: ChannelKind) -> Self {
        match channel {
            ChannelKind::Telegram => Self {
                typing: true,
                editable_message: true,
                slow_notice_limit: 1,
            },
            ChannelKind::Wechat
            | ChannelKind::Whatsapp
            | ChannelKind::Feishu
            | ChannelKind::Lark => Self {
                typing: false,
                editable_message: false,
                slow_notice_limit: 1,
            },
            ChannelKind::Ui => Self {
                typing: false,
                editable_message: true,
                slow_notice_limit: 0,
            },
        }
    }
}

/// Shared low-noise projection state. Progress evidence may be arbitrarily
/// frequent, while a chat transport emits at most one slow notice and never
/// emits progress after observing a terminal state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelProgressProjectionState {
    last_sequence: u64,
    slow_notice_count: u8,
    terminal: bool,
}

impl ChannelProgressProjectionState {
    pub fn should_emit_progress(
        &mut self,
        sequence: u64,
        elapsed_seconds: u64,
        slow_threshold_seconds: u64,
        capabilities: ChannelProgressCapabilities,
    ) -> bool {
        if self.terminal || sequence <= self.last_sequence {
            return false;
        }
        self.last_sequence = sequence;
        self.should_emit_slow_notice(elapsed_seconds, slow_threshold_seconds, capabilities)
    }

    pub fn should_emit_slow_notice(
        &mut self,
        elapsed_seconds: u64,
        slow_threshold_seconds: u64,
        capabilities: ChannelProgressCapabilities,
    ) -> bool {
        if self.terminal
            || capabilities.slow_notice_limit == 0
            || slow_threshold_seconds == 0
            || elapsed_seconds < slow_threshold_seconds
            || self.slow_notice_count >= capabilities.slow_notice_limit
        {
            return false;
        }
        self.slow_notice_count = self.slow_notice_count.saturating_add(1);
        true
    }

    pub fn mark_terminal(&mut self) {
        self.terminal = true;
    }

    pub fn notice_sent(&self) -> bool {
        self.slow_notice_count > 0
    }
}

#[cfg(test)]
#[path = "channel_progress_tests.rs"]
mod tests;
