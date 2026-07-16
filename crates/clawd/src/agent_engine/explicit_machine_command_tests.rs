use std::ffi::OsStr;

use super::{backtick_command_segment, leading_command_sequence};

#[test]
fn backticks_define_an_explicit_machine_command() {
    assert_eq!(
        backtick_command_segment("please run `git status --short`"),
        Some("git status --short".to_string())
    );
}

#[test]
fn unresolved_templates_are_not_commands() {
    assert_eq!(backtick_command_segment("run `cargo test <crate>`"), None);
}

#[test]
fn ordinary_language_is_not_a_machine_command() {
    assert_eq!(
        backtick_command_segment("inspect the current repository"),
        None
    );
}

#[test]
fn leading_sequence_requires_three_resolvable_tokens() {
    let empty_path = OsStr::new("");
    assert_eq!(
        leading_command_sequence("git status cargo", Some(empty_path)),
        None
    );
}
