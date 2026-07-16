/// Reads a machine-only contract hint used by deterministic test fixtures.
/// Production requests never activate this parser.
#[cfg(test)]
pub(crate) fn value(request: &str, wanted_key: &str) -> Option<String> {
    let hint_block = request
        .split_once("[CONTRACT_TEST_HINT]")?
        .1
        .split_once("[/CONTRACT_TEST_HINT]")?
        .0;
    hint_block.lines().map(str::trim).find_map(|line| {
        let (key, value) = line.split_once('=')?;
        let value = value.trim();
        (key.trim() == wanted_key && !value.is_empty()).then(|| value.to_string())
    })
}

#[cfg(not(test))]
pub(crate) fn value(_request: &str, _wanted_key: &str) -> Option<String> {
    None
}
