use crate::error::{OfficeError, OfficeResult};
use crate::operations::NormalizedOperation;
use crate::package::{resolve_input_path, OfficePackage};
use crate::pptx_write::PptxWriteResult;
use crate::xml::{attr_value_qualified, local_name, relationship_map};
use quick_xml::escape::escape;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Debug)]
struct SlideEntry {
    id: String,
    relationship_id: String,
    path: String,
}

pub fn edit_pptx(
    package: &OfficePackage,
    operations: &[NormalizedOperation],
) -> OfficeResult<PptxWriteResult> {
    let mut members = package.members.clone();
    let mut changed_refs = Vec::new();
    for operation in operations {
        match operation.kind.as_str() {
            "set_properties" => set_properties(&mut members, operation)?,
            "add_slide" => {
                let id = add_slide(&mut members, operation, None)?;
                changed_refs.push(id);
            }
            "duplicate_slide" => {
                let id = duplicate_slide(&mut members, operation)?;
                changed_refs.push(id);
            }
            "move_slide" => move_slide(&mut members, operation)?,
            "hide_slide" => hide_slide(&mut members, operation)?,
            "delete_slide" => delete_slide(&mut members, operation)?,
            "replace_slide_text" => replace_slide_text(&mut members, operation)?,
            "set_slide_layout" => set_slide_layout(&mut members, operation)?,
            "add_text" => add_text(&mut members, operation)?,
            "add_notes" => add_notes(&mut members, operation)?,
            "replace_image" => replace_image(&mut members, operation)?,
            "add_image" => add_image(&mut members, operation)?,
            "add_table" => add_table(&mut members, operation)?,
            "add_chart" => add_chart(&mut members, operation)?,
            "add_shape" => add_shape(&mut members, operation)?,
            "add_link" => add_link(&mut members, operation)?,
            "set_transition" => set_transition(&mut members, operation)?,
            _ => {
                return Err(OfficeError::unsupported(
                    "PPTX edit operation is not implemented without potential layout loss",
                    json!({"operation_id": operation.id, "op": operation.kind}),
                ))
            }
        }
        changed_refs.extend(operation.object_refs());
    }
    changed_refs.sort();
    changed_refs.dedup();
    Ok(PptxWriteResult {
        members,
        changed_refs,
        preservation: vec![
            "themes_masters_layouts_preserved".to_string(),
            "unknown_package_parts_preserved".to_string(),
            "untouched_slides_preserved".to_string(),
        ],
    })
}

fn add_slide(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    source: Option<(Vec<u8>, Option<Vec<u8>>)>,
) -> OfficeResult<String> {
    let number = next_part_number(members, "ppt/slides/slide", ".xml");
    let path = format!("ppt/slides/slide{number}.xml");
    let relationship_id = format!("rIdRustClawSlide{}", Uuid::new_v4().simple());
    let entries = slide_entries(members)?;
    let presentation = member_text(members, "ppt/presentation.xml")?.to_string();
    let slide_numeric_id = max_slide_numeric_id(&presentation).max(255) + 1;
    let node = format!(
        "<p:sldId id=\"{slide_numeric_id}\" r:id=\"{}\"/>",
        xml(&relationship_id)
    );
    members.insert(
        "ppt/presentation.xml".into(),
        insert_before(&presentation, "</p:sldIdLst>", &node)?.into_bytes(),
    );
    add_relationship(
        members,
        "ppt/_rels/presentation.xml.rels",
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
        &format!("slides/slide{number}.xml"),
        false,
    )?;
    ensure_content_override(
        members,
        &format!("/{path}"),
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
    )?;
    match source {
        Some((slide, relationships)) => {
            members.insert(path.clone(), slide);
            if let Some(relationships) = relationships {
                members.insert(slide_relationships_path(&path), relationships);
            }
        }
        None => {
            let title = operation.optional_string("title").unwrap_or("");
            let body = operation
                .value("body")
                .map(string_list)
                .transpose()?
                .unwrap_or_default();
            members.insert(path.clone(), slide_xml(title, &body).into_bytes());
            let layout = resolve_layout_path(members, operation)?;
            add_relationship(
                members,
                &slide_relationships_path(&path),
                &format!("rIdRustClawLayout{}", Uuid::new_v4().simple()),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout",
                &relative_slide_target(&layout),
                false,
            )?;
        }
    }
    let id = stable_slide_id(&path);
    if operation.value("notes").is_some() {
        add_notes_to_slide(members, &id, operation)?;
    }
    if operation.bool("hidden").unwrap_or(false) {
        let mut hide = operation.clone();
        hide.fields
            .insert("slide_id".into(), Value::String(id.clone()));
        hide.fields.insert("hidden".into(), Value::Bool(true));
        hide_slide(members, &hide)?;
    }
    if let Some(position) = operation.optional_usize("position") {
        let mut move_operation = operation.clone();
        move_operation
            .fields
            .insert("slide_id".into(), Value::String(id.clone()));
        move_operation
            .fields
            .insert("position".into(), json!(position));
        move_slide(members, &move_operation)?;
    } else if entries.is_empty() {
        return Ok(id);
    }
    Ok(id)
}

