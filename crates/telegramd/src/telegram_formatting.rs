use super::*;

pub(super) async fn send_text_or_image(
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    answer: &str,
) -> anyhow::Result<()> {
    const PREFIX: &str = "IMAGE_FILE:";
    const FILE_PREFIX: &str = "FILE:";
    const VOICE_PREFIX: &str = "VOICE_FILE:";
    const EPHEMERAL_PREFIX: &str = "EPHEMERAL:";
    const EPHEMERAL_IMAGE_SAVED_TOKEN: &str = "EPHEMERAL:IMAGE_SAVED";
    let parsed_buttons = extract_url_buttons_from_text(answer);
    let answer = parsed_buttons.text_without_buttons.as_str();
    let url_buttons = &parsed_buttons.buttons;

    let mut image_paths = dedupe_preserve_order(extract_prefixed_paths(answer, PREFIX));
    let explicit_file_tokens = dedupe_preserve_order(extract_prefixed_tokens(answer, FILE_PREFIX));
    let (explicit_file_paths, missing_explicit_file_tokens) =
        resolve_delivery_paths(&explicit_file_tokens);
    let mut file_paths = explicit_file_paths.clone();
    let voice_paths = dedupe_preserve_order(extract_prefixed_paths(answer, VOICE_PREFIX));
    let inferred_write_paths = if file_paths.is_empty() {
        dedupe_preserve_order(extract_written_file_paths(answer))
    } else {
        Vec::new()
    };
    if !inferred_write_paths.is_empty() {
        file_paths.extend(inferred_write_paths.clone());
        file_paths = dedupe_preserve_order(file_paths);
    }
    // If both IMAGE_FILE and FILE contain the same path, keep FILE only.
    let file_set = file_paths.iter().cloned().collect::<HashSet<_>>();
    image_paths.retain(|p| !file_set.contains(p));

    if !image_paths.is_empty()
        || !file_paths.is_empty()
        || !voice_paths.is_empty()
        || !missing_explicit_file_tokens.is_empty()
    {
        debug!(
            "phase=deliver_media chat_id={} answer_fp={} image_count={} file_count={} voice_count={} preface_preview={}",
            chat_id.0,
            text_fingerprint_hex(answer),
            image_paths.len(),
            file_paths.len(),
            voice_paths.len(),
            text_preview_for_log(answer, 120)
        );
        let ephemeral_image_saved_hint = answer.lines().any(|line| {
            line.trim()
                .eq_ignore_ascii_case(EPHEMERAL_IMAGE_SAVED_TOKEN)
        });
        let mut text_without_tokens = strip_prefixed_tokens(
            answer,
            &[PREFIX, FILE_PREFIX, VOICE_PREFIX, EPHEMERAL_PREFIX],
        )
        .trim()
        .to_string();
        if !inferred_write_paths.is_empty() {
            text_without_tokens = strip_written_file_confirmation_lines(&text_without_tokens)
                .trim()
                .to_string();
        }
        if !text_without_tokens.is_empty() {
            let sent = if url_buttons.is_empty() {
                send_telegram_text(bot, chat_id, &text_without_tokens)
                    .await
                    .context("send file preface text failed")?
            } else {
                send_telegram_text_with_url_buttons(
                    bot,
                    chat_id,
                    &text_without_tokens,
                    &url_buttons,
                )
                .await
                .context("send file preface text with buttons failed")?
            };
            debug!(
                "phase=deliver_media_preface chat_id={} answer_fp={} telegram_msg_id={} text_preview={}",
                chat_id.0,
                text_fingerprint_hex(&text_without_tokens),
                sent.id.0,
                text_preview_for_log(&text_without_tokens, 120)
            );
            if state.ephemeral_image_saved_seconds > 0 && ephemeral_image_saved_hint {
                let bot_clone = bot.clone();
                let msg_id = sent.id;
                let secs = state.ephemeral_image_saved_seconds;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    let _ = bot_clone.delete_message(chat_id, msg_id).await;
                });
            }
        } else if explicit_file_paths.is_empty()
            && missing_explicit_file_tokens.is_empty()
            && !inferred_write_paths.is_empty()
        {
            if let Some(inline_text) =
                inline_single_small_text_file(&file_paths, &image_paths, &voice_paths)
            {
                let sent = if url_buttons.is_empty() {
                    send_telegram_text(bot, chat_id, &inline_text)
                        .await
                        .context("send inline text file body failed")?
                } else {
                    send_telegram_text_with_url_buttons(bot, chat_id, &inline_text, &url_buttons)
                        .await
                        .context("send inline text file body with buttons failed")?
                };
                debug!(
                    "phase=deliver_inline_text_file chat_id={} answer_fp={} telegram_msg_id={} text_preview={}",
                    chat_id.0,
                    text_fingerprint_hex(&inline_text),
                    sent.id.0,
                    text_preview_for_log(&inline_text, 120)
                );
                return Ok(());
            }
        }
        if !missing_explicit_file_tokens.is_empty() {
            warn!(
                "phase=deliver_media_missing_file chat_id={} missing_paths={:?}",
                chat_id.0, missing_explicit_file_tokens
            );
            let missing_paths = missing_explicit_file_tokens.join("\n");
            let missing_text = state.i18n.t_with(
                "telegram.msg.delivery_file_missing",
                &[("paths", &missing_paths)],
            );
            let _ = send_telegram_text(bot, chat_id, &missing_text).await;
        }

        for path in image_paths {
            bot.send_photo(chat_id, InputFile::file(path))
                .await
                .context("send image file failed")?;
        }

        for path in file_paths {
            // FILE: always means "send as document/file", even for image extensions.
            bot.send_document(chat_id, InputFile::file(path))
                .await
                .context("send document file failed")?;
        }

        for path in voice_paths {
            if let Err(err) = bot.send_voice(chat_id, InputFile::file(path.clone())).await {
                warn!("send_voice failed for {}: {}", path, err);
                bot.send_document(chat_id, InputFile::file(path))
                    .await
                    .context("fallback send voice as document failed")?;
            }
        }
        return Ok(());
    }

    let sent = if url_buttons.is_empty() {
        send_telegram_text(bot, chat_id, answer)
            .await
            .context("send text message failed")?
    } else {
        send_telegram_text_with_url_buttons(bot, chat_id, answer, &url_buttons)
            .await
            .context("send text message with buttons failed")?
    };
    debug!(
        "phase=deliver_text chat_id={} answer_fp={} telegram_msg_id={} answer_preview={}",
        chat_id.0,
        text_fingerprint_hex(answer),
        sent.id.0,
        text_preview_for_log(answer, 120)
    );
    Ok(())
}

