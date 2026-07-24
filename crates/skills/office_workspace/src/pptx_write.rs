use crate::error::{OfficeError, OfficeResult};
use crate::operations::NormalizedOperation;
use crate::package::{resolve_input_path, OfficePackage};
use quick_xml::escape::escape;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub struct PptxWriteResult {
    pub members: BTreeMap<String, Vec<u8>>,
    pub changed_refs: Vec<String>,
    pub preservation: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct SlideBuild {
    title: String,
    paragraphs: Vec<String>,
    notes: Vec<String>,
    hidden: bool,
    layout: String,
    tables: Vec<Vec<Vec<String>>>,
    charts: Vec<ChartBuild>,
    images: Vec<ImageBuild>,
    shapes: Vec<ShapeBuild>,
    links: Vec<LinkBuild>,
    transition: Option<String>,
}

#[derive(Clone, Debug)]
struct ChartBuild {
    title: String,
    categories: Vec<String>,
    values: Vec<f64>,
    chart_type: String,
}

#[derive(Clone, Debug)]
struct ImageBuild {
    path: String,
    alt: String,
}

#[derive(Clone, Debug)]
struct ShapeBuild {
    text: String,
    shape: String,
}

#[derive(Clone, Debug)]
struct LinkBuild {
    text: String,
    url: String,
}

pub fn create_pptx(operations: &[NormalizedOperation]) -> OfficeResult<PptxWriteResult> {
    let mut slides = Vec::<SlideBuild>::new();
    for operation in operations {
        apply_create_operation(&mut slides, operation)?;
    }
    if slides.is_empty() {
        return Err(OfficeError::new(
            "invalid_operation",
            "presentation creation requires at least one add_slide operation",
            json!({}),
        ));
    }
    Ok(PptxWriteResult {
        members: build_presentation(&slides)?,
        changed_refs: operations
            .iter()
            .flat_map(NormalizedOperation::object_refs)
            .collect(),
        preservation: vec!["new_package".to_string()],
    })
}

pub fn edit_pptx(
    package: &OfficePackage,
    operations: &[NormalizedOperation],
) -> OfficeResult<PptxWriteResult> {
    let mut members = package.members.clone();
    let mut changed_refs = Vec::new();
    for operation in operations {
        match operation.kind.as_str() {
            "replace_slide_text" => {
                let slide_id = operation.string("slide_id")?;
                let index = slide_index(slide_id)?;
                let path = format!("ppt/slides/slide{index}.xml");
                let source = member_text(&members, &path)?.to_string();
                let expected = operation.string("match")?;
                let replacement = operation.string("text")?;
                members.insert(
                    path,
                    replace_drawing_text(&source, expected, replacement)?.into_bytes(),
                );
                changed_refs.push(slide_id.to_string());
            }
            "add_notes" => {
                let slide_id = operation.string("slide_id")?;
                let index = slide_index(slide_id)?;
                let notes = operation
                    .value("notes")
                    .map(string_list)
                    .transpose()?
                    .unwrap_or_else(|| {
                        operation
                            .optional_string("text")
                            .map(|text| vec![text.to_string()])
                            .unwrap_or_default()
                    });
                if notes.is_empty() {
                    return Err(invalid_operation_field(operation, "notes|text"));
                }
                members.insert(
                    format!("ppt/notesSlides/notesSlide{index}.xml"),
                    notes_xml(index, &notes).into_bytes(),
                );
                ensure_slide_relation(
                    &mut members,
                    index,
                    "notesSlide",
                    &format!("../notesSlides/notesSlide{index}.xml"),
                    &format!("rIdRustClawNotes{index}"),
                    false,
                )?;
                changed_refs.push(format!("{slide_id}:notes"));
            }
            "move_slide" => {
                let slide_id = operation.string("slide_id")?;
                let index = slide_index(slide_id)?;
                let new_position = operation.usize("position")?;
                let presentation = member_text(&members, "ppt/presentation.xml")?.to_string();
                members.insert(
                    "ppt/presentation.xml".into(),
                    move_slide_id(&presentation, index, new_position)?.into_bytes(),
                );
                changed_refs.push(slide_id.to_string());
            }
            "hide_slide" => {
                let slide_id = operation.string("slide_id")?;
                let index = slide_index(slide_id)?;
                let hidden = operation.bool("hidden").unwrap_or(true);
                let path = format!("ppt/slides/slide{index}.xml");
                let slide = member_text(&members, &path)?.to_string();
                members.insert(path, set_slide_hidden(&slide, hidden).into_bytes());
                changed_refs.push(slide_id.to_string());
            }
            "replace_image" => {
                let media_id = operation.string("media_id")?;
                let source = resolve_input_path(operation.string("path")?)?;
                replace_media(&mut members, media_id, &source)?;
                changed_refs.push(media_id.to_string());
            }
            "delete_slide" => {
                let slide_id = operation.string("slide_id")?;
                let index = slide_index(slide_id)?;
                let presentation = member_text(&members, "ppt/presentation.xml")?.to_string();
                let relation_id = format!("rIdSlide{index}");
                members.insert(
                    "ppt/presentation.xml".into(),
                    remove_slide_id(&presentation, &relation_id)?.into_bytes(),
                );
                members.remove(&format!("ppt/slides/slide{index}.xml"));
                members.remove(&format!("ppt/slides/_rels/slide{index}.xml.rels"));
                changed_refs.push(slide_id.to_string());
            }
            _ => {
                return Err(OfficeError::unsupported(
                    "PPTX edit operation is not implemented without potential layout loss",
                    json!({"operation_id": operation.id, "op": operation.kind}),
                ))
            }
        }
    }
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

fn apply_create_operation(
    slides: &mut Vec<SlideBuild>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    match operation.kind.as_str() {
        "set_properties" => {}
        "add_slide" => {
            let title = operation.optional_string("title").unwrap_or("").to_string();
            let paragraphs = operation
                .value("body")
                .map(string_list)
                .transpose()?
                .unwrap_or_default();
            let notes = operation
                .value("notes")
                .map(string_list)
                .transpose()?
                .unwrap_or_default();
            slides.push(SlideBuild {
                title,
                paragraphs,
                notes,
                hidden: operation.bool("hidden").unwrap_or(false),
                layout: operation
                    .optional_string("layout")
                    .unwrap_or("title_and_content")
                    .to_string(),
                ..SlideBuild::default()
            });
        }
        "add_text" => {
            slide_mut(slides, operation.string("slide_id")?)?
                .paragraphs
                .push(operation.string("text")?.to_string());
        }
        "add_notes" => {
            let slide = slide_mut(slides, operation.string("slide_id")?)?;
            let values = operation
                .value("notes")
                .map(string_list)
                .transpose()?
                .unwrap_or_else(|| {
                    operation
                        .optional_string("text")
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_default()
                });
            if values.is_empty() {
                return Err(invalid_operation_field(operation, "notes|text"));
            }
            slide.notes.extend(values);
        }
        "add_table" => {
            let rows = operation
                .value("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid_operation_field(operation, "rows"))?
                .iter()
                .map(|row| {
                    row.as_array()
                        .ok_or_else(|| invalid_operation_field(operation, "rows"))
                        .map(|cells| cells.iter().map(scalar_text).collect::<Vec<_>>())
                })
                .collect::<OfficeResult<Vec<_>>>()?;
            slide_mut(slides, operation.string("slide_id")?)?
                .tables
                .push(rows);
        }
        "add_chart" => {
            let categories = operation
                .value("categories")
                .map(string_list)
                .transpose()?
                .ok_or_else(|| invalid_operation_field(operation, "categories"))?;
            let values = operation
                .value("values")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid_operation_field(operation, "values"))?
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .ok_or_else(|| invalid_operation_field(operation, "values"))
                })
                .collect::<OfficeResult<Vec<_>>>()?;
            if categories.len() != values.len() || categories.is_empty() {
                return Err(OfficeError::new(
                    "chart_shape_mismatch",
                    "chart categories and values must have the same non-zero length",
                    json!({"operation_id": operation.id}),
                ));
            }
            slide_mut(slides, operation.string("slide_id")?)?
                .charts
                .push(ChartBuild {
                    title: operation
                        .optional_string("title")
                        .unwrap_or("Chart")
                        .to_string(),
                    categories,
                    values,
                    chart_type: operation
                        .optional_string("chart_type")
                        .unwrap_or("column")
                        .to_string(),
                });
        }
        "add_image" => {
            let path = resolve_input_path(operation.string("path")?)?;
            validate_image_path(&path)?;
            slide_mut(slides, operation.string("slide_id")?)?
                .images
                .push(ImageBuild {
                    path: path.display().to_string(),
                    alt: operation
                        .optional_string("alt")
                        .unwrap_or("image")
                        .to_string(),
                });
        }
        "add_shape" => {
            slide_mut(slides, operation.string("slide_id")?)?
                .shapes
                .push(ShapeBuild {
                    text: operation.optional_string("text").unwrap_or("").to_string(),
                    shape: operation
                        .optional_string("shape")
                        .unwrap_or("rect")
                        .to_string(),
                });
        }
        "add_link" => {
            slide_mut(slides, operation.string("slide_id")?)?
                .links
                .push(LinkBuild {
                    text: operation.string("text")?.to_string(),
                    url: operation.string("url")?.to_string(),
                });
        }
        "set_transition" => {
            slide_mut(slides, operation.string("slide_id")?)?.transition =
                Some(operation.string("transition")?.to_string());
        }
        _ => {
            return Err(OfficeError::unsupported(
                "PPTX create operation is not implemented",
                json!({"operation_id": operation.id, "op": operation.kind}),
            ))
        }
    }
    Ok(())
}

