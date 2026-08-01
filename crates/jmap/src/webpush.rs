use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use irixmail_core::{Error, Result};
use irixmail_store::{Store, Subspace};
use ring::rand::SecureRandom;
use ring::{aead, agreement, hkdf, rand, signature};
use serde::{Deserialize, Serialize};

const TAG_VAPID_KEY: u8 = 0x35;
const JWT_LIFETIME_SECS: u64 = 12 * 3600;

pub struct VapidKeys {
    pkcs8: Vec<u8>,
    public_key: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct StoredVapid {
    pkcs8: String,
    public: String,
}

impl VapidKeys {
    pub fn public_key_b64(&self) -> String {
        URL_SAFE_NO_PAD.encode(&self.public_key)
    }
}

pub fn load_or_create_vapid(store: &dyn Store) -> Result<VapidKeys> {
    let key = vapid_key();
    if let Some(bytes) = store.get(&key)? {
        let stored: StoredVapid = serde_json::from_slice(&bytes)
            .map_err(|err| Error::store(format!("vapid key record: {err}")))?;
        let pkcs8 = URL_SAFE_NO_PAD
            .decode(&stored.pkcs8)
            .map_err(|err| Error::store(format!("vapid key record: {err}")))?;
        let public_key = URL_SAFE_NO_PAD
            .decode(&stored.public)
            .map_err(|err| Error::store(format!("vapid key record: {err}")))?;
        return Ok(VapidKeys { pkcs8, public_key });
    }
    let rng = rand::SystemRandom::new();
    let pkcs8 =
        signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| Error::store("vapid key generation failed"))?;
    let pair = signature::EcdsaKeyPair::from_pkcs8(
        &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        pkcs8.as_ref(),
        &rng,
    )
    .map_err(|_| Error::store("vapid key generation failed"))?;
    let keys = VapidKeys {
        pkcs8: pkcs8.as_ref().to_vec(),
        public_key: signature::KeyPair::public_key(&pair).as_ref().to_vec(),
    };
    let stored = StoredVapid {
        pkcs8: URL_SAFE_NO_PAD.encode(&keys.pkcs8),
        public: URL_SAFE_NO_PAD.encode(&keys.public_key),
    };
    let bytes = serde_json::to_vec(&stored)
        .map_err(|err| Error::store(format!("vapid key record: {err}")))?;
    store.put(&key, &bytes)?;
    Ok(keys)
}

pub fn application_server_key(store: &dyn Store) -> Result<String> {
    Ok(load_or_create_vapid(store)?.public_key_b64())
}

pub fn vapid_authorization(
    keys: &VapidKeys,
    endpoint: &str,
    subject: &str,
    now: u64,
) -> Result<String> {
    let origin = endpoint_origin(endpoint)
        .ok_or_else(|| Error::invalid_input("push endpoint is not an absolute url"))?;
    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::json!({
            "aud": origin,
            "exp": now + JWT_LIFETIME_SECS,
            "sub": subject,
        })
        .to_string(),
    );
    let signing_input = format!("{header}.{claims}");
    let rng = rand::SystemRandom::new();
    let pair = signature::EcdsaKeyPair::from_pkcs8(
        &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        &keys.pkcs8,
        &rng,
    )
    .map_err(|_| Error::store("vapid key is unusable"))?;
    let sig = pair
        .sign(&rng, signing_input.as_bytes())
        .map_err(|_| Error::store("vapid signing failed"))?;
    Ok(format!(
        "vapid t={signing_input}.{}, k={}",
        URL_SAFE_NO_PAD.encode(sig.as_ref()),
        keys.public_key_b64()
    ))
}

fn endpoint_origin(endpoint: &str) -> Option<String> {
    let (scheme, rest) = endpoint.split_once("://")?;
    let host = rest.split('/').next()?;
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}"))
}

// RFC 8291 aes128gcm, single record: header = salt(16) rs(4) idlen(1) server-public(65).
pub fn encrypt(p256dh: &[u8], auth: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    let rng = rand::SystemRandom::new();
    let private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
        .map_err(|_| Error::store("push encryption failed"))?;
    let server_public = private
        .compute_public_key()
        .map_err(|_| Error::store("push encryption failed"))?;
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, p256dh);
    let shared = agreement::agree_ephemeral(private, &peer, |secret| secret.to_vec())
        .map_err(|_| Error::invalid_input("push subscription key is invalid"))?;

    let mut salt = [0u8; 16];
    rng.fill(&mut salt)
        .map_err(|_| Error::store("push encryption failed"))?;

    encrypt_parts(
        &shared,
        server_public.as_ref(),
        &salt,
        p256dh,
        auth,
        payload,
    )
}

fn encrypt_parts(
    shared: &[u8],
    server_public: &[u8],
    salt: &[u8; 16],
    p256dh: &[u8],
    auth: &[u8],
    payload: &[u8],
) -> Result<Vec<u8>> {
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(p256dh);
    info.extend_from_slice(server_public);
    let ikm = derive(auth, shared, &info, 32)?;

    let cek = derive(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16)?;
    let nonce = derive(salt, &ikm, b"Content-Encoding: nonce\0", 12)?;

    let mut record = payload.to_vec();
    record.push(0x02);
    let unbound = aead::UnboundKey::new(&aead::AES_128_GCM, &cek)
        .map_err(|_| Error::store("push encryption failed"))?;
    let key = aead::LessSafeKey::new(unbound);
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&nonce);
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce_bytes),
        aead::Aad::empty(),
        &mut record,
    )
    .map_err(|_| Error::store("push encryption failed"))?;

    let mut body = Vec::with_capacity(86 + record.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&4096u32.to_be_bytes());
    body.push(65);
    body.extend_from_slice(server_public);
    body.extend_from_slice(&record);
    Ok(body)
}

