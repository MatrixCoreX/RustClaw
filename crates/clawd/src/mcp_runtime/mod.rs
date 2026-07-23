mod client;
mod manager;
mod types;

pub(crate) use manager::{McpRuntime, MCP_CATALOG_SEARCH_CAPABILITY};
pub(crate) use types::{McpLifecycleSnapshot, McpProbeOutcome, McpToolDescriptor};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_loop_tests.rs"]
mod agent_loop_tests;

#[cfg(test)]
#[path = "test_support.rs"]
pub(crate) mod test_support;