fn build_presentation(slides: &[SlideBuild]) -> OfficeResult<BTreeMap<String, Vec<u8>>> {
    let mut members = BTreeMap::new();
    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#,
    );
    let mut presentation_ids = String::new();
    let mut presentation_rels = String::new();
    let mut media_index = 0usize;
    let mut chart_index = 0usize;
    for (index, slide) in slides.iter().enumerate() {
        let number = index + 1;
        content_types.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{number}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
        presentation_ids.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rIdSlide{number}\"/>",
            255 + number
        ));
        presentation_rels.push_str(&format!(
            "<Relationship Id=\"rIdSlide{number}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{number}.xml\"/>"
        ));
        let built = build_slide(slide, number, &mut media_index, &mut chart_index)?;
        members.insert(
            format!("ppt/slides/slide{number}.xml"),
            built.xml.into_bytes(),
        );
        members.insert(
            format!("ppt/slides/_rels/slide{number}.xml.rels"),
            built.relationships.into_bytes(),
        );
        for (name, bytes) in built.parts {
            if name.starts_with("ppt/charts/") {
                content_types.push_str(&format!(
                    "<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.chart+xml\"/>"
                ));
            }
            if let Some(extension) = Path::new(&name)
                .extension()
                .and_then(|value| value.to_str())
            {
                if name.starts_with("ppt/media/") {
                    content_types.push_str(&format!(
                        "<Default Extension=\"{}\" ContentType=\"{}\"/>",
                        xml(extension),
                        image_content_type(extension)
                    ));
                }
            }
            members.insert(name, bytes);
        }
        if !slide.notes.is_empty() {
            content_types.push_str(&format!(
                "<Override PartName=\"/ppt/notesSlides/notesSlide{number}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml\"/>"
            ));
            members.insert(
                format!("ppt/notesSlides/notesSlide{number}.xml"),
                notes_xml(number, &slide.notes).into_bytes(),
            );
        }
    }
    content_types.push_str("</Types>");
    members.insert("[Content_Types].xml".into(), content_types.into_bytes());
    members.insert(
        "_rels/.rels".into(),
        br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#.to_vec(),
    );
    members.insert(
        "ppt/presentation.xml".into(),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdMaster1\"/></p:sldMasterIdLst><p:sldIdLst>{presentation_ids}</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\" type=\"screen16x9\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
        )
        .into_bytes(),
    );
    members.insert(
        "ppt/_rels/presentation.xml.rels".into(),
        format!(
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdMaster1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>{presentation_rels}</Relationships>"
        )
        .into_bytes(),
    );
    members.insert(
        "ppt/slideMasters/slideMaster1.xml".into(),
        slide_master_xml().as_bytes().to_vec(),
    );
    members.insert(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels".into(),
        br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdLayout1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rIdTheme1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#.to_vec(),
    );
    members.insert(
        "ppt/slideLayouts/slideLayout1.xml".into(),
        slide_layout_xml().as_bytes().to_vec(),
    );
    members.insert(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
        br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdMaster1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#.to_vec(),
    );
    members.insert(
        "ppt/theme/theme1.xml".into(),
        theme_xml().as_bytes().to_vec(),
    );
    Ok(members)
}

