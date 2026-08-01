use crate::account::Account;
use crate::app_password;
use crate::credential::Credential;
use crate::password;
use irixmail_core::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginPurpose {
    Interactive,
    Mail,
}

impl LoginPurpose {
    pub fn allows_app_password(&self) -> bool {
        matches!(self, LoginPurpose::Mail)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticatedBy {
    PrimaryPassword { mfa_required: bool },
    AppPassword { id: u64 },
}

impl AuthenticatedBy {
    pub fn mfa_required(&self) -> bool {
        matches!(
            self,
            AuthenticatedBy::PrimaryPassword { mfa_required: true }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authentication {
    Granted(AuthenticatedBy),
    Denied,
}

impl Authentication {
    pub fn is_granted(&self) -> bool {
        matches!(self, Authentication::Granted(_))
    }

    pub fn granted_by(&self) -> Option<AuthenticatedBy> {
        match self {
            Authentication::Granted(by) => Some(*by),
            Authentication::Denied => None,
        }
    }
}

pub async fn authenticate_blocking(
    account: &Account,
    credentials: &[Credential],
    purpose: LoginPurpose,
    secret: &str,
) -> Result<Authentication> {
    let account = account.clone();
    let credentials = credentials.to_vec();
    let secret = secret.to_string();
    tokio::task::spawn_blocking(move || authenticate(&account, &credentials, purpose, &secret))
        .await
        .map_err(|err| {
            irixmail_core::Error::Internal(format!("the verification task failed: {err}"))
        })?
}

pub fn authenticate(
    account: &Account,
    credentials: &[Credential],
    purpose: LoginPurpose,
    secret: &str,
) -> Result<Authentication> {
    if !account.is_active() {
        return Ok(Authentication::Denied);
    }

    let mut matched: Option<AuthenticatedBy> = None;

    for credential in credentials {
        match credential {
            Credential::PrimaryPassword(primary) => {
                if password::verify(secret, &primary.hash)? && matched.is_none() {
                    let mfa_required =
                        purpose == LoginPurpose::Interactive && has_enabled_totp(credentials);
                    matched = Some(AuthenticatedBy::PrimaryPassword { mfa_required });
                }
            }
            Credential::AppPassword(app) => {
                if purpose.allows_app_password()
                    && app_password::verify(secret, app)?
                    && matched.is_none()
                {
                    matched = Some(AuthenticatedBy::AppPassword { id: app.id });
                }
            }
            Credential::Totp(_) | Credential::ApiKey(_) => {}
        }
    }

    Ok(match matched {
        Some(by) => Authentication::Granted(by),
        None => Authentication::Denied,
    })
}

fn has_enabled_totp(credentials: &[Credential]) -> bool {
    credentials.iter().any(|credential| match credential {
        Credential::Totp(totp) => totp.enabled,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Role;
    use crate::account::{Forwarding, VacationResponder};
    use crate::app_password as app_password_service;
    use crate::credential::{AppPassword, EncryptedSecret, PrimaryPassword, Totp};

    fn account() -> Account {
        Account {
            id: 7,
            local_part: "alice".to_string(),
            domain_id: 42,
            display_name: "Alice Adams".to_string(),
            enabled: true,
            role: Role::User,
            aliases: Vec::new(),
            forwarding: Forwarding::default(),
            quota_bytes: 0,
            quota_messages: 0,
            locale: String::new(),
            timezone: String::new(),
            signature: String::new(),
            vacation: VacationResponder::default(),
            created_at: 1_700_000_000_000,
        }
    }

    fn primary(plaintext: &str) -> Credential {
        Credential::PrimaryPassword(PrimaryPassword {
            hash: password::hash(plaintext).expect("password hashes"),
            updated_at: 0,
        })
    }

    fn enabled_totp() -> Credential {
        Credential::Totp(Totp {
            secret: EncryptedSecret {
                nonce: vec![1, 2, 3],
                ciphertext: vec![4, 5, 6],
            },
            enabled: true,
            recovery_codes: Vec::new(),
            enrolled_at: 0,
        })
    }

    fn app(id: u64, name: &str) -> (Credential, String) {
        let minted = app_password_service::generate(id, name, 0).expect("app password generates");
        (Credential::AppPassword(minted.record), minted.plaintext)
    }

    #[test]
    fn purpose_allows_app_passwords_only_for_mail() {
        assert!(LoginPurpose::Mail.allows_app_password());
        assert!(!LoginPurpose::Interactive.allows_app_password());
    }

    #[test]
    fn the_primary_password_signs_in_an_interactive_login() {
        let creds = vec![primary("correct horse battery staple")];
        let outcome = authenticate(
            &account(),
            &creds,
            LoginPurpose::Interactive,
            "correct horse battery staple",
        )
        .expect("authentication runs");
        assert_eq!(
            outcome,
            Authentication::Granted(AuthenticatedBy::PrimaryPassword {
                mfa_required: false
            })
        );
        assert!(outcome.is_granted());
        assert_eq!(
            outcome.granted_by(),
            Some(AuthenticatedBy::PrimaryPassword {
                mfa_required: false
            })
        );
    }

    #[test]
    fn a_wrong_primary_password_is_denied() {
        let creds = vec![primary("the-right-one")];
        let outcome = authenticate(
            &account(),
            &creds,
            LoginPurpose::Interactive,
            "the-wrong-one",
        )
        .expect("authentication runs");
        assert_eq!(outcome, Authentication::Denied);
        assert!(!outcome.is_granted());
        assert_eq!(outcome.granted_by(), None);
    }

    #[test]
    fn an_interactive_login_against_a_totp_account_owes_a_second_factor() {
        let creds = vec![primary("pass"), enabled_totp()];
        let outcome = authenticate(&account(), &creds, LoginPurpose::Interactive, "pass")
            .expect("authentication runs");
        assert_eq!(
            outcome,
            Authentication::Granted(AuthenticatedBy::PrimaryPassword { mfa_required: true })
        );
        assert!(outcome.granted_by().unwrap().mfa_required());
    }

    #[test]
    fn a_disabled_totp_factor_does_not_owe_a_second_factor() {
        let mut creds = vec![primary("pass"), enabled_totp()];
        if let Credential::Totp(totp) = &mut creds[1] {
            totp.enabled = false;
        }
        let outcome = authenticate(&account(), &creds, LoginPurpose::Interactive, "pass")
            .expect("authentication runs");
        assert_eq!(
            outcome,
            Authentication::Granted(AuthenticatedBy::PrimaryPassword {
                mfa_required: false
            })
        );
    }

    #[test]
    fn a_mail_login_against_a_totp_account_is_never_challenged() {
        let creds = vec![primary("pass"), enabled_totp()];
        let outcome = authenticate(&account(), &creds, LoginPurpose::Mail, "pass")
            .expect("authentication runs");
        assert_eq!(
            outcome,
            Authentication::Granted(AuthenticatedBy::PrimaryPassword {
                mfa_required: false
            })
        );
    }

    #[test]
    fn an_app_password_signs_in_a_mail_login() {
        let (credential, plaintext) = app(99, "iPhone Mail");
        let creds = vec![primary("pass"), credential];
        let outcome = authenticate(&account(), &creds, LoginPurpose::Mail, &plaintext)
            .expect("authentication runs");
        assert_eq!(
            outcome,
            Authentication::Granted(AuthenticatedBy::AppPassword { id: 99 })
        );
    }

    #[test]
    fn an_app_password_is_refused_for_an_interactive_login() {
        let (credential, plaintext) = app(99, "iPhone Mail");
        let creds = vec![credential];
        let outcome = authenticate(&account(), &creds, LoginPurpose::Interactive, &plaintext)
            .expect("authentication runs");
        assert_eq!(outcome, Authentication::Denied);
    }

    #[test]
    fn the_primary_password_still_signs_in_a_mail_login_beside_app_passwords() {
        let (credential, _plaintext) = app(1, "client");
        let creds = vec![credential, primary("pass")];
        let outcome = authenticate(&account(), &creds, LoginPurpose::Mail, "pass")
            .expect("authentication runs");
        assert_eq!(
            outcome,
            Authentication::Granted(AuthenticatedBy::PrimaryPassword {
                mfa_required: false
            })
        );
    }

    #[test]
    fn a_disabled_account_is_refused_before_any_secret_is_checked() {
        let mut account = account();
        account.enabled = false;
        let creds = vec![primary("pass")];
        let outcome = authenticate(&account, &creds, LoginPurpose::Interactive, "pass")
            .expect("authentication runs");
        assert_eq!(outcome, Authentication::Denied);
    }

    #[test]
    fn an_account_with_no_accepted_credential_is_denied() {
        let creds: Vec<Credential> = vec![enabled_totp()];
        let outcome = authenticate(&account(), &creds, LoginPurpose::Interactive, "anything")
            .expect("authentication runs");
        assert_eq!(outcome, Authentication::Denied);
    }

    #[test]
    fn an_unknown_app_password_is_denied() {
        let (credential, _plaintext) = app(1, "phone");
        let creds = vec![credential];
        let outcome = authenticate(&account(), &creds, LoginPurpose::Mail, "not-the-secret")
            .expect("authentication runs");
        assert_eq!(outcome, Authentication::Denied);
    }

    #[test]
    fn the_first_matching_app_password_is_the_one_named() {
        let (first, first_plain) = app(1, "phone");
        let (second, _second_plain) = app(2, "laptop");
        let creds = vec![first, second];
        let outcome = authenticate(&account(), &creds, LoginPurpose::Mail, &first_plain)
            .expect("authentication runs");
        assert_eq!(
            outcome,
            Authentication::Granted(AuthenticatedBy::AppPassword { id: 1 })
        );
    }

    #[test]
    fn a_malformed_stored_hash_surfaces_as_an_error() {
        let creds = vec![Credential::AppPassword(AppPassword {
            id: 1,
            name: "client".to_string(),
            hash: "not-a-phc-string".to_string(),
            created_at: 0,
            last_used_at: None,
        })];
        let result = authenticate(&account(), &creds, LoginPurpose::Mail, "anything");
        assert!(matches!(result, Err(irixmail_core::Error::Internal(_))));
    }
}
