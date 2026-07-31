use super::*;

#[test]
fn polling_is_the_only_default_transport_and_never_reads_webhook_secret() {
    let transport = resolve_telegram_update_transport(
        "polling",
        "127.0.0.1:8090",
        "",
        "TELEGRAM_WEBHOOK_SECRET",
        &|_| panic!("polling must not read webhook secret"),
    )
    .expect("polling transport");
    assert!(matches!(transport, TelegramUpdateTransport::Polling));
}

#[test]
fn webhook_requires_https_loopback_listener_and_environment_secret() {
    let transport = resolve_telegram_update_transport(
        "webhook",
        "127.0.0.1:8090",
        "https://example.test/telegram/update",
        "TELEGRAM_WEBHOOK_SECRET",
        &|name| (name == "TELEGRAM_WEBHOOK_SECRET").then(|| "secret_123".to_string()),
    )
    .expect("webhook transport");
    let TelegramUpdateTransport::Webhook(webhook) = transport else {
        panic!("expected webhook")
    };
    assert!(webhook.listen.ip().is_loopback());
    assert_eq!(webhook.public_url.scheme(), "https");
    assert_eq!(webhook.secret_token, "secret_123");
}

#[test]
fn webhook_fails_closed_for_missing_or_invalid_secret() {
    let missing = resolve_telegram_update_transport(
        "webhook",
        "127.0.0.1:8090",
        "https://example.test/update",
        "TELEGRAM_WEBHOOK_SECRET",
        &|_| None,
    )
    .err()
    .expect("missing secret must fail");
    assert!(missing
        .to_string()
        .contains("telegram_webhook_secret_environment_missing"));

    let invalid = resolve_telegram_update_transport(
        "webhook",
        "127.0.0.1:8090",
        "https://example.test/update",
        "TELEGRAM_WEBHOOK_SECRET",
        &|_| Some("contains whitespace".to_string()),
    )
    .err()
    .expect("invalid secret must fail");
    assert!(invalid
        .to_string()
        .contains("telegram_webhook_secret_invalid"));

    let invalid_env = resolve_telegram_update_transport(
        "webhook",
        "127.0.0.1:8090",
        "https://example.test/update",
        "1TELEGRAM_SECRET",
        &|_| Some("secret_123".to_string()),
    )
    .err()
    .expect("invalid environment selector must fail");
    assert!(invalid_env
        .to_string()
        .contains("telegram_webhook_secret_env_invalid"));
}

#[test]
fn webhook_rejects_external_listener_insecure_url_and_ambiguous_mode() {
    let secret = |_: &str| Some("secret_123".to_string());
    for (mode, listen, url, expected) in [
        (
            "webhook",
            "0.0.0.0:8090",
            "https://example.test/update",
            "telegram_webhook_listen_must_be_loopback",
        ),
        (
            "webhook",
            "127.0.0.1:8090",
            "http://example.test/update",
            "telegram_webhook_public_url_invalid",
        ),
        (
            "polling,webhook",
            "127.0.0.1:8090",
            "https://example.test/update",
            "telegram_update_mode_invalid",
        ),
    ] {
        let error = resolve_telegram_update_transport(
            mode,
            listen,
            url,
            "TELEGRAM_WEBHOOK_SECRET",
            &secret,
        )
        .err()
        .expect("invalid transport must fail");
        assert!(error.to_string().contains(expected));
    }
}
