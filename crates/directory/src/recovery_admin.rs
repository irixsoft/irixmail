use crate::account::Role;
use crate::password;
use irixmail_core::{Error, Result};

pub const ENV_VAR: &str = "IRIXMAIL_RECOVERY_ADMIN";

const FIELD_SEPARATOR: char = ':';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryAdmin {
    user: String,
    hash: String,
}

impl RecoveryAdmin {
    pub fn from_env() -> Result<Option<Self>> {
        match std::env::var(ENV_VAR) {
            Ok(value) => Self::parse(&value).map(Some),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(Error::config(format!(
                "{ENV_VAR} is set but its value is not valid unicode"
            ))),
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        let (user, hash) = value.split_once(FIELD_SEPARATOR).ok_or_else(|| {
            Error::config(format!("{ENV_VAR} must be in the form user:password-hash"))
        })?;

        if user.is_empty() {
            return Err(Error::config(format!("{ENV_VAR} has an empty identity")));
        }
        if hash.is_empty() {
            return Err(Error::config(format!(
                "{ENV_VAR} has an empty password hash"
            )));
        }

        Ok(Self {
            user: user.to_string(),
            hash: hash.to_string(),
        })
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn role(&self) -> Role {
        Role::Admin
    }

    pub fn matches(&self, identity: &str) -> bool {
        constant_time_eq(self.user.as_bytes(), identity.as_bytes())
    }

    pub fn verify(&self, identity: &str, secret: &str) -> Result<bool> {
        let secret_matches = password::verify(secret, &self.hash)?;
        Ok(self.matches(identity) && secret_matches)
    }
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

    fn admin(user: &str, secret: &str) -> RecoveryAdmin {
        let hash = password::hash(secret).expect("the secret hashes");
        RecoveryAdmin::parse(&format!("{user}:{hash}")).expect("the value parses")
    }

    #[test]
    fn a_well_formed_value_parses_into_its_two_halves() {
        let parsed = RecoveryAdmin::parse("root:$argon2id$v=19$m=19456,t=2,p=1$abc$def")
            .expect("the value parses");
        assert_eq!(parsed.user(), "root");
        assert_eq!(parsed.hash, "$argon2id$v=19$m=19456,t=2,p=1$abc$def");
    }

    #[test]
    fn only_the_first_colon_separates_the_identity_from_the_hash() {
        let parsed = RecoveryAdmin::parse("root:hash:with:colons").expect("the value parses");
        assert_eq!(parsed.user(), "root");
        assert_eq!(parsed.hash, "hash:with:colons");
    }

    #[test]
    fn a_value_with_no_colon_is_a_configuration_error() {
        let result = RecoveryAdmin::parse("rootonly");
        assert!(matches!(result, Err(Error::Config(_))));
    }

    #[test]
    fn an_empty_identity_is_a_configuration_error() {
        let result = RecoveryAdmin::parse(":some-hash");
        assert!(matches!(result, Err(Error::Config(_))));
    }

    #[test]
    fn an_empty_hash_is_a_configuration_error() {
        let result = RecoveryAdmin::parse("root:");
        assert!(matches!(result, Err(Error::Config(_))));
    }

    #[test]
    fn the_recovery_admin_always_holds_the_administrative_role() {
        let parsed = admin("root", "secret");
        assert_eq!(parsed.role(), Role::Admin);
    }

    #[test]
    fn the_configured_identity_matches_and_a_near_one_does_not() {
        let parsed = admin("recovery", "secret");
        assert!(parsed.matches("recovery"));
        assert!(!parsed.matches("recoverY"));
        assert!(!parsed.matches("recover"));
        assert!(!parsed.matches("recovery2"));
        assert!(!parsed.matches(""));
    }

    #[test]
    fn the_right_identity_and_secret_verify() {
        let parsed = admin("root", "break-glass-secret");
        assert!(parsed
            .verify("root", "break-glass-secret")
            .expect("verification runs"));
    }

    #[test]
    fn the_right_identity_with_a_wrong_secret_does_not_verify() {
        let parsed = admin("root", "break-glass-secret");
        assert!(!parsed
            .verify("root", "the-wrong-secret")
            .expect("verification runs"));
    }

    #[test]
    fn a_wrong_identity_with_the_right_secret_does_not_verify() {
        let parsed = admin("root", "break-glass-secret");
        assert!(!parsed
            .verify("intruder", "break-glass-secret")
            .expect("verification runs"));
    }

    #[test]
    fn a_malformed_configured_hash_surfaces_as_an_error_on_verify() {
        let parsed = RecoveryAdmin::parse("root:not-a-phc-string").expect("the value parses");
        let result = parsed.verify("root", "anything");
        assert!(matches!(result, Err(Error::Internal(_))));
    }

    #[test]
    fn parsing_succeeds_even_for_a_hash_the_verifier_will_reject() {
        let parsed = RecoveryAdmin::parse("root:not-a-phc-string").expect("the value parses");
        assert_eq!(parsed.user(), "root");
    }

    #[test]
    fn from_env_reports_none_when_the_variable_is_unset() {
        temp_env_unset(ENV_VAR);
        let result = RecoveryAdmin::from_env().expect("reading the environment succeeds");
        assert_eq!(result, None);
    }

    #[test]
    fn from_env_parses_a_set_variable() {
        let hash = password::hash("secret").expect("the secret hashes");
        temp_env_set(ENV_VAR, &format!("root:{hash}"));
        let result = RecoveryAdmin::from_env().expect("reading the environment succeeds");
        temp_env_unset(ENV_VAR);
        let parsed = result.expect("a recovery admin is configured");
        assert_eq!(parsed.user(), "root");
        assert_eq!(parsed.role(), Role::Admin);
    }

    #[test]
    fn from_env_surfaces_a_malformed_variable_as_a_configuration_error() {
        temp_env_set(ENV_VAR, "no-colon-here");
        let result = RecoveryAdmin::from_env();
        temp_env_unset(ENV_VAR);
        assert!(matches!(result, Err(Error::Config(_))));
    }

    fn temp_env_set(key: &str, value: &str) {
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn temp_env_unset(key: &str) {
        unsafe {
            std::env::remove_var(key);
        }
    }
}
