use std::future::Future;

use instant_acme::{Account, AccountCredentials, LetsEncrypt, NewAccount};

use irixmail_core::{Error, Result};

use crate::acme_persist::AcmePersist;

pub fn production_directory() -> &'static str {
    LetsEncrypt::Production.url()
}

pub fn staging_directory() -> &'static str {
    LetsEncrypt::Staging.url()
}

pub struct AcmeAccount {
    account: Account,
}

impl AcmeAccount {
    pub async fn create(
        directory_url: &str,
        contact_email: Option<&str>,
    ) -> Result<(Self, String)> {
        let contact = contact_email.map(|email| format!("mailto:{email}"));
        let contact_refs: Vec<&str> = contact.as_deref().into_iter().collect();
        let new_account = NewAccount {
            contact: &contact_refs,
            terms_of_service_agreed: true,
            only_return_existing: false,
        };
        let (account, credentials) = Account::create(&new_account, directory_url, None)
            .await
            .map_err(|err| Error::internal(format!("ACME account creation failed: {err}")))?;
        let serialized = serde_json::to_string(&credentials).map_err(|err| {
            Error::internal(format!("could not serialize ACME credentials: {err}"))
        })?;
        Ok((Self { account }, serialized))
    }

    pub async fn from_serialized(serialized: &str) -> Result<Self> {
        let credentials: AccountCredentials = serde_json::from_str(serialized)
            .map_err(|err| Error::internal(format!("could not parse ACME credentials: {err}")))?;
        let account = Account::from_credentials(credentials)
            .await
            .map_err(|err| Error::internal(format!("could not restore ACME account: {err}")))?;
        Ok(Self { account })
    }

    pub fn account(&self) -> &Account {
        &self.account
    }
}

pub async fn load_or_create(
    persist: &AcmePersist,
    directory_url: &str,
    contact_email: Option<&str>,
) -> Result<AcmeAccount> {
    load_or_create_with(
        persist,
        |serialized| async move { AcmeAccount::from_serialized(&serialized).await },
        || async move { AcmeAccount::create(directory_url, contact_email).await },
    )
    .await
}

pub async fn load_or_create_with<A, R, C, RFut, CFut>(
    persist: &AcmePersist,
    restore: R,
    create: C,
) -> Result<A>
where
    R: FnOnce(String) -> RFut,
    RFut: Future<Output = Result<A>>,
    C: FnOnce() -> CFut,
    CFut: Future<Output = Result<(A, String)>>,
{
    if let Some(serialized) = persist.load()? {
        match restore(serialized).await {
            Ok(account) => return Ok(account),
            Err(err) => {
                tracing::warn!(error = %err, "stored ACME credentials were rejected; creating a new account");
            }
        }
    }
    let (account, serialized) = create().await?;
    if let Err(err) = persist.save(&serialized) {
        tracing::warn!(error = %err, "could not persist the ACME credentials");
    }
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_urls_point_at_lets_encrypt() {
        assert!(production_directory().contains("acme-v02.api.letsencrypt.org"));
        assert!(staging_directory().contains("acme-staging-v02.api.letsencrypt.org"));
    }

    #[tokio::test]
    async fn malformed_credentials_are_rejected() {
        assert!(AcmeAccount::from_serialized("not json").await.is_err());
    }

    fn temp_persist(tag: &str) -> AcmePersist {
        let dir = std::env::temp_dir().join(format!(
            "irixmail-acme-account-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        AcmePersist::new(dir)
    }

    #[tokio::test]
    async fn stored_credentials_are_restored_without_creating() {
        let persist = temp_persist("stored");
        persist.save("stored-creds").unwrap();

        let account = load_or_create_with(
            &persist,
            |serialized| async move { Ok(format!("restored:{serialized}")) },
            || async move { Err::<(String, String), _>(Error::internal("create must not run")) },
        )
        .await
        .unwrap();

        assert_eq!(account, "restored:stored-creds");
        assert_eq!(persist.load().unwrap().as_deref(), Some("stored-creds"));
    }

    #[tokio::test]
    async fn a_fresh_account_is_created_and_persisted() {
        let persist = temp_persist("fresh");

        let account = load_or_create_with(
            &persist,
            |_| async move { Err::<String, _>(Error::internal("nothing stored")) },
            || async move { Ok(("account".to_string(), "new-creds".to_string())) },
        )
        .await
        .unwrap();

        assert_eq!(account, "account");
        assert_eq!(persist.load().unwrap().as_deref(), Some("new-creds"));
    }

    #[tokio::test]
    async fn rejected_stored_credentials_fall_back_to_create_and_repersist() {
        let persist = temp_persist("rejected");
        persist.save("corrupt").unwrap();

        let account = load_or_create_with(
            &persist,
            |_| async move { Err::<String, _>(Error::internal("rejected")) },
            || async move { Ok(("account".to_string(), "replacement-creds".to_string())) },
        )
        .await
        .unwrap();

        assert_eq!(account, "account");
        assert_eq!(
            persist.load().unwrap().as_deref(),
            Some("replacement-creds")
        );
    }

    #[tokio::test]
    async fn a_failed_create_surfaces_the_error_and_stores_nothing() {
        let persist = temp_persist("failed");

        let result = load_or_create_with(
            &persist,
            |_| async move { Err::<String, _>(Error::internal("nothing stored")) },
            || async move { Err::<(String, String), _>(Error::internal("directory unreachable")) },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(persist.load().unwrap(), None);
    }
}
