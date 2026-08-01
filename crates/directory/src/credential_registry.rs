use std::sync::Arc;

use irixmail_core::{Error, IdGenerator, Result};
use irixmail_store::{Store, Subspace, WriteOp};

use crate::credential::{AppPassword, Credential, PrimaryPassword, Totp};

const TAG_CREDENTIAL: u8 = 0x22;

#[derive(Clone)]
pub struct CredentialRegistry {
    store: Arc<dyn Store>,
    ids: Arc<IdGenerator>,
}

impl CredentialRegistry {
    pub fn new(store: Arc<dyn Store>, ids: Arc<IdGenerator>) -> Self {
        Self { store, ids }
    }

    pub fn list(&self, account_id: u64) -> Result<Vec<Credential>> {
        match self.store.get(&record_key(account_id))? {
            Some(bytes) => decode(&bytes),
            None => Ok(Vec::new()),
        }
    }

    pub fn set_primary_password(&self, account_id: u64, hash: impl Into<String>) -> Result<()> {
        let mut credentials = self.list(account_id)?;
        credentials.retain(|credential| !matches!(credential, Credential::PrimaryPassword(_)));
        credentials.push(Credential::PrimaryPassword(PrimaryPassword {
            hash: hash.into(),
            updated_at: self.now_ms(),
        }));
        self.write(account_id, &credentials)
    }

    pub fn add_app_password(&self, account_id: u64, app_password: AppPassword) -> Result<()> {
        let mut credentials = self.list(account_id)?;
        credentials.push(Credential::AppPassword(app_password));
        self.write(account_id, &credentials)
    }

    pub fn list_app_passwords(&self, account_id: u64) -> Result<Vec<AppPassword>> {
        Ok(self
            .list(account_id)?
            .into_iter()
            .filter_map(|credential| match credential {
                Credential::AppPassword(record) => Some(record),
                _ => None,
            })
            .collect())
    }

    pub fn revoke_app_password(&self, account_id: u64, id: u64) -> Result<bool> {
        let mut credentials = self.list(account_id)?;
        let before = credentials.len();
        credentials.retain(
            |credential| !matches!(credential, Credential::AppPassword(record) if record.id == id),
        );
        if credentials.len() == before {
            return Ok(false);
        }
        self.write(account_id, &credentials)?;
        Ok(true)
    }

    pub fn set_totp(&self, account_id: u64, totp: Totp) -> Result<()> {
        let mut credentials = self.list(account_id)?;
        credentials.retain(|credential| !matches!(credential, Credential::Totp(_)));
        credentials.push(Credential::Totp(totp));
        self.write(account_id, &credentials)
    }

    pub fn clear_totp(&self, account_id: u64) -> Result<()> {
        let mut credentials = self.list(account_id)?;
        let before = credentials.len();
        credentials.retain(|credential| !matches!(credential, Credential::Totp(_)));
        if credentials.len() != before {
            self.write(account_id, &credentials)?;
        }
        Ok(())
    }

    pub fn remove_all(&self, account_id: u64) -> Result<()> {
        self.store.batch(&[WriteOp::Delete {
            key: record_key(account_id),
        }])
    }

    fn write(&self, account_id: u64, credentials: &[Credential]) -> Result<()> {
        let key = record_key(account_id);
        let op = if credentials.is_empty() {
            WriteOp::Delete { key }
        } else {
            WriteOp::Set {
                key,
                value: encode(credentials)?,
            }
        };
        self.store.batch(&[op])
    }

    fn now_ms(&self) -> u64 {
        IdGenerator::timestamp_of(self.ids.generate()) * 1_000
    }
}

fn record_key(account_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + std::mem::size_of::<u64>());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_CREDENTIAL);
    key.extend_from_slice(&account_id.to_be_bytes());
    key
}

fn encode(credentials: &[Credential]) -> Result<Vec<u8>> {
    serde_json::to_vec(credentials)
        .map_err(|err| Error::serialize(format!("could not encode credentials: {err}")))
}

