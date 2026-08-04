use super::*;

#[test]
fn chat_progress_is_deduplicated_and_stops_after_terminal() {
    let capabilities = ChannelProgressCapabilities::for_channel(ChannelKind::Telegram);
    let mut state = ChannelProgressProjectionState::default();
    assert!(!state.should_emit_progress(1, 1, 10, capabilities));
    assert!(state.should_emit_progress(2, 10, 10, capabilities));
    assert!(!state.should_emit_progress(3, 11, 10, capabilities));
    state.mark_terminal();
    assert!(!state.should_emit_progress(4, 20, 10, capabilities));
}
