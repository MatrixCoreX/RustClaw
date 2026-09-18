#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportConfig {
    #[default]
    Stdio,
    Sse,
    StreamableHttp,
}

impl McpTransportConfig {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Sse => "sse",
            Self::StreamableHttp => "streamable_http",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mcp_planner_visible_tools")]
    pub planner_visible_tools: usize,
    #[serde(default = "default_mcp_catalog_search_max_results")]
    pub catalog_search_max_results: usize,
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            planner_visible_tools: default_mcp_planner_visible_tools(),
            catalog_search_max_results: default_mcp_catalog_search_max_results(),
            servers: HashMap::new(),
        }
    }
}

impl McpConfig {
    pub fn enabled_server_names(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut names: Vec<String> = self
            .servers
            .iter()
            .filter(|(_, server)| server.enabled)
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub transport: McpTransportConfig,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub env_refs: HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub auth_token_env: Option<String>,
    #[serde(default)]
    pub oauth_client_id_env: Option<String>,
    #[serde(default)]
    pub oauth_client_secret_env: Option<String>,
    #[serde(default)]
    pub oauth_scopes: Vec<String>,
    #[serde(default)]
    pub oauth_resource: Option<String>,
    #[serde(default = "default_mcp_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_mcp_max_concurrency")]
    pub max_concurrency: usize,
    #[serde(default = "default_mcp_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default = "default_mcp_max_schema_bytes")]
    pub max_schema_bytes: usize,
    #[serde(default = "default_mcp_max_tools")]
    pub max_tools: usize,
    #[serde(default = "default_mcp_health_check_seconds")]
    pub health_check_seconds: u64,
    #[serde(default = "default_mcp_reconnect_base_seconds")]
    pub reconnect_base_seconds: u64,
    #[serde(default = "default_mcp_reconnect_max_seconds")]
    pub reconnect_max_seconds: u64,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub capability_prefix: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub tool_policies: HashMap<String, McpToolPolicyConfig>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: McpTransportConfig::default(),
            command: None,
            args: Vec::new(),
            env: HashMap::new(),
            env_refs: HashMap::new(),
            url: None,
            auth_token_env: None,
            oauth_client_id_env: None,
            oauth_client_secret_env: None,
            oauth_scopes: Vec::new(),
            oauth_resource: None,
            timeout_seconds: default_mcp_timeout_seconds(),
            max_concurrency: default_mcp_max_concurrency(),
            max_output_bytes: default_mcp_max_output_bytes(),
            max_schema_bytes: default_mcp_max_schema_bytes(),
            max_tools: default_mcp_max_tools(),
            health_check_seconds: default_mcp_health_check_seconds(),
            reconnect_base_seconds: default_mcp_reconnect_base_seconds(),
            reconnect_max_seconds: default_mcp_reconnect_max_seconds(),
            trusted: false,
            capability_prefix: None,
            allowed_tools: Vec::new(),
            tool_policies: HashMap::new(),
        }
    }
}

impl McpServerConfig {
    pub fn uses_oauth_client_credentials(&self) -> bool {
        self.oauth_client_id_env.is_some() || self.oauth_client_secret_env.is_some()
    }

    pub fn auth_mode_token(&self) -> &'static str {
        if self.uses_oauth_client_credentials() {
            "oauth_client_credentials"
        } else if self.auth_token_env.is_some() {
            "bearer_env"
        } else {
            "none"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpToolEffectConfig {
    Observe,
    #[default]
    Mutate,
    Validate,
    External,
}

impl McpToolEffectConfig {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Mutate => "mutate",
            Self::Validate => "validate",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpToolRiskConfig {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

impl McpToolRiskConfig {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpToolPolicyConfig {
    #[serde(default)]
    pub effect: McpToolEffectConfig,
    #[serde(default)]
    pub risk_level: McpToolRiskConfig,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default)]
    pub isolation_profile: Option<crate::skill_registry::CapabilityIsolationProfile>,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default)]
    pub filesystem_write: bool,
    #[serde(default)]
    pub external_publish: bool,
    #[serde(default)]
    pub credential_access: bool,
    #[serde(default)]
    pub subprocess: bool,
    #[serde(default)]
    pub package_install: bool,
    #[serde(default)]
    pub privilege_escalation: bool,
}

impl Default for McpToolPolicyConfig {
    fn default() -> Self {
        Self {
            effect: McpToolEffectConfig::Mutate,
            risk_level: McpToolRiskConfig::Unknown,
            idempotent: false,
            isolation_profile: None,
            network_access: false,
            filesystem_write: false,
            external_publish: false,
            credential_access: false,
            subprocess: false,
            package_install: false,
            privilege_escalation: false,
        }
    }
}

fn default_mcp_timeout_seconds() -> u64 {
    30
}

fn default_mcp_max_concurrency() -> usize {
    2
}

fn default_mcp_max_output_bytes() -> usize {
    256 * 1024
}

fn default_mcp_max_schema_bytes() -> usize {
    64 * 1024
}

fn default_mcp_max_tools() -> usize {
    128
}

fn default_mcp_planner_visible_tools() -> usize {
    32
}

fn default_mcp_catalog_search_max_results() -> usize {
    20
}

fn default_mcp_health_check_seconds() -> u64 {
    30
}

fn default_mcp_reconnect_base_seconds() -> u64 {
    2
}

fn default_mcp_reconnect_max_seconds() -> u64 {
    60
}
