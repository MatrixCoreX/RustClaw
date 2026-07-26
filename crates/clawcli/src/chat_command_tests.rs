use super::{command_specs, parse_chat_command, ChatCommand};
use crate::chat_session::PermissionMode;

fn parse(input: &str) -> Result<ChatCommand, super::ChatCommandError> {
    parse_chat_command(input).expect("slash command")
}

#[test]
fn parser_supports_the_complete_explicit_command_surface() {
    let tokens = command_specs()
        .iter()
        .map(|spec| spec.token)
        .collect::<Vec<_>>();
    for required in [
        "/help",
        "/new",
        "/resume",
        "/resume-task",
        "/detach",
        "/cancel",
        "/status",
        "/continue",
        "/approve",
        "/approve-scope",
        "/deny",
        "/model",
        "/compact",
        "/diff",
        "/permissions",
        "/files",
        "/attachments",
        "/file",
        "/image",
        "/goal",
        "/exit",
    ] {
        assert!(tokens.contains(&required), "missing {required}");
    }
    assert!(!tokens.contains(&"/attach"));
}

#[test]
fn parser_uses_shell_quoting_and_escaped_spaces() {
    assert_eq!(
        parse(r#"/diff "src/with space.rs" docs/中文.md"#).unwrap(),
        ChatCommand::Diff(vec![
            "src/with space.rs".to_string(),
            "docs/中文.md".to_string()
        ])
    );
    assert_eq!(
        parse(r#"/file docs/with\ space.md"#).unwrap(),
        ChatCommand::File("docs/with space.md".into())
    );
}

#[test]
fn parser_separates_machine_errors_from_backend_policy() {
    let unknown = parse("/stats").expect_err("unknown command");
    assert_eq!(unknown.error_code, "chat_command_unknown");
    assert_eq!(unknown.suggestion.as_deref(), Some("/status"));

    let missing = parse("/resume").expect_err("missing argument");
    assert_eq!(missing.error_code, "chat_command_argument_missing");

    let invalid_mode = parse("/permissions maybe").expect_err("invalid mode");
    assert_eq!(invalid_mode.error_code, "chat_permission_mode_invalid");
    assert_eq!(invalid_mode.argument.as_deref(), Some("maybe"));

    let quote = parse(r#"/file "unterminated"#).expect_err("invalid quoting");
    assert_eq!(quote.error_code, "chat_command_parse_failed");

    let removed = parse("/attach task-1").expect_err("removed ambiguous alias");
    assert_eq!(removed.error_code, "chat_command_unknown");
}

#[test]
fn parser_keeps_permissions_and_model_typed() {
    assert_eq!(
        parse("/permissions safe").unwrap(),
        ChatCommand::Permissions(Some(PermissionMode::Safe))
    );
    assert_eq!(
        parse("/permissions ask").unwrap(),
        ChatCommand::Permissions(Some(PermissionMode::Ask))
    );
    assert_eq!(
        parse("/permissions yolo").unwrap(),
        ChatCommand::Permissions(Some(PermissionMode::Yolo))
    );
    assert_eq!(
        parse("/model default").unwrap(),
        ChatCommand::Model(Some("default".to_string()))
    );
    assert_eq!(parse("/model").unwrap(), ChatCommand::Model(None));
}

#[test]
fn ordinary_and_code_span_text_are_not_commands() {
    assert!(parse_chat_command("continue").is_none());
    assert!(parse_chat_command("`/status`").is_none());
    assert!(parse_chat_command("请检查 /status").is_none());
}
