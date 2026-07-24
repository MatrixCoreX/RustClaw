use crate::error::{OfficeError, OfficeResult};
use crate::model::{
    MediaArtifact, OfficeFormat, OfficeWarning, PackageEvidence, RelationshipEvidence,
    SourceArtifact,
};
use crate::xml::{attr_value, local_name};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use zip::ZipArchive;

const DEFAULT_MAX_ENTRIES: usize = 10_000;
const DEFAULT_MAX_MEMBER_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_MAX_EXPANSION_RATIO: u64 = 200;

#[derive(Clone, Debug)]
pub struct PackageLimits {
    pub max_entries: usize,
    pub max_member_bytes: u64,
    pub max_total_bytes: u64,
    pub max_expansion_ratio: u64,
}

impl Default for PackageLimits {
    fn default() -> Self {
        Self {
            max_entries: env_usize("OFFICE_MAX_ZIP_ENTRIES", DEFAULT_MAX_ENTRIES),
            max_member_bytes: env_u64("OFFICE_MAX_MEMBER_BYTES", DEFAULT_MAX_MEMBER_BYTES),
            max_total_bytes: env_u64("OFFICE_MAX_TOTAL_BYTES", DEFAULT_MAX_TOTAL_BYTES),
            max_expansion_ratio: env_u64("OFFICE_MAX_EXPANSION_RATIO", DEFAULT_MAX_EXPANSION_RATIO),
        }
    }
}

#[derive(Clone, Debug)]
pub struct OfficePackage {
    pub format: OfficeFormat,
    pub source: SourceArtifact,
    pub members: BTreeMap<String, Vec<u8>>,
    pub evidence: PackageEvidence,
    pub media: Vec<MediaArtifact>,
    pub warnings: Vec<OfficeWarning>,
}

impl OfficePackage {
    pub fn open(path: &Path, expected: Option<OfficeFormat>) -> OfficeResult<Self> {
        Self::open_with_limits(path, expected, &PackageLimits::default())
    }

