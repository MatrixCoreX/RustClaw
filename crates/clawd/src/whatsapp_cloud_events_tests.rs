use super::*;

#[test]
fn meta_signature_verification_is_constant_time_and_exact() {
    let secret = "fixture-secret";
    let body = br#"{"entry":[]}"#;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac");
    mac.update(body);
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-hub-signature-256",
        signature.parse().expect("signature header"),
    );
    assert_eq!(
        verify_signature(secret, &headers, "x-hub-signature-256", body),
        Ok(())
    );
    assert_eq!(
        verify_signature("wrong", &headers, "x-hub-signature-256", body),
        Err(())
    );
}
