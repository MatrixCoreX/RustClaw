use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::BTreeMap;

pub fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

pub fn attr_value(element: &BytesStart<'_>, local: &[u8]) -> Option<String> {
    element
        .attributes()
        .with_checks(false)
        .filter_map(Result::ok)
        .find(|attribute| local_name(attribute.key.as_ref()) == local)
        .map(|attribute| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
}

pub fn attr_value_qualified(element: &BytesStart<'_>, qualified: &[u8]) -> Option<String> {
    element
        .attributes()
        .with_checks(false)
        .filter_map(Result::ok)
        .find(|attribute| attribute.key.as_ref() == qualified)
        .map(|attribute| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
}

pub fn collect_text(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut values = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(text)) => {
                if let Ok(value) = text.unescape() {
                    let value = value.trim();
                    if !value.is_empty() {
                        values.push(value.to_string());
                    }
                }
            }
            Ok(Event::CData(text)) => {
                let value = String::from_utf8_lossy(text.as_ref()).trim().to_string();
                if !value.is_empty() {
                    values.push(value);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    values
}

pub fn relationship_map(xml: &str) -> BTreeMap<String, (String, String, bool)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut relationships = BTreeMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if local_name(element.name().as_ref()) == b"Relationship" =>
            {
                let Some(id) = attr_value(&element, b"Id") else {
                    continue;
                };
                let target = attr_value(&element, b"Target").unwrap_or_default();
                let relation_type = attr_value(&element, b"Type").unwrap_or_default();
                let external = attr_value(&element, b"TargetMode")
                    .is_some_and(|mode| mode.eq_ignore_ascii_case("external"));
                relationships.insert(id, (target, relation_type, external));
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    relationships
}