fn duplicate_slide(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<String> {
    let source = require_slide(members, operation.string("slide_id")?)?;
    let relationships_path = slide_relationships_path(&source.path);
    if members
        .get(&relationships_path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(relationship_map)
        .is_some_and(|relationships| {
            relationships
                .values()
                .any(|(_, kind, _)| kind.ends_with("/notesSlide"))
        })
    {
        return Err(OfficeError::unsupported(
            "duplicate_slide requires a source slide without speaker notes",
            json!({"slide_id": source.id}),
        ));
    }
    let slide = members
        .get(&source.path)
        .cloned()
        .ok_or_else(|| missing_part(&source.path))?;
    let relationships = members.get(&relationships_path).cloned();
    add_slide(members, operation, Some((slide, relationships)))
}

fn move_slide(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, operation.string("slide_id")?)?;
    let position = operation.usize("position")?;
    let presentation = member_text(members, "ppt/presentation.xml")?.to_string();
    let ranges = slide_id_ranges(&presentation)?;
    if position == 0 || position > ranges.len() {
        return Err(OfficeError::new(
            "invalid_selector",
            "slide position is outside the presentation",
            json!({"position": position, "slide_count": ranges.len()}),
        ));
    }
    let mut nodes = ranges
        .iter()
        .map(|range| presentation[range.0..range.1].to_string())
        .collect::<Vec<_>>();
    let current = nodes
        .iter()
        .position(|node| attribute_value(node, "r:id").as_deref() == Some(&entry.relationship_id))
        .ok_or_else(|| object_not_found(&entry.id))?;
    let node = nodes.remove(current);
    nodes.insert(position - 1, node);
    let start = ranges.first().map(|range| range.0).unwrap_or(0);
    let end = ranges.last().map(|range| range.1).unwrap_or(start);
    members.insert(
        "ppt/presentation.xml".into(),
        format!(
            "{}{}{}",
            &presentation[..start],
            nodes.join(""),
            &presentation[end..]
        )
        .into_bytes(),
    );
    Ok(())
}

fn hide_slide(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, operation.string("slide_id")?)?;
    let source = member_text(members, &entry.path)?.to_string();
    let hidden = operation.bool("hidden").unwrap_or(true);
    members.insert(
        entry.path,
        set_root_attribute(&source, "p:sld", "show", hidden.then_some("0"))?.into_bytes(),
    );
    Ok(())
}

fn delete_slide(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entries = slide_entries(members)?;
    if entries.len() <= 1 {
        return Err(OfficeError::new(
            "last_slide",
            "a presentation must retain at least one slide",
            json!({}),
        ));
    }
    let slide_id = operation.string("slide_id")?;
    let entry = entries
        .iter()
        .find(|entry| entry.id == slide_id)
        .cloned()
        .ok_or_else(|| object_not_found(slide_id))?;
    let presentation = member_text(members, "ppt/presentation.xml")?.to_string();
    members.insert(
        "ppt/presentation.xml".into(),
        remove_element_by_attribute(&presentation, "p:sldId", "r:id", &entry.relationship_id)?
            .into_bytes(),
    );
    let relationships = member_text(members, "ppt/_rels/presentation.xml.rels")?.to_string();
    members.insert(
        "ppt/_rels/presentation.xml.rels".into(),
        remove_element_by_attribute(&relationships, "Relationship", "Id", &entry.relationship_id)?
            .into_bytes(),
    );
    remove_content_override(members, &format!("/{}", entry.path))?;
    members.remove(&entry.path);
    members.remove(&slide_relationships_path(&entry.path));
    Ok(())
}

fn replace_slide_text(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, operation.string("slide_id")?)?;
    let source = member_text(members, &entry.path)?.to_string();
    let expected = operation.string("match")?;
    let escaped = xml(expected);
    if !source.contains(&escaped) {
        return Err(OfficeError::new(
            "source_conflict",
            "expected slide text is absent from the selected revision",
            json!({"expected_text": expected, "slide_id": entry.id}),
        ));
    }
    members.insert(
        entry.path,
        source
            .replacen(&escaped, &xml(operation.string("text")?), 1)
            .into_bytes(),
    );
    Ok(())
}