struct BuiltSlide {
    xml: String,
    relationships: String,
    parts: BTreeMap<String, Vec<u8>>,
}

fn build_slide(
    slide: &SlideBuild,
    number: usize,
    media_index: &mut usize,
    chart_index: &mut usize,
) -> OfficeResult<BuiltSlide> {
    let mut shapes = String::from(
        "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>",
    );
    let mut shape_id = 2usize;
    if !slide.title.is_empty() {
        shapes.push_str(&text_shape(
            shape_id,
            "Title",
            &[slide.title.clone()],
            457200,
            274638,
            11277600,
            1143000,
            true,
        ));
        shape_id += 1;
    }
    if !slide.paragraphs.is_empty() {
        shapes.push_str(&text_shape(
            shape_id,
            "Content",
            &slide.paragraphs,
            685800,
            1600200,
            10820400,
            4114800,
            false,
        ));
        shape_id += 1;
    }
    for shape in &slide.shapes {
        shapes.push_str(&basic_shape(shape_id, shape));
        shape_id += 1;
    }
    for table in &slide.tables {
        shapes.push_str(&table_shape(shape_id, table));
        shape_id += 1;
    }
    let mut relationships = vec![
        r#"<Relationship Id="rIdLayout" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>"#.to_string(),
    ];
    let mut parts = BTreeMap::new();
    for image in &slide.images {
        *media_index += 1;
        let extension = image_extension(Path::new(&image.path))?;
        let media_name = format!("ppt/media/image{media_index}.{extension}");
        let bytes = fs::read(&image.path).map_err(|error| {
            OfficeError::new(
                "source_unavailable",
                format!("cannot read presentation image: {error}"),
                json!({"path": image.path}),
            )
        })?;
        parts.insert(media_name, bytes);
        let relation_id = format!("rIdImage{media_index}");
        relationships.push(format!(
            "<Relationship Id=\"{relation_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/image{media_index}.{extension}\"/>"
        ));
        shapes.push_str(&image_shape(shape_id, &relation_id, &image.alt));
        shape_id += 1;
    }
    for chart in &slide.charts {
        *chart_index += 1;
        let relation_id = format!("rIdChart{chart_index}");
        relationships.push(format!(
            "<Relationship Id=\"{relation_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart\" Target=\"../charts/chart{chart_index}.xml\"/>"
        ));
        parts.insert(
            format!("ppt/charts/chart{chart_index}.xml"),
            chart_xml(chart).into_bytes(),
        );
        shapes.push_str(&chart_shape(shape_id, &relation_id));
        shape_id += 1;
    }
    for (index, link) in slide.links.iter().enumerate() {
        let relation_id = format!("rIdLink{}", index + 1);
        relationships.push(format!(
            "<Relationship Id=\"{relation_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
            xml(&link.url)
        ));
        shapes.push_str(&link_shape(shape_id, &relation_id, &link.text));
        shape_id += 1;
    }
    if !slide.notes.is_empty() {
        relationships.push(format!(
            "<Relationship Id=\"rIdRustClawNotes{number}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide\" Target=\"../notesSlides/notesSlide{number}.xml\"/>"
        ));
    }
    let transition = slide
        .transition
        .as_deref()
        .map(transition_xml)
        .unwrap_or_default();
    let hidden = if slide.hidden { " show=\"0\"" } else { "" };
    let layout_marker = xml(&slide.layout);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"{hidden}><p:cSld name=\"{layout_marker}\"><p:spTree>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>{transition}</p:sld>"
    );
    let relationships = format!(
        "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>",
        relationships.join("")
    );
    Ok(BuiltSlide {
        xml,
        relationships,
        parts,
    })
}