pub(super) fn inline_single_small_text_file(
    file_paths: &[String],
    image_paths: &[String],
    voice_paths: &[String],
) -> Option<String> {
    if !image_paths.is_empty() || !voice_paths.is_empty() || file_paths.len() != 1 {
        return None;
    }
    let path = file_paths.first()?;
    if !is_inline_text_file(path) {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > TELEGRAM_INLINE_TEXT_FILE_MAX_CHARS
        || trimmed.lines().count() > TELEGRAM_INLINE_TEXT_FILE_MAX_LINES
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(super) fn is_inline_text_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".txt")
        || lower.ends_with(".md")
        || lower.ends_with(".markdown")
        || lower.ends_with(".json")
        || lower.ends_with(".csv")
        || lower.ends_with(".log")
}

/// Max characters per Telegram message (conservative; platform limit ~4096).
const TELEGRAM_TEXT_CHUNK_CHARS: usize = 3500;
const TELEGRAM_INLINE_TEXT_FILE_MAX_CHARS: usize = 3000;
const TELEGRAM_INLINE_TEXT_FILE_MAX_LINES: usize = 120;

pub(super) fn telegram_text_payload(text: &str) -> (String, Option<ParseMode>) {
    let trimmed = text.trim();
    if let Some(code_body) = code_or_command_block_body(trimmed) {
        return (
            format!(
                "<pre><code>{}</code></pre>",
                escape_telegram_html(&code_body)
            ),
            Some(ParseMode::Html),
        );
    }
    let normalized = normalize_markdown_heading_markers(text);
    if let Some(structured_html) = render_structured_message_html(&normalized) {
        return (structured_html, Some(ParseMode::Html));
    }
    if let Some(inline_html) = render_inline_code_html(&normalized) {
        return (inline_html, Some(ParseMode::Html));
    }
    if let Some(copyable_html) = render_copyable_tokens_html(&normalized) {
        return (copyable_html, Some(ParseMode::Html));
    }
    (normalized, None)
}

