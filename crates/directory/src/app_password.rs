use crate::password;
use crate::AppPassword;
use irixmail_core::{Error, Result};
use rand::{rngs::OsRng, TryRngCore};

const SECRET_BYTES: usize = 24;

#[derive(Debug, Clone)]
pub struct GeneratedAppPassword {
    pub plaintext: String,
    pub record: AppPassword,
}

pub fn generate(id: u64, name: &str, created_at: u64) -> Result<GeneratedAppPassword> {
    let plaintext = generate_secret()?;
    let hash = password::hash(&plaintext)?;
    let record = AppPassword {
        id,
        name: name.to_string(),
        hash,
        created_at,
        last_used_at: None,
    };
    Ok(GeneratedAppPassword { plaintext, record })
}

pub fn list(app_passwords: &[AppPassword]) -> Vec<&AppPassword> {
    let mut listed: Vec<&AppPassword> = app_passwords.iter().collect();
    listed.sort_by_key(|entry| entry.created_at);
    listed
}

pub fn verify(candidate: &str, record: &AppPassword) -> Result<bool> {
    password::verify(candidate, &record.hash)
}

pub fn verify_any<'a>(
    candidate: &str,
    app_passwords: &'a [AppPassword],
) -> Result<Option<&'a AppPassword>> {
    let mut matched = None;
    for record in app_passwords {
        if verify(candidate, record)? && matched.is_none() {
            matched = Some(record);
        }
    }
    Ok(matched)
}

pub fn revoke(app_passwords: &mut Vec<AppPassword>, id: u64) -> bool {
    let before = app_passwords.len();
    app_passwords.retain(|record| record.id != id);
    app_passwords.len() != before
}

fn generate_secret() -> Result<String> {
    let mut bytes = [0u8; SECRET_BYTES];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|err| Error::Internal(format!("could not generate an app password: {err}")))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: u64, name: &str, created_at: u64) -> GeneratedAppPassword {
        generate(id, name, created_at).expect("app password generates")
    }

    #[test]
    fn a_generated_record_carries_the_supplied_fields() {
        let minted = sample(7, "iPhone Mail", 1_700_000_000_000);
        assert_eq!(minted.record.id, 7);
        assert_eq!(minted.record.name, "iPhone Mail");
        assert_eq!(minted.record.created_at, 1_700_000_000_000);
        assert_eq!(minted.record.last_used_at, None);
    }

    #[test]
    fn a_generated_record_holds_an_argon2id_hash_not_the_plaintext() {
        let minted = sample(1, "client", 0);
        assert!(
            minted.record.hash.starts_with("$argon2id$"),
            "got {}",
            minted.record.hash
        );
        assert!(
            !minted.record.hash.contains(&minted.plaintext),
            "the plaintext leaked into the stored hash"
        );
    }

    #[test]
    fn two_generated_secrets_differ() {
        let first = sample(1, "client", 0);
        let second = sample(2, "client", 0);
        assert_ne!(
            first.plaintext, second.plaintext,
            "each secret must be independently random"
        );
    }

    #[test]
    fn the_minted_secret_verifies_against_its_own_record() {
        let minted = sample(1, "client", 0);
        assert!(verify(&minted.plaintext, &minted.record).expect("verification runs"));
    }

    #[test]
    fn a_wrong_secret_does_not_verify() {
        let minted = sample(1, "client", 0);
        assert!(!verify("not-the-secret", &minted.record).expect("verification runs"));
    }

    #[test]
    fn a_malformed_stored_hash_is_an_error_not_a_mismatch() {
        let record = AppPassword {
            id: 1,
            name: "client".to_string(),
            hash: "not-a-phc-string".to_string(),
            created_at: 0,
            last_used_at: None,
        };
        assert!(matches!(
            verify("anything", &record),
            Err(Error::Internal(_))
        ));
    }

    #[test]
    fn verify_any_finds_the_matching_record() {
        let one = sample(1, "phone", 0);
        let two = sample(2, "laptop", 0);
        let stored = vec![one.record.clone(), two.record.clone()];
        let found = verify_any(&two.plaintext, &stored)
            .expect("search runs")
            .expect("a record matches");
        assert_eq!(found.id, 2);
    }

    #[test]
    fn verify_any_reports_no_match_for_an_unknown_secret() {
        let one = sample(1, "phone", 0);
        let stored = vec![one.record];
        assert!(verify_any(" unrelated-secret", &stored)
            .expect("search runs")
            .is_none());
    }

    #[test]
    fn verify_any_over_an_empty_list_matches_nothing() {
        assert!(verify_any("anything", &[]).expect("search runs").is_none());
    }

    #[test]
    fn list_orders_records_oldest_first() {
        let newer = sample(1, "newer", 2_000);
        let older = sample(2, "older", 1_000);
        let stored = vec![newer.record, older.record];
        let listed = list(&stored);
        assert_eq!(listed[0].name, "older");
        assert_eq!(listed[1].name, "newer");
    }

    #[test]
    fn list_over_an_empty_account_is_empty() {
        assert!(list(&[]).is_empty());
    }

    #[test]
    fn revoking_a_known_id_removes_only_that_record() {
        let one = sample(1, "phone", 0);
        let two = sample(2, "laptop", 0);
        let mut stored = vec![one.record, two.record];
        assert!(revoke(&mut stored, 1));
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, 2);
    }

    #[test]
    fn revoking_an_unknown_id_changes_nothing() {
        let one = sample(1, "phone", 0);
        let mut stored = vec![one.record];
        assert!(!revoke(&mut stored, 999));
        assert_eq!(stored.len(), 1);
    }

    #[test]
    fn a_revoked_secret_no_longer_authenticates() {
        let one = sample(1, "phone", 0);
        let plaintext = one.plaintext.clone();
        let mut stored = vec![one.record];
        assert!(revoke(&mut stored, 1));
        assert!(verify_any(&plaintext, &stored)
            .expect("search runs")
            .is_none());
    }

    #[test]
    fn base32_encoding_uses_only_the_alphabet() {
        let minted = sample(1, "client", 0);
        assert!(
            minted
                .plaintext
                .bytes()
                .all(|byte| BASE32_ALPHABET.contains(&byte)),
            "secret {} used a character outside the base32 alphabet",
            minted.plaintext
        );
    }
}
