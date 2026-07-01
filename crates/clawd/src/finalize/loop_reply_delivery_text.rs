pub(super) fn final_answer_text_from_delivery(delivery_messages: &[String]) -> String {
    let publishable_messages = delivery_messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .filter(|message| !crate::finalize::is_non_answer_separator_message(message))
        .filter(|message| !crate::finalize::looks_like_planner_artifact(message))
        .collect::<Vec<_>>();
    if !publishable_messages.is_empty() {
        return publishable_messages.join("\n\n");
    }
    delivery_messages
        .iter()
        .rev()
        .find_map(|message| {
            let trimmed = message.trim();
            (!trimmed.is_empty()
                && !crate::finalize::is_non_answer_separator_message(trimmed)
                && !crate::finalize::looks_like_planner_artifact(trimmed))
            .then_some(trimmed.to_string())
        })
        .unwrap_or_default()
}

pub(super) fn delivery_is_single_line_text(delivery: &str) -> bool {
    delivery
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
        == 1
}

#[cfg(test)]
pub(super) fn single_publishable_delivery_message(delivery_messages: &[String]) -> Option<&str> {
    let mut publishable = delivery_messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .filter(|message| !crate::finalize::is_non_answer_separator_message(message));
    let first = publishable.next()?;
    publishable.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_answer_text_from_delivery_joins_publishable_chunks() {
        let delivery = vec![
            "**执行过程**\n1. 调用技能 `read_file`".to_string(),
            "第一部分内容。".to_string(),
            "第二部分内容。".to_string(),
        ];

        assert_eq!(
            final_answer_text_from_delivery(&delivery),
            "第一部分内容。\n\n第二部分内容。"
        );
    }

    #[test]
    fn final_answer_text_from_delivery_ignores_minimax_tool_call_markup() {
        let delivery = vec![
            "我需要读取 Cargo.toml。\n<minimax:tool_call>\n<invoke name=\"fs_basic\"></invoke>\n</minimax:tool_call>".to_string(),
            "Cargo.toml 是 Rust 项目的包清单。".to_string(),
        ];

        assert_eq!(
            final_answer_text_from_delivery(&delivery),
            "Cargo.toml 是 Rust 项目的包清单。"
        );
    }

    #[test]
    fn final_answer_text_from_delivery_ignores_non_answer_separator() {
        assert_eq!(
            final_answer_text_from_delivery(&["---SEPARATOR---".to_string()]),
            ""
        );
        assert_eq!(
            final_answer_text_from_delivery(&[
                "---SEPARATOR---".to_string(),
                "state=true can_poll=true".to_string(),
            ]),
            "state=true can_poll=true"
        );
        assert!(single_publishable_delivery_message(&["---SEPARATOR---".to_string()]).is_none());
    }
}