pub(super) async fn send_telegram_text(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
) -> anyhow::Result<Message> {
    let chunks = chunk_text_for_channel(
        text,
        TELEGRAM_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    if chunks.is_empty() {
        return Err(anyhow::anyhow!("empty text"));
    }
    if chunks.len() == 1 {
        let (body, parse_mode) = telegram_text_payload(&chunks[0]);
        let req = bot.send_message(chat_id, body);
        let req = if let Some(mode) = parse_mode {
            req.parse_mode(mode)
        } else {
            req
        };
        return Ok(req.await?);
    }
    let n = chunks.len();
    info!(
        "send_chunks channel=telegram chat_id={:?} original_len={} chunk_count={}",
        chat_id,
        text.len(),
        n
    );
    // Long text: send each chunk as plain text (no HTML/code) with segment hint.
    let mut last = None;
    for (i, chunk) in chunks.into_iter().enumerate() {
        let segment_text = format!("（{}/{}）\n{}", i + 1, n, chunk);
        let (body, parse_mode) = telegram_text_payload(&segment_text);
        info!(
            "send_chunk channel=telegram chat_id={:?} index={} total={}",
            chat_id,
            i + 1,
            n
        );
        let req = bot.send_message(chat_id, body);
        let req = if let Some(mode) = parse_mode {
            req.parse_mode(mode)
        } else {
            req
        };
        let msg = req.await?;
        last = Some(msg);
    }
    Ok(last.expect("chunks non-empty"))
}

pub(super) async fn send_telegram_text_with_url_buttons(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    buttons: &[UrlButtonSpec],
) -> anyhow::Result<Message> {
    let chunks = chunk_text_for_channel(
        text,
        TELEGRAM_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    if chunks.is_empty() {
        return Err(anyhow::anyhow!("empty text"));
    }
    let Some(keyboard) = build_url_button_markup(buttons) else {
        return send_telegram_text(bot, chat_id, text).await;
    };
    if chunks.len() == 1 {
        let (body, parse_mode) = telegram_text_payload(&chunks[0]);
        let req = bot.send_message(chat_id, body).reply_markup(keyboard);
        let req = if let Some(mode) = parse_mode {
            req.parse_mode(mode)
        } else {
            req
        };
        return Ok(req.await?);
    }
    let n = chunks.len();
    let mut last = None;
    for (i, chunk) in chunks.into_iter().enumerate() {
        let segment_text = format!("（{}/{}）\n{}", i + 1, n, chunk);
        let (body, parse_mode) = telegram_text_payload(&segment_text);
        let req = bot.send_message(chat_id, body);
        let req = if i + 1 == n {
            req.reply_markup(keyboard.clone())
        } else {
            req
        };
        let req = if let Some(mode) = parse_mode {
            req.parse_mode(mode)
        } else {
            req
        };
        let msg = req.await?;
        last = Some(msg);
    }
    Ok(last.expect("chunks non-empty"))
}

pub(super) fn escape_telegram_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(super) fn render_inline_code_html(text: &str) -> Option<String> {
    if !text.contains('`') || text.contains("```") || has_delivery_prefix(text.trim()) {
        return None;
    }
    let mut out = String::new();
    let mut buf = String::new();
    let mut in_code = false;
    let mut saw_code = false;
    for ch in text.chars() {
        if ch == '`' {
            if in_code {
                if buf.is_empty() {
                    out.push('`');
                } else {
                    out.push_str("<code>");
                    out.push_str(&escape_telegram_html(&buf));
                    out.push_str("</code>");
                    saw_code = true;
                }
                buf.clear();
                in_code = false;
            } else {
                out.push_str(&escape_telegram_html(&buf));
                buf.clear();
                in_code = true;
            }
        } else {
            buf.push(ch);
        }
    }
    if in_code {
        out.push('`');
    }
    out.push_str(&escape_telegram_html(&buf));
    if saw_code {
        Some(out)
    } else {
        None
    }
}

pub(super) fn render_structured_message_html(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || has_delivery_prefix(trimmed)
        || trimmed.contains("```")
        || trimmed.starts_with("<pre>")
    {
        return None;
    }

    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 {
        return None;
    }

    let mut structured_score = 0usize;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if split_bullet_marker(trimmed).is_some() {
            structured_score += 1;
            continue;
        }
        if parse_structured_key_value(trimmed).is_some() {
            structured_score += 1;
            continue;
        }
        if looks_like_command_example_line(trimmed) || looks_like_section_header_line(trimmed) {
            structured_score += 1;
        }
    }

    if structured_score < 2 {
        return None;
    }

    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        out.push(render_structured_line_html(line));
    }
    Some(out.join("\n"))
}

pub(super) fn normalize_markdown_heading_markers(text: &str) -> String {
    text.lines()
        .map(normalize_markdown_heading_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn normalize_markdown_heading_line(line: &str) -> String {
    let indent_len = line.len().saturating_sub(line.trim_start().len());
    let indent = &line[..indent_len];
    let mut body = line[indent_len..].to_string();
    let mut changed = false;

    if let Some(stripped) = strip_markdown_heading_prefix(&body) {
        body = stripped.to_string();
        changed = true;
    }

    let (list_prefix, rest) = split_list_prefix(&body);
    let rest_trimmed = rest.trim();
    let unwrapped = strip_surrounding_emphasis(rest_trimmed);
    if unwrapped != rest_trimmed && (changed || looks_like_heading_text(&unwrapped)) {
        let rebuilt = if list_prefix.is_empty() {
            unwrapped.to_string()
        } else {
            format!("{list_prefix}{unwrapped}")
        };
        return format!("{indent}{rebuilt}");
    }

    if changed {
        return format!("{indent}{}", body.trim_start());
    }

    line.to_string()
}

pub(super) fn strip_markdown_heading_prefix(input: &str) -> Option<&str> {
    let mut idx = 0usize;
    let bytes = input.as_bytes();
    while idx < bytes.len() && bytes[idx] == b'#' {
        idx += 1;
    }
    if idx == 0 || idx > 6 {
        return None;
    }
    if idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
        return Some(input[idx..].trim_start());
    }
    None
}

pub(super) fn split_list_prefix(input: &str) -> (&str, &str) {
    let trimmed = input.trim_start();
    let leading_ws_len = input.len().saturating_sub(trimmed.len());
    for marker in ["- ", "* ", "+ ", "• "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            let prefix_len = leading_ws_len + marker.len();
            return (&input[..prefix_len], rest);
        }
    }

    let mut digit_count = 0usize;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            digit_count += 1;
        } else {
            break;
        }
    }
    if digit_count > 0 {
        let after_digits = &trimmed[digit_count..];
        if let Some(rest) = after_digits.strip_prefix(". ") {
            let prefix_len = leading_ws_len + digit_count + 2;
            return (&input[..prefix_len], rest);
        }
        if let Some(rest) = after_digits.strip_prefix(") ") {
            let prefix_len = leading_ws_len + digit_count + 2;
            return (&input[..prefix_len], rest);
        }
    }

    ("", input)
}

