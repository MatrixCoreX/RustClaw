use anyhow::{Context, Result};

use crate::{client, output};

use super::common::get_v1_json;

pub(crate) fn run_skills(base_url: &str, key: &str, config: bool, json_output: bool) -> Result<()> {
    let path = if config { "/skills/config" } else { "/skills" };
    let body = get_v1_json(base_url, key, path, "skills")?;
    if json_output {
        output::print_json_pretty(&body);
    } else {
        output::print_skill_table(&body);
    }
    Ok(())
}

pub(crate) fn run_capabilities(base_url: &str, key: &str, json_output: bool) -> Result<()> {
    let body = get_v1_json(base_url, key, "/capabilities", "capabilities")?;
    if json_output {
        output::print_json_pretty(&body);
    } else {
        output::print_capability_table(&body);
    }
    Ok(())
}

pub(crate) fn run_reload_skills(base_url: &str, key: &str) -> Result<()> {
    let url = format!("{}/admin/reload-skills", client::base_v1(base_url));
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .send()
        .context("reload-skills failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse reload-skills response")?;
    output::print_json_pretty(&body);
    if !status.is_success() {
        anyhow::bail!("reload-skills returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}