fn text_shape(
    id: usize,
    name: &str,
    paragraphs: &[String],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    title: bool,
) -> String {
    let paragraphs = paragraphs
        .iter()
        .map(|text| {
            format!(
                "<a:p><a:r><a:rPr lang=\"en-US\" sz=\"{}\"/><a:t>{}</a:t></a:r></a:p>",
                if title { 3200 } else { 1800 },
                xml(text)
            )
        })
        .collect::<String>();
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{} {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{paragraphs}</p:txBody></p:sp>",
        xml(name)
    )
}

fn basic_shape(id: usize, shape: &ShapeBuild) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Shape {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"914400\" y=\"3657600\"/><a:ext cx=\"2743200\" cy=\"1371600\"/></a:xfrm><a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
        xml(&shape.shape),
        xml(&shape.text)
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
                    let value = row.get(index).cloned().unwrap_or_default();
                    format!(
                        "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>",
                        xml(&value)
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

fn image_shape(id: usize, relation_id: &str, alt: &str) -> String {
    format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Image {id}\" descr=\"{}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x=\"7772400\" y=\"1600200\"/><a:ext cx=\"3657600\" cy=\"2743200\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
        xml(alt),
        xml(relation_id)
    )
}

fn chart_shape(id: usize, relation_id: &str) -> String {
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Chart {id}\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"5486400\" y=\"2514600\"/><a:ext cx=\"5486400\" cy=\"3200400\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart r:id=\"{}\"/></a:graphicData></a:graphic></p:graphicFrame>",
        xml(relation_id)
    )
}