pub(super) fn strip_surrounding_emphasis(input: &str) -> String {
    let mut out = input.trim().to_string();
    loop {
        let next = if out.len() >= 4 && out.starts_with("**") && out.ends_with("**") {
            Some(out[2..out.len().saturating_sub(2)].trim().to_string())
        } else if out.len() >= 4 && out.starts_with("__") && out.ends_with("__") {
            Some(out[2..out.len().saturating_sub(2)].trim().to_string())
        } else if out.len() >= 2 && out.starts_with('*') && out.ends_with('*') {
            Some(out[1..out.len().saturating_sub(1)].trim().to_string())
        } else if out.len() >= 2 && out.starts_with('_') && out.ends_with('_') {
            Some(out[1..out.len().saturating_sub(1)].trim().to_string())
        } else {
            None
        };
        match next {
            Some(v) if !v.is_empty() => out = v,
            _ => break,
        }
    }
    out
}

pub(super) fn looks_like_heading_text(input: &str) -> bool {
    let len = input.chars().count();
    len > 0 && len <= 80
}

pub(super) fn render_structured_line_html(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if looks_like_command_example_line(trimmed) {
        return format!("<code>{}</code>", escape_telegram_html(trimmed));
    }

    if let Some((marker, rest)) = split_bullet_marker(trimmed) {
        if looks_like_command_example_line(rest) {
            return format!(
                "{} <code>{}</code>",
                escape_telegram_html(marker),
                escape_telegram_html(rest)
            );
        }
        if let Some((key, sep, value)) = parse_structured_key_value(rest) {
            return format!(
                "{} <b>{}{}</b> {}",
                escape_telegram_html(marker),
                escape_telegram_html(key),
                escape_telegram_html(sep),
                render_inline_copyable_fragment_html(value)
            );
        }
        return format!(
            "{} {}",
            escape_telegram_html(marker),
            render_inline_copyable_fragment_html(rest)
        );
    }

    if let Some((key, sep, value)) = parse_structured_key_value(trimmed) {
        return format!(
            "<b>{}{}</b> {}",
            escape_telegram_html(key),
            escape_telegram_html(sep),
            render_inline_copyable_fragment_html(value)
        );
    }

    if looks_like_section_header_line(trimmed) {
        return format!("<b>{}</b>", escape_telegram_html(trimmed));
    }

    render_inline_copyable_fragment_html(trimmed)
}

pub(super) fn render_inline_copyable_fragment_html(text: &str) -> String {
    if let Some(rendered) = render_inline_code_html(text) {
        return rendered;
    }
    if let Some(rendered) = render_copyable_tokens_html(text) {
        return rendered;
    }
    escape_telegram_html(text)
}

pub(super) fn split_bullet_marker(input: &str) -> Option<(&str, &str)> {
    for marker in ["- ", "* ", "+ ", "• "] {
        if let Some(rest) = input.strip_prefix(marker) {
            return Some((marker.trim_end(), rest.trim_start()));
        }
    }

    let digit_count = input.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count > 0 {
        let after_digits = &input[digit_count..];
        if let Some(rest) = after_digits.strip_prefix(". ") {
            return Some((&input[..digit_count + 1], rest.trim_start()));
        }
        if let Some(rest) = after_digits.strip_prefix(") ") {
            return Some((&input[..digit_count + 1], rest.trim_start()));
        }
    }

    None
}

pub(super) fn parse_structured_key_value(input: &str) -> Option<(&str, &str, &str)> {
    for sep in ["：", ": ", "=", "＝"] {
        if let Some((left, right)) = input.split_once(sep) {
            let key = left.trim();
            let value = right.trim();
            if key.is_empty() || value.is_empty() {
                continue;
            }
            let key_len = key.chars().count();
            if key_len > 36 || key.contains('\n') || value.contains('\n') {
                continue;
            }
            return Some((key, sep, value));
        }
    }
    None
}

