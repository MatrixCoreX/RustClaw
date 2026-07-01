pub(super) fn active_observed_facts_have_bound_target(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    session_snapshot
        .active_observed_facts
        .as_ref()
        .and_then(|facts| facts.bound_target.as_deref())
        .map(str::trim)
        .is_some_and(|target| !target.is_empty())
}

pub(super) fn active_session_has_bound_target(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    active_bound_targets(session_snapshot)
        .into_iter()
        .any(|target| !target.trim().is_empty())
}

fn active_bound_targets(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> Vec<&str> {
    let mut targets = Vec::new();
    if let Some(facts) = session_snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts
            .bound_target
            .as_deref()
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            targets.push(target);
        }
    }
    if let Some(frame) = session_snapshot.active_followup_frame.as_ref() {
        if matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::List
        ) {
            if let Some(target) = frame
                .bound_target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty())
            {
                targets.push(target);
            }
        }
    }
    targets
}

pub(super) fn single_component_locator_hint(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || std::path::Path::new(trimmed).is_absolute()
        || std::path::Path::new(trimmed).components().count() != 1
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(super) const SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST: &str =
    "session_alias_locator_prebound_from_current_request";