fn set_slide_layout(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, operation.string("slide_id")?)?;
    let layout = resolve_layout_path(members, operation)?;
    let relationships_path = slide_relationships_path(&entry.path);
    let relationships = member_text(members, &relationships_path)?.to_string();
    let updated = replace_relationship_target(
        &relationships,
        "slideLayout",
        &relative_slide_target(&layout),
    )?;
    members.insert(relationships_path, updated.into_bytes());
    Ok(())
}

fn add_text(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    append_shape(members, operation.string("slide_id")?, |id| {
        text_shape(id, operation.string("text").unwrap_or_default())
    })
}

fn add_shape(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    append_shape(members, operation.string("slide_id")?, |id| {
        shape_xml(
            id,
            operation.optional_string("shape").unwrap_or("rect"),
            operation.optional_string("text").unwrap_or(""),
        )
    })
}

fn add_table(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let rows = operation
        .value("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_field(operation, "rows"))?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| invalid_field(operation, "rows"))
                .map(|cells| cells.iter().map(scalar_text).collect::<Vec<_>>())
        })
        .collect::<OfficeResult<Vec<_>>>()?;
    append_shape(members, operation.string("slide_id")?, |id| {
        table_shape(id, &rows)
    })
}

fn add_link(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, operation.string("slide_id")?)?;
    let relationship_id = format!("rIdRustClawLink{}", Uuid::new_v4().simple());
    add_relationship(
        members,
        &slide_relationships_path(&entry.path),
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
        operation.string("url")?,
        true,
    )?;
    append_shape_to_entry(members, &entry, |id| {
        link_shape(
            id,
            &relationship_id,
            operation.string("text").unwrap_or_default(),
        )
    })
}

fn add_image(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, operation.string("slide_id")?)?;
    let source = resolve_input_path(operation.string("path")?)?;
    let extension = image_extension(&source)?;
    let bytes = fs::read(&source).map_err(|error| {
        OfficeError::new(
            "source_unavailable",
            format!("cannot read presentation image: {error}"),
            json!({"path": source.display().to_string()}),
        )
    })?;
    let number = next_part_number(members, "ppt/media/image", "");
    let member = format!("ppt/media/image{number}.{extension}");
    let relationship_id = format!("rIdRustClawImage{}", Uuid::new_v4().simple());
    add_relationship(
        members,
        &slide_relationships_path(&entry.path),
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
        &format!("../media/image{number}.{extension}"),
        false,
    )?;
    members.insert(member, bytes);
    ensure_content_default(members, extension, image_content_type(extension))?;
    append_shape_to_entry(members, &entry, |id| {
        image_shape(
            id,
            &relationship_id,
            operation.optional_string("alt").unwrap_or("image"),
        )
    })
}

fn replace_image(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let media_id = operation.string("media_id")?;
    let index = media_id
        .strip_prefix("media_")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| OfficeError::invalid("media_id must use media_<index> format"))?;
    let member = members
        .keys()
        .filter(|name| name.starts_with("ppt/media/"))
        .nth(index - 1)
        .cloned()
        .ok_or_else(|| object_not_found(media_id))?;
    let source = resolve_input_path(operation.string("path")?)?;
    let replacement_extension = image_extension(&source)?;
    let current_extension = Path::new(&member)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if current_extension != replacement_extension {
        return Err(OfficeError::unsupported(
            "image replacement must preserve the package media type",
            json!({"existing_extension": current_extension, "source_extension": replacement_extension}),
        ));
    }
    members.insert(
        member,
        fs::read(&source).map_err(|error| {
            OfficeError::new(
                "source_unavailable",
                format!("cannot read replacement image: {error}"),
                json!({"path": source.display().to_string()}),
            )
        })?,
    );
    Ok(())
}

fn add_chart(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, operation.string("slide_id")?)?;
    let categories = operation
        .value("categories")
        .map(string_list)
        .transpose()?
        .ok_or_else(|| invalid_field(operation, "categories"))?;
    let values = operation
        .value("values")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_field(operation, "values"))?
        .iter()
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| invalid_field(operation, "values"))
        })
        .collect::<OfficeResult<Vec<_>>>()?;
    if categories.is_empty() || categories.len() != values.len() {
        return Err(OfficeError::new(
            "chart_shape_mismatch",
            "chart categories and values must have the same non-zero length",
            json!({"operation_id": operation.id}),
        ));
    }
    let number = next_part_number(members, "ppt/charts/chart", ".xml");
    let chart_path = format!("ppt/charts/chart{number}.xml");
    let relationship_id = format!("rIdRustClawChart{}", Uuid::new_v4().simple());
    add_relationship(
        members,
        &slide_relationships_path(&entry.path),
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart",
        &format!("../charts/chart{number}.xml"),
        false,
    )?;
    members.insert(
        chart_path.clone(),
        chart_xml(
            operation.optional_string("title").unwrap_or("Chart"),
            operation.optional_string("chart_type").unwrap_or("column"),
            &categories,
            &values,
        )
        .into_bytes(),
    );
    ensure_content_override(
        members,
        &format!("/{chart_path}"),
        "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
    )?;
    append_shape_to_entry(members, &entry, |id| chart_shape(id, &relationship_id))
}