pub(super) fn looks_like_command_example_line(input: &str) -> bool {
    input.starts_with('/') || command_after_label_separator(input)
}

fn command_after_label_separator(input: &str) -> bool {
    [":", "："].iter().any(|sep| {
        input.split_once(sep).is_some_and(|(label, rest)| {
            let label_len = label.trim().chars().count();
            (1..=24).contains(&label_len) && rest.trim_start().starts_with('/')
        })
    })
}

pub(super) fn looks_like_section_header_line(input: &str) -> bool {
    let len = input.chars().count();
    len > 0
        && len <= 48
        && !input.contains('\n')
        && (input.ends_with('：') || input.ends_with(':'))
        && parse_structured_key_value(input).is_none()
}

pub(super) fn render_copyable_tokens_html(text: &str) -> Option<String> {
    if text.trim().is_empty()
        || has_delivery_prefix(text.trim())
        || text.contains("```")
        || text.starts_with("<pre>")
    {
        return None;
    }

    let mut out = String::new();
    let mut plain_start = 0usize;
    let mut wrapped_any = false;
    let mut token_start: Option<usize> = None;
    let mut indices: Vec<(usize, char)> = text.char_indices().collect();
    indices.push((text.len(), '\0'));

    for (idx, ch) in &indices {
        let is_token_char = is_copyable_token_char(*ch);
        match (token_start, is_token_char) {
            (None, true) => {
                token_start = Some(*idx);
            }
            (Some(start), false) => {
                if plain_start < start {
                    out.push_str(&escape_telegram_html(&text[plain_start..start]));
                }
                let token = &text[start..*idx];
                if let Some(html) = wrap_copyable_token_html(token) {
                    out.push_str(&html);
                    wrapped_any = true;
                } else {
                    out.push_str(&escape_telegram_html(token));
                }
                plain_start = *idx;
                token_start = None;
            }
            _ => {}
        }
    }

    if plain_start < text.len() {
        out.push_str(&escape_telegram_html(&text[plain_start..]));
    }

    if wrapped_any {
        Some(out)
    } else {
        None
    }
}

pub(super) fn is_copyable_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/' | '%' | '@' | '+')
}

pub(super) fn wrap_copyable_token_html(token: &str) -> Option<String> {
    let (prefix, core, suffix) = split_token_affixes(token);
    if core.is_empty() || !is_copyable_token_core(core) {
        return None;
    }
    Some(format!(
        "{}<code>{}</code>{}",
        escape_telegram_html(prefix),
        escape_telegram_html(core),
        escape_telegram_html(suffix)
    ))
}

pub(super) fn split_token_affixes(token: &str) -> (&str, &str, &str) {
    let chars: Vec<(usize, char)> = token.char_indices().collect();
    if chars.is_empty() {
        return ("", "", "");
    }
    let mut left = 0usize;
    while left < chars.len() && is_token_wrapper_prefix(chars[left].1) {
        left += 1;
    }
    let mut right = chars.len();
    while right > left && is_token_wrapper_suffix(chars[right - 1].1) {
        right -= 1;
    }
    let core_start = if left < chars.len() {
        chars[left].0
    } else {
        token.len()
    };
    let core_end = if right < chars.len() {
        chars[right].0
    } else {
        token.len()
    };
    (
        &token[..core_start],
        &token[core_start..core_end],
        &token[core_end..],
    )
}

pub(super) fn is_token_wrapper_prefix(ch: char) -> bool {
    matches!(
        ch,
        '"' | '\'' | '`' | '(' | '[' | '{' | '<' | '（' | '【' | '《'
    )
}

pub(super) fn is_token_wrapper_suffix(ch: char) -> bool {
    matches!(
        ch,
        '"' | '\''
            | '`'
            | ')'
            | ']'
            | '}'
            | '>'
            | ','
            | ';'
            | '!'
            | '?'
            | ':'
            | '，'
            | '。'
            | '；'
            | '！'
            | '？'
            | '：'
            | '）'
            | '】'
            | '》'
    )
}

pub(super) fn is_copyable_token_core(core: &str) -> bool {
    looks_like_ip_or_endpoint(core)
        || looks_like_number_value(core)
        || looks_like_general_command_token(core)
}

pub(super) fn looks_like_ip_or_endpoint(core: &str) -> bool {
    is_ipv4_token(core) || is_ipv6_token(core) || is_host_port_token(core)
}

pub(super) fn is_ipv4_token(core: &str) -> bool {
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|part| {
        !part.is_empty()
            && part.chars().all(|c| c.is_ascii_digit())
            && part.parse::<u16>().map(|v| v <= 255).unwrap_or(false)
    })
}

pub(super) fn is_ipv6_token(core: &str) -> bool {
    let colon_count = core.chars().filter(|&c| c == ':').count();
    if colon_count < 2 {
        return false;
    }
    core.chars()
        .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
}

