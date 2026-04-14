use std::fs;

pub(crate) fn read_toml_text(path: &str) -> std::io::Result<String> {
    fs::read_to_string(path)
}