fn add_notes(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    add_notes_to_slide(members, operation.string("slide_id")?, operation)
}

fn add_notes_to_slide(
    members: &mut BTreeMap<String, Vec<u8>>,
    slide_id: &str,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, slide_id)?;
    let notes = operation
        .value("notes")
        .map(string_list)
        .transpose()?
        .unwrap_or_else(|| {
            operation
                .optional_string("text")
                .map(|value| vec![value.to_string()])
                .unwrap_or_default()
        });
    if notes.is_empty() {
        return Err(invalid_field(operation, "notes|text"));
    }
    let number = next_part_number(members, "ppt/notesSlides/notesSlide", ".xml");
    let path = format!("ppt/notesSlides/notesSlide{number}.xml");
    let relationship_id = format!("rIdRustClawNotes{}", Uuid::new_v4().simple());
    add_relationship(
        members,
        &slide_relationships_path(&entry.path),
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide",
        &format!("../notesSlides/notesSlide{number}.xml"),
        false,
    )?;
    members.insert(path.clone(), notes_xml(number, &notes).into_bytes());
    ensure_content_override(
        members,
        &format!("/{path}"),
        "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml",
    )
}

fn set_transition(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let entry = require_slide(members, operation.string("slide_id")?)?;
    let source = member_text(members, &entry.path)?.to_string();
    let value = transition_xml(operation.string("transition")?);
    let updated = if let Some(start) = source.find("<p:transition") {
        let opening_end = source[start..]
            .find('>')
            .map(|value| start + value + 1)
            .ok_or_else(|| malformed("p:transition"))?;
        let end = if source[start..opening_end].trim_end().ends_with("/>") {
            opening_end
        } else {
            source[opening_end..]
                .find("</p:transition>")
                .map(|value| opening_end + value + "</p:transition>".len())
                .ok_or_else(|| malformed("p:transition"))?
        };
        format!("{}{}{}", &source[..start], value, &source[end..])
    } else {
        insert_before(&source, "</p:sld>", &value)?
    };
    members.insert(entry.path, updated.into_bytes());
    Ok(())
}

fn set_properties(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let mut source = members
        .get("docProps/core.xml")
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or(
            "<?xml version=\"1.0\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"></cp:coreProperties>",
        )
        .to_string();
    for (field, tag) in [
        ("title", "dc:title"),
        ("subject", "dc:subject"),
        ("creator", "dc:creator"),
    ] {
        if let Some(value) = operation.optional_string(field) {
            source = set_or_insert_element(&source, tag, value, "</cp:coreProperties>")?;
        }
    }
    members.insert("docProps/core.xml".into(), source.into_bytes());
    Ok(())
}

fn append_shape(
    members: &mut BTreeMap<String, Vec<u8>>,
    slide_id: &str,
    build: impl FnOnce(usize) -> String,
) -> OfficeResult<()> {
    let entry = require_slide(members, slide_id)?;
    append_shape_to_entry(members, &entry, build)
}

fn append_shape_to_entry(
    members: &mut BTreeMap<String, Vec<u8>>,
    entry: &SlideEntry,
    build: impl FnOnce(usize) -> String,
) -> OfficeResult<()> {
    let source = member_text(members, &entry.path)?.to_string();
    let id = max_non_visual_id(&source) + 1;
    members.insert(
        entry.path.clone(),
        insert_before(&source, "</p:spTree>", &build(id))?.into_bytes(),
    );
    Ok(())
}

fn slide_entries(members: &BTreeMap<String, Vec<u8>>) -> OfficeResult<Vec<SlideEntry>> {
    let presentation = member_text(members, "ppt/presentation.xml")?;
    let relationships =
        member_text(members, "ppt/_rels/presentation.xml.rels").map(relationship_map)?;
    let mut reader = Reader::from_str(presentation);
    reader.config_mut().trim_text(true);
    let mut output = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if local_name(element.name().as_ref()) == b"sldId" =>
            {
                let relationship_id = attr_value_qualified(&element, b"r:id").unwrap_or_default();
                let path = relationships
                    .get(&relationship_id)
                    .filter(|(_, _, external)| !external)
                    .map(|(target, _, _)| normalize_ppt_target(target))
                    .ok_or_else(|| {
                        OfficeError::new(
                            "missing_package_part",
                            "slide relationship is missing",
                            json!({"relationship_id": relationship_id}),
                        )
                    })?;
                output.push(SlideEntry {
                    id: stable_slide_id(&path),
                    relationship_id,
                    path,
                });
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(OfficeError::new(
                    "malformed_xml",
                    format!("cannot parse presentation XML: {error}"),
                    json!({"part": "ppt/presentation.xml"}),
                ))
            }
            _ => {}
        }
    }
    Ok(output)
}

