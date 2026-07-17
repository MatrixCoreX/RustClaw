use super::*;

#[test]
fn clawcli_exposes_coding_workflow_command_surface() {
    let cmd = Cli::command();
    let command_names = cmd
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect::<std::collections::BTreeSet<_>>();

    for required in [
        "submit",
        "exec",
        "code",
        "goal",
        "session",
        "get",
        "watch",
        "events",
        "report",
        "review",
        "continue",
        "resume-task",
        "cancel-task",
        "permission",
        "subagents",
        "llm-trace",
        "replay",
        "wait",
    ] {
        assert!(command_names.contains(required), "missing {required}");
    }

    let permission = cmd
        .find_subcommand("permission")
        .expect("permission command");
    let permission_names = permission
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    for required in ["inspect", "explain", "capability"] {
        assert!(permission_names.contains(required), "missing {required}");
    }

    let goal = cmd.find_subcommand("goal").expect("goal command");
    let goal_names = goal
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    for required in ["start", "status", "pause", "resume", "edit", "clear"] {
        assert!(goal_names.contains(required), "missing {required}");
    }

    let code = cmd.find_subcommand("code").expect("code command");
    let code_names = code
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    for required in ["run", "status", "review", "continue", "diff", "rewind"] {
        assert!(code_names.contains(required), "missing {required}");
    }

    let session = cmd.find_subcommand("session").expect("session command");
    let session_names = session
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    for required in [
        "list",
        "show",
        "resume",
        "continue-latest",
        "archive",
        "delete",
        "fork",
    ] {
        assert!(session_names.contains(required), "missing {required}");
    }

    let replay = cmd.find_subcommand("replay").expect("replay command");
    let replay_names = replay
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    for required in ["export", "run", "diff"] {
        assert!(replay_names.contains(required), "missing {required}");
    }
}

#[test]
fn clawcli_parses_persisted_chat_thread_options() {
    match Cli::try_parse_from(["clawcli", "chat", "--thread-id", "thread_01", "--jsonl"])
        .expect("parse chat thread options")
        .cmd
    {
        Some(Command::Chat {
            new_thread,
            thread_id,
            jsonl,
        }) => {
            assert!(!new_thread);
            assert_eq!(thread_id.as_deref(), Some("thread_01"));
            assert!(jsonl);
        }
        _ => panic!("expected chat command"),
    }

    assert!(matches!(
        Cli::try_parse_from(["clawcli", "chat", "--new"])
            .expect("parse new chat thread")
            .cmd,
        Some(Command::Chat {
            new_thread: true,
            thread_id: None,
            jsonl: false,
        })
    ));
    assert!(Cli::try_parse_from(["clawcli", "chat", "--new", "--thread-id", "thread_01"]).is_err());
}

#[test]
fn clawcli_parses_code_subcommands_and_prompt_fallback() {
    match Cli::try_parse_from(["clawcli", "code", "status", "task-1"])
        .expect("parse status")
        .cmd
    {
        Some(Command::Code {
            command: CodeCommand::Status { task_id, .. },
        }) => assert_eq!(task_id, "task-1"),
        _ => panic!("expected code status"),
    }

    match Cli::try_parse_from(["clawcli", "code", "review", "task-1", "--json"])
        .expect("parse review")
        .cmd
    {
        Some(Command::Code {
            command: CodeCommand::Review { task_id, json, .. },
        }) => {
            assert_eq!(task_id, "task-1");
            assert!(json);
        }
        _ => panic!("expected code review"),
    }

    match Cli::try_parse_from(["clawcli", "code", "continue", "task-1", "next", "step"])
        .expect("parse continue")
        .cmd
    {
        Some(Command::Code {
            command: CodeCommand::Continue {
                task_id, message, ..
            },
        }) => {
            assert_eq!(task_id, "task-1");
            assert_eq!(message, vec!["next".to_string(), "step".to_string()]);
        }
        _ => panic!("expected code continue"),
    }

    match Cli::try_parse_from(["clawcli", "code", "run", "fix", "the", "test"])
        .expect("parse run")
        .cmd
    {
        Some(Command::Code {
            command: CodeCommand::Run { prompt, .. },
        }) => assert_eq!(
            prompt,
            vec!["fix".to_string(), "the".to_string(), "test".to_string()]
        ),
        _ => panic!("expected code run"),
    }

    match Cli::try_parse_from([
        "clawcli",
        "code",
        "diff",
        "--checkpoint-id",
        "checkpoint-1",
        "--path",
        "src/lib.rs",
        "--jsonl",
    ])
    .expect("parse workspace diff")
    .cmd
    {
        Some(Command::Code {
            command:
                CodeCommand::Diff {
                    checkpoint_id,
                    paths,
                    jsonl,
                    ..
                },
        }) => {
            assert_eq!(checkpoint_id.as_deref(), Some("checkpoint-1"));
            assert_eq!(paths, vec!["src/lib.rs".to_string()]);
            assert!(jsonl);
        }
        _ => panic!("expected code diff"),
    }

    match Cli::try_parse_from([
        "clawcli",
        "code",
        "rewind",
        "--checkpoint-id",
        "checkpoint-2",
        "--json",
    ])
    .expect("parse workspace rewind")
    .cmd
    {
        Some(Command::Code {
            command:
                CodeCommand::Rewind {
                    checkpoint_id,
                    json,
                    ..
                },
        }) => {
            assert_eq!(checkpoint_id, "checkpoint-2");
            assert!(json);
        }
        _ => panic!("expected code rewind"),
    }

    match Cli::try_parse_from(["clawcli", "code", "fix", "the", "test"])
        .expect("parse prompt fallback")
        .cmd
    {
        Some(Command::Code {
            command: CodeCommand::Prompt(prompt),
        }) => assert_eq!(
            prompt,
            vec!["fix".to_string(), "the".to_string(), "test".to_string()]
        ),
        _ => panic!("expected prompt fallback"),
    }
}

