pub mod account;
pub mod account_registry;
pub mod address;
pub mod address_index;
pub mod api_key;
pub mod api_key_registry;
pub mod app_password;
pub mod authenticate;
pub mod authz;
pub mod credential;
pub mod credential_registry;
pub mod dkim_registry;
pub mod domain;
pub mod domain_registry;
pub mod ip_rules;
pub mod login;
pub mod password;
pub mod recovery_admin;
pub mod secret_cipher;
pub mod server;
pub mod throttle;
pub mod totp;
pub mod totp_flow;

pub use account::{Account, Forwarding, Role, VacationResponder};
pub use account_registry::AccountRegistry;
pub use address::{AddressEntry, Target};
pub use address_index::AddressIndex;
pub use api_key::GeneratedApiKey;
pub use api_key_registry::ApiKeyRegistry;
pub use app_password::GeneratedAppPassword;
pub use authenticate::{
    authenticate, authenticate_blocking, AuthenticatedBy, Authentication, LoginPurpose,
};
pub use authz::{authorize, Access};
pub use credential::{
    ApiKey, AppPassword, Credential, CredentialKind, EncryptedSecret, PrimaryPassword, Totp,
};
pub use credential_registry::CredentialRegistry;
pub use dkim_registry::DkimKeyRegistry;
pub use domain::{DnsRecordKind, DnsStatus, Domain};
pub use domain_registry::DomainRegistry;
pub use ip_rules::{IpAction, IpRule, IpRuleRegistry};
pub use login::{attempt_login, attempt_login_blocking, LoginAttempt};
pub use recovery_admin::RecoveryAdmin;
pub use secret_cipher::SecretCipher;
pub use server::Directory;
pub use throttle::{Throttle, ThrottlePolicy, DEFAULT_MAX_FAILURES, DEFAULT_WINDOW};
pub use totp::RecoveryCodes;
pub use totp_flow::{ChallengeOutcome, LoginOutcome};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
