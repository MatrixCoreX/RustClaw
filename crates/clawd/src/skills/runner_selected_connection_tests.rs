use super::*;
use claw_core::llm_connection_environment::{
    ConnectionEnvironment, SELECTED_IDENTITY_ENV, SELECTED_RELAY_ENV,
};
use std::collections::BTreeMap;

fn command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("unused-test-command");
    for name in SELECTED_IDENTITY_ENV {
        command.env(name, "untrusted-parent-value");
    }
    command
}

fn environment(command: &tokio::process::Command) -> BTreeMap<String, Option<String>> {
    command
        .as_std()
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_str().unwrap().to_string(),
                value.map(|v| v.to_str().unwrap().to_string()),
            )
        })
        .collect()
}

fn connection(hosted_relay: bool) -> crate::llm_gateway::SelectedLlmConnection {
    crate::llm_gateway::SelectedLlmConnection {
        vendor: "custom".into(),
        provider_type: "openai_compat".into(),
        base_url: "https://selected.example/v1".into(),
        model: "configured-model".into(),
        api_key: "never-export-this-literal".into(),
        hosted_relay,
    }
}

#[test]
fn selected_identity_comes_from_the_current_connection_not_parent_environment() {
    for hosted_relay in [false, true] {
        let mut command = command();
        let selected = connection(hosted_relay);
        apply_selected_connection_environment(&mut command, Some(&selected));
        let values = environment(&command);
        let identity =
            ConnectionEnvironment::from_env_with(|name| values.get(name).cloned().flatten());
        assert_eq!(identity.is_enrolled_relay(), hosted_relay);
        assert!(identity.permits_vendor_credential("custom"));
        assert_eq!(identity.permits_vendor_credential("minimax"), hosted_relay);
        assert_eq!(
            values.get(SELECTED_RELAY_ENV).cloned().flatten(),
            Some(hosted_relay.to_string())
        );
        assert_eq!(
            values.get("OPENAI_BASE_URL").cloned().flatten(),
            Some(selected.base_url)
        );
        assert_eq!(
            values.get("OPENAI_MODEL").cloned().flatten(),
            Some(selected.model)
        );
        assert!(!values.keys().any(|name| name.ends_with("_API_KEY")));
        assert!(!format!("{values:?}").contains("never-export-this-literal"));
    }
}

#[test]
fn no_selected_connection_removes_all_inherited_identity_fields() {
    let mut command = command();
    apply_selected_connection_environment(&mut command, None);
    let values = environment(&command);
    for name in SELECTED_IDENTITY_ENV {
        assert_eq!(values.get(name), Some(&None));
    }
    assert_eq!(
        ConnectionEnvironment::from_env_with(|name| values.get(name).cloned().flatten()),
        ConnectionEnvironment::Standalone
    );
}
