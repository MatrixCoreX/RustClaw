use crate::AppState;

pub(crate) const GLOBAL_CONVERSATION_EVENT_KEY: &str = "conversation-input-events";

pub(crate) fn notify(state: &AppState) {
    state
        .metrics
        .conversation_input_event_notifier
        .notify(GLOBAL_CONVERSATION_EVENT_KEY, crate::now_ts_u64());
}