#[test]
fn clawcli_parses_session_subcommands() {
    match Cli::try_parse_from([
        "clawcli",
        "session",
        "list",
        "--user-id",
        "7",
        "--chat-id",
        "9",
        "--json",
    ])
    .expect("parse session list")
    .cmd
    {
        Some(Command::Session {
            command:
                SessionCommand::List {
                    user_id,
                    chat_id,
                    json,
                },
        }) => {
            assert_eq!(user_id, 7);
            assert_eq!(chat_id, 9);
            assert!(json);
        }
        _ => panic!("expected session list"),
    }

    match Cli::try_parse_from(["clawcli", "session", "show", "task-1", "--json"])
        .expect("parse session show")
        .cmd
    {
        Some(Command::Session {
            command: SessionCommand::Show { session_id, json },
        }) => {
            assert_eq!(session_id, "task-1");
            assert!(json);
        }
        _ => panic!("expected session show"),
    }

    match Cli::try_parse_from([
        "clawcli", "session", "resume", "task-1", "continue", "work", "--json",
    ])
    .expect("parse session resume")
    .cmd
    {
        Some(Command::Session {
            command:
                SessionCommand::Resume {
                    session_id,
                    message,
                    json,
                },
        }) => {
            assert_eq!(session_id, "task-1");
            assert_eq!(message, vec!["continue".to_string(), "work".to_string()]);
            assert!(json);
        }
        _ => panic!("expected session resume"),
    }

    match Cli::try_parse_from([
        "clawcli",
        "session",
        "continue-latest",
        "continue",
        "work",
        "--json",
    ])
    .expect("parse latest session continuation")
    .cmd
    {
        Some(Command::Session {
            command: SessionCommand::ContinueLatest { message, json },
        }) => {
            assert_eq!(message, vec!["continue".to_string(), "work".to_string()]);
            assert!(json);
        }
        _ => panic!("expected latest session continuation"),
    }

    match Cli::try_parse_from(["clawcli", "session", "archive", "task-1", "--json"])
        .expect("parse session archive")
        .cmd
    {
        Some(Command::Session {
            command: SessionCommand::Archive { session_id, json },
        }) => {
            assert_eq!(session_id, "task-1");
            assert!(json);
        }
        _ => panic!("expected session archive"),
    }

    match Cli::try_parse_from(["clawcli", "session", "delete", "task-1", "--json"])
        .expect("parse session delete")
        .cmd
    {
        Some(Command::Session {
            command: SessionCommand::Delete { session_id, json },
        }) => {
            assert_eq!(session_id, "task-1");
            assert!(json);
        }
        _ => panic!("expected session delete"),
    }

    match Cli::try_parse_from(["clawcli", "session", "fork", "task-1", "task-2", "--json"])
        .expect("parse session fork")
        .cmd
    {
        Some(Command::Session {
            command:
                SessionCommand::Fork {
                    session_id,
                    new_session_id,
                    json,
                },
        }) => {
            assert_eq!(session_id, "task-1");
            assert_eq!(new_session_id, "task-2");
            assert!(json);
        }
        _ => panic!("expected session fork"),
    }
}

#[test]
fn clawcli_parses_closed_approval_decision_protocol() {
    match Cli::try_parse_from([
        "clawcli",
        "resume-task",
        "task-1",
        "--approval-request-id",
        "approval-1",
        "--approval-decision",
        "always-for-scope",
    ])
    .expect("parse scoped approval")
    .cmd
    {
        Some(Command::ResumeTask {
            approval_request_id,
            approval_decision,
            ..
        }) => {
            assert_eq!(approval_request_id.as_deref(), Some("approval-1"));
            assert!(matches!(
                approval_decision,
                Some(task::ApprovalDecisionArg::AlwaysForScope)
            ));
        }
        _ => panic!("expected scoped resume-task approval decision"),
    }

    match Cli::try_parse_from([
        "clawcli",
        "resume-task",
        "task-1",
        "--approval-request-id",
        "approval-1",
        "--approval-decision",
        "deny",
    ])
    .expect("parse approval denial")
    .cmd
    {
        Some(Command::ResumeTask {
            approval_request_id,
            approval_decision,
            ..
        }) => {
            assert_eq!(approval_request_id.as_deref(), Some("approval-1"));
            assert!(matches!(
                approval_decision,
                Some(task::ApprovalDecisionArg::Deny)
            ));
        }
        _ => panic!("expected resume-task approval decision"),
    }

    assert!(Cli::try_parse_from([
        "clawcli",
        "resume-task",
        "task-1",
        "--approval-request-id",
        "approval-1",
        "--approval-decision",
        "approve",
    ])
    .is_err());
    assert!(Cli::try_parse_from([
        "clawcli",
        "resume-task",
        "task-1",
        "--approve",
        "--approval-request-id",
        "approval-1",
    ])
    .is_err());
}
