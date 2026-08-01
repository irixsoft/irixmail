use argon2::password_hash::rand_core::OsRng as ArgonOsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use irixmail_core::{Error, Result};
use rand::{rngs::OsRng, TryRngCore};
use totp_rs::{Algorithm, TOTP};

const CODE_DIGITS: usize = 6;

const STEP_SECONDS: u64 = 30;

const STEP_SKEW: u8 = 1;

const SECRET_BYTES: usize = 20;

const RECOVERY_CODE_COUNT: usize = 10;

const RECOVERY_CODE_BYTES: usize = 10;

pub fn generate_secret() -> Result<Vec<u8>> {
    let mut secret = vec![0u8; SECRET_BYTES];
    OsRng
        .try_fill_bytes(&mut secret)
        .map_err(|err| Error::Internal(format!("could not generate a TOTP secret: {err}")))?;
    Ok(secret)
}

pub fn provisioning_uri(secret: &[u8], issuer: &str, account_name: &str) -> Result<String> {
    let totp = build_totp(
        secret.to_vec(),
        Some(issuer.to_string()),
        account_name.to_string(),
    )?;
    Ok(totp.get_url())
}

pub fn secret_base32(secret: &[u8]) -> Result<String> {
    let totp = build_totp(secret.to_vec(), None, String::new())?;
    Ok(totp.get_secret_base32())
}

pub fn verify_code(secret: &[u8], code: &str, unix_time: u64) -> Result<bool> {
    let totp = build_totp(secret.to_vec(), None, String::new())?;
    Ok(totp.check(code, unix_time))
}

#[derive(Debug, Clone)]
pub struct RecoveryCodes {
    pub plaintext: Vec<String>,
    pub hashes: Vec<String>,
}

pub fn generate_recovery_codes() -> Result<RecoveryCodes> {
    let mut plaintext = Vec::with_capacity(RECOVERY_CODE_COUNT);
    let mut hashes = Vec::with_capacity(RECOVERY_CODE_COUNT);
    for _ in 0..RECOVERY_CODE_COUNT {
        let code = generate_recovery_code()?;
        let hash = hash_recovery_code(&code)?;
        plaintext.push(code);
        hashes.push(hash);
    }
    Ok(RecoveryCodes { plaintext, hashes })
}

pub fn consume_recovery_code(stored: &mut Vec<String>, code: &str) -> Result<bool> {
    let mut matched_index = None;
    for (index, hash) in stored.iter().enumerate() {
        if verify_recovery_code(code, hash)? && matched_index.is_none() {
            matched_index = Some(index);
        }
    }
    match matched_index {
        Some(index) => {
            stored.remove(index);
            Ok(true)
        }
        None => Ok(false),
    }
}

fn build_totp(secret: Vec<u8>, issuer: Option<String>, account_name: String) -> Result<TOTP> {
    TOTP::new(
        Algorithm::SHA1,
        CODE_DIGITS,
        STEP_SKEW,
        STEP_SECONDS,
        secret,
        issuer,
        account_name,
    )
    .map_err(|err| Error::InvalidInput(format!("invalid TOTP configuration: {err}")))
}

fn generate_recovery_code() -> Result<String> {
    let mut bytes = [0u8; RECOVERY_CODE_BYTES];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|err| Error::Internal(format!("could not generate a recovery code: {err}")))?;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let (first, second) = hex.split_at(hex.len() / 2);
    Ok(format!("{first}-{second}"))
}

fn hash_recovery_code(code: &str) -> Result<String> {
    let salt = SaltString::generate(&mut ArgonOsRng);
    Argon2::default()
        .hash_password(code.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| Error::Internal(format!("recovery code hashing failed: {err}")))
}

