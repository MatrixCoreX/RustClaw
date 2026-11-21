use serde::{Deserialize, Serialize};
use std::fmt;

/// A bounded host task identifier. Existence and access remain repository checks.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct TaskReadId(String);

impl TryFrom<String> for TaskReadId {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if let Ok(id) = uuid::Uuid::parse_str(&value) {
            return Ok(Self(id.to_string()));
        }
        let (parent, child) = value.split_once(":child:").ok_or("task_id_invalid")?;
        let parent = uuid::Uuid::parse_str(parent).map_err(|_| "task_id_invalid")?;
        let (scope, nonce) = child.rsplit_once(':').ok_or("task_id_invalid")?;
        if value.len() > 1024
            || scope.is_empty()
            || nonce.len() != 32
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !scope
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.:-".contains(&b))
        {
            return Err("task_id_invalid");
        }
        Ok(Self(format!("{parent}:child:{child}")))
    }
}

impl fmt::Display for TaskReadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
#[path = "task_read_id_tests.rs"]
mod tests;

impl From<TaskReadId> for String {
    fn from(value: TaskReadId) -> Self {
        value.0
    }
}