fn derive(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Result<Vec<u8>> {
    struct Len(usize);
    impl hkdf::KeyType for Len {
        fn len(&self) -> usize {
            self.0
        }
    }
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm);
    let info_parts = [info];
    let okm = prk
        .expand(&info_parts, Len(len))
        .map_err(|_| Error::store("push key derivation failed"))?;
    let mut out = vec![0u8; len];
    okm.fill(&mut out)
        .map_err(|_| Error::store("push key derivation failed"))?;
    Ok(out)
}

fn vapid_key() -> Vec<u8> {
    vec![Subspace::Registry.as_byte(), TAG_VAPID_KEY]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    #[test]
    fn the_vapid_key_is_generated_once_and_reused() {
        let ctx = test_context();
        let first = load_or_create_vapid(ctx.store.as_ref()).unwrap();
        let second = load_or_create_vapid(ctx.store.as_ref()).unwrap();
        assert_eq!(first.public_key_b64(), second.public_key_b64());
        assert!(!first.public_key_b64().is_empty());
    }

    #[test]
    fn the_vapid_jwt_verifies_against_the_public_key() {
        let ctx = test_context();
        let keys = load_or_create_vapid(ctx.store.as_ref()).unwrap();
        let header = vapid_authorization(
            &keys,
            "https://push.example.net/send/abc",
            "mailto:admin@example.com",
            1_700_000_000,
        )
        .unwrap();

        let token = header
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split(',').next())
            .unwrap();
        let mut parts = token.rsplitn(2, '.');
        let sig = URL_SAFE_NO_PAD.decode(parts.next().unwrap()).unwrap();
        let signing_input = parts.next().unwrap();
        let public = URL_SAFE_NO_PAD
            .decode(header.split("k=").nth(1).unwrap())
            .unwrap();
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, &public)
            .verify(signing_input.as_bytes(), &sig)
            .expect("jwt signature verifies");

        let claims_b64 = signing_input.split('.').nth(1).unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims_b64).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["sub"], "mailto:admin@example.com");
    }

    #[test]
    fn an_encrypted_push_round_trips_through_the_rfc8291_schedule() {
        let rng = rand::SystemRandom::new();
        let client_private =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let client_public = client_private.compute_public_key().unwrap();
        let mut auth = [0u8; 16];
        rng.fill(&mut auth).unwrap();

        let payload = br#"{"@type":"StateChange"}"#;
        let body = encrypt(client_public.as_ref(), &auth, payload).unwrap();

        let salt = &body[..16];
        let rs = u32::from_be_bytes(body[16..20].try_into().unwrap());
        assert_eq!(rs, 4096);
        assert_eq!(body[20], 65);
        let server_public = &body[21..86];
        let mut ciphertext = body[86..].to_vec();
        assert_ne!(&ciphertext[..payload.len()], payload.as_slice());

        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, server_public);
        let shared =
            agreement::agree_ephemeral(client_private, &peer, |secret| secret.to_vec()).unwrap();
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(client_public.as_ref());
        info.extend_from_slice(server_public);
        let ikm = derive(&auth, &shared, &info, 32).unwrap();
        let cek = derive(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16).unwrap();
        let nonce = derive(salt, &ikm, b"Content-Encoding: nonce\0", 12).unwrap();

        let unbound = aead::UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap();
        let key = aead::LessSafeKey::new(unbound);
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes.copy_from_slice(&nonce);
        let plain = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce_bytes),
                aead::Aad::empty(),
                &mut ciphertext,
            )
            .expect("push payload decrypts");
        assert_eq!(&plain[..payload.len()], payload.as_slice());
        assert_eq!(plain[payload.len()], 0x02);
    }

    #[test]
    fn the_rfc8291_known_answer_vector_is_reproduced() {
        let plaintext = b"When I grow up, I want to be a watermelon";
        let ua_public = URL_SAFE_NO_PAD
            .decode("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4")
            .unwrap();
        let as_public = URL_SAFE_NO_PAD
            .decode("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8")
            .unwrap();
        let auth = URL_SAFE_NO_PAD.decode("BTBZMqHH6r4Tts7J_aSIgg").unwrap();
        let salt: [u8; 16] = URL_SAFE_NO_PAD
            .decode("DGv6ra1nlYgDCS1FRnbzlw")
            .unwrap()
            .try_into()
            .unwrap();
        let ecdh_secret = URL_SAFE_NO_PAD
            .decode("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs")
            .unwrap();
        let expected = URL_SAFE_NO_PAD
            .decode(concat!(
                "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml",
                "mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT",
                "pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
            ))
            .unwrap();

        let body = encrypt_parts(
            &ecdh_secret,
            &as_public,
            &salt,
            &ua_public,
            &auth,
            plaintext,
        )
        .unwrap();
        assert_eq!(body, expected);
    }

    #[test]
    fn the_origin_is_extracted_from_the_endpoint() {
        assert_eq!(
            endpoint_origin("https://fcm.googleapis.com/fcm/send/x").as_deref(),
            Some("https://fcm.googleapis.com")
        );
        assert_eq!(endpoint_origin("nonsense"), None);
    }
}
