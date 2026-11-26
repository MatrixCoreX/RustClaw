use quick_xml::{events::Event, Reader};

const MAX_EXCERPT_CHARS: usize = 600;

pub(super) fn from_tags(xml: &str, tags: &[&str]) -> Option<String> {
    tags.iter().find_map(|tag| {
        let raw = super::extract_tag_text(xml, tag)?;
        // A plain XML text value can contain a literal comparison such as "2 < 3".
        let markup = raw
            .as_bytes()
            .windows(2)
            .any(|v| v[0] == b'<' && (v[1].is_ascii_alphabetic() || matches!(v[1], b'/' | b'!')));
        let text = if markup { markup_text(&raw)? } else { raw };
        let text = super::compact_text(&text);
        if text.is_empty() {
            return None;
        }
        if text.chars().count() <= MAX_EXCERPT_CHARS {
            return Some(text);
        }
        // Keep complete sentences so a cutoff cannot change a number, unit or qualifier.
        let prefix: String = text.chars().take(MAX_EXCERPT_CHARS).collect();
        let end = prefix
            .char_indices()
            .filter_map(|(at, ch)| {
                let after = at + ch.len_utf8();
                if "。！？".contains(ch)
                    || (".!?".contains(ch) && text[after..].starts_with(char::is_whitespace))
                {
                    Some(after)
                } else {
                    None
                }
            })
            .last()?;
        Some(prefix[..end].to_owned())
    })
}

fn markup_text(html: &str) -> Option<String> {
    let mut reader = Reader::from_str(html);
    let mut text = String::new();
    loop {
        match reader.read_event().ok()? {
            Event::Eof => break,
            Event::Text(event) => text.push_str(&event.decode().ok()?),
            Event::CData(event) => text.push_str(&event.decode().ok()?),
            Event::GeneralRef(event) => {
                let escaped = format!("&{};", event.decode().ok()?);
                if escaped == "&nbsp;" {
                    text.push(' ');
                } else {
                    text.push_str(&super::xml_unescape(&escaped));
                }
            }
            Event::Start(event) | Event::Empty(event) => {
                let name = event.local_name();
                let name = std::str::from_utf8(name.as_ref())
                    .ok()?
                    .to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "script" | "style" | "object" | "iframe" | "template"
                ) {
                    return None;
                }
                if matches!(
                    name.as_str(),
                    "p" | "div" | "br" | "li" | "section" | "h1" | "h2"
                ) {
                    text.push(' ');
                }
            }
            Event::End(event) => {
                if [b"p".as_slice(), b"div", b"li", b"section"]
                    .contains(&event.local_name().as_ref())
                {
                    text.push(' ');
                }
            }
            _ => {}
        }
    }
    Some(text)
}
