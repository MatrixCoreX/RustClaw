pub(crate) fn strip_read_range_line_prefix(line: &str) -> &str {
    let Some((prefix, rest)) = line.split_once('|') else {
        return line;
    };
    let prefix = prefix.trim();
    if prefix.is_empty() || prefix.len() > 6 || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return line;
    }
    rest
}
