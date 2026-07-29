use anyhow::{anyhow, Result};

#[derive(Debug)]
pub(super) enum ExtractOutcome {
    Text {
        text: String,
        parser_version: String,
    },
    Skip {
        reason: String,
    },
}

pub(super) fn extract_document(bytes: &[u8], file_type: &str) -> Result<ExtractOutcome> {
    if bytes.iter().any(|byte| *byte == 0) {
        return Ok(ExtractOutcome::Skip {
            reason: "binary content contains NUL bytes".to_string(),
        });
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let raw = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            return Ok(ExtractOutcome::Skip {
                reason: "content is not valid UTF-8".to_string(),
            })
        }
    };
    let (text, parser_version) = match file_type {
        "json" => {
            let value: serde_json::Value =
                serde_json::from_str(raw).map_err(|error| anyhow!("invalid JSON: {error}"))?;
            (
                serde_json::to_string_pretty(&value)?,
                "json-structured-v1".to_string(),
            )
        }
        "csv" | "tsv" => (normalize_delimited(raw), format!("{}-rows-v1", file_type)),
        "html" | "htm" => (strip_html(raw), "html-visible-text-v1".to_string()),
        _ => (raw.replace("\r\n", "\n"), "utf8-text-v2".to_string()),
    };
    if text.trim().is_empty() {
        return Ok(ExtractOutcome::Skip {
            reason: "document has no extractable text".to_string(),
        });
    }
    Ok(ExtractOutcome::Text {
        text,
        parser_version,
    })
}

fn normalize_delimited(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    let mut last_space = false;
    for ch in raw.chars() {
        match ch {
            '<' => {
                in_tag = true;
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            '>' => in_tag = false,
            _ if in_tag => {}
            '&' => {
                out.push(' ');
                last_space = true;
            }
            _ if ch.is_whitespace() => {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            _ => {
                out.push(ch);
                last_space = false;
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "ingest_extract_tests.rs"]
mod tests;
