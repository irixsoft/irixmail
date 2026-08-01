use irixmail_core::Result;
use irixmail_store::{Flow, KeyPrefix, Store, Subspace};

const TAG_SUBSCRIPTION: u8 = 0x2a;

pub fn subscribe(store: &dyn Store, account_id: u32, name: &str) -> Result<bool> {
    let key = subscription_key(account_id, name);
    if store.exists(&key)? {
        return Ok(false);
    }
    store.put(&key, &[])?;
    Ok(true)
}

pub fn unsubscribe(store: &dyn Store, account_id: u32, name: &str) -> Result<bool> {
    let key = subscription_key(account_id, name);
    if !store.exists(&key)? {
        return Ok(false);
    }
    store.delete(&key)?;
    Ok(true)
}

pub fn subscriptions(store: &dyn Store, account_id: u32) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let account_be = account_id.to_be_bytes();
    store.iterate(&KeyPrefix::subspace(Subspace::Registry), &mut |key, _| {
        if key.len() > 6 && key[1] == TAG_SUBSCRIPTION && key[2..6] == account_be {
            if let Ok(name) = std::str::from_utf8(&key[6..]) {
                names.push(name.to_string());
            }
        }
        Ok(Flow::Continue)
    })?;
    Ok(names)
}

fn subscription_key(account_id: u32, name: &str) -> Vec<u8> {
    let name = normalize(name);
    let mut key = Vec::with_capacity(6 + name.len());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_SUBSCRIPTION);
    key.extend_from_slice(&account_id.to_be_bytes());
    key.extend_from_slice(name.as_bytes());
    key
}

fn normalize(name: &str) -> String {
    if name.eq_ignore_ascii_case("INBOX") {
        "INBOX".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

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

        fn batch(&self, ops: &[irixmail_store::WriteOp]) -> Result<()> {
            for op in ops {
                match op {
                    irixmail_store::WriteOp::Set { key, value } => self.put(key, value)?,
                    irixmail_store::WriteOp::Delete { key } => self.delete(key)?,
                    irixmail_store::WriteOp::Add { .. } => {}
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            Ok(0)
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            Ok(0)
        }
    }

    #[test]
    fn subscribing_persists_and_lists_per_account() {
        let store = MemStore::default();
        assert!(subscribe(&store, 1, "Work").unwrap());
        assert!(!subscribe(&store, 1, "Work").unwrap());
        assert!(subscribe(&store, 2, "Other").unwrap());

        assert_eq!(subscriptions(&store, 1).unwrap(), vec!["Work"]);
        assert_eq!(subscriptions(&store, 2).unwrap(), vec!["Other"]);
    }

    #[test]
    fn unsubscribing_removes_only_an_existing_subscription() {
        let store = MemStore::default();
        assert!(!unsubscribe(&store, 1, "Work").unwrap());
        subscribe(&store, 1, "Work").unwrap();
        assert!(unsubscribe(&store, 1, "Work").unwrap());
        assert!(subscriptions(&store, 1).unwrap().is_empty());
    }

    #[test]
    fn inbox_is_normalized_case_insensitively() {
        let store = MemStore::default();
        assert!(subscribe(&store, 1, "inbox").unwrap());
        assert!(!subscribe(&store, 1, "INBOX").unwrap());
        assert_eq!(subscriptions(&store, 1).unwrap(), vec!["INBOX"]);
        assert!(unsubscribe(&store, 1, "Inbox").unwrap());
    }
}
