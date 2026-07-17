mod client;
mod manager;
mod types;

pub(crate) use manager::McpRuntime;
pub(crate) use types::McpToolDescriptor;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "test_support.rs"]
pub(crate) mod test_support;
