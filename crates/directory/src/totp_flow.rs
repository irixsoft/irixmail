use crate::account::Account;
use crate::authenticate::{authenticate, AuthenticatedBy, Authentication, LoginPurpose};
use crate::credential::{Credential, EncryptedSecret, Totp};
use crate::totp;
use irixmail_core::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginOutcome {
    Granted,
    SecondFactorRequired,
    Denied,
}

impl LoginOutcome {
    pub fn is_granted(&self) -> bool {
        matches!(self, LoginOutcome::Granted)
    }

    pub fn needs_second_factor(&self) -> bool {
        matches!(self, LoginOutcome::SecondFactorRequired)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeOutcome {
    Granted,
    Denied,
}

impl ChallengeOutcome {
    pub fn is_granted(&self) -> bool {
        matches!(self, ChallengeOutcome::Granted)
    }
}

pub fn begin(
    account: &Account,
    credentials: &[Credential],
    password: &str,
) -> Result<LoginOutcome> {
    let outcome = authenticate(account, credentials, LoginPurpose::Interactive, password)?;
    Ok(match outcome {
        Authentication::Granted(AuthenticatedBy::PrimaryPassword {
            mfa_required: false,
        }) => LoginOutcome::Granted,
        Authentication::Granted(AuthenticatedBy::PrimaryPassword { mfa_required: true }) => {
            LoginOutcome::SecondFactorRequired
        }
        Authentication::Granted(AuthenticatedBy::AppPassword { .. }) | Authentication::Denied => {
            LoginOutcome::Denied
        }
    })
}

pub fn complete<F>(
    credentials: &mut [Credential],
    code: &str,
    unix_time: u64,
    decrypt: F,
) -> Result<ChallengeOutcome>
where
    F: FnOnce(&EncryptedSecret) -> Result<Vec<u8>>,
{
    let Some(totp_factor) = enabled_totp_mut(credentials) else {
        return Ok(ChallengeOutcome::Denied);
    };

    let secret = decrypt(&totp_factor.secret)?;
    if totp::verify_code(&secret, code, unix_time)? {
        return Ok(ChallengeOutcome::Granted);
    }

    if totp::consume_recovery_code(&mut totp_factor.recovery_codes, code)? {
        return Ok(ChallengeOutcome::Granted);
    }

    Ok(ChallengeOutcome::Denied)
}

fn enabled_totp_mut(credentials: &mut [Credential]) -> Option<&mut Totp> {
    credentials
        .iter_mut()
        .find_map(|credential| match credential {
            Credential::Totp(totp) if totp.enabled => Some(totp),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Forwarding, Role, VacationResponder};
    use crate::app_password;
    use crate::credential::{EncryptedSecret, PrimaryPassword};
    use crate::password;
    use crate::totp as totp_service;
    use irixmail_core::Error;

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

    fn plain_secret(bytes: &[u8]) -> EncryptedSecret {
        EncryptedSecret {
            nonce: Vec::new(),
            ciphertext: bytes.to_vec(),
        }
    }

    fn decrypt(secret: &EncryptedSecret) -> Result<Vec<u8>> {
        Ok(secret.ciphertext.clone())
    }

    fn totp_factor(secret: &[u8], recovery_codes: Vec<String>) -> Credential {
        Credential::Totp(Totp {
            secret: plain_secret(secret),
            enabled: true,
            recovery_codes,
            enrolled_at: 0,
        })
    }

    fn current_code(secret: &[u8], unix_time: u64) -> String {
        for candidate in 0..1_000_000u32 {
            let code = format!("{candidate:06}");
            if totp_service::verify_code(secret, &code, unix_time).expect("verify runs") {
                return code;
            }
        }
        panic!("no code in the search space verified");
    }

    #[test]
    fn a_correct_password_without_a_factor_is_granted() {
        let creds = vec![primary("correct horse battery staple")];
        let outcome =
            begin(&account(), &creds, "correct horse battery staple").expect("begin runs");
        assert_eq!(outcome, LoginOutcome::Granted);
        assert!(outcome.is_granted());
        assert!(!outcome.needs_second_factor());
    }

    #[test]
    fn a_correct_password_with_a_factor_owes_a_second_factor() {
        let secret = totp_service::generate_secret().expect("secret generates");
        let creds = vec![primary("pass"), totp_factor(&secret, Vec::new())];
        let outcome = begin(&account(), &creds, "pass").expect("begin runs");
        assert_eq!(outcome, LoginOutcome::SecondFactorRequired);
        assert!(outcome.needs_second_factor());
        assert!(!outcome.is_granted());
    }

    #[test]
    fn a_disabled_factor_lets_the_password_alone_grant() {
        let secret = totp_service::generate_secret().expect("secret generates");
        let mut totp = totp_factor(&secret, Vec::new());
        if let Credential::Totp(factor) = &mut totp {
            factor.enabled = false;
        }
        let creds = vec![primary("pass"), totp];
        let outcome = begin(&account(), &creds, "pass").expect("begin runs");
        assert_eq!(outcome, LoginOutcome::Granted);
    }

    #[test]
    fn a_wrong_password_is_denied() {
        let creds = vec![primary("the-right-one")];
        let outcome = begin(&account(), &creds, "the-wrong-one").expect("begin runs");
        assert_eq!(outcome, LoginOutcome::Denied);
    }

    #[test]
    fn a_disabled_account_is_denied_at_the_password_step() {
        let mut account = account();
        account.enabled = false;
        let creds = vec![primary("pass")];
        let outcome = begin(&account, &creds, "pass").expect("begin runs");
        assert_eq!(outcome, LoginOutcome::Denied);
    }

    #[test]
    fn an_app_password_is_never_accepted_interactively() {
        let minted = app_password::generate(1, "iPhone Mail", 0).expect("app password generates");
        let creds = vec![Credential::AppPassword(minted.record)];
        let outcome = begin(&account(), &creds, &minted.plaintext).expect("begin runs");
        assert_eq!(outcome, LoginOutcome::Denied);
    }

    #[test]
    fn the_current_code_completes_the_challenge() {
        let secret = totp_service::generate_secret().expect("secret generates");
        let now = 1_700_000_000;
        let code = current_code(&secret, now);
        let mut creds = vec![primary("pass"), totp_factor(&secret, Vec::new())];
        let outcome = complete(&mut creds, &code, now, decrypt).expect("complete runs");
        assert_eq!(outcome, ChallengeOutcome::Granted);
        assert!(outcome.is_granted());
    }

    #[test]
    fn a_wrong_code_is_denied() {
        let secret = totp_service::generate_secret().expect("secret generates");
        let now = 1_700_000_000;
        let wrong = current_code(&secret, now + 600);
        let mut creds = vec![primary("pass"), totp_factor(&secret, Vec::new())];
        let outcome = complete(&mut creds, &wrong, now, decrypt).expect("complete runs");
        assert_eq!(outcome, ChallengeOutcome::Denied);
    }

    #[test]
    fn a_recovery_code_completes_the_challenge_and_is_consumed() {
        let secret = totp_service::generate_secret().expect("secret generates");
        let now = 1_700_000_000;
        let codes = totp_service::generate_recovery_codes().expect("codes generate");
        let used = codes.plaintext[0].clone();
        let mut creds = vec![primary("pass"), totp_factor(&secret, codes.hashes.clone())];

        let outcome = complete(&mut creds, &used, now, decrypt).expect("complete runs");
        assert_eq!(outcome, ChallengeOutcome::Granted);

        let remaining = match &creds[1] {
            Credential::Totp(factor) => factor.recovery_codes.len(),
            _ => panic!("the factor moved"),
        };
        assert_eq!(remaining, codes.hashes.len() - 1);

        let replay = complete(&mut creds, &used, now, decrypt).expect("complete runs");
        assert_eq!(replay, ChallengeOutcome::Denied);
    }

    #[test]
    fn a_challenge_against_an_account_without_an_enabled_factor_is_denied() {
        let mut creds = vec![primary("pass")];
        let outcome =
            complete(&mut creds, "123456", 1_700_000_000, decrypt).expect("complete runs");
        assert_eq!(outcome, ChallengeOutcome::Denied);
    }

    #[test]
    fn a_challenge_does_not_consult_a_disabled_factor() {
        let secret = totp_service::generate_secret().expect("secret generates");
        let now = 1_700_000_000;
        let code = current_code(&secret, now);
        let mut totp = totp_factor(&secret, Vec::new());
        if let Credential::Totp(factor) = &mut totp {
            factor.enabled = false;
        }
        let mut creds = vec![primary("pass"), totp];
        let outcome = complete(&mut creds, &code, now, decrypt).expect("complete runs");
        assert_eq!(outcome, ChallengeOutcome::Denied);
    }

    #[test]
    fn a_decryption_failure_surfaces_as_an_error() {
        let secret = totp_service::generate_secret().expect("secret generates");
        let mut creds = vec![primary("pass"), totp_factor(&secret, Vec::new())];
        let failing = |_: &EncryptedSecret| -> Result<Vec<u8>> {
            Err(Error::Internal("field key unavailable".to_string()))
        };
        let result = complete(&mut creds, "123456", 1_700_000_000, failing);
        assert!(matches!(result, Err(Error::Internal(_))));
    }

    #[test]
    fn a_too_short_secret_surfaces_as_invalid_input() {
        let mut creds = vec![primary("pass"), totp_factor(&[0u8; 2], Vec::new())];
        let result = complete(&mut creds, "123456", 1_700_000_000, decrypt);
        assert!(matches!(result, Err(Error::InvalidInput(_))));
    }
}
