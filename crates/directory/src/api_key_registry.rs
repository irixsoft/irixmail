use std::sync::Arc;

use irixmail_core::{Error, IdGenerator, Result};
use irixmail_store::{Store, Subspace, WriteOp};

use crate::api_key;
use crate::credential::ApiKey;
use crate::secret_cipher::SecretCipher;

const TAG_API_KEY: u8 = 0x32;

#[derive(Clone)]
pub struct ApiKeyRegistry {
    store: Arc<dyn Store>,
    ids: Arc<IdGenerator>,
}

impl ApiKeyRegistry {
    pub fn new(store: Arc<dyn Store>, ids: Arc<IdGenerator>) -> Self {
        Self { store, ids }
    }

    pub fn create(&self, name: &str, cipher: &SecretCipher) -> Result<(ApiKey, String)> {
        let generated = api_key::generate()?;
        let id = self.ids.generate();
        let record = ApiKey {
            id,
            name: name.trim().to_string(),
            secret: cipher.encrypt(generated.plaintext.as_bytes())?,
            created_at: IdGenerator::timestamp_of(id) * 1_000,
            last_used_at: None,
        };
        let mut keys = self.list()?;
        keys.push(record.clone());
        self.write(&keys)?;
        Ok((record, generated.plaintext))
    }

    pub fn list(&self) -> Result<Vec<ApiKey>> {
        match self.store.get(&record_key())? {
            Some(bytes) => decode(&bytes),
            None => Ok(Vec::new()),
        }
    }

    pub fn revoke(&self, id: u64) -> Result<bool> {
        let mut keys = self.list()?;
        if !api_key::revoke(&mut keys, id) {
            return Ok(false);
        }
        self.write(&keys)?;
        Ok(true)
    }

    pub fn verify(&self, candidate: &str, cipher: &SecretCipher) -> Result<Option<ApiKey>> {
        let keys = self.list()?;
        Ok(api_key::verify_any(candidate, &keys, |secret| {
            String::from_utf8(cipher.decrypt(secret)?).map_err(|err| {
                Error::Internal(format!("a stored API key secret is not valid text: {err}"))
            })
        })?
        .cloned())
    }

    fn write(&self, keys: &[ApiKey]) -> Result<()> {
        let key = record_key();
        let op = if keys.is_empty() {
            WriteOp::Delete { key }
        } else {
            WriteOp::Set {
                key,
                value: encode(keys)?,
            }
        };
        self.store.batch(&[op])
    }
}

fn record_key() -> Vec<u8> {
    vec![Subspace::Registry.as_byte(), TAG_API_KEY]
}

fn encode(keys: &[ApiKey]) -> Result<Vec<u8>> {
    serde_json::to_vec(keys)
        .map_err(|err| Error::serialize(format!("could not encode API keys: {err}")))
}

fn decode(bytes: &[u8]) -> Result<Vec<ApiKey>> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::serialize(format!("could not decode API keys: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use irixmail_store::{Flow, KeyPrefix};

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
                if key.starts_with(&bound) && visit(key, value)? == Flow::Stop {
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
                    WriteOp::Add { .. } => unreachable!("API keys do not use counters"),
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("API keys do not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("API keys do not use counters")
        }
    }

    fn registry() -> (ApiKeyRegistry, Arc<MemStore>, SecretCipher) {
        let backing = Arc::new(MemStore::default());
        let registry = ApiKeyRegistry::new(
            Arc::clone(&backing) as Arc<dyn Store>,
            Arc::new(IdGenerator::new(0)),
        );
        let cipher =
            SecretCipher::from_master_key(&SecretCipher::generate_master_key().unwrap()).unwrap();
        (registry, backing, cipher)
    }

    #[test]
    fn an_empty_registry_lists_nothing() {
        let (registry, _, _) = registry();
        assert!(registry.list().unwrap().is_empty());
    }

    #[test]
    fn a_created_key_persists_and_verifies() {
        let (registry, _, cipher) = registry();
        let (record, plaintext) = registry.create("ci", &cipher).unwrap();
        let listed = registry.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "ci");
        let found = registry.verify(&plaintext, &cipher).unwrap().unwrap();
        assert_eq!(found.id, record.id);
    }

    #[test]
    fn the_secret_is_not_persisted_in_plaintext() {
        let (registry, backing, cipher) = registry();
        let (_, plaintext) = registry.create("ci", &cipher).unwrap();
        let raw = backing
            .map
            .lock()
            .unwrap()
            .get(&record_key())
            .cloned()
            .expect("the key list persisted");
        assert!(
            !String::from_utf8_lossy(&raw).contains(&plaintext),
            "the API key secret was persisted in plaintext"
        );
    }

    #[test]
    fn a_wrong_candidate_does_not_verify() {
        let (registry, _, cipher) = registry();
        registry.create("ci", &cipher).unwrap();
        assert!(registry.verify("not-the-key", &cipher).unwrap().is_none());
    }

    #[test]
    fn a_revoked_key_no_longer_verifies() {
        let (registry, _, cipher) = registry();
        let (record, plaintext) = registry.create("ci", &cipher).unwrap();
        assert!(registry.revoke(record.id).unwrap());
        assert!(registry.verify(&plaintext, &cipher).unwrap().is_none());
        assert!(registry.list().unwrap().is_empty());
    }

    #[test]
    fn revoking_an_unknown_id_reports_no_change() {
        let (registry, _, cipher) = registry();
        registry.create("ci", &cipher).unwrap();
        assert!(!registry.revoke(999).unwrap());
        assert_eq!(registry.list().unwrap().len(), 1);
    }

    #[test]
    fn each_key_verifies_independently() {
        let (registry, _, cipher) = registry();
        let (first, first_plain) = registry.create("ci", &cipher).unwrap();
        let (second, second_plain) = registry.create("backup", &cipher).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(
            registry.verify(&first_plain, &cipher).unwrap().unwrap().id,
            first.id
        );
        assert_eq!(
            registry.verify(&second_plain, &cipher).unwrap().unwrap().id,
            second.id
        );
    }

    #[test]
    fn keys_persist_across_registry_instances_over_one_store() {
        let backing = Arc::new(MemStore::default());
        let ids = Arc::new(IdGenerator::new(0));
        let cipher =
            SecretCipher::from_master_key(&SecretCipher::generate_master_key().unwrap()).unwrap();
        let first = ApiKeyRegistry::new(Arc::clone(&backing) as Arc<dyn Store>, Arc::clone(&ids));
        let (_, plaintext) = first.create("ci", &cipher).unwrap();

        let second = ApiKeyRegistry::new(backing as Arc<dyn Store>, ids);
        assert!(second.verify(&plaintext, &cipher).unwrap().is_some());
    }
}
