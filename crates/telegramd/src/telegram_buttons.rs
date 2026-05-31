use std::collections::HashMap;

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

const BUTTON_PREFIX: &str = "BUTTON:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UrlButtonSpec {
    pub(crate) label: String,
    pub(crate) url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedUrlButtons {
    pub(crate) text_without_buttons: String,
    pub(crate) buttons: Vec<UrlButtonSpec>,
}

pub(crate) fn extract_url_buttons_from_text(text: &str) -> ExtractedUrlButtons {
    let mut kept_lines = Vec::new();
    let mut buttons = Vec::new();
    let mut seen_labels: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(button_body) = trimmed.strip_prefix(BUTTON_PREFIX).map(str::trim) else {
            kept_lines.push(line.to_string());
            continue;
        };
        let Some((label_raw, url_raw)) = button_body
            .split_once('：')
            .or_else(|| button_body.split_once(':'))
        else {
            kept_lines.push(line.to_string());
            continue;
        };
        let url = url_raw.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            kept_lines.push(line.to_string());
            continue;
        }
        let base_label = label_raw.trim();
        if base_label.is_empty() {
            kept_lines.push(line.to_string());
            continue;
        }
        let count = seen_labels.entry(base_label.to_string()).or_insert(0);
        *count += 1;
        let label = if *count == 1 {
            base_label.to_string()
        } else {
            format!("{base_label} {}", *count)
        };
        buttons.push(UrlButtonSpec {
            label,
            url: url.to_string(),
        });
    }
    let text_without_buttons = kept_lines
        .join("\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    ExtractedUrlButtons {
        text_without_buttons,
        buttons,
    }
}

pub(crate) fn build_url_button_markup(buttons: &[UrlButtonSpec]) -> Option<InlineKeyboardMarkup> {
    let rows = buttons
        .iter()
        .filter_map(|button| {
            button
                .url
                .parse()
                .ok()
                .map(|parsed| vec![InlineKeyboardButton::url(button.label.clone(), parsed)])
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        None
    } else {
        Some(InlineKeyboardMarkup::new(rows))
    }
}

#[cfg(test)]
#[path = "telegram_buttons_tests.rs"]
mod tests;