pub(super) fn is_host_port_token(core: &str) -> bool {
    let Some((host, port)) = core.rsplit_once(':') else {
        return false;
    };
    if host.is_empty() || port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") || is_ipv4_token(host) {
        return true;
    }
    host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

pub(super) fn looks_like_number_value(core: &str) -> bool {
    let cleaned = core.replace(',', "").replace('_', "");
    if cleaned.chars().all(|c| c.is_ascii_digit()) {
        return cleaned.len() >= 4;
    }
    let Some((left, right)) = cleaned.split_once('.') else {
        return false;
    };
    if left.is_empty()
        || right.is_empty()
        || !left.chars().all(|c| c.is_ascii_digit())
        || !right.chars().all(|c| c.is_ascii_digit())
    {
        return false;
    }
    left.len() + right.len() >= 4
}

pub(super) fn looks_like_general_command_token(core: &str) -> bool {
    let lower = core.to_ascii_lowercase();
    if lower.starts_with("--") && lower.len() > 2 {
        return true;
    }
    if lower.starts_with('-')
        && lower.len() > 2
        && lower[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return true;
    }
    matches!(
        lower.as_str(),
        "rustclaw"
            | "cargo"
            | "npm"
            | "pnpm"
            | "yarn"
            | "python"
            | "python3"
            | "pip"
            | "pip3"
            | "git"
            | "curl"
            | "wget"
            | "ssh"
            | "systemctl"
            | "docker"
            | "kubectl"
            | "node"
            | "bash"
            | "sh"
            | "zsh"
    )
}

/// 新闻摘要、RSS 列表、报告式多行说明等不应被当作代码块发送。
pub(super) fn should_never_format_as_code(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    if t.starts_with("sources_ok=") || t.starts_with("sources_failed=") {
        return true;
    }
    let lines: Vec<&str> = t.lines().map(str::trim).filter(|s| !s.is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    let mut numbered_list_lines = 0usize;
    let mut summary_marker_lines = 0usize;
    for line in &lines {
        let after_digits = line.trim_start_matches(|c: char| c.is_ascii_digit());
        if line.len() > after_digits.len()
            && (after_digits.starts_with(". ") || after_digits.starts_with("."))
        {
            numbered_list_lines += 1;
        }
        let lower = line.to_ascii_lowercase();
        if lower.contains("summary:")
            || lower.contains("topic:")
            || lower.contains("time:")
            || (lower.starts_with("from ") && line.len() < 80)
            || line.contains("🔗")
            || line.contains("🧾")
        {
            summary_marker_lines += 1;
        }
    }
    if numbered_list_lines >= 2 {
        return true;
    }
    if summary_marker_lines >= 1 && lines.len() <= 30 {
        return true;
    }
    false
}

pub(super) fn code_or_command_block_body(text: &str) -> Option<String> {
    if text.is_empty()
        || text.len() > 3600
        || has_delivery_prefix(text)
        || text.starts_with("<pre>")
    {
        return None;
    }
    if should_never_format_as_code(text) {
        return None;
    }
    if let Some((lang, unfenced)) = strip_markdown_code_fence(text) {
        let trimmed = unfenced.trim();
        if !trimmed.is_empty() {
            if language_is_shell(&lang) || looks_like_shell_command_block(trimmed) {
                return Some(add_shell_prompt_prefix(trimmed));
            }
            return Some(trimmed.to_string());
        }
    }
    if looks_like_shell_command_line(text) {
        return Some(add_shell_prompt_prefix(text.trim()));
    }
    if looks_like_shell_command_block(text) {
        return Some(add_shell_prompt_prefix(text.trim()));
    }
    if looks_like_single_line_code(text) || looks_like_multiline_code(text) {
        return Some(text.trim().to_string());
    }
    None
}

pub(super) fn has_delivery_prefix(text: &str) -> bool {
    text.starts_with("FILE:")
        || text.starts_with("IMAGE_FILE:")
        || text.starts_with("VOICE_FILE:")
        || text.starts_with("EPHEMERAL:")
}

pub(super) fn strip_markdown_code_fence(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return None;
    }
    let lines = trimmed.lines().collect::<Vec<_>>();
    if lines.len() < 2 {
        return None;
    }
    let first = lines.first()?.trim_start();
    let last = lines.last()?.trim();
    if !first.starts_with("```") || !last.starts_with("```") {
        return None;
    }
    let lang = first.trim_start_matches("```").trim().to_string();
    Some((lang, lines[1..lines.len().saturating_sub(1)].join("\n")))
}

pub(super) fn language_is_shell(lang: &str) -> bool {
    matches!(
        lang.trim().to_ascii_lowercase().as_str(),
        "bash" | "sh" | "zsh" | "shell" | "console" | "terminal"
    )
}

