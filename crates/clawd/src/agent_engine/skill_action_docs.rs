//! Package-authored action sections; selection consumes structured action IDs only.
use std::collections::BTreeSet;

const OPEN: &str = "<!--a1:";
const CLOSE: &str = "<!--/a1-->";

/// Unknown, malformed, or unmarked documents retain their full legacy body.
/// Shared policy and multilingual sections outside action blocks always remain.
pub(super) fn render(body: &str, selected: &BTreeSet<String>) -> Option<String> {
    let mut active: Option<BTreeSet<String>> = None;
    let mut known = BTreeSet::new();
    let mut lines = Vec::new();
    let mut found = false;
    for line in body.lines() {
        if let Some(raw) = line.strip_prefix(OPEN).and_then(|v| v.strip_suffix("-->")) {
            if active.is_some() {
                return None;
            }
            let actions = raw.split(',').map(str::to_string).collect::<BTreeSet<_>>();
            if actions.is_empty()
                || actions.iter().any(|a| {
                    a.is_empty()
                        || !a.as_bytes()[0].is_ascii_lowercase()
                        || !a
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                })
            {
                return None;
            }
            known.extend(actions.clone());
            active = Some(actions);
            found = true;
        } else if line == CLOSE {
            active.take()?;
        } else if line.contains("<!--a") || line.contains("<!--/a") {
            return None;
        } else if active
            .as_ref()
            .is_none_or(|actions| !actions.is_disjoint(selected))
        {
            lines.push(line);
        }
    }
    if !found || active.is_some() || !selected.is_subset(&known) {
        return None;
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
#[path = "skill_action_docs_tests.rs"]
mod tests;