fn require_slide(members: &BTreeMap<String, Vec<u8>>, slide_id: &str) -> OfficeResult<SlideEntry> {
    slide_entries(members)?
        .into_iter()
        .find(|entry| entry.id == slide_id)
        .ok_or_else(|| object_not_found(slide_id))
}

fn resolve_layout_path(
    members: &BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<String> {
    let requested = operation
        .optional_string("layout_path")
        .or_else(|| operation.optional_string("layout"))
        .unwrap_or("ppt/slideLayouts/slideLayout1.xml");
    let path = if requested.starts_with("ppt/") {
        requested.to_string()
    } else if requested.starts_with("slideLayout") {
        format!("ppt/slideLayouts/{requested}.xml")
    } else {
        return Err(OfficeError::new(
            "invalid_selector",
            "layout must be a package path or slideLayout machine token",
            json!({"layout": requested}),
        ));
    };
    if !members.contains_key(&path) {
        return Err(OfficeError::new(
            "object_not_found",
            "selected slide layout does not exist",
            json!({"layout_path": path}),
        ));
    }
    Ok(path)
}

fn replace_relationship_target(source: &str, kind: &str, target: &str) -> OfficeResult<String> {
    let mut cursor = 0usize;
    while let Some(relative) = source[cursor..].find("<Relationship") {
        let start = cursor + relative;
        let end = source[start..]
            .find('>')
            .map(|value| start + value + 1)
            .ok_or_else(|| malformed("Relationship"))?;
        let opening = &source[start..end];
        if attribute_value(opening, "Type")
            .is_some_and(|value| value.ends_with(&format!("/{kind}")))
        {
            let opening = replace_or_add_attribute(opening, "Target", target);
            return Ok(format!("{}{}{}", &source[..start], opening, &source[end..]));
        }
        cursor = end;
    }
    Err(OfficeError::new(
        "object_not_found",
        "selected slide relationship does not exist",
        json!({"relationship_kind": kind}),
    ))
}

fn add_relationship(
    members: &mut BTreeMap<String, Vec<u8>>,
    path: &str,
    id: &str,
    kind: &str,
    target: &str,
    external: bool,
) -> OfficeResult<()> {
    let source = members
        .get(path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or(
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>",
        )
        .to_string();
    let value = format!(
        "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
        xml(id),
        xml(kind),
        xml(target),
        if external {
            " TargetMode=\"External\""
        } else {
            ""
        }
    );
    members.insert(
        path.to_string(),
        insert_before(&source, "</Relationships>", &value)?.into_bytes(),
    );
    Ok(())
}

fn ensure_content_override(
    members: &mut BTreeMap<String, Vec<u8>>,
    part_name: &str,
    content_type: &str,
) -> OfficeResult<()> {
    let source = member_text(members, "[Content_Types].xml")?.to_string();
    if source.contains(&format!("PartName=\"{}\"", xml(part_name))) {
        return Ok(());
    }
    let value = format!(
        "<Override PartName=\"{}\" ContentType=\"{}\"/>",
        xml(part_name),
        xml(content_type)
    );
    members.insert(
        "[Content_Types].xml".into(),
        insert_before(&source, "</Types>", &value)?.into_bytes(),
    );
    Ok(())
}

fn ensure_content_default(
    members: &mut BTreeMap<String, Vec<u8>>,
    extension: &str,
    content_type: &str,
) -> OfficeResult<()> {
    let source = member_text(members, "[Content_Types].xml")?.to_string();
    if source.contains(&format!("Extension=\"{}\"", xml(extension))) {
        return Ok(());
    }
    let value = format!(
        "<Default Extension=\"{}\" ContentType=\"{}\"/>",
        xml(extension),
        xml(content_type)
    );
    members.insert(
        "[Content_Types].xml".into(),
        insert_before(&source, "</Types>", &value)?.into_bytes(),
    );
    Ok(())
}

fn remove_content_override(
    members: &mut BTreeMap<String, Vec<u8>>,
    part_name: &str,
) -> OfficeResult<()> {
    let source = member_text(members, "[Content_Types].xml")?.to_string();
    members.insert(
        "[Content_Types].xml".into(),
        remove_element_by_attribute(&source, "Override", "PartName", part_name)?.into_bytes(),
    );
    Ok(())
}

fn remove_element_by_attribute(
    source: &str,
    element: &str,
    attribute: &str,
    expected: &str,
) -> OfficeResult<String> {
    let Some((start, end)) = find_element(source, element, |opening| {
        attribute_value(opening, attribute).as_deref() == Some(expected)
    })?
    else {
        return Err(OfficeError::new(
            "object_not_found",
            "selected package relationship or content type does not exist",
            json!({"element": element, "attribute": attribute, "value": expected}),
        ));
    };
    Ok(format!("{}{}", &source[..start], &source[end..]))
}

fn find_element(
    source: &str,
    element: &str,
    predicate: impl Fn(&str) -> bool,
) -> OfficeResult<Option<(usize, usize)>> {
    let token = format!("<{element}");
    let mut cursor = 0usize;
    while let Some(relative) = source[cursor..].find(&token) {
        let start = cursor + relative;
        let boundary = source.as_bytes().get(start + token.len()).copied();
        if !matches!(boundary, Some(b' ') | Some(b'>') | Some(b'/')) {
            cursor = start + token.len();
            continue;
        }
        let opening_end = source[start..]
            .find('>')
            .map(|value| start + value + 1)
            .ok_or_else(|| malformed(element))?;
        let opening = &source[start..opening_end];
        let end = if opening.trim_end().ends_with("/>") {
            opening_end
        } else {
            let closing = format!("</{element}>");
            source[opening_end..]
                .find(&closing)
                .map(|value| opening_end + value + closing.len())
                .ok_or_else(|| malformed(element))?
        };
        if predicate(opening) {
            return Ok(Some((start, end)));
        }
        cursor = end;
    }
    Ok(None)
}

fn slide_id_ranges(source: &str) -> OfficeResult<Vec<(usize, usize)>> {
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while let Some((start, end)) = find_element(&source[cursor..], "p:sldId", |_| true)? {
        output.push((cursor + start, cursor + end));
        cursor += end;
    }
    Ok(output)
}

fn set_root_attribute(
    source: &str,
    root: &str,
    attribute: &str,
    value: Option<&str>,
) -> OfficeResult<String> {
    let start = source
        .find(&format!("<{root}"))
        .ok_or_else(|| malformed(root))?;
    let end = source[start..]
        .find('>')
        .map(|value| start + value + 1)
        .ok_or_else(|| malformed(root))?;
    let opening = if let Some(value) = value {
        replace_or_add_attribute(&source[start..end], attribute, value)
    } else {
        remove_attribute(&source[start..end], attribute)
    };
    Ok(format!("{}{}{}", &source[..start], opening, &source[end..]))
}

fn replace_or_add_attribute(opening: &str, name: &str, value: &str) -> String {
    for quote in ['"', '\''] {
        let prefix = format!("{name}={quote}");
        if let Some(start) = opening.find(&prefix) {
            let value_start = start + prefix.len();
            if let Some(relative) = opening[value_start..].find(quote) {
                let end = value_start + relative;
                return format!(
                    "{}{}{}",
                    &opening[..value_start],
                    xml(value),
                    &opening[end..]
                );
            }
        }
    }
    let index = opening
        .rfind("/>")
        .or_else(|| opening.rfind('>'))
        .unwrap_or(opening.len());
    format!(
        "{} {name}=\"{}\"{}",
        &opening[..index],
        xml(value),
        &opening[index..]
    )
}

fn remove_attribute(opening: &str, name: &str) -> String {
    for quote in ['"', '\''] {
        let prefix = format!("{name}={quote}");
        if let Some(start) = opening.find(&prefix) {
            let value_start = start + prefix.len();
            if let Some(relative) = opening[value_start..].find(quote) {
                let end = value_start + relative + 1;
                let whitespace = opening[..start]
                    .rfind(|character: char| !character.is_whitespace())
                    .map(|value| value + 1)
                    .unwrap_or(start);
                return format!("{}{}", &opening[..whitespace], &opening[end..]);
            }
        }
    }
    opening.to_string()
}

fn max_non_visual_id(source: &str) -> usize {
    source
        .match_indices("<p:cNvPr")
        .filter_map(|(start, _)| {
            let end = source[start..].find('>').map(|value| start + value + 1)?;
            attribute_value(&source[start..end], "id")?.parse().ok()
        })
        .max()
        .unwrap_or(1)
}

fn max_slide_numeric_id(source: &str) -> usize {
    source
        .match_indices("<p:sldId")
        .filter_map(|(start, _)| {
            let end = source[start..].find('>').map(|value| start + value + 1)?;
            attribute_value(&source[start..end], "id")?.parse().ok()
        })
        .max()
        .unwrap_or(255)
}

fn next_part_number(members: &BTreeMap<String, Vec<u8>>, prefix: &str, suffix: &str) -> usize {
    let used = members
        .keys()
        .filter_map(|name| {
            let value = name.strip_prefix(prefix)?;
            let number = if suffix.is_empty() {
                value.split_once('.').map(|(value, _)| value)?
            } else {
                value.strip_suffix(suffix)?
            };
            number.parse::<usize>().ok()
        })
        .collect::<BTreeSet<_>>();
    (1..).find(|value| !used.contains(value)).unwrap_or(1)
}

fn slide_xml(title: &str, body: &[String]) -> String {
    let title_shape = (!title.is_empty())
        .then(|| text_shape(2, title))
        .unwrap_or_default();
    let body_shapes = body
        .iter()
        .enumerate()
        .map(|(index, text)| text_shape(index + 3, text))
        .collect::<String>();
    format!(
        "<?xml version=\"1.0\"?><p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{title_shape}{body_shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    )
}

fn text_shape(id: usize, text: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Text {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"914400\" y=\"{}\"/><a:ext cx=\"9144000\" cy=\"685800\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
        457200 + id * 500000,
        xml(text)
    )
}

fn shape_xml(id: usize, shape: &str, text: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Shape {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"914400\" y=\"3657600\"/><a:ext cx=\"2743200\" cy=\"1371600\"/></a:xfrm><a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
        xml(shape),
        xml(text)
    )
}

fn table_shape(id: usize, rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let grid = (0..columns)
        .map(|_| "<a:gridCol w=\"1800000\"/>")
        .collect::<String>();
    let rows = rows
        .iter()
        .map(|row| {
            let cells = (0..columns)
                .map(|index| {
                    format!(
                        "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>",
                        xml(row.get(index).map(String::as_str).unwrap_or(""))
                    )
                })
                .collect::<String>();
            format!("<a:tr h=\"500000\">{cells}</a:tr>")
        })
        .collect::<String>();
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Table {id}\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"914400\" y=\"2514600\"/><a:ext cx=\"9144000\" cy=\"2500000\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr firstRow=\"1\" bandRow=\"1\"/><a:tblGrid>{grid}</a:tblGrid>{rows}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"
    )
}

fn image_shape(id: usize, relationship_id: &str, alt: &str) -> String {
    format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Image {id}\" descr=\"{}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x=\"7772400\" y=\"1600200\"/><a:ext cx=\"3657600\" cy=\"2743200\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
        xml(alt),
        xml(relationship_id)
    )
}

fn chart_shape(id: usize, relationship_id: &str) -> String {
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Chart {id}\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"5486400\" y=\"2514600\"/><a:ext cx=\"5486400\" cy=\"3200400\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart r:id=\"{}\"/></a:graphicData></a:graphic></p:graphicFrame>",
        xml(relationship_id)
    )
}