    pub fn open_with_limits(
        path: &Path,
        expected: Option<OfficeFormat>,
        limits: &PackageLimits,
    ) -> OfficeResult<Self> {
        let metadata = fs::metadata(path).map_err(|error| {
            OfficeError::new(
                "source_unavailable",
                format!("cannot read source package: {error}"),
                json!({"path": path.display().to_string()}),
            )
        })?;
        if !metadata.is_file() {
            return Err(OfficeError::invalid("source path must be a file"));
        }
        let bytes = fs::read(path).map_err(|error| {
            OfficeError::new(
                "source_unavailable",
                format!("cannot read source package: {error}"),
                json!({"path": path.display().to_string()}),
            )
        })?;
        let sha256 = hash_bytes(&bytes);
        let revision = format!("sha256:{sha256}");
        let file = std::io::Cursor::new(bytes);
        let mut zip = ZipArchive::new(file).map_err(|error| {
            OfficeError::new(
                "malformed_package",
                format!("invalid OOXML ZIP package: {error}"),
                json!({"path": path.display().to_string()}),
            )
        })?;
        if zip.len() > limits.max_entries {
            return Err(OfficeError::new(
                "package_limit_exceeded",
                "OOXML package has too many members",
                json!({"member_count": zip.len(), "max_entries": limits.max_entries}),
            ));
        }

        let mut members = BTreeMap::new();
        let mut total_uncompressed = 0u64;
        for index in 0..zip.len() {
            let mut member = zip.by_index(index).map_err(|error| {
                OfficeError::new(
                    "malformed_package",
                    format!("cannot inspect ZIP member: {error}"),
                    json!({"member_index": index}),
                )
            })?;
            if member.encrypted() {
                return Err(OfficeError::new(
                    "encrypted_package",
                    "encrypted Office packages are not supported",
                    json!({"member": member.name()}),
                ));
            }
            let name = canonical_member_name(member.name())?;
            if member.is_dir() {
                continue;
            }
            if member.size() > limits.max_member_bytes {
                return Err(OfficeError::new(
                    "package_limit_exceeded",
                    "OOXML member exceeds the uncompressed size limit",
                    json!({"member": name, "size": member.size(), "limit": limits.max_member_bytes}),
                ));
            }
            total_uncompressed = total_uncompressed.saturating_add(member.size());
            if total_uncompressed > limits.max_total_bytes {
                return Err(OfficeError::new(
                    "package_limit_exceeded",
                    "OOXML package exceeds the total uncompressed size limit",
                    json!({"total": total_uncompressed, "limit": limits.max_total_bytes}),
                ));
            }
            let compressed = member.compressed_size().max(1);
            if member.size() / compressed > limits.max_expansion_ratio {
                return Err(OfficeError::new(
                    "package_expansion_rejected",
                    "OOXML member exceeds the compression expansion limit",
                    json!({
                        "member": name,
                        "compressed_size": member.compressed_size(),
                        "size": member.size(),
                        "max_ratio": limits.max_expansion_ratio
                    }),
                ));
            }
            let mut content = Vec::with_capacity(member.size() as usize);
            member.read_to_end(&mut content).map_err(|error| {
                OfficeError::new(
                    "malformed_package",
                    format!("cannot read ZIP member: {error}"),
                    json!({"member": name}),
                )
            })?;
            members.insert(name, content);
        }

        let format = detect_format(path, &members)?;
        if let Some(expected) = expected {
            if expected != format {
                return Err(OfficeError::new(
                    "format_mismatch",
                    "Office package format does not match the requested capability",
                    json!({"expected": expected.as_str(), "actual": format.as_str()}),
                ));
            }
        }
        validate_required_parts(format, &members)?;

        let macro_members = members
            .keys()
            .filter(|name| {
                let lower = name.to_ascii_lowercase();
                lower.ends_with("vbaproject.bin") || lower.contains("/macros/")
            })
            .cloned()
            .collect::<Vec<_>>();
        if !macro_members.is_empty() {
            return Err(OfficeError::new(
                "macro_enabled_package",
                "macro-enabled Office packages are rejected",
                json!({"members": macro_members}),
            ));
        }

        let embedded_members = members
            .keys()
            .filter(|name| name.to_ascii_lowercase().contains("/embeddings/"))
            .cloned()
            .collect::<Vec<_>>();
        let (external_relationships, mut warnings) = inspect_relationships(&members);
        if !external_relationships.is_empty() {
            warnings.push(OfficeWarning::new(
                "external_relationships_present",
                None,
                json!({"count": external_relationships.len()}),
            ));
        }
        if !embedded_members.is_empty() {
            warnings.push(OfficeWarning::new(
                "embedded_objects_present",
                None,
                json!({"members": embedded_members}),
            ));
        }
        let media = collect_media(&members);
        let evidence = PackageEvidence {
            member_count: members.len(),
            total_uncompressed_bytes: total_uncompressed,
            content_types_present: members.contains_key("[Content_Types].xml"),
            external_relationships,
            macro_members: Vec::new(),
            embedded_members,
        };
        Ok(Self {
            format,
            source: SourceArtifact {
                path: path.display().to_string(),
                sha256,
                size_bytes: metadata.len(),
                revision,
                parent_sha256: None,
            },
            members,
            evidence,
            media,
            warnings,
        })
    }

    pub fn text(&self, name: &str) -> OfficeResult<&str> {
        let bytes = self.members.get(name).ok_or_else(|| {
            OfficeError::new(
                "missing_package_part",
                "required OOXML package part is missing",
                json!({"member": name}),
            )
        })?;
        std::str::from_utf8(bytes).map_err(|error| {
            OfficeError::new(
                "malformed_xml",
                format!("OOXML package part is not UTF-8 XML: {error}"),
                json!({"member": name}),
            )
        })
    }
}

