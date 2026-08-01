use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Credential {
    PrimaryPassword(PrimaryPassword),
    Totp(Totp),
    AppPassword(AppPassword),
    ApiKey(ApiKey),
}

impl Credential {
    pub fn kind(&self) -> CredentialKind {
        match self {
            Credential::PrimaryPassword(_) => CredentialKind::PrimaryPassword,
            Credential::Totp(_) => CredentialKind::Totp,
            Credential::AppPassword(_) => CredentialKind::AppPassword,
            Credential::ApiKey(_) => CredentialKind::ApiKey,
        }
    }

    pub fn is_interactive_login(&self) -> bool {
        matches!(self, Credential::PrimaryPassword(_))
    }

    pub fn is_mail_login(&self) -> bool {
        matches!(
            self,
            Credential::PrimaryPassword(_) | Credential::AppPassword(_)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialKind {
    PrimaryPassword,
    Totp,
    AppPassword,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimaryPassword {
    pub hash: String,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totp {
    pub secret: EncryptedSecret,
    pub enabled: bool,
    pub recovery_codes: Vec<String>,
    pub enrolled_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPassword {
    pub id: u64,
    pub name: String,
    pub hash: String,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: u64,
    pub name: String,
    pub secret: EncryptedSecret,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

// Fields are crate-private so callers outside this crate can only obtain an
// EncryptedSecret through SecretCipher::encrypt — plaintext cannot be persisted.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSecret {
    pub(crate) nonce: Vec<u8>,
    pub(crate) ciphertext: Vec<u8>,
}

impl std::fmt::Debug for EncryptedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedSecret").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_encrypted_secret() -> EncryptedSecret {
        EncryptedSecret {
            nonce: vec![1, 2, 3, 4],
            ciphertext: vec![9, 8, 7, 6, 5],
        }
    }

    #[test]
    fn each_variant_reports_its_own_kind() {
        let primary = Credential::PrimaryPassword(PrimaryPassword {
            hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".to_string(),
            updated_at: 1_700_000_000_000,
        });
        let totp = Credential::Totp(Totp {
            secret: sample_encrypted_secret(),
            enabled: true,
            recovery_codes: vec!["$argon2id$recovery".to_string()],
            enrolled_at: 1_700_000_000_000,
        });
        let app = Credential::AppPassword(AppPassword {
            id: 1,
            name: "iPhone Mail".to_string(),
            hash: "$argon2id$app".to_string(),
            created_at: 1_700_000_000_000,
            last_used_at: None,
        });
        let key = Credential::ApiKey(ApiKey {
            id: 2,
            name: "backup-script".to_string(),
            secret: sample_encrypted_secret(),
            created_at: 1_700_000_000_000,
            last_used_at: None,
        });

        assert_eq!(primary.kind(), CredentialKind::PrimaryPassword);
        assert_eq!(totp.kind(), CredentialKind::Totp);
        assert_eq!(app.kind(), CredentialKind::AppPassword);
        assert_eq!(key.kind(), CredentialKind::ApiKey);
    }

    #[test]
    fn only_the_primary_password_signs_in_interactively() {
        let primary = Credential::PrimaryPassword(PrimaryPassword {
            hash: "$argon2id$hash".to_string(),
            updated_at: 0,
        });
        let app = Credential::AppPassword(AppPassword {
            id: 1,
            name: "client".to_string(),
            hash: "$argon2id$app".to_string(),
            created_at: 0,
            last_used_at: None,
        });

        assert!(primary.is_interactive_login());
        assert!(!app.is_interactive_login());
    }

    #[test]
    fn the_password_and_app_passwords_authenticate_mail_sessions() {
        let primary = Credential::PrimaryPassword(PrimaryPassword {
            hash: "$argon2id$hash".to_string(),
            updated_at: 0,
        });
        let app = Credential::AppPassword(AppPassword {
            id: 1,
            name: "client".to_string(),
            hash: "$argon2id$app".to_string(),
            created_at: 0,
            last_used_at: None,
        });
        let totp = Credential::Totp(Totp {
            secret: sample_encrypted_secret(),
            enabled: true,
            recovery_codes: Vec::new(),
            enrolled_at: 0,
        });
        let key = Credential::ApiKey(ApiKey {
            id: 2,
            name: "automation".to_string(),
            secret: sample_encrypted_secret(),
            created_at: 0,
            last_used_at: None,
        });

        assert!(primary.is_mail_login());
        assert!(app.is_mail_login());
        assert!(!totp.is_mail_login());
        assert!(!key.is_mail_login());
    }

    #[test]
    fn an_encrypted_secret_debug_does_not_print_its_bytes() {
        let rendered = format!("{:?}", sample_encrypted_secret());
        assert!(rendered.contains("EncryptedSecret"));
        assert!(!rendered.contains('9'));
        assert!(!rendered.contains('5'));
    }

    #[test]
    fn a_credential_round_trips_through_json() {
        let credentials = vec![
            Credential::PrimaryPassword(PrimaryPassword {
                hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".to_string(),
                updated_at: 1_700_000_000_000,
            }),
            Credential::Totp(Totp {
                secret: sample_encrypted_secret(),
                enabled: true,
                recovery_codes: vec![
                    "$argon2id$recovery-one".to_string(),
                    "$argon2id$recovery-two".to_string(),
                ],
                enrolled_at: 1_700_000_500_000,
            }),
            Credential::AppPassword(AppPassword {
                id: 100,
                name: "Thunderbird".to_string(),
                hash: "$argon2id$app".to_string(),
                created_at: 1_700_000_000_000,
                last_used_at: Some(1_700_001_000_000),
            }),
            Credential::ApiKey(ApiKey {
                id: 200,
                name: "ci".to_string(),
                secret: sample_encrypted_secret(),
                created_at: 1_700_000_000_000,
                last_used_at: None,
            }),
        ];

        for credential in credentials {
            let encoded = serde_json::to_string(&credential).expect("credential serializes");
            let decoded: Credential =
                serde_json::from_str(&encoded).expect("credential deserializes");
            assert_eq!(decoded, credential);
        }
    }
}
