use std::sync::Arc;

use irixmail_core::{Error, Result};
use irixmail_dns::dkim_keys::{generate_ed25519, DkimKey};
use irixmail_store::{Store, Subspace, WriteOp};

const TAG_DKIM_KEY: u8 = 0x27;

#[derive(Clone)]
pub struct DkimKeyRegistry {
    store: Arc<dyn Store>,
}

impl DkimKeyRegistry {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    pub fn get(&self, domain_id: u64) -> Result<Option<DkimKey>> {
        match self.store.get(&record_key(domain_id))? {
            Some(bytes) => Ok(Some(decode(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn set(&self, domain_id: u64, key: &DkimKey) -> Result<()> {
        self.store.batch(&[WriteOp::Set {
            key: record_key(domain_id),
            value: encode(key)?,
        }])
    }

    pub fn get_or_create(&self, domain_id: u64, selector: &str) -> Result<DkimKey> {
        if let Some(key) = self.get(domain_id)? {
            return Ok(key);
        }
        let key = generate_ed25519(selector)?;
        self.set(domain_id, &key)?;
        Ok(key)
    }

    pub fn remove(&self, domain_id: u64) -> Result<()> {
        self.store.batch(&[WriteOp::Delete {
            key: record_key(domain_id),
        }])
    }
}

fn record_key(domain_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + std::mem::size_of::<u64>());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_DKIM_KEY);
    key.extend_from_slice(&domain_id.to_be_bytes());
    key
}

fn encode(key: &DkimKey) -> Result<Vec<u8>> {
    serde_json::to_vec(key)
        .map_err(|err| Error::serialize(format!("could not encode DKIM key: {err}")))
}

fn decode(bytes: &[u8]) -> Result<DkimKey> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::serialize(format!("could not decode DKIM key: {err}")))
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
                    WriteOp::Add { .. } => unreachable!("the DKIM registry does not use counters"),
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the DKIM registry does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the DKIM registry does not use counters")
        }
    }

    fn registry() -> DkimKeyRegistry {
        DkimKeyRegistry::new(Arc::new(MemStore::default()))
    }

    #[test]
    fn a_domain_without_a_key_returns_none() {
        assert!(registry().get(1).unwrap().is_none());
    }

    #[test]
    fn get_or_create_persists_and_is_stable() {
        let registry = registry();
        let first = registry.get_or_create(1, "default").unwrap();
        let second = registry.get_or_create(1, "default").unwrap();
        assert_eq!(first.public_key_b64, second.public_key_b64);
        assert_eq!(first.selector, "default");
    }

    #[test]
    fn keys_are_isolated_per_domain() {
        let registry = registry();
        let one = registry.get_or_create(1, "default").unwrap();
        let two = registry.get_or_create(2, "default").unwrap();
        assert_ne!(one.public_key_b64, two.public_key_b64);
    }

    #[test]
    fn remove_clears_the_key() {
        let registry = registry();
        registry.get_or_create(1, "default").unwrap();
        registry.remove(1).unwrap();
        assert!(registry.get(1).unwrap().is_none());
    }
}
