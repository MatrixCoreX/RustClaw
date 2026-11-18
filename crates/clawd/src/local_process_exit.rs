use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChildExit {
    pub(crate) code: Option<i32>,
    pub(crate) signal: Option<i32>,
}
