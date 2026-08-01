use irixmail_core::{Error, Result};
use rand::{rngs::OsRng, TryRngCore};

const SECRET_BYTES: usize = 32;

#[derive(Debug, Clone)]
pub struct GeneratedApiKey {
    pub plaintext: String,
}

pub fn generate() -> Result<GeneratedApiKey> {
    let plaintext = generate_secret()?;
    Ok(GeneratedApiKey { plaintext })
}

pub fn list(api_keys: &[crate::ApiKey]) -> Vec<&crate::ApiKey> {
    let mut listed: Vec<&crate::ApiKey> = api_keys.iter().collect();
    listed.sort_by_key(|entry| entry.created_at);
    listed
}

pub fn verify(candidate: &str, secret: &str) -> bool {
    constant_time_eq(candidate.as_bytes(), secret.as_bytes())
}

pub fn verify_any<'a, F>(
    candidate: &str,
    api_keys: &'a [crate::ApiKey],
    mut decrypt: F,
) -> Result<Option<&'a crate::ApiKey>>
where
    F: FnMut(&crate::EncryptedSecret) -> Result<String>,
{
    let mut matched = None;
    for key in api_keys {
        let secret = decrypt(&key.secret)?;
        if verify(candidate, &secret) && matched.is_none() {
            matched = Some(key);
        }
    }
    Ok(matched)
}

pub fn revoke(api_keys: &mut Vec<crate::ApiKey>, id: u64) -> bool {
    let before = api_keys.len();
    api_keys.retain(|key| key.id != id);
    api_keys.len() != before
}

fn generate_secret() -> Result<String> {
    let mut bytes = [0u8; SECRET_BYTES];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|err| Error::Internal(format!("could not generate an API key: {err}")))?;
    Ok(base32_lower(&bytes))
}

const BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn base32_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            out.push(BASE32_ALPHABET[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(BASE32_ALPHABET[index] as char);
    }
    out
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let width = left.len().max(right.len());
    let mut difference = (left.len() ^ right.len()) as u8;
    for index in 0..width {
        let lhs = left.get(index).copied().unwrap_or(0);
        let rhs = right.get(index).copied().unwrap_or(0);
        difference |= lhs ^ rhs;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiKey, EncryptedSecret};

    fn stored(id: u64, plaintext: &str, created_at: u64) -> ApiKey {
        ApiKey {
            id,
            name: format!("key-{id}"),
            secret: EncryptedSecret {
                nonce: Vec::new(),
                ciphertext: plaintext.bytes().rev().collect(),
            },
            created_at,
            last_used_at: None,
        }
    }

    fn decrypt(secret: &EncryptedSecret) -> Result<String> {
        let reversed: Vec<u8> = secret.ciphertext.iter().rev().copied().collect();
        String::from_utf8(reversed)
            .map_err(|err| Error::Internal(format!("secret is not valid text: {err}")))
    }

    #[test]
    fn a_generated_secret_is_an_unbroken_base32_token() {
        let minted = generate().expect("an API key generates");
        assert!(
            minted
                .plaintext
                .bytes()
                .all(|byte| BASE32_ALPHABET.contains(&byte)),
            "secret {} used a character outside the base32 alphabet",
            minted.plaintext
        );
        assert!(!minted.plaintext.is_empty());
    }

    #[test]
    fn two_generated_secrets_differ() {
        let first = generate().expect("an API key generates");
        let second = generate().expect("an API key generates");
        assert_ne!(
            first.plaintext, second.plaintext,
            "each secret must be independently random"
        );
    }

    #[test]
    fn the_minted_secret_verifies_against_itself() {
        let minted = generate().expect("an API key generates");
        assert!(verify(&minted.plaintext, &minted.plaintext));
    }

    #[test]
    fn a_wrong_key_does_not_verify() {
        let minted = generate().expect("an API key generates");
        assert!(!verify("not-the-key", &minted.plaintext));
    }

    #[test]
    fn a_key_that_is_a_prefix_of_the_secret_does_not_verify() {
        assert!(!verify("secre", "secret"));
        assert!(!verify("secret", "secre"));
    }

    #[test]
    fn an_empty_candidate_only_matches_an_empty_secret() {
        assert!(verify("", ""));
        assert!(!verify("", "secret"));
        assert!(!verify("secret", ""));
    }

    #[test]
    fn verify_any_finds_the_matching_key() {
        let keys = vec![stored(1, "first-secret", 0), stored(2, "second-secret", 0)];
        let found = verify_any("second-secret", &keys, decrypt)
            .expect("search runs")
            .expect("a key matches");
        assert_eq!(found.id, 2);
    }

    #[test]
    fn verify_any_reports_no_match_for_an_unknown_key() {
        let keys = vec![stored(1, "first-secret", 0)];
        assert!(verify_any("unknown", &keys, decrypt)
            .expect("search runs")
            .is_none());
    }

    #[test]
    fn verify_any_over_an_empty_list_matches_nothing() {
        assert!(verify_any("anything", &[], decrypt)
            .expect("search runs")
            .is_none());
    }

    #[test]
    fn verify_any_surfaces_a_decryption_failure() {
        let mut broken = stored(1, "secret", 0);
        broken.secret.ciphertext = vec![0xff, 0xfe];
        let keys = vec![broken];
        assert!(matches!(
            verify_any("secret", &keys, decrypt),
            Err(Error::Internal(_))
        ));
    }

    #[test]
    fn list_orders_keys_oldest_first() {
        let keys = vec![stored(1, "newer", 2_000), stored(2, "older", 1_000)];
        let listed = list(&keys);
        assert_eq!(listed[0].id, 2);
        assert_eq!(listed[1].id, 1);
    }

    #[test]
    fn list_over_an_empty_account_is_empty() {
        assert!(list(&[]).is_empty());
    }

    #[test]
    fn revoking_a_known_id_removes_only_that_key() {
        let mut keys = vec![stored(1, "first", 0), stored(2, "second", 0)];
        assert!(revoke(&mut keys, 1));
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, 2);
    }

    #[test]
    fn revoking_an_unknown_id_changes_nothing() {
        let mut keys = vec![stored(1, "first", 0)];
        assert!(!revoke(&mut keys, 999));
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn a_revoked_key_no_longer_authenticates() {
        let mut keys = vec![stored(1, "first-secret", 0)];
        assert!(revoke(&mut keys, 1));
        assert!(verify_any("first-secret", &keys, decrypt)
            .expect("search runs")
            .is_none());
    }

    #[test]
    fn constant_time_eq_matches_naive_equality() {
        let cases = [
            (&b""[..], &b""[..]),
            (&b"a"[..], &b"a"[..]),
            (&b"abc"[..], &b"abd"[..]),
            (&b"abc"[..], &b"ab"[..]),
            (&b"ab"[..], &b"abc"[..]),
        ];
        for (left, right) in cases {
            assert_eq!(constant_time_eq(left, right), left == right);
        }
    }
}