fn link_shape(id: usize, relation_id: &str, text: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Link {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"914400\" y=\"5943600\"/><a:ext cx=\"3657600\" cy=\"457200\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr><a:hlinkClick r:id=\"{}\"/></a:rPr><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
        xml(relation_id),
        xml(text)
    )
}

fn chart_xml(chart: &ChartBuild) -> String {
    let tag = match chart.chart_type.as_str() {
        "line" => "lineChart",
        "pie" => "pieChart",
        _ => "barChart",
    };
    let categories = chart
        .categories
        .iter()
        .enumerate()
        .map(|(index, value)| format!("<c:pt idx=\"{index}\"><c:v>{}</c:v></c:pt>", xml(value)))
        .collect::<String>();
    let values = chart
        .values
        .iter()
        .enumerate()
        .map(|(index, value)| format!("<c:pt idx=\"{index}\"><c:v>{value}</c:v></c:pt>"))
        .collect::<String>();
    format!(
        "<?xml version=\"1.0\"?><c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:{tag}><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:cat><c:strLit><c:ptCount val=\"{}\"/>{categories}</c:strLit></c:cat><c:val><c:numLit><c:ptCount val=\"{}\"/>{values}</c:numLit></c:val></c:ser></c:{tag}></c:plotArea></c:chart></c:chartSpace>",
        xml(&chart.title),
        chart.categories.len(),
        chart.values.len()
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

fn replace_drawing_text(source: &str, expected: &str, replacement: &str) -> OfficeResult<String> {
    let escaped_expected = xml(expected);
    if !source.contains(&escaped_expected) {
        return Err(OfficeError::new(
            "source_conflict",
            "expected slide text is absent from the selected revision",
            json!({"expected_text": expected}),
        ));
    }
    Ok(source.replacen(&escaped_expected, &xml(replacement), 1))
}

fn move_slide_id(source: &str, slide_index: usize, new_position: usize) -> OfficeResult<String> {
    let ranges = slide_id_ranges(source)?;
    if slide_index == 0
        || slide_index > ranges.len()
        || new_position == 0
        || new_position > ranges.len()
    {
        return Err(OfficeError::new(
            "invalid_selector",
            "slide move index is outside the presentation",
            json!({"slide_index": slide_index, "position": new_position, "slide_count": ranges.len()}),
        ));
    }
    let items = ranges
        .iter()
        .map(|(start, end)| source[*start..*end].to_string())
        .collect::<Vec<_>>();
    let mut reordered = items.clone();
    let item = reordered.remove(slide_index - 1);
    reordered.insert(new_position - 1, item);
    let start = ranges.first().map(|range| range.0).unwrap_or(0);
    let end = ranges.last().map(|range| range.1).unwrap_or(start);
    Ok(format!(
        "{}{}{}",
        &source[..start],
        reordered.join(""),
        &source[end..]
    ))
}

fn remove_slide_id(source: &str, relationship_id: &str) -> OfficeResult<String> {
    for (start, end) in slide_id_ranges(source)? {
        if source[start..end].contains(&format!("r:id=\"{}\"", xml(relationship_id))) {
            return Ok(format!("{}{}", &source[..start], &source[end..]));
        }
    }
    Err(OfficeError::new(
        "object_not_found",
        "selected slide relationship does not exist",
        json!({"relationship_id": relationship_id}),
    ))
}

fn slide_id_ranges(source: &str) -> OfficeResult<Vec<(usize, usize)>> {
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = source[cursor..].find("<p:sldId") {
        let start = cursor + relative;
        let boundary = source.as_bytes().get(start + 8).copied();
        if !matches!(boundary, Some(b' ') | Some(b'>') | Some(b'/')) {
            cursor = start + 8;
            continue;
        }
        let end = source[start..]
            .find("/>")
            .map(|relative| start + relative + 2)
            .ok_or_else(|| malformed_xml("p:sldId"))?;
        output.push((start, end));
        cursor = end;
    }
    Ok(output)
}

fn set_slide_hidden(source: &str, hidden: bool) -> String {
    if hidden {
        if source.contains("<p:sld ") {
            source.replacen("<p:sld ", "<p:sld show=\"0\" ", 1)
        } else {
            source.replacen("<p:sld>", "<p:sld show=\"0\">", 1)
        }
    } else {
        source.replace(" show=\"0\"", "")
    }
}

fn ensure_slide_relation(
    members: &mut BTreeMap<String, Vec<u8>>,
    slide_index: usize,
    kind: &str,
    target: &str,
    id: &str,
    external: bool,
) -> OfficeResult<()> {
    let path = format!("ppt/slides/_rels/slide{slide_index}.xml.rels");
    let source = members
        .get(&path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#);
    if source.contains(&format!("Id=\"{}\"", xml(id))) {
        return Ok(());
    }
    let relation = format!(
        "<Relationship Id=\"{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{}\" Target=\"{}\"{}/>",
        xml(id),
        xml(kind),
        xml(target),
        if external { " TargetMode=\"External\"" } else { "" }
    );
    let updated = insert_before(source, "</Relationships>", &relation)?;
    members.insert(path, updated.into_bytes());
    Ok(())
}

fn replace_media(
    members: &mut BTreeMap<String, Vec<u8>>,
    media_id: &str,
    source: &Path,
) -> OfficeResult<()> {
    let index = media_id
        .strip_prefix("media_")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|index| *index > 0)
        .ok_or_else(|| OfficeError::invalid("media_id must use media_<index> format"))?;
    let member = members
        .keys()
        .filter(|name| name.starts_with("ppt/media/"))
        .nth(index - 1)
        .cloned()
        .ok_or_else(|| {
            OfficeError::new(
                "object_not_found",
                "selected presentation image does not exist",
                json!({"media_id": media_id}),
            )
        })?;
    let existing = Path::new(&member)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let replacement = image_extension(source)?;
    if existing != replacement {
        return Err(OfficeError::unsupported(
            "image replacement must preserve the package media type",
            json!({"existing_extension": existing, "source_extension": replacement}),
        ));
    }
    let bytes = fs::read(source).map_err(|error| {
        OfficeError::new(
            "source_unavailable",
            format!("cannot read replacement image: {error}"),
            json!({"path": source.display().to_string()}),
        )
    })?;
    members.insert(member, bytes);
    Ok(())
}

fn slide_mut<'a>(slides: &'a mut [SlideBuild], slide_id: &str) -> OfficeResult<&'a mut SlideBuild> {
    let index = slide_index(slide_id)?;
    let slide_count = slides.len();
    slides.get_mut(index - 1).ok_or_else(|| {
        OfficeError::new(
            "object_not_found",
            "selected slide does not exist",
            json!({"slide_id": slide_id, "slide_count": slide_count}),
        )
    })
}