fn link_shape(id: usize, relationship_id: &str, text: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Link {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"914400\" y=\"5943600\"/><a:ext cx=\"3657600\" cy=\"457200\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr><a:hlinkClick r:id=\"{}\"/></a:rPr><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
        xml(relationship_id),
        xml(text)
    )
}

fn chart_xml(title: &str, chart_type: &str, categories: &[String], values: &[f64]) -> String {
    let tag = match chart_type {
        "line" => "lineChart",
        "pie" => "pieChart",
        _ => "barChart",
    };
    let categories = categories
        .iter()
        .enumerate()
        .map(|(index, value)| format!("<c:pt idx=\"{index}\"><c:v>{}</c:v></c:pt>", xml(value)))
        .collect::<String>();
    let values = values
        .iter()
        .enumerate()
        .map(|(index, value)| format!("<c:pt idx=\"{index}\"><c:v>{value}</c:v></c:pt>"))
        .collect::<String>();
    format!(
        "<?xml version=\"1.0\"?><c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:{tag}><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:cat><c:strLit>{categories}</c:strLit></c:cat><c:val><c:numLit>{values}</c:numLit></c:val></c:ser></c:{tag}></c:plotArea></c:chart></c:chartSpace>",
        xml(title)
    )
}

fn notes_xml(number: usize, notes: &[String]) -> String {
    let paragraphs = notes
        .iter()
        .map(|value| format!("<a:p><a:r><a:t>{}</a:t></a:r></a:p>", xml(value)))
        .collect::<String>();
    format!(
        "<?xml version=\"1.0\"?><p:notes xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><p:cSld name=\"Notes {number}\"><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Notes Text\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{paragraphs}</p:txBody></p:sp></p:spTree></p:cSld></p:notes>"
    )
}

