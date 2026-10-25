use quick_xml::events::{BytesText, Event};
use quick_xml::Reader;

/// Preserve a logical text node across quick-xml's separate reference events.
/// Trimming each fragment would remove spaces around `&amp;`, and consumers
/// such as named ranges must receive the whole node before changing state.
pub(crate) struct TextReader<'a> {
    reader: Reader<&'a [u8]>,
    pending: Option<quick_xml::Result<Event<'a>>>,
    trim: bool,
}

impl<'a> TextReader<'a> {
    pub(crate) fn new(xml: &'a str, trim: bool) -> Self {
        let mut reader = Reader::from_str(xml);
        // Keep malformed references in the logical node, as before 0.38;
        // decode_text rejects that entire node rather than returning a prefix.
        reader.config_mut().allow_dangling_amp = true;
        Self {
            reader,
            pending: None,
            trim,
        }
    }

    pub(crate) fn read_event(&mut self) -> quick_xml::Result<Event<'a>> {
        loop {
            let event = self
                .pending
                .take()
                .unwrap_or_else(|| self.reader.read_event())?;
            if !matches!(event, Event::Text(_) | Event::GeneralRef(_)) {
                return Ok(event);
            }
            let mut escaped = String::new();
            append_escaped(&mut escaped, event)?;
            loop {
                match self.reader.read_event() {
                    Ok(event @ (Event::Text(_) | Event::GeneralRef(_))) => {
                        append_escaped(&mut escaped, event)?;
                    }
                    other => {
                        self.pending = Some(other);
                        break;
                    }
                }
            }
            if self.trim {
                escaped = escaped.trim_matches([' ', '\t', '\r', '\n']).to_string();
                if escaped.is_empty() {
                    continue;
                }
            }
            return Ok(Event::Text(BytesText::from_escaped(escaped)));
        }
    }
}

fn append_escaped(output: &mut String, event: Event<'_>) -> quick_xml::Result<()> {
    match event {
        Event::Text(text) => output.push_str(&text.decode()?),
        Event::GeneralRef(reference) => {
            output.push('&');
            output.push_str(&reference.decode()?);
            output.push(';');
        }
        _ => unreachable!("only text and references are coalesced"),
    }
    Ok(())
}

/// Use the same predefined/numeric entity policy as BytesText::unescape did.
/// Decode once without XML EOL normalization and never expand custom entities.
pub(crate) fn decode_text(text: &BytesText<'_>) -> quick_xml::Result<String> {
    Ok(quick_xml::escape::unescape(&text.decode()?)?.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_nodes(xml: &str, trim: bool) -> quick_xml::Result<Vec<String>> {
        let mut reader = TextReader::new(xml, trim);
        let mut values = Vec::new();
        loop {
            match reader.read_event()? {
                Event::Text(text) => values.push(decode_text(&text)?),
                Event::Eof => return Ok(values),
                _ => {}
            }
        }
    }

    #[test]
    fn references_preserve_whole_text_nodes_and_surrounding_spaces() {
        let xml = "<t>  A &amp; B &#38; C &#x1F642; &lt;&gt;&quot;&apos;  </t>";
        assert_eq!(text_nodes(xml, true).unwrap(), ["A & B & C 🙂 <>\"'"]);
        assert_eq!(text_nodes(xml, false).unwrap(), ["  A & B & C 🙂 <>\"'  "]);
    }

    #[test]
    fn escaped_reference_is_decoded_once_and_eol_is_not_normalized() {
        assert_eq!(
            text_nodes("<t>&amp;lt;\r\nnext&#13;</t>", false).unwrap(),
            ["&lt;\r\nnext\r"]
        );
    }

    #[test]
    fn unknown_entities_are_not_silently_dropped_or_expanded_from_dtd() {
        assert!(text_nodes("<t>before &unknown; after</t>", false).is_err());
        assert!(text_nodes("<t>before &amp after</t>", false).is_err());
        assert!(text_nodes("<t>before & after</t>", false).is_err());
        assert!(text_nodes(
            "<!DOCTYPE t [<!ENTITY custom 'secret'>]><t>&custom;</t>",
            false
        )
        .is_err());
    }

    #[test]
    fn coalescing_keeps_markup_cdata_and_reader_errors_at_their_boundaries() {
        let mut reader = TextReader::new("<t>A &amp; B<![CDATA[ raw ]]></t>", true);
        assert!(matches!(reader.read_event().unwrap(), Event::Start(_)));
        let Event::Text(text) = reader.read_event().unwrap() else {
            panic!("text node");
        };
        assert_eq!(decode_text(&text).unwrap(), "A & B");
        assert!(matches!(reader.read_event().unwrap(), Event::CData(_)));
        assert!(matches!(reader.read_event().unwrap(), Event::End(_)));
        assert!(text_nodes("<t>A &amp; B</wrong>", false).is_err());
    }
}