pub(super) fn add_shell_prompt_prefix(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                String::new()
            } else if trimmed.starts_with('$') {
                trimmed.to_string()
            } else {
                format!("$ {trimmed}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn looks_like_shell_command_line(text: &str) -> bool {
    if text.is_empty() || text.len() > 320 || text.contains('\n') {
        return false;
    }
    let first = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| matches!(c, '"' | '\'' | '`'));
    if first.is_empty() {
        return false;
    }
    let command_heads = [
        "bash",
        "sh",
        "zsh",
        "python",
        "python3",
        "pip",
        "pip3",
        "uv",
        "node",
        "npm",
        "pnpm",
        "yarn",
        "cargo",
        "rustclaw",
        "git",
        "curl",
        "wget",
        "ssh",
        "scp",
        "rsync",
        "ls",
        "pwd",
        "cd",
        "cat",
        "cp",
        "mv",
        "rm",
        "mkdir",
        "chmod",
        "chown",
        "touch",
        "head",
        "tail",
        "grep",
        "rg",
        "sed",
        "awk",
        "find",
        "echo",
        "printf",
        "export",
        "env",
        "source",
        "sudo",
        "systemctl",
        "service",
        "journalctl",
        "docker",
        "docker-compose",
        "kubectl",
        "sqlite3",
        "mysql",
        "psql",
        "ps",
        "pgrep",
        "pkill",
        "kill",
        "killall",
        "uname",
        "df",
        "du",
        "top",
        "htop",
        "free",
        "mount",
        "umount",
        "ip",
        "ifconfig",
        "ss",
        "netstat",
        "lsof",
        "tar",
        "zip",
        "unzip",
        "make",
        "cmake",
        "go",
        "java",
        "javac",
        "perl",
        "ruby",
        "php",
        "lua",
        "deno",
        "npx",
        "brew",
        "apt",
        "apt-get",
        "yum",
        "dnf",
        "pacman",
    ];
    if command_heads
        .iter()
        .any(|cmd| first.eq_ignore_ascii_case(cmd))
    {
        return true;
    }
    first.starts_with("./")
        || first.starts_with("../")
        || first.starts_with("~/")
        || (first.starts_with('/') && first.contains('/'))
}

pub(super) fn looks_like_shell_command_block(text: &str) -> bool {
    if text.is_empty() || !text.contains('\n') || text.len() > 3600 {
        return false;
    }
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    let shell_like = lines
        .iter()
        .filter(|line| looks_like_shell_command_line(line) || line.starts_with('$'))
        .count();
    shell_like >= 2 && shell_like * 2 >= lines.len()
}

pub(super) fn looks_like_single_line_code(text: &str) -> bool {
    if text.is_empty() || text.len() > 320 || text.contains('\n') {
        return false;
    }
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let starters = [
        "fn ",
        "pub fn ",
        "async fn ",
        "let ",
        "const ",
        "var ",
        "val ",
        "def ",
        "class ",
        "import ",
        "from ",
        "export ",
        "#include ",
        "package ",
        "interface ",
        "type ",
        "enum ",
        "impl ",
        "SELECT ",
        "INSERT ",
        "UPDATE ",
        "DELETE ",
        "CREATE ",
        "ALTER ",
        "DROP ",
        "{",
        "[",
        "<?php",
        "#!/usr/bin/env ",
        "#!/bin/bash",
        "#!/bin/sh",
    ];
    if starters
        .iter()
        .any(|s| trimmed.starts_with(s) || lower.starts_with(&s.to_ascii_lowercase()))
    {
        return true;
    }
    (trimmed.contains("=>") && (trimmed.contains('{') || trimmed.contains('(')))
        || (trimmed.contains("::") && (trimmed.contains("fn") || trimmed.contains("impl")))
        || (trimmed.ends_with(';') && (trimmed.contains('=') || trimmed.contains('(')))
        || (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

pub(super) fn looks_like_multiline_code(text: &str) -> bool {
    if text.is_empty() || !text.contains('\n') || text.len() > 3600 {
        return false;
    }
    if should_never_format_as_code(text) {
        return false;
    }
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    let mut score = 0usize;
    for line in &lines {
        let lower = line.to_ascii_lowercase();
        if line.starts_with("#!") {
            score += 2;
        }
        if line.starts_with('$') || line.starts_with("sudo ") || line.starts_with("./") {
            score += 2;
        }
        if looks_like_shell_command_line(line) || looks_like_single_line_code(line) {
            score += 1;
        }
        let looks_like_label_colon = line.ends_with(':')
            && (line.len() < 40
                || lower.contains("summary")
                || lower.contains("topic")
                || lower.contains("time:")
                || lower.starts_with("from ")
                || line.contains("🔗")
                || line.contains("🧾"));
        if line.ends_with('{')
            || line.ends_with('}')
            || line.ends_with(';')
            || (line.ends_with(':') && !looks_like_label_colon)
            || line.starts_with("```")
        {
            score += 1;
        }
        if lower.starts_with("if ")
            || lower.starts_with("for ")
            || lower.starts_with("while ")
            || lower.starts_with("return ")
            || lower.starts_with("match ")
            || lower.starts_with("case ")
            || lower.starts_with("try:")
            || lower.starts_with("except")
            || lower.starts_with("finally:")
            || lower.starts_with("with ")
            || lower.starts_with("echo ")
        {
            score += 1;
        }
    }
    score >= 4
}

pub(super) fn extract_prefixed_paths(answer: &str, prefix: &str) -> Vec<String> {
    let tokens = extract_prefixed_tokens(answer, prefix);
    let (resolved, _) = resolve_delivery_paths(&tokens);
    resolved
}

pub(super) fn extract_prefixed_tokens(answer: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in answer.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let cleaned = normalize_path_token(rest.trim());
            if !cleaned.is_empty() {
                out.push(cleaned.to_string());
            }
        }
    }
    out
}

