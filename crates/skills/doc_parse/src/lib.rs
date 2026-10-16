use serde::Serialize;
use std::fmt;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, Debug)]
pub struct PdfExtractionLimits {
    pub max_source_bytes: u64,
    pub max_pages: u32,
    pub max_text_chars: usize,
}

impl Default for PdfExtractionLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 64 * 1024 * 1024,
            max_pages: 2_000,
            max_text_chars: 8_000_000,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PdfTextPage {
    pub page: u32,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PdfTextExtraction {
    pub parser_version: &'static str,
    pub total_pages: u32,
    pub pages: Vec<PdfTextPage>,
    pub text_layer_source: &'static str,
    pub lossy_utf8: bool,
}

#[derive(Debug)]
pub struct DocumentParseError {
    pub code: &'static str,
    pub message: String,
}

impl DocumentParseError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for DocumentParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DocumentParseError {}

pub fn extract_pdf_text(
    path: &Path,
    page_start: Option<u32>,
    page_end: Option<u32>,
    limits: PdfExtractionLimits,
) -> Result<PdfTextExtraction, DocumentParseError> {
    extract_pdf_text_with_commands(path, page_start, page_end, limits, "pdfinfo", "pdftotext")
}

fn extract_pdf_text_with_commands(
    path: &Path,
    page_start: Option<u32>,
    page_end: Option<u32>,
    limits: PdfExtractionLimits,
    pdfinfo_command: &str,
    pdftotext_command: &str,
) -> Result<PdfTextExtraction, DocumentParseError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        DocumentParseError::new(
            "source_unavailable",
            format!("cannot inspect PDF source: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(DocumentParseError::new(
            "invalid_source_kind",
            "PDF source must be a file",
        ));
    }
    if metadata.len() > limits.max_source_bytes {
        return Err(DocumentParseError::new(
            "source_limit_exceeded",
            format!(
                "PDF source exceeds the byte limit: {} > {}",
                metadata.len(),
                limits.max_source_bytes
            ),
        ));
    }
    if pdf_has_encryption_dictionary(path)? {
        return Err(DocumentParseError::new(
            "password_protected_document",
            "encrypted PDF documents require an unlocked source",
        ));
    }

    let total_pages = pdf_page_count(path, pdfinfo_command)?;
    let start = page_start.unwrap_or(1).max(1);
    let end = page_end.unwrap_or(total_pages).min(total_pages);
    if start > end {
        return Err(DocumentParseError::new(
            "invalid_page_range",
            "PDF page range does not select any page",
        ));
    }
    let selected_pages = end.saturating_sub(start).saturating_add(1);
    if selected_pages > limits.max_pages {
        return Err(DocumentParseError::new(
            "page_limit_exceeded",
            format!(
                "PDF page selection exceeds the page limit: {selected_pages} > {}",
                limits.max_pages
            ),
        ));
    }

    let mut command = Command::new(pdftotext_command);
    command
        .env("LC_ALL", "C")
        .arg("-q")
        .arg("-f")
        .arg(start.to_string())
        .arg("-l")
        .arg(end.to_string())
        .arg(path.as_os_str())
        .arg("-");
    let output = command.output().map_err(|error| {
        DocumentParseError::new(
            "parser_dependency_unavailable",
            format!("cannot run pdftotext: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(DocumentParseError::new(
            "malformed_document",
            format!(
                "pdftotext rejected the PDF: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let lossy_utf8 = std::str::from_utf8(&output.stdout).is_err();
    let text = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    if text.chars().count() > limits.max_text_chars {
        return Err(DocumentParseError::new(
            "text_limit_exceeded",
            format!(
                "PDF text exceeds the character limit: > {}",
                limits.max_text_chars
            ),
        ));
    }
    let mut page_texts = text.split('\u{c}').collect::<Vec<_>>();
    while page_texts.last().is_some_and(|page| page.trim().is_empty()) {
        page_texts.pop();
    }
    let pages = page_texts
        .into_iter()
        .enumerate()
        .map(|(index, text)| PdfTextPage {
            page: start.saturating_add(index as u32),
            text: text.trim().to_string(),
        })
        .collect::<Vec<_>>();
    Ok(PdfTextExtraction {
        parser_version: "poppler-text-layer-v1",
        total_pages,
        pages,
        text_layer_source: "embedded_text_layer",
        lossy_utf8,
    })
}

fn pdf_has_encryption_dictionary(path: &Path) -> Result<bool, DocumentParseError> {
    const TOKEN: &[u8] = b"/Encrypt";
    const BUFFER_BYTES: usize = 64 * 1024;
    let file = std::fs::File::open(path).map_err(|error| {
        DocumentParseError::new(
            "source_unavailable",
            format!("cannot open PDF source: {error}"),
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut buffer = vec![0_u8; BUFFER_BYTES];
    let mut carry = Vec::with_capacity(TOKEN.len().saturating_sub(1));
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            DocumentParseError::new(
                "source_unavailable",
                format!("cannot inspect PDF source: {error}"),
            )
        })?;
        if read == 0 {
            return Ok(false);
        }
        carry.extend_from_slice(&buffer[..read]);
        if contains_pdf_encrypt_reference(&carry) {
            return Ok(true);
        }
        let keep = 64.min(carry.len());
        carry.drain(..carry.len().saturating_sub(keep));
    }
}

fn contains_pdf_encrypt_reference(bytes: &[u8]) -> bool {
    const TOKEN: &[u8] = b"/Encrypt";
    bytes
        .windows(TOKEN.len())
        .enumerate()
        .any(|(index, token)| {
            if token != TOKEN {
                return false;
            }
            let mut tail = &bytes[index + TOKEN.len()..];
            consume_pdf_whitespace(&mut tail)
                && consume_ascii_digits(&mut tail)
                && consume_pdf_whitespace(&mut tail)
                && consume_ascii_digits(&mut tail)
                && consume_pdf_whitespace(&mut tail)
                && tail.first() == Some(&b'R')
        })
}

fn consume_pdf_whitespace(bytes: &mut &[u8]) -> bool {
    let before = bytes.len();
    *bytes = bytes.trim_ascii_start();
    bytes.len() < before
}

fn consume_ascii_digits(bytes: &mut &[u8]) -> bool {
    let count = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if count == 0 {
        return false;
    }
    *bytes = &bytes[count..];
    true
}

fn pdf_page_count(path: &Path, command: &str) -> Result<u32, DocumentParseError> {
    let output = Command::new(command)
        .env("LC_ALL", "C")
        .arg(path.as_os_str())
        .output()
        .map_err(|error| {
            DocumentParseError::new(
                "parser_dependency_unavailable",
                format!("cannot run pdfinfo: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(DocumentParseError::new(
            "malformed_document",
            format!(
                "pdfinfo rejected the PDF: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("Pages:"))
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|pages| *pages > 0)
        .ok_or_else(|| {
            DocumentParseError::new(
                "malformed_document",
                "pdfinfo did not report a positive page count",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_pdf_has_a_stable_machine_error() {
        let error = extract_pdf_text(
            Path::new("/definitely/missing/document.pdf"),
            None,
            None,
            PdfExtractionLimits::default(),
        )
        .expect_err("missing source must fail");
        assert_eq!(error.code, "source_unavailable");
    }

    #[test]
    fn encrypted_pdf_marker_has_a_stable_machine_error_without_running_parser() {
        let path = std::env::temp_dir().join(format!(
            "doc-parse-encrypted-{}-{}.pdf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, b"%PDF-1.7\ntrailer << /Encrypt 4 0 R >>\n%%EOF")
            .expect("write encrypted marker fixture");
        let error = extract_pdf_text(&path, None, None, PdfExtractionLimits::default())
            .expect_err("encrypted source must fail before external parser");
        assert_eq!(error.code, "password_protected_document");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn missing_optional_pdf_parser_has_a_stable_machine_error() {
        let path = std::env::temp_dir().join(format!(
            "doc-parse-missing-parser-{}-{}.pdf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, b"%PDF-1.4\n%%EOF").expect("write parser dependency fixture");
        let error = extract_pdf_text_with_commands(
            &path,
            None,
            None,
            PdfExtractionLimits::default(),
            "agent-runtime-definitely-missing-pdfinfo",
            "agent-runtime-definitely-missing-pdftotext",
        )
        .expect_err("missing parser dependency must fail deterministically");
        assert_eq!(error.code, "parser_dependency_unavailable");
        std::fs::remove_file(path).ok();
    }
}
