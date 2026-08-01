use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use irixmail_core::Result;

use crate::key::{KeyPrefix, Subspace};
use crate::traits_store::{Flow, Store, WriteOp};

const TAG_EXPIRING: u8 = 0x40;

pub struct ExpiringStore {
    store: Arc<dyn Store>,
}

impl ExpiringStore {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    pub fn set(&self, key: &[u8], ttl: Duration) -> Result<()> {
        self.set_at(key, ttl, unix_now())
    }

    pub fn set_at(&self, key: &[u8], ttl: Duration, now: u64) -> Result<()> {
        let expiry = now.saturating_add(ttl.as_secs());
        self.store.put(&full_key(key), &expiry.to_be_bytes())
    }

    pub fn contains(&self, key: &[u8]) -> Result<bool> {
        self.contains_at(key, unix_now())
    }

    pub fn contains_at(&self, key: &[u8], now: u64) -> Result<bool> {
        match self.store.get(&full_key(key))? {
            Some(value) => Ok(decode_expiry(&value).is_some_and(|expiry| expiry > now)),
            None => Ok(false),
        }
    }

    pub fn sweep_expired(&self) -> Result<usize> {
        self.sweep_expired_at(unix_now())
    }

    pub fn sweep_expired_at(&self, now: u64) -> Result<usize> {
        let mut expired = Vec::new();
        self.store.iterate_from(
            &KeyPrefix::subspace(Subspace::Registry),
            &[Subspace::Registry.as_byte(), TAG_EXPIRING],
            &mut |key, value| {
                if key.len() < 2 || key[1] != TAG_EXPIRING {
                    return Ok(if key[1..] > [TAG_EXPIRING][..] {
                        Flow::Stop
                    } else {
                        Flow::Continue
                    });
                }
                if decode_expiry(value).is_none_or(|expiry| expiry <= now) {
                    expired.push(WriteOp::Delete { key: key.to_vec() });
                }
                Ok(Flow::Continue)
            },
        )?;
        let removed = expired.len();
        if removed > 0 {
            self.store.batch(&expired)?;
        }
        Ok(removed)
    }
}

fn full_key(key: &[u8]) -> Vec<u8> {
    let mut full = Vec::with_capacity(2 + key.len());
    full.push(Subspace::Registry.as_byte());
    full.push(TAG_EXPIRING);
    full.extend_from_slice(key);
    full
}

fn decode_expiry(value: &[u8]) -> Option<u64> {
    let bytes: [u8; 8] = value.try_into().ok()?;
    Some(u64::from_be_bytes(bytes))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
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
                    WriteOp::Add { .. } => unreachable!(),
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!()
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!()
        }
    }

    fn expiring() -> (Arc<MemStore>, ExpiringStore) {
        let mem = Arc::new(MemStore::default());
        let store = ExpiringStore::new(Arc::clone(&mem) as Arc<dyn Store>);
        (mem, store)
    }

    #[test]
    fn an_unexpired_key_is_found() {
        let (_, store) = expiring();
        store
            .set_at(b"gl:a@b/c@d", Duration::from_secs(3600), 1_000)
            .unwrap();

        assert!(store.contains_at(b"gl:a@b/c@d", 1_100).unwrap());
    }

    #[test]
    fn a_missing_key_reads_as_absent() {
        let (_, store) = expiring();
        assert!(!store.contains_at(b"never-set", 1_000).unwrap());
    }

    #[test]
    fn an_expired_key_reads_as_absent_before_any_sweep() {
        let (_, store) = expiring();
        store
            .set_at(b"gl:a@b/c@d", Duration::from_secs(60), 1_000)
            .unwrap();

        assert!(!store.contains_at(b"gl:a@b/c@d", 1_060).unwrap());
    }

    #[test]
    fn writing_an_existing_key_replaces_its_expiry() {
        let (_, store) = expiring();
        store.set_at(b"k", Duration::from_secs(10), 1_000).unwrap();
        store.set_at(b"k", Duration::from_secs(500), 1_000).unwrap();

        assert!(store.contains_at(b"k", 1_100).unwrap());
    }

    #[test]
    fn keys_live_under_the_registry_expiring_tag() {
        let (mem, store) = expiring();
        store
            .set_at(b"gl:x", Duration::from_secs(60), 1_000)
            .unwrap();

        let map = mem.map.lock().unwrap();
        let (key, value) = map.iter().next().unwrap();
        assert_eq!(key[0], Subspace::Registry.as_byte());
        assert_eq!(key[1], TAG_EXPIRING);
        assert_eq!(&key[2..], b"gl:x");
        assert_eq!(value.as_slice(), &1_060u64.to_be_bytes());
    }

    #[test]
    fn the_sweep_removes_only_expired_entries() {
        let (mem, store) = expiring();
        store
            .set_at(b"live", Duration::from_secs(3600), 1_000)
            .unwrap();
        store
            .set_at(b"dead-one", Duration::from_secs(10), 1_000)
            .unwrap();
        store
            .set_at(b"dead-two", Duration::from_secs(20), 1_000)
            .unwrap();

        assert_eq!(store.sweep_expired_at(2_000).unwrap(), 2);

        assert!(store.contains_at(b"live", 2_000).unwrap());
        assert_eq!(mem.map.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_sweep_of_an_all_live_set_removes_nothing() {
        let (_, store) = expiring();
        store
            .set_at(b"a", Duration::from_secs(3600), 1_000)
            .unwrap();
        store
            .set_at(b"b", Duration::from_secs(3600), 1_000)
            .unwrap();

        assert_eq!(store.sweep_expired_at(1_100).unwrap(), 0);
    }

    #[test]
    fn the_sweep_leaves_other_registry_keys_untouched() {
        let (mem, store) = expiring();
        let settings = crate::runtime_settings::settings_key();
        mem.put(&settings, b"{}").unwrap();
        store
            .set_at(b"dead", Duration::from_secs(1), 1_000)
            .unwrap();

        assert_eq!(store.sweep_expired_at(2_000).unwrap(), 1);
        assert_eq!(mem.get(&settings).unwrap().as_deref(), Some(&b"{}"[..]));
    }

    #[test]
    fn a_malformed_value_reads_as_absent_and_is_swept() {
        let (mem, store) = expiring();
        let mut key = vec![Subspace::Registry.as_byte(), TAG_EXPIRING];
        key.extend_from_slice(b"broken");
        mem.put(&key, b"not-a-u64").unwrap();

        assert!(!store.contains_at(b"broken", 0).unwrap());
        assert_eq!(store.sweep_expired_at(0).unwrap(), 1);
    }
}