pub fn resolve_input_path(value: &str) -> OfficeResult<PathBuf> {
    let path = Path::new(value);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(OfficeError::new(
            "path_traversal",
            "parent path traversal is not allowed",
            json!({"path": value}),
        ));
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let root = std::env::var_os("WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    Ok(root.join(path))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn canonical_member_name(name: &str) -> OfficeResult<String> {
    if name.is_empty() || name.starts_with('/') || name.starts_with('\\') || name.contains('\\') {
        return Err(OfficeError::new(
            "path_traversal",
            "unsafe OOXML ZIP member path",
            json!({"member": name}),
        ));
    }
    let path = Path::new(name);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(OfficeError::new(
            "path_traversal",
            "unsafe OOXML ZIP member path",
            json!({"member": name}),
        ));
    }
    Ok(path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/"))
}

fn detect_format(path: &Path, members: &BTreeMap<String, Vec<u8>>) -> OfficeResult<OfficeFormat> {
    let by_members = if members.contains_key("word/document.xml") {
        Some(OfficeFormat::Docx)
    } else if members.contains_key("xl/workbook.xml") {
        Some(OfficeFormat::Xlsx)
    } else if members.contains_key("ppt/presentation.xml") {
        Some(OfficeFormat::Pptx)
    } else {
        None
    };
    let by_extension = path
        .extension()
        .and_then(|value| value.to_str())
        .and_then(OfficeFormat::from_extension);
    by_members.or(by_extension).ok_or_else(|| {
        OfficeError::new(
            "unsupported_format",
            "supported Office formats are docx, xlsx, and pptx",
            json!({"path": path.display().to_string()}),
        )
    })
}

fn validate_required_parts(
    format: OfficeFormat,
    members: &BTreeMap<String, Vec<u8>>,
) -> OfficeResult<()> {
    let required = match format {
        OfficeFormat::Docx => "word/document.xml",
        OfficeFormat::Xlsx => "xl/workbook.xml",
        OfficeFormat::Pptx => "ppt/presentation.xml",
    };
    for member in ["[Content_Types].xml", "_rels/.rels", required] {
        if !members.contains_key(member) {
            return Err(OfficeError::new(
                "missing_package_part",
                "required OOXML package part is missing",
                json!({"member": member, "format": format.as_str()}),
            ));
        }
    }
    Ok(())
}

fn inspect_relationships(
    members: &BTreeMap<String, Vec<u8>>,
) -> (Vec<RelationshipEvidence>, Vec<OfficeWarning>) {
    let mut external = Vec::new();
    let mut warnings = Vec::new();
    for (name, bytes) in members.iter().filter(|(name, _)| name.ends_with(".rels")) {
        let Ok(xml) = std::str::from_utf8(bytes) else {
            warnings.push(OfficeWarning::new(
                "malformed_relationship_part",
                Some(name.clone()),
                json!({}),
            ));
            continue;
        };
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        loop {
            match reader.read_event() {
                Ok(Event::Start(element)) | Ok(Event::Empty(element))
                    if local_name(element.name().as_ref()) == b"Relationship" =>
                {
                    let target = attr_value(&element, b"Target").unwrap_or_default();
                    let relationship_type = attr_value(&element, b"Type").unwrap_or_default();
                    let is_external = attr_value(&element, b"TargetMode")
                        .is_some_and(|mode| mode.eq_ignore_ascii_case("external"));
                    if is_external {
                        external.push(RelationshipEvidence {
                            source_part: name.clone(),
                            target,
                            relationship_type,
                            external: true,
                            untrusted: true,
                        });
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => {
                    warnings.push(OfficeWarning::new(
                        "malformed_relationship_part",
                        Some(name.clone()),
                        json!({}),
                    ));
                    break;
                }
                _ => {}
            }
        }
    }
    (external, warnings)
}

fn collect_media(members: &BTreeMap<String, Vec<u8>>) -> Vec<MediaArtifact> {
    members
        .iter()
        .filter(|(name, _)| {
            name.starts_with("word/media/")
                || name.starts_with("xl/media/")
                || name.starts_with("ppt/media/")
        })
        .enumerate()
        .map(|(index, (name, bytes))| MediaArtifact {
            id: format!("media_{}", index + 1),
            package_member: name.clone(),
            content_type: content_type_from_name(name),
            sha256: hash_bytes(bytes),
            size_bytes: bytes.len() as u64,
        })
        .collect()
}

fn content_type_from_name(name: &str) -> Option<String> {
    let extension = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    let content_type = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "emf" => "image/emf",
        "wmf" => "image/wmf",
        _ => return None,
    };
    Some(content_type.to_string())
}

fn env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn env_usize(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

#[cfg(test)]
#[path = "package_tests.rs"]
mod tests;
