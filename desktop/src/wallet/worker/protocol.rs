use crate::{
    asset_operations::protocol::{Capabilities, Intent},
    Result,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
use zeroize::Zeroize;

pub const MAX_FRAME: usize = 131_072;
#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Open {
        directory: PathBuf,
    },
    Status {},
    Lock {},
    Account {
        id: Uuid,
    },
    Initialize {
        password: String,
    },
    Unlock {
        password: String,
    },
    Create {
        name: String,
    },
    Backup {
        id: Uuid,
        vault_password: String,
        password: String,
        path: PathBuf,
    },
    Restore {
        password: String,
        path: PathBuf,
        name: String,
    },
    Sign {
        id: Uuid,
        password: String,
        payload: String,
        cap: Capabilities,
        intent: Intent,
    },
}
impl Drop for Request {
    fn drop(&mut self) {
        match self {
            Self::Initialize { password }
            | Self::Unlock { password }
            | Self::Restore { password, .. }
            | Self::Sign { password, .. } => password.zeroize(),
            Self::Backup {
                vault_password,
                password,
                ..
            } => {
                vault_password.zeroize();
                password.zeroize();
            }
            _ => {}
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub version: u32,
    pub id: u64,
    pub request: Request,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub version: u32,
    pub id: u64,
    pub result: Result<serde_json::Value>,
}