fn decode(bytes: &[u8]) -> Result<Vec<Credential>> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::serialize(format!("could not decode credentials: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use irixmail_store::{Flow, KeyPrefix};

    use crate::app_password;

    #[derive(Default)]
    struct MemStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl Store for MemStore {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            Ok(self.map.lock().unwrap().get(key).cloned())
        }

        fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
            self.map
                .lock()
                .unwrap()
                .insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &[u8]) -> Result<()> {
            self.map.lock().unwrap().remove(key);
            Ok(())
        }

        fn iterate(
            &self,
            prefix: &KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            let bound = prefix.encode();
            let map = self.map.lock().unwrap();
            for (key, value) in map.iter() {
                if !key.starts_with(&bound) {
                    continue;
                }
                if visit(key, value)? == Flow::Stop {
                    break;
                }
            }
            Ok(())
        }

        fn batch(&self, ops: &[WriteOp]) -> Result<()> {
            let mut map = self.map.lock().unwrap();
            for op in ops {
                match op {
                    WriteOp::Set { key, value } => {
                        map.insert(key.clone(), value.clone());
                    }
                    WriteOp::Delete { key } => {
                        map.remove(key);
                    }
                    WriteOp::Add { .. } => {
                        unreachable!("the credential registry does not use counters")
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the credential registry does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the credential registry does not use counters")
        }
    }

    fn registry() -> CredentialRegistry {
        CredentialRegistry::new(Arc::new(MemStore::default()), Arc::new(IdGenerator::new(0)))
    }

    #[test]
    fn an_account_without_credentials_lists_nothing() {
        let registry = registry();
        assert!(registry.list(1).unwrap().is_empty());
        assert!(registry.list_app_passwords(1).unwrap().is_empty());
    }

    #[test]
    fn a_primary_password_round_trips() {
        let registry = registry();
        registry.set_primary_password(1, "$argon2id$one").unwrap();
        let listed = registry.list(1).unwrap();
        assert_eq!(listed.len(), 1);
        match &listed[0] {
            Credential::PrimaryPassword(primary) => assert_eq!(primary.hash, "$argon2id$one"),
            other => panic!("expected a primary password, got {other:?}"),
        }
    }

    #[test]
    fn setting_a_primary_password_replaces_the_previous_one() {
        let registry = registry();
        registry.set_primary_password(1, "$argon2id$one").unwrap();
        registry.set_primary_password(1, "$argon2id$two").unwrap();
        let primaries: Vec<_> = registry
            .list(1)
            .unwrap()
            .into_iter()
            .filter(|credential| matches!(credential, Credential::PrimaryPassword(_)))
            .collect();
        assert_eq!(primaries.len(), 1);
        match &primaries[0] {
            Credential::PrimaryPassword(primary) => assert_eq!(primary.hash, "$argon2id$two"),
            other => panic!("expected a primary password, got {other:?}"),
        }
    }

    #[test]
    fn an_app_password_is_added_and_listed() {
        let registry = registry();
        let minted = app_password::generate(7, "iPhone", 1_000).unwrap();
        registry.add_app_password(1, minted.record.clone()).unwrap();
        let listed = registry.list_app_passwords(1).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, 7);
        assert_eq!(listed[0].name, "iPhone");
    }

    #[test]
    fn revoking_an_app_password_removes_only_that_record() {
        let registry = registry();
        registry
            .add_app_password(1, app_password::generate(1, "phone", 0).unwrap().record)
            .unwrap();
        registry
            .add_app_password(1, app_password::generate(2, "laptop", 0).unwrap().record)
            .unwrap();
        assert!(registry.revoke_app_password(1, 1).unwrap());
        let remaining = registry.list_app_passwords(1).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, 2);
    }

    #[test]
    fn revoking_an_unknown_app_password_reports_no_change() {
        let registry = registry();
        registry
            .add_app_password(1, app_password::generate(1, "phone", 0).unwrap().record)
            .unwrap();
        assert!(!registry.revoke_app_password(1, 999).unwrap());
        assert_eq!(registry.list_app_passwords(1).unwrap().len(), 1);
    }

    #[test]
    fn a_primary_password_and_app_passwords_coexist() {
        let registry = registry();
        registry
            .set_primary_password(1, "$argon2id$primary")
            .unwrap();
        registry
            .add_app_password(1, app_password::generate(5, "client", 0).unwrap().record)
            .unwrap();
        assert!(registry.revoke_app_password(1, 5).unwrap());
        let listed = registry.list(1).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(matches!(listed[0], Credential::PrimaryPassword(_)));
    }

    #[test]
    fn remove_all_clears_every_credential() {
        let registry = registry();
        registry
            .set_primary_password(1, "$argon2id$primary")
            .unwrap();
        registry
            .add_app_password(1, app_password::generate(5, "client", 0).unwrap().record)
            .unwrap();
        registry.remove_all(1).unwrap();
        assert!(registry.list(1).unwrap().is_empty());
    }

    #[test]
    fn credentials_persist_across_registry_instances_over_one_store() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let ids = Arc::new(IdGenerator::new(0));
        let first = CredentialRegistry::new(Arc::clone(&store), Arc::clone(&ids));
        first.set_primary_password(42, "$argon2id$shared").unwrap();

        let second = CredentialRegistry::new(store, ids);
        let listed = second.list(42).unwrap();
        assert_eq!(listed.len(), 1);
        match &listed[0] {
            Credential::PrimaryPassword(primary) => assert_eq!(primary.hash, "$argon2id$shared"),
            other => panic!("expected a primary password, got {other:?}"),
        }
    }

    #[test]
    fn a_totp_secret_is_encrypted_at_rest_and_decrypts_on_read() {
        use crate::secret_cipher::SecretCipher;
        use crate::totp;

        let backing = Arc::new(MemStore::default());
        let registry = CredentialRegistry::new(
            Arc::clone(&backing) as Arc<dyn Store>,
            Arc::new(IdGenerator::new(0)),
        );
        let cipher =
            SecretCipher::from_master_key(&SecretCipher::generate_master_key().unwrap()).unwrap();
        let plain = totp::generate_secret().unwrap();

        registry
            .set_totp(
                1,
                Totp {
                    secret: cipher.encrypt(&plain).unwrap(),
                    enabled: true,
                    recovery_codes: Vec::new(),
                    enrolled_at: 0,
                },
            )
            .unwrap();

        let raw = backing
            .map
            .lock()
            .unwrap()
            .get(&record_key(1))
            .cloned()
            .expect("the credential record persisted");
        let rendered = String::from_utf8(raw).expect("the record is JSON text");
        let plain_rendered = serde_json::to_string(&plain).unwrap();
        assert!(
            !rendered.contains(&plain_rendered),
            "the TOTP secret was persisted in plaintext"
        );

        let stored = match &registry.list(1).unwrap()[0] {
            Credential::Totp(totp) => totp.secret.clone(),
            other => panic!("expected a totp credential, got {other:?}"),
        };
        assert_eq!(cipher.decrypt(&stored).unwrap(), plain);
    }

    #[test]
    fn the_accounts_credentials_are_isolated_from_other_accounts() {
        let registry = registry();
        registry.set_primary_password(1, "$argon2id$one").unwrap();
        registry.set_primary_password(2, "$argon2id$two").unwrap();
        assert!(registry.list(3).unwrap().is_empty());
        assert_eq!(registry.list(1).unwrap().len(), 1);
        assert_eq!(registry.list(2).unwrap().len(), 1);
    }
}
