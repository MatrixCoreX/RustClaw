/// Build the client used for exact internal core API endpoints. Platform API
/// and media-download traffic use separate clients with their own policies.
pub fn core_api_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
}

#[cfg(test)]
#[path = "channel_http_tests.rs"]
mod tests;
