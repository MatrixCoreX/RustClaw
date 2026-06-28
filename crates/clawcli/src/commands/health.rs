use anyhow::{Context, Result};
use reqwest::blocking::Client;

use crate::{client, output};

pub(crate) fn run_health(base_url: &str, key: Option<&str>) -> Result<()> {
    let url = format!("{}/health", client::base_v1(base_url));
    let mut req = Client::new().get(&url);
    if let Some(k) = key {
        req = req.header("x-rustclaw-key", k);
    }
    let resp = req.send().context("request failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse health response")?;
    output::print_json_pretty(&body);
    if !status.is_success() {
        anyhow::bail!("health returned {}", status);
    }
    Ok(())
}
