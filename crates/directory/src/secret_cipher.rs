use std::io::Write;
use std::path::Path;

use aes_gcm_siv::aead::{Aead, KeyInit};
use aes_gcm_siv::{Aes256GcmSiv, Nonce};
use irixmail_core::{Error, Result};
use rand::{rngs::OsRng, TryRngCore};

use crate::credential::EncryptedSecret;

const KEY_CONTEXT: &str = "irixmail 2026-07-05 credential field encryption v1";

const MASTER_KEY_BYTES: usize = 32;

const MIN_MASTER_KEY_BYTES: usize = 16;

const NONCE_BYTES: usize = 12;

#[derive(Clone)]
pub struct SecretCipher {
    key: [u8; MASTER_KEY_BYTES],
}

impl std::fmt::Debug for SecretCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretCipher").finish_non_exhaustive()
    }
}

impl SecretCipher {
    pub fn from_master_key(master: &[u8]) -> Result<Self> {
        if master.len() < MIN_MASTER_KEY_BYTES {
            return Err(Error::InvalidInput(format!(
                "the master key must be at least {MIN_MASTER_KEY_BYTES} bytes"
            )));
        }
        Ok(Self {
            key: blake3::derive_key(KEY_CONTEXT, master),
        })
    }

    pub fn generate_master_key() -> Result<[u8; MASTER_KEY_BYTES]> {
        let mut master = [0u8; MASTER_KEY_BYTES];
        OsRng
            .try_fill_bytes(&mut master)
            .map_err(|err| Error::Internal(format!("could not generate a master key: {err}")))?;
        Ok(master)
    }

    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Self::from_master_key(&decode_hex(contents.trim())?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let master = Self::generate_master_key()?;
                match write_key_file(path, &master) {
                    Ok(()) => Self::from_master_key(&master),
                    Err(WriteKeyError::Raced) => Self::load_or_create(path),
                    Err(WriteKeyError::Io(err)) => Err(Error::Internal(format!(
                        "could not write the master key file {}: {err}",
                        path.display()
                    ))),
                }
            }
            Err(err) => Err(Error::Internal(format!(
                "could not read the master key file {}: {err}",
                path.display()
            ))),
        }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedSecret> {
        let mut nonce = [0u8; NONCE_BYTES];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|err| Error::Internal(format!("could not generate a nonce: {err}")))?;
        let ciphertext = self
            .cipher()?
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| Error::Internal("credential encryption failed".to_string()))?;
        Ok(EncryptedSecret {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    pub fn decrypt(&self, secret: &EncryptedSecret) -> Result<Vec<u8>> {
        if secret.nonce.len() != NONCE_BYTES {
            return Err(Error::Internal(
                "credential decryption failed: malformed nonce".to_string(),
            ));
        }
        self.cipher()?
            .decrypt(
                Nonce::from_slice(&secret.nonce),
                secret.ciphertext.as_slice(),
            )
            .map_err(|_| Error::Internal("credential decryption failed".to_string()))
    }

    fn cipher(&self) -> Result<Aes256GcmSiv> {
        Aes256GcmSiv::new_from_slice(&self.key)
            .map_err(|err| Error::Internal(format!("could not build the field cipher: {err}")))
    }
}

enum WriteKeyError {
    Raced,
    Io(std::io::Error),
}

fn write_key_file(path: &Path, master: &[u8]) -> std::result::Result<(), WriteKeyError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(WriteKeyError::Io)?;
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(WriteKeyError::Raced)
        }
        Err(err) => return Err(WriteKeyError::Io(err)),
    };
    let mut rendered: String = master.iter().map(|byte| format!("{byte:02x}")).collect();
    rendered.push('\n');
    file.write_all(rendered.as_bytes())
        .map_err(WriteKeyError::Io)?;
    file.sync_all().map_err(WriteKeyError::Io)?;
    Ok(())
}

fn decode_hex(text: &str) -> Result<Vec<u8>> {
    if text.len() != MASTER_KEY_BYTES * 2 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidInput(
            "the master key file is malformed".to_string(),
        ));
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&text[index..index + 2], 16).map_err(|err| {
                Error::InvalidInput(format!("the master key file is malformed: {err}"))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("irixmail-cipher-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn cipher() -> SecretCipher {
        SecretCipher::from_master_key(&SecretCipher::generate_master_key().unwrap()).unwrap()
    }

    #[test]
    fn a_secret_round_trips() {
        let cipher = cipher();
        let sealed = cipher.encrypt(b"the totp seed").unwrap();
        assert_eq!(cipher.decrypt(&sealed).unwrap(), b"the totp seed");
    }

    #[test]
    fn the_ciphertext_does_not_contain_the_plaintext() {
        let cipher = cipher();
        let plaintext = b"a twenty byte secret";
        let sealed = cipher.encrypt(plaintext).unwrap();
        assert!(!sealed
            .ciphertext
            .windows(plaintext.len())
            .any(|window| window == plaintext));
        assert_ne!(sealed.ciphertext, plaintext);
    }

    #[test]
    fn each_encryption_uses_a_fresh_nonce() {
        let cipher = cipher();
        let first = cipher.encrypt(b"same input").unwrap();
        let second = cipher.encrypt(b"same input").unwrap();
        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.ciphertext, second.ciphertext);
    }

    #[test]
    fn a_tampered_ciphertext_does_not_decrypt() {
        let cipher = cipher();
        let mut sealed = cipher.encrypt(b"secret").unwrap();
        sealed.ciphertext[0] ^= 0x01;
        assert!(matches!(cipher.decrypt(&sealed), Err(Error::Internal(_))));
    }

    #[test]
    fn a_different_master_key_does_not_decrypt() {
        let sealed = cipher().encrypt(b"secret").unwrap();
        assert!(matches!(cipher().decrypt(&sealed), Err(Error::Internal(_))));
    }

    #[test]
    fn a_malformed_nonce_is_refused() {
        let cipher = cipher();
        let mut sealed = cipher.encrypt(b"secret").unwrap();
        sealed.nonce.pop();
        assert!(matches!(cipher.decrypt(&sealed), Err(Error::Internal(_))));
    }

    #[test]
    fn a_short_master_key_is_rejected() {
        assert!(matches!(
            SecretCipher::from_master_key(b"short"),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn load_or_create_persists_a_key_that_survives_reload() {
        let dir = TempDir::new();
        let path = dir.path.join("keys").join("credential.key");

        let first = SecretCipher::load_or_create(&path).unwrap();
        let sealed = first.encrypt(b"stable across restarts").unwrap();

        let second = SecretCipher::load_or_create(&path).unwrap();
        assert_eq!(second.decrypt(&sealed).unwrap(), b"stable across restarts");
    }

    #[cfg(unix)]
    #[test]
    fn the_created_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        let path = dir.path.join("credential.key");
        SecretCipher::load_or_create(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_corrupt_key_file_is_an_error_not_a_silent_new_key() {
        let dir = TempDir::new();
        let path = dir.path.join("credential.key");
        std::fs::write(&path, "not-hex").unwrap();
        assert!(matches!(
            SecretCipher::load_or_create(&path),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn the_debug_rendering_hides_the_key() {
        let rendered = format!("{:?}", cipher());
        assert!(!rendered.contains("key:"));
    }
}
