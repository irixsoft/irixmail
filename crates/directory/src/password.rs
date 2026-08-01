use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use irixmail_core::{Error, Result};

pub fn hash(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| Error::Internal(format!("password hashing failed: {err}")))
}

pub fn verify_dummy(password: &str) {
    static DUMMY_HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let stored = DUMMY_HASH
        .get_or_init(|| hash("irixmail-timing-equalizer").expect("the dummy secret hashes"));
    let _ = verify(password, stored);
}

pub fn verify(password: &str, stored: &str) -> Result<bool> {
    let parsed = PasswordHash::new(stored)
        .map_err(|err| Error::Internal(format!("stored password hash is malformed: {err}")))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(err) => Err(Error::Internal(format!(
            "password verification failed: {err}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_is_phc_encoded_argon2id() {
        let encoded = hash("correct horse battery staple").expect("hashing succeeds");
        assert!(
            encoded.starts_with("$argon2id$"),
            "expected an Argon2id PHC string, got {encoded}"
        );
    }

    #[test]
    fn the_hash_never_contains_the_plaintext() {
        let secret = "super-secret-passphrase";
        let encoded = hash(secret).expect("hashing succeeds");
        assert!(
            !encoded.contains(secret),
            "the plaintext leaked into the stored hash"
        );
    }

    #[test]
    fn hashing_the_same_secret_twice_uses_a_fresh_salt() {
        let first = hash("repeated-secret").expect("hashing succeeds");
        let second = hash("repeated-secret").expect("hashing succeeds");
        assert_ne!(
            first, second,
            "identical secrets must hash to different strings via random salts"
        );
    }

    #[test]
    fn the_correct_secret_verifies() {
        let encoded = hash("the-right-one").expect("hashing succeeds");
        assert!(verify("the-right-one", &encoded).expect("verification runs"));
    }

    #[test]
    fn a_wrong_secret_does_not_verify() {
        let encoded = hash("the-right-one").expect("hashing succeeds");
        assert!(!verify("the-wrong-one", &encoded).expect("verification runs"));
    }

    #[test]
    fn each_independently_minted_hash_verifies_its_own_secret() {
        let first = hash("repeated-secret").expect("hashing succeeds");
        let second = hash("repeated-secret").expect("hashing succeeds");
        assert!(verify("repeated-secret", &first).expect("verification runs"));
        assert!(verify("repeated-secret", &second).expect("verification runs"));
    }

    #[test]
    fn an_empty_secret_round_trips() {
        let encoded = hash("").expect("hashing an empty secret succeeds");
        assert!(verify("", &encoded).expect("verification runs"));
        assert!(!verify("not-empty", &encoded).expect("verification runs"));
    }

    #[test]
    fn a_malformed_stored_hash_is_an_error_not_a_mismatch() {
        let result = verify("anything", "not-a-phc-string");
        assert!(
            matches!(result, Err(Error::Internal(_))),
            "a corrupt stored hash should surface as an error"
        );
    }
}
