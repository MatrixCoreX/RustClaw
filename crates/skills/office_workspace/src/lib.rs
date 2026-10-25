mod docx;
mod docx_structure_edit;
mod docx_write;
mod engine;
mod error;
mod model;
mod mutation;
mod operations;
mod package;
mod package_write;
mod pptx;
mod pptx_edit;
mod pptx_write;
mod range;
mod renderer;
mod xlsx;
mod xlsx_edit;
mod xlsx_write;
mod xml;
mod xml_text;

#[cfg(test)]
mod test_support;

pub use engine::{execute, inspect_for_ingestion};
pub use error::OfficeError;
pub use model::{
    DocumentBlock, OfficeArtifactEnvelope, OfficeFormat, OfficeTable, OfficeWarning,
    PresentationEvidence, SkillRequest, SkillResponse, WorkbookEvidence,
};

use docx::read_docx;
use package::OfficePackage;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct LegacyDocxBlock {
    pub id: String,
    pub text: String,
    pub style: Option<String>,
    pub heading_level: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct LegacyDocxTable {
    pub id: String,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct LegacyDocxEvidence {
    pub blocks: Vec<LegacyDocxBlock>,
    pub tables: Vec<LegacyDocxTable>,
}

pub fn read_docx_for_legacy_parser(path: &Path) -> Result<LegacyDocxEvidence, OfficeError> {
    let package = OfficePackage::open(path, Some(OfficeFormat::Docx))?;
    let evidence = read_docx(&package)?;
    Ok(LegacyDocxEvidence {
        blocks: evidence
            .blocks
            .into_iter()
            .filter(|block| block.source_part == "word/document.xml")
            .map(|block| LegacyDocxBlock {
                id: block.id,
                text: block.text,
                style: block.style,
                heading_level: block.heading_level,
            })
            .collect(),
        tables: evidence
            .tables
            .into_iter()
            .filter(|table| table.source_part == "word/document.xml")
            .map(|table| LegacyDocxTable {
                id: table.id,
                rows: table.rows,
            })
            .collect(),
    })
}
