const NNI_REMOTE_API_PREFIX: &str = "/v1/nni/server";
const NNI_REMOTE_API_TIMEOUT_SECONDS: u64 = 45;

fn nni_remote_api_endpoint(node_url: &str, route: &str) -> String {
    let node_url = node_url.trim_end_matches('/');
    let route = route.strip_prefix('/').unwrap_or(route);
    format!("{node_url}{NNI_REMOTE_API_PREFIX}/{route}")
}

fn nni_remote_api_timeout() -> Duration {
    Duration::from_secs(NNI_REMOTE_API_TIMEOUT_SECONDS)
}

fn nni_selected_remote_node(config: &NniConfigResponse) -> Option<&String> {
    config.selected_node_url.as_ref()
}

fn nni_selected_remote_nodes(
    config: &NniConfigResponse,
) -> std::option::Iter<'_, String> {
    config.selected_node_url.iter()
}

#[cfg(test)]
mod nni_remote_api_tests {
    use super::*;

    #[test]
    fn endpoint_uses_one_versioned_api_prefix() {
        assert_eq!(
            nni_remote_api_endpoint("https://node.example/", "/bancor/market"),
            "https://node.example/v1/nni/server/bancor/market"
        );
    }

    #[test]
    fn remote_timeout_allows_slow_nodes_without_becoming_unbounded() {
        assert_eq!(nni_remote_api_timeout(), Duration::from_secs(45));
    }
}