fn transition_xml(kind: &str) -> String {
    let tag = match kind {
        "fade" => "fade",
        "push" => "push",
        "wipe" => "wipe",
        _ => "fade",
    };
    format!("<p:transition><p:{tag}/></p:transition>")
}

fn set_or_insert_element(
    source: &str,
    tag: &str,
    value: &str,
    parent_close: &str,
) -> OfficeResult<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    if let Some(start) = source.find(&open) {
        let content_start = start + open.len();
        let end = source[content_start..]
            .find(&close)
            .map(|value| content_start + value)
            .ok_or_else(|| malformed(tag))?;
        return Ok(format!(
            "{}{}{}",
            &source[..content_start],
            xml(value),
            &source[end..]
        ));
    }
    insert_before(
        source,
        parent_close,
        &format!("<{tag}>{}</{tag}>", xml(value)),
    )
}

fn image_extension(path: &Path) -> OfficeResult<&str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| OfficeError::invalid("image source requires a file extension"))?;
    match extension.as_str() {
        "png" => Ok("png"),
        "jpg" | "jpeg" => Ok("jpeg"),
        "gif" => Ok("gif"),
        _ => Err(OfficeError::unsupported(
            "supported presentation image inputs are PNG, JPEG, and GIF",
            json!({"extension": extension}),
        )),
    }
}

