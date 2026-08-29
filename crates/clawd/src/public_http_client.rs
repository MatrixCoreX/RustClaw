use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use reqwest::Client;
use serde::Deserialize;

const PUBLIC_DNS_DOH_ENDPOINT: &str = "https://1.1.1.1/dns-query";

#[derive(Debug)]
struct PublicDnsResolver {
    bootstrap_client: Client,
}

#[derive(Debug, Deserialize)]
struct DnsOverHttpsResponse {
    #[serde(rename = "Status")]
    status: u16,
    #[serde(rename = "Answer", default)]
    answers: Vec<DnsOverHttpsAnswer>,
}

#[derive(Debug, Deserialize)]
struct DnsOverHttpsAnswer {
    #[serde(rename = "type")]
    record_type: u16,
    data: String,
}

impl PublicDnsResolver {
    fn new() -> anyhow::Result<Self> {
        let bootstrap_client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8))
            .build()?;
        Ok(Self { bootstrap_client })
    }
}

impl Resolve for PublicDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let bootstrap_client = self.bootstrap_client.clone();
        Box::pin(async move {
            if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
                return Err(public_dns_error(
                    io::ErrorKind::PermissionDenied,
                    "public endpoint DNS resolved to a non-public address",
                ));
            }
            let addresses = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?
                .collect::<Vec<_>>();
            if !addresses.is_empty() && addresses.iter().all(|address| is_public_ip(address.ip())) {
                return Ok(Box::new(addresses.into_iter()) as Addrs);
            }
            let fallback_addresses =
                resolve_public_addresses_with_doh(&bootstrap_client, &host).await?;
            if fallback_addresses.is_empty() {
                return Err(public_dns_error(
                    io::ErrorKind::NotFound,
                    "public endpoint DNS returned no addresses",
                ));
            }
            Ok(Box::new(
                fallback_addresses
                    .into_iter()
                    .map(|address| std::net::SocketAddr::new(address, 0)),
            ) as Addrs)
        })
    }
}

async fn resolve_public_addresses_with_doh(
    client: &Client,
    host: &str,
) -> Result<Vec<IpAddr>, Box<dyn std::error::Error + Send + Sync>> {
    let mut addresses = Vec::new();
    for record_type in ["A", "AAAA"] {
        let response = client
            .get(PUBLIC_DNS_DOH_ENDPOINT)
            .header(reqwest::header::ACCEPT, "application/dns-json")
            .query(&[("name", host), ("type", record_type)])
            .send()
            .await?
            .error_for_status()?
            .json::<DnsOverHttpsResponse>()
            .await?;
        addresses.extend(parse_public_doh_addresses(&response)?);
    }
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

fn parse_public_doh_addresses(
    response: &DnsOverHttpsResponse,
) -> Result<Vec<IpAddr>, Box<dyn std::error::Error + Send + Sync>> {
    if response.status != 0 {
        return Ok(Vec::new());
    }
    let addresses = response
        .answers
        .iter()
        .filter(|answer| matches!(answer.record_type, 1 | 28))
        .filter_map(|answer| answer.data.parse::<IpAddr>().ok())
        .collect::<Vec<_>>();
    if addresses.iter().any(|address| !is_public_ip(*address)) {
        return Err(public_dns_error(
            io::ErrorKind::PermissionDenied,
            "public DNS fallback returned a non-public address",
        ));
    }
    Ok(addresses)
}

fn public_dns_error(
    kind: io::ErrorKind,
    message: &'static str,
) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(io::Error::new(kind, message))
}

pub(crate) fn build_public_http_client() -> anyhow::Result<Client> {
    Ok(Client::builder()
        .dns_resolver(Arc::new(PublicDnsResolver::new()?))
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()?)
}

pub(crate) fn validate_public_https_base_url(raw: &str) -> Result<reqwest::Url, &'static str> {
    let url = reqwest::Url::parse(raw).map_err(|_| "remote_url_invalid")?;
    if url.scheme() != "https" {
        return Err("remote_url_https_required");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("remote_url_credentials_forbidden");
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("remote_url_components_forbidden");
    }
    let host = url.host_str().ok_or("remote_url_host_required")?;
    if let Ok(address) = host.parse::<IpAddr>() {
        if !is_public_ip(address) {
            return Err("remote_url_non_public_address");
        }
    }
    Ok(url)
}

pub(crate) fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !matches!(
        octets,
        [0, ..]
            | [10, ..]
            | [100, 64..=127, ..]
            | [127, ..]
            | [169, 254, ..]
            | [172, 16..=31, ..]
            | [192, 0, 0, ..]
            | [192, 0, 2, ..]
            | [192, 88, 99, ..]
            | [192, 168, ..]
            | [198, 18..=19, ..]
            | [198, 51, 100, ..]
            | [203, 0, 113, ..]
            | [224..=255, ..]
    )
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = address.segments();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_public_ipv4_and_ipv6_ranges() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "192.168.1.1",
            "100.64.0.1",
            "198.18.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_ip(address.parse().expect("valid test address")));
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn public_base_url_requires_https_and_rejects_ambiguous_components() {
        assert!(validate_public_https_base_url("https://api.example.test").is_ok());
        assert_eq!(
            validate_public_https_base_url("http://api.example.test").unwrap_err(),
            "remote_url_https_required"
        );
        assert_eq!(
            validate_public_https_base_url("https://user@api.example.test").unwrap_err(),
            "remote_url_credentials_forbidden"
        );
        assert_eq!(
            validate_public_https_base_url("https://127.0.0.1").unwrap_err(),
            "remote_url_non_public_address"
        );
        assert_eq!(
            validate_public_https_base_url("https://api.example.test?next=/admin").unwrap_err(),
            "remote_url_components_forbidden"
        );
    }

    #[tokio::test]
    async fn resolver_rejects_names_that_resolve_to_private_addresses() {
        let resolver = PublicDnsResolver::new().unwrap();
        let result = resolver
            .resolve("localhost".parse().expect("localhost is a valid DNS name"))
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn doh_parser_accepts_only_public_address_records() {
        let response = DnsOverHttpsResponse {
            status: 0,
            answers: vec![
                DnsOverHttpsAnswer {
                    record_type: 5,
                    data: "edge.example.test.".to_string(),
                },
                DnsOverHttpsAnswer {
                    record_type: 1,
                    data: "1.1.1.1".to_string(),
                },
                DnsOverHttpsAnswer {
                    record_type: 28,
                    data: "2606:4700:4700::1111".to_string(),
                },
            ],
        };
        assert_eq!(parse_public_doh_addresses(&response).unwrap().len(), 2);

        let private = DnsOverHttpsResponse {
            status: 0,
            answers: vec![DnsOverHttpsAnswer {
                record_type: 1,
                data: "198.18.0.1".to_string(),
            }],
        };
        assert!(parse_public_doh_addresses(&private).is_err());
    }
}
