use k256::ecdsa::{signature::Signer, Signature, SigningKey};
use k256::elliptic_curve::rand_core::OsRng;
use k256::PublicKey;
use ripemd::Ripemd160;
use zeroize::{Zeroize, Zeroizing};

const NNI_OWNER_KEY_SUFFIX: &[u8] = b"K1";
const NNI_OWNER_CHECKSUM_BYTES: usize = 4;
const NNI_OWNER_PUBLIC_KEY_BYTES: usize = 33;
const NNI_OWNER_PRIVATE_KEY_BYTES: usize = 32;

#[derive(Serialize)]
struct NniOwnerKeyPairResponse {
    key_type: &'static str,
    encoding: &'static str,
    public_key: String,
    private_key: String,
    private_key_persisted: bool,
}

fn nni_owner_checksum(payload: &[u8]) -> [u8; NNI_OWNER_CHECKSUM_BYTES] {
    let mut digest = <Ripemd160 as ripemd::Digest>::new();
    ripemd::Digest::update(&mut digest, payload);
    ripemd::Digest::update(&mut digest, NNI_OWNER_KEY_SUFFIX);
    let result = ripemd::Digest::finalize(digest);
    let mut checksum = [0_u8; NNI_OWNER_CHECKSUM_BYTES];
    checksum.copy_from_slice(&result[..NNI_OWNER_CHECKSUM_BYTES]);
    checksum
}

fn encode_nni_owner_payload(payload: &[u8]) -> String {
    let mut encoded = Vec::with_capacity(payload.len() + NNI_OWNER_CHECKSUM_BYTES);
    encoded.extend_from_slice(payload);
    encoded.extend_from_slice(&nni_owner_checksum(payload));
    bs58::encode(encoded).into_string()
}

fn decode_nni_owner_payload(
    value: &str,
    expected_payload_bytes: usize,
    error: &'static str,
) -> Result<Vec<u8>, &'static str> {
    let normalized = value.trim();
    if normalized.is_empty()
        || normalized.starts_with("EOS")
        || normalized.starts_with("PUB_")
        || normalized.starts_with("PVT_")
    {
        return Err(error);
    }
    let decoded = bs58::decode(normalized).into_vec().map_err(|_| error)?;
    if decoded.len() != expected_payload_bytes + NNI_OWNER_CHECKSUM_BYTES {
        return Err(error);
    }
    let (payload, checksum) = decoded.split_at(expected_payload_bytes);
    if checksum != nni_owner_checksum(payload) || encode_nni_owner_payload(payload) != normalized {
        return Err(error);
    }
    Ok(payload.to_vec())
}

fn nni_owner_public_key_from_signing_key(signing_key: &SigningKey) -> String {
    encode_nni_owner_payload(signing_key.verifying_key().to_encoded_point(true).as_bytes())
}

fn generate_nni_owner_key_pair() -> NniOwnerKeyPairResponse {
    let signing_key = SigningKey::random(&mut OsRng);
    NniOwnerKeyPairResponse {
        key_type: "K1",
        encoding: "eos_base58_v1",
        public_key: nni_owner_public_key_from_signing_key(&signing_key),
        private_key: encode_nni_owner_payload(signing_key.to_bytes().as_slice()),
        private_key_persisted: false,
    }
}

fn normalize_nni_owner_public_key(value: &str) -> Result<String, &'static str> {
    let payload = decode_nni_owner_payload(
        value,
        NNI_OWNER_PUBLIC_KEY_BYTES,
        "nni_owner_pubkey_invalid",
    )?;
    PublicKey::from_sec1_bytes(&payload).map_err(|_| "nni_owner_pubkey_invalid")?;
    Ok(encode_nni_owner_payload(&payload))
}

fn nni_owner_public_key_from_private(value: &str) -> Result<String, &'static str> {
    let private_bytes = Zeroizing::new(decode_nni_owner_payload(
        value,
        NNI_OWNER_PRIVATE_KEY_BYTES,
        "nni_owner_private_key_invalid",
    )?);
    let signing_key = SigningKey::from_slice(&private_bytes)
        .map_err(|_| "nni_owner_private_key_invalid")?;
    Ok(nni_owner_public_key_from_signing_key(&signing_key))
}

fn sign_nni_owner_payload(
    owner_private_key: &mut String,
    payload: &str,
) -> Result<(String, String), &'static str> {
    let result = (|| {
        let private_bytes = Zeroizing::new(decode_nni_owner_payload(
            owner_private_key,
            NNI_OWNER_PRIVATE_KEY_BYTES,
            "nni_owner_private_key_invalid",
        )?);
        let signing_key = SigningKey::from_slice(&private_bytes)
            .map_err(|_| "nni_owner_private_key_invalid")?;
        let public_key = nni_owner_public_key_from_signing_key(&signing_key);
        let signature: Signature = signing_key.sign(payload.as_bytes());
        Ok((public_key, hex::encode(signature.to_bytes())))
    })();
    owner_private_key.zeroize();
    result
}

async fn nni_owner_generate(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<NniOwnerKeyPairResponse>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(generate_nni_owner_key_pair()),
            error: None,
        }),
    )
}

#[cfg(test)]
mod nni_owner_identity_tests {
    use super::*;

    #[test]
    fn owner_keys_use_prefixless_eos_k1_encoding() {
        let signing_key = SigningKey::from_slice(&{
            let mut scalar = [0_u8; 32];
            scalar[31] = 1;
            scalar
        })
        .expect("valid scalar");
        let public_key = nni_owner_public_key_from_signing_key(&signing_key);
        assert_eq!(
            public_key,
            "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M"
        );
        assert_eq!(normalize_nni_owner_public_key(&public_key), Ok(public_key));
    }

    #[test]
    fn private_key_is_cleared_after_signing() {
        let generated = generate_nni_owner_key_pair();
        let mut private_key = generated.private_key;
        let (public_key, signature) =
            sign_nni_owner_payload(&mut private_key, "canonical-payload").expect("sign");
        assert_eq!(public_key, generated.public_key);
        assert_eq!(signature.len(), 128);
        assert!(private_key.chars().all(|character| character == '\0'));
    }

    #[test]
    fn prefixed_owner_keys_are_rejected() {
        let generated = generate_nni_owner_key_pair();
        assert_eq!(
            normalize_nni_owner_public_key(&format!("PUB_K1_{}", generated.public_key)),
            Err("nni_owner_pubkey_invalid")
        );
    }
}