fn image_content_type(extension: &str) -> &'static str {
    match extension {
        "jpeg" | "jpg" => "image/jpeg",
        "gif" => "image/gif",
        _ => "image/png",
    }
}

fn member_text<'a>(members: &'a BTreeMap<String, Vec<u8>>, name: &str) -> OfficeResult<&'a str> {
    members
        .get(name)
        .ok_or_else(|| missing_part(name))
        .and_then(|bytes| {
            std::str::from_utf8(bytes).map_err(|error| {
                OfficeError::new(
                    "malformed_xml",
                    format!("PPTX package part is not UTF-8 XML: {error}"),
                    json!({"member": name}),
                )
            })
        })
}

fn insert_before(source: &str, needle: &str, content: &str) -> OfficeResult<String> {
    let index = source.rfind(needle).ok_or_else(|| malformed(needle))?;
    Ok(format!(
        "{}{}{}",
        &source[..index],
        content,
        &source[index..]
    ))
}

fn attribute_value(opening: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let prefix = format!("{name}={quote}");
        if let Some(start) = opening.find(&prefix) {
            let value_start = start + prefix.len();
            let end = opening[value_start..].find(quote)? + value_start;
            return Some(opening[value_start..end].to_string());
        }
    }
    None
}

fn slide_relationships_path(slide_path: &str) -> String {
    let (base, file) = slide_path.rsplit_once('/').unwrap_or(("", slide_path));
    format!("{base}/_rels/{file}.rels")
}

fn relative_slide_target(path: &str) -> String {
    path.strip_prefix("ppt/")
        .map(|value| format!("../{value}"))
        .unwrap_or_else(|| path.to_string())
}

fn normalize_ppt_target(target: &str) -> String {
    if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else if target.starts_with("ppt/") {
        target.to_string()
    } else {
        normalize_segments(&format!("ppt/{target}"))
    }
}

fn normalize_segments(value: &str) -> String {
    let mut output = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                output.pop();
            }
            _ => output.push(part),
        }
    }
    output.join("/")
}

fn stable_slide_id(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(_, file)| file)
        .unwrap_or(path)
        .strip_prefix("slide")
        .and_then(|value| value.strip_suffix(".xml"))
        .filter(|value| {
            !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
        })
        .map(|value| format!("slide_{value}"))
        .unwrap_or_else(|| format!("slide_part_{}", crate::package::hash_bytes(path.as_bytes())))
}

fn string_list(value: &Value) -> OfficeResult<Vec<String>> {
    match value {
        Value::String(value) => Ok(vec![value.clone()]),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| OfficeError::invalid("text arrays must contain strings"))
            })
            .collect(),
        _ => Err(OfficeError::invalid(
            "text content must be a string or string array",
        )),
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn invalid_field(operation: &NormalizedOperation, field: &str) -> OfficeError {
    OfficeError::new(
        "invalid_operation",
        "operation field is missing or invalid",
        json!({"operation_id": operation.id, "op": operation.kind, "field": field}),
    )
}

fn missing_part(part: &str) -> OfficeError {
    OfficeError::new(
        "missing_package_part",
        "required PPTX package part is missing",
        json!({"member": part}),
    )
}

fn object_not_found(id: &str) -> OfficeError {
    OfficeError::new(
        "object_not_found",
        "selected presentation object does not exist",
        json!({"object_id": id}),
    )
}

fn malformed(element: &str) -> OfficeError {
    OfficeError::new(
        "malformed_xml",
        "selected presentation XML element is malformed",
        json!({"element": element}),
    )
}

fn xml(value: &str) -> String {
    escape(value).into_owned()
}

#[cfg(test)]
#[path = "pptx_edit_tests.rs"]
mod tests;
