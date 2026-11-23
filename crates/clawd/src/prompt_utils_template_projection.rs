//! Remove only the exact editor scaffold from trusted prompt templates.
//! Run before interpolation: request text and observed evidence stay byte-for-byte intact.
use std::borrow::Cow;

const EDITOR_SCAFFOLD: &str = "<!-- Reserved for language-specific reinforcement.\nUse these optional subheading labels when needed:\n### zh-CN\n- ...\n### en\n- ...\nKeep only language-specific nuances here; keep general rules in the main prompt body.\n-->";

pub(crate) fn project_template(template: &str) -> Cow<'_, str> {
    if !template.contains(EDITOR_SCAFFOLD) {
        return Cow::Borrowed(template);
    }
    let mut result = String::with_capacity(template.len());
    let mut offset = 0;
    let mut fence: Option<(char, usize)> = None;
    while offset < template.len() {
        let remaining = &template[offset..];
        if fence.is_none()
            && remaining.starts_with(EDITOR_SCAFFOLD)
            && remaining[EDITOR_SCAFFOLD.len()..]
                .chars()
                .next()
                .is_none_or(|c| c == '\n')
        {
            offset += EDITOR_SCAFFOLD.len();
            continue;
        }
        let length = remaining.find('\n').map_or(remaining.len(), |i| i + 1);
        let line = &remaining[..length];
        let trimmed = line.trim_start();
        if let Some(delimiter @ ('`' | '~')) = trimmed.chars().next() {
            let count = trimmed.chars().take_while(|c| *c == delimiter).count();
            if count >= 3 {
                match fence {
                    None => fence = Some((delimiter, count)),
                    Some((open, size))
                        if open == delimiter
                            && count >= size
                            && trimmed[count..].trim().is_empty() =>
                    {
                        fence = None;
                    }
                    _ => {}
                }
            }
        }
        result.push_str(line);
        offset += length;
    }
    Cow::Owned(result)
}

#[cfg(test)]
#[path = "prompt_utils_template_projection_tests.rs"]
mod tests;
