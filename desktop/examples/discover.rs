//! Explicit LAN smoke: `cargo run --no-default-features --example discover -- --scan`.
#[tokio::main]
async fn main() {
    let scan = std::env::args().any(|arg| arg == "--scan");
    let report =
        agent_desktop::discovery::discover(scan, tokio_util::sync::CancellationToken::new()).await;
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
