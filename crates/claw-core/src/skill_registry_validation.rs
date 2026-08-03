pub(super) fn validate_named_capability(
    name: &str,
    token: &str,
    label: &str,
) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err(format!("{label} name length must be 1..=64: `{token}`"));
    }
    if !name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    }) {
        return Err(format!("{label} name must match [a-z0-9_]: `{token}`"));
    }
    Ok(())
}
