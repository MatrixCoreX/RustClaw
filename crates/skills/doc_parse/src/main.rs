use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::ZipArchive;

#[derive(Clone, Copy, Debug)]
enum DocType {
    Md,
    Txt,
    Html,
    Pdf,
    Docx,
    Unknown,
}

impl DocType {
    fn from_path(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "md" | "markdown" => Self::Md,
            "txt" => Self::Txt,
            "html" | "htm" => Self::Html,
            "pdf" => Self::Pdf,
            "docx" => Self::Docx,
            _ => Self::Unknown,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Md => "md",
            Self::Txt => "txt",
            Self::Html => "html",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum TableMode {
    Basic,
    Strict,
}

impl TableMode {
    fn from_value(v: Option<&str>) -> Self {
        match v.unwrap_or("basic").to_ascii_lowercase().as_str() {
            "strict" => Self::Strict,
            _ => Self::Basic,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct PageRange {
    start: Option<u32>,
    end: Option<u32>,
}

#[derive(Clone, Debug)]
struct ParseOptions {
    path: PathBuf,
    mode: String,
    max_chars: usize,
    include_metadata: bool,
    table_mode: TableMode,
    page_range: PageRange,
}

#[derive(Serialize, Clone, Debug)]
struct Section {
    id: String,
    title: String,
    level: u8,
    content: String,
}

#[derive(Serialize, Clone, Debug)]
struct Table {
    id: String,
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

#[derive(Serialize, Clone, Debug)]
struct Metadata {
    title: String,
    pages: u32,
    #[serde(rename = "type")]
    doc_type: String,
    path: String,
    encoding: String,
    truncated: bool,
    truncation_notice: Option<String>,
    page_range_applied: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
struct ParsePayload {
    text: String,
    tables: Vec<Table>,
    sections: Vec<Section>,
    metadata: Option<Metadata>,
    status: String,
    error_code: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct ParseResultInternal {
    text: String,
    sections: Vec<Section>,
    tables: Vec<Table>,
    title: String,
    pages: u32,
    encoding: String,
}

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let req: Value = serde_json::from_str(&line).unwrap_or_else(|_| json!({"request_id":"unknown"}));
        let request_id = req
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let action = req
            .get("args")
            .and_then(|a| a.get("action"))
            .or_else(|| req.get("action"))
            .and_then(Value::as_str)
            .unwrap_or("parse_doc");

        let payload = if action != "parse_doc" {
            ParsePayload {
                text: String::new(),
                tables: vec![],
                sections: vec![],
                metadata: None,
                status: "error".to_string(),
                error_code: Some("INVALID_ACTION".to_string()),
                error: Some(format!("unsupported action: {action}")),
            }
        } else {
            handle_parse_doc(&req)
        };

        let out = json!({
            "request_id": request_id,
            "status": "ok",
            "text": serde_json::to_string(&payload)?,
            "error_text": Value::Null,
            "extra": { "action": "parse_doc" }
        });
        writeln!(stdout, "{}", serde_json::to_string(&out)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn handle_parse_doc(req: &Value) -> ParsePayload {
    match parse_options(req).and_then(parse_document) {
        Ok((mut parsed, opts, doc_type)) => {
            let mut truncated = false;
            let mut notice = None;
            if parsed.text.chars().count() > opts.max_chars {
                parsed.text = parsed.text.chars().take(opts.max_chars).collect::<String>();
                truncated = true;
                notice = Some(format!(
                    "content truncated by max_chars={} (original exceeds limit)",
                    opts.max_chars
                ));
            }
            let metadata = if opts.include_metadata {
                Some(Metadata {
                    title: parsed.title.clone(),
                    pages: parsed.pages,
                    doc_type: doc_type.as_str().to_string(),
                    path: opts.path.display().to_string(),
                    encoding: parsed.encoding,
                    truncated,
                    truncation_notice: notice,
                    page_range_applied: page_range_str(&opts.page_range),
                })
            } else {
                None
            };

            ParsePayload {
                text: parsed.text,
                tables: parsed.tables,
                sections: parsed.sections,
                metadata,
                status: "ok".to_string(),
                error_code: None,
                error: None,
            }
        }
        Err(err) => ParsePayload {
            text: String::new(),
            tables: vec![],
            sections: vec![],
            metadata: None,
            status: "error".to_string(),
            error_code: Some(classify_error(&err)),
            error: Some(err.to_string()),
        },
    }
}

fn parse_options(req: &Value) -> Result<ParseOptions> {
    let args = req.get("args").unwrap_or(req);
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing required args.path"))?;
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .to_string();
    let max_chars = args
        .get("max_chars")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(12000)
        .clamp(100, 2_000_000);
    let include_metadata = args
        .get("include_metadata")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let table_mode = TableMode::from_value(args.get("table_mode").and_then(Value::as_str));
    let page_range = parse_page_range(args.get("page_range"));
    Ok(ParseOptions {
        path,
        mode,
        max_chars,
        include_metadata,
        table_mode,
        page_range,
    })
}

fn parse_page_range(v: Option<&Value>) -> PageRange {
    match v {
        Some(Value::String(s)) => {
            if let Some((a, b)) = s.split_once('-') {
                PageRange {
                    start: a.trim().parse::<u32>().ok(),
                    end: b.trim().parse::<u32>().ok(),
                }
            } else {
                let n = s.trim().parse::<u32>().ok();
                PageRange { start: n, end: n }
            }
        }
        Some(Value::Object(map)) => PageRange {
            start: map.get("start").and_then(Value::as_u64).map(|n| n as u32),
            end: map.get("end").and_then(Value::as_u64).map(|n| n as u32),
        },
        _ => PageRange::default(),
    }
}

fn page_range_str(pr: &PageRange) -> Option<String> {
    match (pr.start, pr.end) {
        (Some(s), Some(e)) => Some(format!("{s}-{e}")),
        (Some(s), None) => Some(format!("{s}-")),
        (None, Some(e)) => Some(format!("-{e}")),
        _ => None,
    }
}

fn parse_document(opts: ParseOptions) -> Result<(ParseResultInternal, ParseOptions, DocType)> {
    if !opts.path.exists() {
        return Err(anyhow!("file not found: {}", opts.path.display()));
    }
    let doc_type = DocType::from_path(&opts.path);
    let result = match doc_type {
        DocType::Md => parse_markdown(&opts.path, opts.table_mode)?,
        DocType::Txt => parse_text(&opts.path)?,
        DocType::Html => parse_html(&opts.path, opts.table_mode)?,
        DocType::Pdf => parse_pdf(&opts.path, &opts.page_range)?,
        DocType::Docx => parse_docx(&opts.path, opts.table_mode)?,
        DocType::Unknown => parse_text(&opts.path)?,
    };

    let final_result = if opts.mode.eq_ignore_ascii_case("text_only") {
        ParseResultInternal {
            sections: vec![],
            tables: vec![],
            ..result
        }
    } else {
        result
    };

    Ok((final_result, opts, doc_type))
}

fn parse_text(path: &Path) -> Result<ParseResultInternal> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| String::from_utf8_lossy(&bytes).to_string());
    let sections = split_plain_sections(&text);
    Ok(ParseResultInternal {
        title: first_non_empty_line(&text).unwrap_or_default(),
        pages: 1,
        encoding: "utf-8-or-lossy".to_string(),
        tables: vec![],
        sections,
        text,
    })
}

fn parse_markdown(path: &Path, table_mode: TableMode) -> Result<ParseResultInternal> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| String::from_utf8_lossy(&bytes).to_string());
    let sections = parse_md_sections(&text);
    let tables = parse_md_tables(&text, table_mode);
    Ok(ParseResultInternal {
        title: sections
            .first()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| first_non_empty_line(&text).unwrap_or_default()),
        pages: 1,
        encoding: "utf-8-or-lossy".to_string(),
        tables,
        sections,
        text,
    })
}

fn parse_html(path: &Path, table_mode: TableMode) -> Result<ParseResultInternal> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let html = String::from_utf8(bytes.clone()).unwrap_or_else(|_| String::from_utf8_lossy(&bytes).to_string());
    let text = strip_html_tags(&html);
    let sections = parse_html_sections(&html);
    let tables = parse_html_tables(&html, table_mode);
    let title = extract_between(&html, "<title", "</title>")
        .map(|s| strip_html_tags(&s))
        .unwrap_or_else(|| first_non_empty_line(&text).unwrap_or_default());
    Ok(ParseResultInternal {
        title,
        pages: 1,
        encoding: "utf-8-or-lossy".to_string(),
        tables,
        sections,
        text,
    })
}

fn parse_pdf(path: &Path, page_range: &PageRange) -> Result<ParseResultInternal> {
    let mut cmd = Command::new("pdftotext");
    cmd.arg("-q");
    if let Some(s) = page_range.start {
        cmd.arg("-f").arg(s.to_string());
    }
    if let Some(e) = page_range.end {
        cmd.arg("-l").arg(e.to_string());
    }
    cmd.arg(path.as_os_str()).arg("-");
    let output = cmd.output().with_context(|| "failed to run pdftotext; install poppler-utils")?;
    if !output.status.success() {
        return Err(anyhow!(
            "pdftotext failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let pages = pdf_page_count(path).unwrap_or(1);
    Ok(ParseResultInternal {
        title: first_non_empty_line(&text).unwrap_or_else(|| file_stem(path)),
        pages,
        encoding: "utf-8".to_string(),
        tables: vec![],
        sections: split_plain_sections(&text),
        text,
    })
}

fn parse_docx(path: &Path, table_mode: TableMode) -> Result<ParseResultInternal> {
    let file = fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut zip = ZipArchive::new(file).with_context(|| "invalid docx zip archive")?;

    let mut document_xml = String::new();
    {
        let mut f = zip
            .by_name("word/document.xml")
            .with_context(|| "missing word/document.xml in docx")?;
        f.read_to_string(&mut document_xml)
            .with_context(|| "failed to read word/document.xml")?;
    }

    let paragraphs = parse_docx_paragraphs(&document_xml);
    let mut sections = vec![];
    let mut current_title = "Document".to_string();
    let mut current_level = 1u8;
    let mut buf = String::new();
    let mut idx = 1usize;
    for (text, level_opt) in paragraphs {
        if text.trim().is_empty() {
            continue;
        }
        if let Some(level) = level_opt {
            if !buf.trim().is_empty() {
                sections.push(Section {
                    id: format!("sec_{idx}"),
                    title: current_title.clone(),
                    level: current_level,
                    content: buf.trim().to_string(),
                });
                idx += 1;
                buf.clear();
            }
            current_title = text;
            current_level = level;
        } else {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(&text);
        }
    }
    if !buf.trim().is_empty() {
        sections.push(Section {
            id: format!("sec_{idx}"),
            title: current_title.clone(),
            level: current_level,
            content: buf.trim().to_string(),
        });
    }

    let tables = parse_docx_tables(&document_xml, table_mode);
    let text = sections
        .iter()
        .map(|s| {
            if s.content.is_empty() {
                s.title.clone()
            } else {
                format!("{}\n{}", s.title, s.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(ParseResultInternal {
        title: sections
            .first()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| file_stem(path)),
        pages: 1,
        encoding: "utf-8".to_string(),
        tables,
        sections,
        text,
    })
}

fn parse_md_sections(text: &str) -> Vec<Section> {
    let heading_re = Regex::new(r"(?m)^(#{1,6})\s+(.+)$").expect("valid regex");
    let mut heads: Vec<(usize, usize, String)> = vec![];
    for cap in heading_re.captures_iter(text) {
        let m0 = cap.get(0).expect("m0");
        let level = cap.get(1).expect("m1").as_str().len() as u8;
        let title = cap.get(2).expect("m2").as_str().trim().to_string();
        heads.push((m0.start(), level as usize, title));
    }
    if heads.is_empty() {
        return split_plain_sections(text);
    }
    let mut out = vec![];
    for (i, (start, level, title)) in heads.iter().enumerate() {
        let end = heads.get(i + 1).map(|h| h.0).unwrap_or(text.len());
        let block = text[*start..end].to_string();
        let content = block
            .lines()
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        out.push(Section {
            id: format!("sec_{}", i + 1),
            title: title.clone(),
            level: *level as u8,
            content,
        });
    }
    out
}

fn split_plain_sections(text: &str) -> Vec<Section> {
    let parts = text
        .split("\n\n")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| Section {
            id: format!("sec_{}", i + 1),
            title: format!("Section {}", i + 1),
            level: 1,
            content: (*p).to_string(),
        })
        .collect()
}

fn parse_md_tables(text: &str, table_mode: TableMode) -> Vec<Table> {
    let mut tables = vec![];
    let mut lines = text.lines().peekable();
    let mut tid = 1usize;
    while let Some(line) = lines.next() {
        if !line.contains('|') {
            continue;
        }
        let next = lines.peek().copied().unwrap_or("");
        if !is_md_separator(next) {
            continue;
        }
        let header = split_md_row(line);
        lines.next();
        let mut rows = vec![];
        while let Some(peek) = lines.peek().copied() {
            if !peek.contains('|') || peek.trim().is_empty() {
                break;
            }
            let row = split_md_row(peek);
            lines.next();
            rows.push(row);
        }
        let table = normalize_table(format!("tbl_{tid}"), header, rows, table_mode);
        if let Some(t) = table {
            tables.push(t);
            tid += 1;
        }
    }
    tables
}

fn is_md_separator(line: &str) -> bool {
    let t = line.trim();
    t.contains('|') && t.chars().all(|c| c == '|' || c == '-' || c == ':' || c.is_whitespace())
}

fn split_md_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|s| s.trim().to_string())
        .collect()
}

fn parse_html_sections(html: &str) -> Vec<Section> {
    let heading_re = Regex::new(r"(?is)<h([1-6])[^>]*>(.*?)</h[1-6]>").expect("valid regex");
    let mut matches = vec![];
    for cap in heading_re.captures_iter(html) {
        let m = cap.get(0).expect("full");
        let level = cap
            .get(1)
            .and_then(|m1| m1.as_str().parse::<u8>().ok())
            .unwrap_or(1);
        let title = strip_html_tags(cap.get(2).map(|x| x.as_str()).unwrap_or(""));
        matches.push((m.start(), m.end(), level, title));
    }
    if matches.is_empty() {
        return split_plain_sections(&strip_html_tags(html));
    }
    let mut out = vec![];
    for (i, (_, end, level, title)) in matches.iter().enumerate() {
        let next_start = matches.get(i + 1).map(|x| x.0).unwrap_or(html.len());
        let content = strip_html_tags(&html[*end..next_start]);
        out.push(Section {
            id: format!("sec_{}", i + 1),
            title: title.clone(),
            level: *level,
            content: content.trim().to_string(),
        });
    }
    out
}

fn parse_html_tables(html: &str, table_mode: TableMode) -> Vec<Table> {
    let table_re = Regex::new(r"(?is)<table[^>]*>(.*?)</table>").expect("valid regex");
    let row_re = Regex::new(r"(?is)<tr[^>]*>(.*?)</tr>").expect("valid regex");
    let cell_re = Regex::new(r"(?is)<t[hd][^>]*>(.*?)</t[hd]>").expect("valid regex");
    let mut out = vec![];
    for (idx, tcap) in table_re.captures_iter(html).enumerate() {
        let inner = tcap.get(1).map(|m| m.as_str()).unwrap_or("");
        let mut rows = vec![];
        for rcap in row_re.captures_iter(inner) {
            let r = rcap.get(1).map(|m| m.as_str()).unwrap_or("");
            let mut cells = vec![];
            for ccap in cell_re.captures_iter(r) {
                let c = ccap.get(1).map(|m| m.as_str()).unwrap_or("");
                cells.push(strip_html_tags(c).trim().to_string());
            }
            if !cells.is_empty() {
                rows.push(cells);
            }
        }
        if rows.is_empty() {
            continue;
        }
        let header = rows.first().cloned().unwrap_or_default();
        let body = rows.into_iter().skip(1).collect::<Vec<_>>();
        if let Some(t) = normalize_table(format!("tbl_{}", idx + 1), header, body, table_mode) {
            out.push(t);
        }
    }
    out
}

fn parse_docx_paragraphs(xml: &str) -> Vec<(String, Option<u8>)> {
    let p_re = Regex::new(r"(?is)<w:p\b[^>]*>(.*?)</w:p>").expect("valid regex");
    let t_re = Regex::new(r"(?is)<w:t[^>]*>(.*?)</w:t>").expect("valid regex");
    let style_re = Regex::new(r#"(?is)<w:pStyle[^>]*w:val="Heading([1-6])""#).expect("valid regex");
    let mut out = vec![];
    for cap in p_re.captures_iter(xml) {
        let p = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let mut text = String::new();
        for tcap in t_re.captures_iter(p) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&xml_unescape(tcap.get(1).map(|m| m.as_str()).unwrap_or("")));
        }
        let level = style_re
            .captures(p)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u8>().ok());
        if !text.trim().is_empty() {
            out.push((text.trim().to_string(), level));
        }
    }
    out
}

fn parse_docx_tables(xml: &str, table_mode: TableMode) -> Vec<Table> {
    let tbl_re = Regex::new(r"(?is)<w:tbl\b[^>]*>(.*?)</w:tbl>").expect("valid regex");
    let tr_re = Regex::new(r"(?is)<w:tr\b[^>]*>(.*?)</w:tr>").expect("valid regex");
    let tc_re = Regex::new(r"(?is)<w:tc\b[^>]*>(.*?)</w:tc>").expect("valid regex");
    let t_re = Regex::new(r"(?is)<w:t[^>]*>(.*?)</w:t>").expect("valid regex");
    let mut out = vec![];
    for (tid, tbl_cap) in tbl_re.captures_iter(xml).enumerate() {
        let tbl = tbl_cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let mut rows = vec![];
        for tr_cap in tr_re.captures_iter(tbl) {
            let tr = tr_cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let mut row = vec![];
            for tc_cap in tc_re.captures_iter(tr) {
                let tc = tc_cap.get(1).map(|m| m.as_str()).unwrap_or("");
                let mut cell = String::new();
                for text_cap in t_re.captures_iter(tc) {
                    if !cell.is_empty() {
                        cell.push(' ');
                    }
                    cell.push_str(&xml_unescape(text_cap.get(1).map(|m| m.as_str()).unwrap_or("")));
                }
                row.push(cell.trim().to_string());
            }
            if !row.is_empty() {
                rows.push(row);
            }
        }
        if rows.is_empty() {
            continue;
        }
        let header = rows.first().cloned().unwrap_or_default();
        let body = rows.into_iter().skip(1).collect::<Vec<_>>();
        if let Some(t) = normalize_table(format!("tbl_{}", tid + 1), header, body, table_mode) {
            out.push(t);
        }
    }
    out
}

fn normalize_table(id: String, header: Vec<String>, rows: Vec<Vec<String>>, mode: TableMode) -> Option<Table> {
    if header.is_empty() {
        return None;
    }
    let hlen = header.len();
    let mut clean_rows = vec![];
    for r in rows {
        if r.is_empty() {
            continue;
        }
        if matches!(mode, TableMode::Strict) && r.len() != hlen {
            continue;
        }
        let mut row = r;
        if row.len() < hlen {
            row.resize(hlen, String::new());
        }
        if row.len() > hlen {
            row.truncate(hlen);
        }
        clean_rows.push(row);
    }
    Some(Table {
        id,
        header,
        rows: clean_rows,
    })
}

fn strip_html_tags(html: &str) -> String {
    let script_re = Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("valid regex");
    let style_re = Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("valid regex");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid regex");
    let ws_re = Regex::new(r"[ \t]+").expect("valid regex");
    let mut s = script_re.replace_all(html, " ").to_string();
    s = style_re.replace_all(&s, " ").to_string();
    s = tag_re.replace_all(&s, " ").to_string();
    s = html_unescape(&s);
    s = ws_re.replace_all(&s, " ").to_string();
    s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn extract_between(text: &str, start_tag: &str, end_tag: &str) -> Option<String> {
    let start_idx = text.to_lowercase().find(&start_tag.to_lowercase())?;
    let after = &text[start_idx..];
    let gt = after.find('>')?;
    let after_gt = &after[gt + 1..];
    let end_idx = after_gt.to_lowercase().find(&end_tag.to_lowercase())?;
    Some(after_gt[..end_idx].to_string())
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|s| s.to_string())
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document")
        .to_string()
}

fn pdf_page_count(path: &Path) -> Option<u32> {
    let out = Command::new("pdfinfo").arg(path.as_os_str()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

fn classify_error(err: &anyhow::Error) -> String {
    let m = err.to_string().to_lowercase();
    if m.contains("file not found") {
        "NOT_FOUND".to_string()
    } else if m.contains("pdftotext") || m.contains("pdfinfo") {
        "DEPENDENCY_MISSING".to_string()
    } else if m.contains("invalid docx") || m.contains("missing word/document.xml") {
        "UNSUPPORTED_FORMAT".to_string()
    } else {
        "PARSE_FAILED".to_string()
    }
}

fn html_unescape(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn xml_unescape(s: &str) -> String {
    html_unescape(s)
}
