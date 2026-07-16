use serde_json::json;

use super::{annotate_readonly_cli_surface_args, command_looks_like_readonly_cli_surface_probe};

#[test]
fn readonly_help_probe_receives_machine_policy_fields() {
    let mut args = json!({"command": "cargo --help"});

    assert!(annotate_readonly_cli_surface_args(&mut args));
    assert_eq!(
        args.get("action").and_then(|value| value.as_str()),
        Some("inspect_cli_help")
    );
    assert_eq!(
        args.get("timeout_seconds").and_then(|value| value.as_i64()),
        Some(10)
    );
}

#[test]
fn explicit_action_is_not_overwritten() {
    let mut args = json!({"action": "run", "command": "cargo --help"});

    assert!(!annotate_readonly_cli_surface_args(&mut args));
    assert_eq!(
        args.get("action").and_then(|value| value.as_str()),
        Some("run")
    );
}

#[test]
fn mutating_shell_surface_is_not_marked_readonly() {
    assert!(!command_looks_like_readonly_cli_surface_probe(
        "sudo cargo --help"
    ));
    assert!(!command_looks_like_readonly_cli_surface_probe("rm --help"));
}