fn slide_index(slide_id: &str) -> OfficeResult<usize> {
    slide_id
        .strip_prefix("slide_")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| OfficeError::invalid("slide_id must use slide_<index> format"))
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

fn transition_xml(kind: &str) -> String {
    let tag = match kind {
        "fade" => "fade",
        "push" => "push",
        "wipe" => "wipe",
        _ => "fade",
    };
    format!("<p:transition><p:{tag}/></p:transition>")
}

fn slide_master_xml() -> &'static str {
    r#"<?xml version="1.0"?><p:sldMaster xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMap accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" bg1="lt1" bg2="lt2" folHlink="folHlink" hlink="hlink" tx1="dk1" tx2="dk2"/><p:sldLayoutIdLst><p:sldLayoutId id="1" r:id="rIdLayout1"/></p:sldLayoutIdLst></p:sldMaster>"#
}

fn slide_layout_xml() -> &'static str {
    r#"<?xml version="1.0"?><p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" type="obj"><p:cSld name="Title and Content"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#
}

fn theme_xml() -> &'static str {
    r#"<?xml version="1.0"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="RustClaw"><a:themeElements><a:clrScheme name="RustClaw"><a:dk1><a:srgbClr val="1F2937"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="334155"/></a:dk2><a:lt2><a:srgbClr val="F8FAFC"/></a:lt2><a:accent1><a:srgbClr val="0F766E"/></a:accent1><a:accent2><a:srgbClr val="2563EB"/></a:accent2><a:accent3><a:srgbClr val="CA8A04"/></a:accent3><a:accent4><a:srgbClr val="7C3AED"/></a:accent4><a:accent5><a:srgbClr val="DC2626"/></a:accent5><a:accent6><a:srgbClr val="0891B2"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="RustClaw"><a:majorFont><a:latin typeface="Aptos Display"/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/></a:minorFont></a:fontScheme><a:fmtScheme name="RustClaw"><a:fillStyleLst/><a:lnStyleLst/><a:effectStyleLst/><a:bgFillStyleLst/></a:fmtScheme></a:themeElements></a:theme>"#
}

fn validate_image_path(path: &Path) -> OfficeResult<()> {
    image_extension(path).map(|_| ())
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
        .ok_or_else(|| {
            OfficeError::new(
                "missing_package_part",
                "required PPTX package part is missing",
                json!({"member": name}),
            )
        })
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
    let index = source.rfind(needle).ok_or_else(|| {
        OfficeError::new(
            "malformed_xml",
            "required presentation closing element is missing",
            json!({"closing_element": needle}),
        )
    })?;
    Ok(format!(
        "{}{}{}",
        &source[..index],
        content,
        &source[index..]
    ))
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

fn xml(value: &str) -> String {
    escape(value).into_owned()
}

fn invalid_operation_field(operation: &NormalizedOperation, field: &str) -> OfficeError {
    OfficeError::new(
        "invalid_operation",
        "operation field is missing or invalid",
        json!({"operation_id": operation.id, "op": operation.kind, "field": field}),
    )
}

fn malformed_xml(element: &str) -> OfficeError {
    OfficeError::new(
        "malformed_xml",
        "selected presentation XML element is malformed",
        json!({"element": element}),
    )
}

#[cfg(test)]
#[path = "pptx_write_tests.rs"]
mod tests;
