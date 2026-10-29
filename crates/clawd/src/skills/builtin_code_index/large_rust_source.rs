use std::io::{self, BufRead, BufReader, Read};

use sha2::{Digest, Sha256};

use super::{
    language_parsers, rust_definition_from_line, SymbolDefinition, MAX_SNIPPET_BYTES,
    MAX_SOURCE_BYTES,
};

pub(super) struct Scan {
    pub(super) sha256: String,
    pub(super) status: &'static str,
    pub(super) symbols: Vec<SymbolDefinition>,
}

pub(super) fn scan(reader: impl Read) -> io::Result<Scan> {
    let mut reader = BufReader::new(reader);
    let mut hash = Sha256::new();
    let mut line = Vec::new();
    let mut line_number = 1;
    let mut oversized = false;
    let mut partial = false;
    let mut symbols = Vec::new();
    let mut symbol_bytes = 0;
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            if !line.is_empty() || oversized {
                observe_line(
                    &line,
                    line_number,
                    oversized,
                    &mut partial,
                    &mut symbols,
                    &mut symbol_bytes,
                );
            }
            break;
        }
        let newline = bytes.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(bytes.len(), |index| index + 1);
        hash.update(&bytes[..consumed]);
        let content_len = newline.unwrap_or(consumed);
        if !oversized {
            if content_len > MAX_SNIPPET_BYTES.saturating_sub(line.len()) {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&bytes[..content_len]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            observe_line(
                &line,
                line_number,
                oversized,
                &mut partial,
                &mut symbols,
                &mut symbol_bytes,
            );
            line.clear();
            oversized = false;
            line_number += 1;
        }
    }
    Ok(Scan {
        sha256: format!("{:x}", hash.finalize()),
        status: if partial {
            "range_index_budget_exceeded"
        } else {
            "range_indexed"
        },
        symbols,
    })
}

fn observe_line(
    line: &[u8],
    line_number: usize,
    oversized: bool,
    partial: &mut bool,
    symbols: &mut Vec<SymbolDefinition>,
    symbol_bytes: &mut usize,
) {
    // The existing snippet quota also bounds fallback lines; never parse a prefix
    // of a truncated line as if it were a complete definition.
    if oversized {
        *partial = true;
        return;
    }
    let Ok(line) = std::str::from_utf8(line) else {
        *partial = true;
        return;
    };
    if let Some((kind, name)) = rust_definition_from_line(line) {
        let bytes = name.len().saturating_mul(2);
        if symbols.len() >= language_parsers::MAX_ENTRIES
            || bytes > (MAX_SOURCE_BYTES as usize).saturating_sub(*symbol_bytes)
        {
            *partial = true;
            return;
        }
        *symbol_bytes += bytes;
        symbols.push(SymbolDefinition {
            name: name.clone(),
            qualified_name: name,
            kind: kind.to_string(),
            line: line_number,
            end_line: line_number,
            visibility: if line.trim_start().starts_with("pub") {
                "public"
            } else {
                "private"
            }
            .into(),
            is_test: false,
            source: "range_line_candidate".into(),
        });
    }
}

#[cfg(test)]
#[path = "large_rust_source_tests.rs"]
mod tests;