fn verify_recovery_code(code: &str, stored: &str) -> Result<bool> {
    let parsed = PasswordHash::new(stored)
        .map_err(|err| Error::Internal(format!("stored recovery code hash is malformed: {err}")))?;
    match Argon2::default().verify_password(code.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(err) => Err(Error::Internal(format!(
            "recovery code verification failed: {err}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_code(secret: &[u8], unix_time: u64) -> String {
        let totp = build_totp(secret.to_vec(), None, String::new()).expect("totp builds");
        totp.generate(unix_time)
    }

    #[test]
    fn a_generated_secret_has_the_recommended_length() {
        let secret = generate_secret().expect("secret generates");
        assert_eq!(secret.len(), SECRET_BYTES);
    }

    #[test]
    fn two_generated_secrets_differ() {
        let first = generate_secret().expect("first secret generates");
        let second = generate_secret().expect("second secret generates");
        assert_ne!(first, second, "each secret must be independently random");
    }

    #[test]
    fn a_provisioning_uri_is_an_otpauth_url_carrying_the_label() {
        let secret = generate_secret().expect("secret generates");
        let uri = provisioning_uri(&secret, "IRIXMAIL", "alice@example.com").expect("uri builds");
        assert!(uri.starts_with("otpauth://totp/"), "got {uri}");
        assert!(uri.contains("issuer=IRIXMAIL"), "got {uri}");
        assert!(uri.contains("secret="), "got {uri}");
        assert!(uri.contains("alice"), "got {uri}");
    }

    #[test]
    fn the_base32_secret_matches_the_provisioning_uri() {
        let secret = generate_secret().expect("secret generates");
        let encoded = secret_base32(&secret).expect("base32 encodes");
        let uri = provisioning_uri(&secret, "IRIXMAIL", "alice@example.com").expect("uri builds");
        assert!(uri.contains(&format!("secret={encoded}")), "got {uri}");
        assert!(encoded
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || (b'2'..=b'7').contains(&byte)));
    }

    #[test]
    fn a_colon_in_the_label_is_rejected() {
        let secret = generate_secret().expect("secret generates");
        let result = provisioning_uri(&secret, "bad:issuer", "alice@example.com");
        assert!(matches!(result, Err(Error::InvalidInput(_))));
    }

    #[test]
    fn the_current_code_verifies() {
        let secret = generate_secret().expect("secret generates");
        let now = 1_700_000_000;
        let code = current_code(&secret, now);
        assert!(verify_code(&secret, &code, now).expect("verification runs"));
    }

    #[test]
    fn a_wrong_code_does_not_verify() {
        let secret = generate_secret().expect("secret generates");
        let now = 1_700_000_000;
        assert!(!verify_code(&secret, "000000", now).expect("verification runs"));
    }

    #[test]
    fn a_code_from_the_neighbouring_step_still_verifies() {
        let secret = generate_secret().expect("secret generates");
        let now = 1_700_000_000;
        let earlier = current_code(&secret, now - STEP_SECONDS);
        let later = current_code(&secret, now + STEP_SECONDS);
        assert!(verify_code(&secret, &earlier, now).expect("verification runs"));
        assert!(verify_code(&secret, &later, now).expect("verification runs"));
    }

    #[test]
    fn a_code_two_steps_away_does_not_verify() {
        let secret = generate_secret().expect("secret generates");
        let now = 1_700_000_000;
        let stale = current_code(&secret, now - STEP_SECONDS * 2);
        assert!(!verify_code(&secret, &stale, now).expect("verification runs"));
    }

    #[test]
    fn a_too_short_secret_surfaces_as_invalid_input() {
        let result = verify_code(&[0u8; 2], "123456", 1_700_000_000);
        assert!(matches!(result, Err(Error::InvalidInput(_))));
    }

    #[test]
    fn a_batch_of_recovery_codes_pairs_plaintext_with_hashes() {
        let codes = generate_recovery_codes().expect("codes generate");
        assert_eq!(codes.plaintext.len(), RECOVERY_CODE_COUNT);
        assert_eq!(codes.hashes.len(), RECOVERY_CODE_COUNT);
        for hash in &codes.hashes {
            assert!(hash.starts_with("$argon2id$"), "got {hash}");
        }
    }

    #[test]
    fn each_recovery_code_is_unique() {
        let codes = generate_recovery_codes().expect("codes generate");
        let mut seen = codes.plaintext.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), codes.plaintext.len(), "codes must not repeat");
    }

    #[test]
    fn a_recovery_code_hash_never_contains_the_plaintext() {
        let codes = generate_recovery_codes().expect("codes generate");
        for (code, hash) in codes.plaintext.iter().zip(&codes.hashes) {
            assert!(!hash.contains(code), "the plaintext leaked into a hash");
        }
    }

    #[test]
    fn a_valid_recovery_code_is_consumed_once() {
        let codes = generate_recovery_codes().expect("codes generate");
        let mut stored = codes.hashes.clone();
        let used = &codes.plaintext[0];
        assert!(consume_recovery_code(&mut stored, used).expect("consume runs"));
        assert_eq!(stored.len(), RECOVERY_CODE_COUNT - 1);
        assert!(
            !consume_recovery_code(&mut stored, used).expect("consume runs"),
            "a consumed code must not be accepted again"
        );
        assert_eq!(stored.len(), RECOVERY_CODE_COUNT - 1);
    }

    #[test]
    fn an_unknown_recovery_code_is_rejected_and_leaves_the_list() {
        let codes = generate_recovery_codes().expect("codes generate");
        let mut stored = codes.hashes.clone();
        assert!(!consume_recovery_code(&mut stored, "ffff-ffff").expect("consume runs"));
        assert_eq!(stored.len(), RECOVERY_CODE_COUNT);
    }

    #[test]
    fn a_malformed_recovery_hash_is_an_error() {
        let mut stored = vec!["not-a-phc-string".to_string()];
        let result = consume_recovery_code(&mut stored, "any-code");
        assert!(matches!(result, Err(Error::Internal(_))));
    }
}