pub(super) fn resolve_delivery_paths(tokens: &[String]) -> (Vec<String>, Vec<String>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for token in tokens {
        if let Some(path) = resolve_delivery_token_path(token) {
            found.push(path);
        } else {
            missing.push(token.clone());
        }
    }
    (dedupe_preserve_order(found), dedupe_preserve_order(missing))
}

pub(super) fn resolve_delivery_token_path(token: &str) -> Option<String> {
    let cleaned = normalize_path_token(token);
    if cleaned.is_empty() {
        return None;
    }
    let candidate = if Path::new(cleaned).is_absolute() {
        PathBuf::from(cleaned)
    } else {
        let cwd = std::env::current_dir().ok()?;
        cwd.join(cleaned)
    };
    if candidate.is_file() {
        return Some(candidate.to_string_lossy().to_string());
    }
    if Path::new(cleaned).is_file() {
        return Some(cleaned.to_string());
    }
    None
}

pub(super) fn is_written_file_confirmation_line(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(rest) = trimmed.strip_prefix("written ") else {
        return false;
    };
    let Some((bytes_text, path_text)) = rest.split_once(" bytes to ") else {
        return false;
    };
    if bytes_text.trim().parse::<u64>().is_err() {
        return false;
    }
    let cleaned = normalize_path_token(path_text.trim());
    !cleaned.is_empty() && Path::new(cleaned).is_file()
}

pub(super) fn extract_written_file_paths(answer: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in answer.lines() {
        if !is_written_file_confirmation_line(line) {
            continue;
        }
        let Some(rest) = line.trim().strip_prefix("written ") else {
            continue;
        };
        let Some((_, path_text)) = rest.split_once(" bytes to ") else {
            continue;
        };
        let cleaned = normalize_path_token(path_text.trim());
        out.push(cleaned.to_string());
    }
    out
}

pub(super) fn strip_written_file_confirmation_lines(answer: &str) -> String {
    answer
        .lines()
        .filter(|line| !is_written_file_confirmation_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn dedupe_preserve_order(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

pub(super) fn text_fingerprint_hex(text: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(super) fn text_preview_for_log(text: &str, max_chars: usize) -> String {
    let normalized = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    normalized.chars().take(max_chars).collect::<String>() + "...(truncated)"
}

pub(super) fn strip_prefixed_tokens(answer: &str, prefixes: &[&str]) -> String {
    answer
        .lines()
        .filter(|line| {
            !prefixes
                .iter()
                .any(|prefix| line.trim_start().starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn strip_delivery_tokens_for_tts(answer: &str) -> String {
    strip_prefixed_tokens(
        answer,
        &["IMAGE_FILE:", "FILE:", "VOICE_FILE:", "EPHEMERAL:"],
    )
    .trim()
    .to_string()
}

pub(super) fn normalize_path_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '`' | '，' | ',' | ':' | '：' | ';' | '。' | ')' | '(' | '）' | '（'
        )
    })
}

pub(super) fn resolve_sendfile_path(
    raw: &str,
    full_access: bool,
    allowed_dirs: &[String],
) -> Result<PathBuf, String> {
    let token = normalize_path_token(raw);
    if token.is_empty() {
        return Err("empty path".to_string());
    }

    let cwd = std::env::current_dir().map_err(|err| format!("read current_dir failed: {err}"))?;
    let candidate = if Path::new(token).is_absolute() {
        PathBuf::from(token)
    } else {
        cwd.join(token)
    };
    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("path with '..' is not allowed".to_string());
    }
    if full_access {
        return Ok(candidate);
    }

    for dir in allowed_dirs {
        if dir == "*" {
            return Ok(candidate);
        }
        let base = if Path::new(dir).is_absolute() {
            PathBuf::from(dir)
        } else {
            cwd.join(dir)
        };
        if candidate.starts_with(&base) {
            return Ok(candidate);
        }
    }

    Err(format!(
        "path is outside allowed dirs: {}",
        allowed_dirs.join(", ")
    ))
}

pub(super) fn is_image_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
}
