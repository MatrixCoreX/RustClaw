use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use claw_core::config::{
    McpConfig, McpServerConfig, McpToolEffectConfig, McpToolPolicyConfig, McpToolRiskConfig,
    McpTransportConfig,
};

use super::McpRuntime;

pub(crate) fn fixture_config() -> McpConfig {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mcp_stdio_fixture.py")
        .to_string_lossy()
        .into_owned();
    let tool_policies = HashMap::from([(
        "lookup".to_string(),
        McpToolPolicyConfig {
            effect: McpToolEffectConfig::Observe,
            risk_level: McpToolRiskConfig::Low,
            idempotent: true,
            ..McpToolPolicyConfig::default()
        },
    )]);
    let server = McpServerConfig {
        enabled: true,
        transport: McpTransportConfig::Stdio,
        command: Some("python3".to_string()),
        args: vec![fixture_path],
        timeout_seconds: 1,
        max_concurrency: 2,
        max_output_bytes: 512,
        max_schema_bytes: 4096,
        max_tools: 8,
        trusted: true,
        allowed_tools: vec![
            "lookup".to_string(),
            "fail".to_string(),
            "slow".to_string(),
            "large".to_string(),
        ],
        tool_policies,
        ..McpServerConfig::default()
    };
    McpConfig {
        enabled: true,
        servers: HashMap::from([("fixture".to_string(), server)]),
        ..McpConfig::default()
    }
}

pub(crate) async fn started_fixture_runtime() -> Arc<McpRuntime> {
    let runtime = Arc::new(McpRuntime::new(fixture_config()));
    runtime.start().await;
    runtime
}
