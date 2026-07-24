use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ENVELOPE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficeFormat {
    Docx,
    Xlsx,
    Pptx,
}

impl OfficeFormat {
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "docx" => Some(Self::Docx),
            "xlsx" => Some(Self::Xlsx),
            "pptx" => Some(Self::Pptx),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceArtifact {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_sha256: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PackageEvidence {
    pub member_count: usize,
    pub total_uncompressed_bytes: u64,
    pub content_types_present: bool,
    pub external_relationships: Vec<RelationshipEvidence>,
    pub macro_members: Vec<String>,
    pub embedded_members: Vec<String>,
    pub artifact_members: Vec<PackageMemberArtifact>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PackageMemberArtifact {
    pub id: String,
    pub package_member: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub storage_kind: String,
    pub content_inline: bool,
    pub untrusted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RelationshipEvidence {
    pub source_part: String,
    pub target: String,
    pub relationship_type: String,
    pub external: bool,
    pub untrusted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OfficeWarning {
    pub code: String,
    pub object_ref: Option<String>,
    pub details: Value,
    pub untrusted: bool,
}

impl OfficeWarning {
    pub fn new(code: impl Into<String>, object_ref: Option<String>, details: Value) -> Self {
        Self {
            code: code.into(),
            object_ref,
            details,
            untrusted: true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TextRun {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hyperlink: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DocumentBlock {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub runs: Vec<TextRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_level: Option<u8>,
    pub source_part: String,
    pub untrusted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OfficeTable {
    pub id: String,
    pub source_part: String,
    pub rows: Vec<Vec<String>>,
    pub untrusted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MediaArtifact {
    pub id: String,
    pub package_member: String,
    pub content_type: Option<String>,
    pub sha256: String,
    pub size_bytes: u64,
    pub storage_kind: String,
    pub content_inline: bool,
    pub untrusted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CellEvidence {
    pub reference: String,
    pub cell_type: String,
    pub value: Option<Value>,
    pub displayed_value: Option<String>,
    pub formula: Option<String>,
    pub style_id: Option<u32>,
    pub hyperlink: Option<String>,
    pub comment: Option<String>,
    pub untrusted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorksheetEvidence {
    pub id: String,
    pub name: String,
    pub state: String,
    pub dimension: Option<String>,
    pub cells: Vec<CellEvidence>,
    pub merged_ranges: Vec<String>,
    pub tables: Vec<String>,
    pub charts: Vec<String>,
    pub images: Vec<String>,
    pub freeze_panes: Vec<String>,
    pub auto_filter: Option<String>,
    pub untrusted: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct WorkbookEvidence {
    pub sheets: Vec<WorksheetEvidence>,
    pub named_ranges: Vec<String>,
    pub date_system: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SlideEvidence {
    pub id: String,
    pub index: usize,
    pub relationship_id: Option<String>,
    pub layout: Option<String>,
    pub hidden: bool,
    pub title: Option<String>,
    pub text: Vec<String>,
    pub notes: Vec<String>,
    pub tables: Vec<OfficeTable>,
    pub charts: Vec<String>,
    pub shapes: Vec<String>,
    pub images: Vec<String>,
    pub untrusted: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PresentationEvidence {
    pub slides: Vec<SlideEvidence>,
    pub masters: Vec<String>,
    pub layouts: Vec<String>,
    pub themes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PageCursor {
    pub offset: usize,
    pub limit: usize,
    pub returned: usize,
    pub total: usize,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OperationRecord {
    pub id: String,
    pub operation: String,
    pub object_refs: Vec<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValidationEvidence {
    pub valid: bool,
    pub checks: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OfficeArtifactEnvelope {
    pub schema_version: u32,
    pub format: OfficeFormat,
    pub source: SourceArtifact,
    pub package: PackageEvidence,
    pub metadata: Value,
    pub document_blocks: Vec<DocumentBlock>,
    pub tables: Vec<OfficeTable>,
    pub workbook: Option<WorkbookEvidence>,
    pub presentation: Option<PresentationEvidence>,
    pub media: Vec<MediaArtifact>,
    pub warnings: Vec<OfficeWarning>,
    pub truncated: bool,
    pub cursor: PageCursor,
    pub operation_log: Vec<OperationRecord>,
    pub validation: ValidationEvidence,
    pub artifacts: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub struct SkillRequest {
    pub request_id: String,
    pub args: Value,
    #[serde(default, rename = "context")]
    pub _context: Option<Value>,
    #[serde(default, rename = "user_id")]
    pub _user_id: Option<i64>,
    #[serde(default, rename = "chat_id")]
    pub _chat_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SkillResponse {
    pub request_id: String,
    pub status: String,
    pub text: String,
    pub error_text: Option<String>,
    pub extra: Value,
}
