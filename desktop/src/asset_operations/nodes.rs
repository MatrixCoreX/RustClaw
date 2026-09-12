use super::direct::DirectSession;
use crate::{wallet::files, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: Uuid,
    pub origin: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDocument {
    pub version: u32,
    pub nodes: Vec<Node>,
    pub selected: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger_id: Option<String>,
}
pub struct Nodes {
    path: PathBuf,
    pub document: NodeDocument,
    pub current: Option<Arc<DirectSession>>,
}
pub fn canonical_origin(raw: &str) -> Result<String> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "wallet_node_invalid")?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && local))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("wallet_node_invalid".into());
    }
    Ok(url.origin().ascii_serialization())
}
impl Nodes {
    pub fn new(path: PathBuf) -> Result<Self> {
        let document: NodeDocument = if std::fs::symlink_metadata(&path).is_ok() {
            serde_json::from_slice(&files::read_private(&path)?)
                .map_err(|_| "wallet_node_config_invalid")?
        } else {
            let origins: Vec<String> =
                serde_json::from_str(include_str!("../../config/asset-nodes.json"))
                    .map_err(|_| "wallet_node_config_invalid")?;
            let nodes: Vec<Node> = origins
                .into_iter()
                .map(|origin| Node {
                    id: Uuid::new_v4(),
                    origin,
                })
                .collect();
            let selected = nodes.first().ok_or("wallet_node_config_invalid")?.id;
            NodeDocument {
                version: 1,
                nodes,
                selected,
                ledger_id: None,
            }
        };
        if document.version != 1
            || document.nodes.is_empty()
            || document.nodes.len() > 32
            || !document.nodes.iter().any(|n| n.id == document.selected)
            || document.ledger_id.as_ref().is_some_and(|id| {
                id.is_empty() || id.len() > 128 || id.chars().any(char::is_control)
            })
        {
            return Err("wallet_node_config_invalid".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut origins = std::collections::HashSet::new();
        for node in &document.nodes {
            if node.id.is_nil()
                || !ids.insert(node.id)
                || !origins.insert(&node.origin)
                || canonical_origin(&node.origin)? != node.origin
            {
                return Err("wallet_node_config_invalid".into());
            }
        }
        let result = Self {
            path,
            document,
            current: None,
        };
        result.persist()?;
        Ok(result)
    }
    pub fn persist(&self) -> Result<()> {
        files::write(
            &self.path,
            &serde_json::to_vec(&self.document).map_err(|_| "wallet_node_config_invalid")?,
        )
    }
    pub fn add(&mut self, origin: &str) -> Result<Node> {
        let origin = canonical_origin(origin)?;
        if let Some(node) = self.document.nodes.iter().find(|n| n.origin == origin) {
            return Ok(node.clone());
        }
        if self.document.nodes.len() >= 32 {
            return Err("wallet_node_limit".into());
        }
        let node = Node {
            id: Uuid::new_v4(),
            origin,
        };
        self.document.nodes.push(node.clone());
        if let Err(e) = self.persist() {
            self.document.nodes.pop();
            return Err(e);
        }
        Ok(node)
    }
    pub fn disconnect(&mut self) {
        if let Some(session) = self.current.take() {
            session.cancelled.cancel();
        }
    }
}
