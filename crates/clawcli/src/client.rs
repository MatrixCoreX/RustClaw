use anyhow::Result;
use reqwest::blocking::Client;
use std::time::Duration;

const V1: &str = "/v1";

pub(crate) fn base_v1(base_url: &str) -> String {
    let u = base_url.trim_end_matches('/');
    format!("{u}{V1}")
}

pub(crate) fn make_client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(60))
        .build()?)
}

pub(crate) fn make_stream_client_with_timeout(timeout: Option<Duration>) -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()?)
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
