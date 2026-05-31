const PACKAGE_MANAGERS: &[&str] = &[
    "apt-get", "apt", "dnf", "yum", "pacman", "apk", "zypper", "brew",
];

fn commandish_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|raw| {
            raw.trim_matches(|ch: char| {
                !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+' | ':' | '/'))
            })
        })
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn text_has_command_context(text: &str, tokens: &[String], manager_index: usize) -> bool {
    manager_index == 0
        || text.contains("command:")
        || text.contains('`')
        || manager_index
            .checked_sub(1)
            .and_then(|index| tokens.get(index))
            .is_some_and(|token| token == "sudo")
        || manager_index
            .checked_sub(2)
            .and_then(|index| tokens.get(index))
            .is_some_and(|token| token == "sudo")
}

fn is_safe_package_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '+' | ':'))
}

fn package_tokens_after(tokens: &[String], start: usize) -> Vec<String> {
    tokens[start..]
        .iter()
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .filter(|token| !token.starts_with('-'))
        .filter(|token| is_safe_package_token(token))
        .map(ToString::to_string)
        .collect()
}

fn find_subcommand(tokens: &[String], mut index: usize, subcommands: &[&str]) -> Option<usize> {
    while index < tokens.len() {
        let token = tokens[index].to_ascii_lowercase();
        if subcommands.iter().any(|sub| token == *sub) {
            return Some(index);
        }
        index += 1;
    }
    None
}

pub(crate) fn package_install_packages_from_commandish_text(text: &str) -> Option<Vec<String>> {
    let tokens = commandish_tokens(text);
    for (index, token) in tokens.iter().enumerate() {
        let manager = token.to_ascii_lowercase();
        if !PACKAGE_MANAGERS.iter().any(|known| manager == *known) {
            continue;
        }
        if !text_has_command_context(text, &tokens, index) {
            continue;
        }
        let package_start = match manager.as_str() {
            "apt-get" | "apt" | "dnf" | "yum" | "zypper" | "brew" => {
                find_subcommand(&tokens, index + 1, &["install"]).map(|pos| pos + 1)
            }
            "apk" => find_subcommand(&tokens, index + 1, &["add"]).map(|pos| pos + 1),
            "pacman" => find_subcommand(&tokens, index + 1, &["-s"]).map(|pos| pos + 1),
            _ => None,
        };
        let Some(package_start) = package_start else {
            continue;
        };
        let packages = package_tokens_after(&tokens, package_start);
        if !packages.is_empty() {
            return Some(packages);
        }
    }
    None
}

fn is_preview_control_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "dry-run" | "dry_run" | "dryrun" | "sudo" | "-n" | "-y"
    ) || PACKAGE_MANAGERS.iter().any(|manager| lower == *manager)
}

fn contains_dry_run_control_token(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.to_ascii_lowercase().as_str(),
            "dry-run" | "dry_run" | "dryrun"
        )
    })
}

pub(crate) fn package_install_packages_from_preview_text(text: &str) -> Option<Vec<String>> {
    if let Some(packages) = package_install_packages_from_commandish_text(text) {
        return Some(packages);
    }
    let tokens = commandish_tokens(text);
    if !contains_dry_run_control_token(&tokens) {
        return None;
    }
    let packages = tokens
        .iter()
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .filter(|token| !is_preview_control_token(token))
        .filter(|token| is_safe_package_token(token))
        .filter(|token| token.chars().any(|ch| ch.is_ascii_lowercase()))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    (packages.len() == 1).then_some(packages)
}

#[cfg(test)]
#[path = "package_commands_tests.rs"]
mod tests;
