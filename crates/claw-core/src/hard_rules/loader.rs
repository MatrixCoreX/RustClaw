use std::fs;

pub(crate) fn read_toml_text(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}
