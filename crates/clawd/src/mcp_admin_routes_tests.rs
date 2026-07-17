use std::collections::BTreeMap;

use super::{
    config_view, render_mcp_config_update, write_mcp_config, McpServerUpdate,
    UpdateMcpConfigRequest,
};

fn configured_server(server_id: &str) -> McpServerUpdate {
    McpServerUpdate {
        server_id: server_id.to_string(),
        enabled: true,
        trusted: true,
        transport: "stdio".to_string(),
        command: Some("/bin/echo".to_string()),
        args: vec!["fixture".to_string()],
        env_refs: BTreeMap::from([("CHILD_TOKEN".to_string(), "RUSTCLAW_MCP_TOKEN".to_string())]),
        allowed_tools: vec!["lookup".to_string()],
        ..McpServerUpdate::default()
    }
}

#[test]
fn config_update_preserves_unmanaged_fields_and_redacts_static_environment() {
    let raw = format!(
        "{}\n# mcp-preserve-marker\n[mcp]\nenabled = false\n\
         [mcp.servers.fixture]\nenabled = false\ntrusted = true\ntransport = \"stdio\"\n\
         command = \"/bin/false\"\nallowed_tools = [\"lookup\"]\n\
         [mcp.servers.fixture.env]\nPRIVATE_LITERAL = \"must-not-reach-api\"\n\
         [mcp.servers.fixture.tool_policies.lookup]\neffect = \"observe\"\nrisk_level = \"low\"\nidempotent = true\n",
        include_str!("../../../configs/config.toml")
    );
    let request = UpdateMcpConfigRequest {
        enabled: true,
        servers: vec![configured_server("fixture")],
    };

    let (updated, parsed) = render_mcp_config_update(&raw, request).expect("render MCP config");

    assert!(updated.contains("# mcp-preserve-marker"));
    assert!(updated.contains("PRIVATE_LITERAL = \"must-not-reach-api\""));
    assert!(updated.contains("[mcp.servers.fixture.tool_policies.lookup]"));
    assert_eq!(parsed.mcp.enabled_server_names(), vec!["fixture"]);
    let view =
        serde_json::to_string(&config_view(&parsed.mcp, false)).expect("serialize config view");
    assert!(!view.contains("must-not-reach-api"));
    assert!(!view.contains("PRIVATE_LITERAL"));
    assert!(view.contains("RUSTCLAW_MCP_TOKEN"));
    assert!(view.contains("\"has_static_env\":true"));
    assert!(view.contains("\"has_advanced_policy\":true"));
}

#[test]
fn config_update_rejects_literal_secret_reference_and_duplicate_server_ids() {
    let raw = include_str!("../../../configs/config.toml");
    let mut invalid = configured_server("fixture");
    invalid.env_refs =
        BTreeMap::from([("CHILD_TOKEN".to_string(), "literal-token-value".to_string())]);
    assert_eq!(
        render_mcp_config_update(
            raw,
            UpdateMcpConfigRequest {
                enabled: true,
                servers: vec![invalid],
            }
        )
        .expect_err("literal value must fail"),
        "mcp_stdio_env_ref_invalid"
    );

    assert_eq!(
        render_mcp_config_update(
            raw,
            UpdateMcpConfigRequest {
                enabled: false,
                servers: vec![configured_server("same"), configured_server("same")],
            }
        )
        .expect_err("duplicate ids must fail"),
        "mcp_server_id_duplicate"
    );
}

#[test]
fn config_writer_preserves_distinct_workspace_and_mounted_content() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-mcp-config-write-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");

    write_mcp_config(
        &root,
        "active_marker = true\n[mcp]\nenabled = false\n",
        "mounted_marker = true\n[mcp]\nenabled = true\n",
    )
    .expect("write MCP config copies");

    assert_eq!(
        std::fs::read_to_string(root.join("configs/config.toml")).expect("active config"),
        "active_marker = true\n[mcp]\nenabled = false\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("docker/config/config.toml")).expect("mounted config"),
        "mounted_marker = true\n[mcp]\nenabled = true\n"
    );
    assert!(std::fs::read_dir(root.join("configs"))
        .expect("config dir")
        .all(|entry| !entry
            .expect("dir entry")
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    std::fs::remove_dir_all(root).expect("remove temp root");
}
