use irixmail_core::Result;

use crate::key::KeyPrefix;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOp {
    Set { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
    Add { key: Vec<u8>, by: i64 },
}

impl WriteOp {
    pub fn key(&self) -> &[u8] {
        match self {
            WriteOp::Set { key, .. } | WriteOp::Delete { key } | WriteOp::Add { key, .. } => key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueAssert {
    pub key: Vec<u8>,
    pub expected: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Stop,
}

pub trait Store: Send + Sync {
    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    fn exists(&self, key: &[u8]) -> Result<bool> {
        Ok(self.get(key)?.is_some())
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<()>;

    fn delete(&self, key: &[u8]) -> Result<()>;

    #[allow(clippy::type_complexity)]
    fn iterate(
        &self,
        prefix: &KeyPrefix,
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
    ) -> Result<()>;

    // fallback skips keys below `start`; seek-capable backends override
    #[allow(clippy::type_complexity)]
    fn iterate_from(
        &self,
        prefix: &KeyPrefix,
        start: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
    ) -> Result<()> {
        self.iterate(prefix, &mut |key, value| {
            if key < start {
                return Ok(Flow::Continue);
            }
            visit(key, value)
        })
    }

    fn batch(&self, ops: &[WriteOp]) -> Result<()>;

    // check-then-write without isolation; backends with transactions override
    fn batch_conditional(&self, asserts: &[ValueAssert], ops: &[WriteOp]) -> Result<bool> {
        for assert in asserts {
            if self.get(&assert.key)? != assert.expected {
                return Ok(false);
            }
        }
        self.batch(ops)?;
        Ok(true)
    }

    fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64>;

    fn counter(&self, key: &[u8]) -> Result<i64>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Collection, Key, Subspace};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemStore {
        fn read_counter(map: &BTreeMap<Vec<u8>, Vec<u8>>, key: &[u8]) -> i64 {
            map.get(key)
                .map(|bytes| {
                    let mut array = [0u8; 8];
                    array.copy_from_slice(bytes);
                    i64::from_le_bytes(array)
                })
                .unwrap_or(0)
        }
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
                    WriteOp::Add { key, by } => {
                        let next = Self::read_counter(&map, key) + by;
                        map.insert(key.clone(), next.to_le_bytes().to_vec());
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
            let mut map = self.map.lock().unwrap();
            let next = Self::read_counter(&map, key) + by;
            map.insert(key.to_vec(), next.to_le_bytes().to_vec());
            Ok(next)
        }

        fn counter(&self, key: &[u8]) -> Result<i64> {
            let map = self.map.lock().unwrap();
            Ok(Self::read_counter(&map, key))
        }
    }

    fn email_key(account: u32, document: u32) -> Vec<u8> {
        Key::new(Subspace::Property, account, Collection::Email, document).encode()
    }

    #[test]
    fn put_get_delete_round_trip() {
        let store = MemStore::default();
        let key = email_key(1, 1);

        assert_eq!(store.get(&key).unwrap(), None);
        store.put(&key, b"hello").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(&b"hello"[..]));

        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        store.delete(&key).unwrap();
    }

    #[test]
    fn exists_default_tracks_presence() {
        let store = MemStore::default();
        let key = email_key(1, 7);

        assert!(!store.exists(&key).unwrap());
        store.put(&key, b"x").unwrap();
        assert!(store.exists(&key).unwrap());
    }

    #[test]
    fn iterate_visits_a_prefix_in_ascending_order() {
        let store = MemStore::default();
        for document in [3u32, 1, 2] {
            store
                .put(&email_key(1, document), &document.to_be_bytes())
                .unwrap();
        }
        store
            .put(
                &Key::new(Subspace::Property, 1, Collection::Mailbox, 9).encode(),
                b"mailbox",
            )
            .unwrap();

        let prefix = KeyPrefix::collection(Subspace::Property, 1, Collection::Email);
        let mut seen = Vec::new();
        store
            .iterate(&prefix, &mut |key, _value| {
                seen.push(key.to_vec());
                Ok(Flow::Continue)
            })
            .unwrap();

        assert_eq!(
            seen,
            vec![email_key(1, 1), email_key(1, 2), email_key(1, 3)]
        );
    }

    #[test]
    fn iterate_stops_early_when_asked() {
        let store = MemStore::default();
        for document in 1..=5u32 {
            store.put(&email_key(2, document), b"v").unwrap();
        }

        let prefix = KeyPrefix::collection(Subspace::Property, 2, Collection::Email);
        let mut count = 0;
        store
            .iterate(&prefix, &mut |_key, _value| {
                count += 1;
                if count == 2 {
                    Ok(Flow::Stop)
                } else {
                    Ok(Flow::Continue)
                }
            })
            .unwrap();

        assert_eq!(count, 2);
    }

    #[test]
    fn batch_applies_a_mixed_write_set() {
        let store = MemStore::default();
        let keep = email_key(1, 1);
        let drop = email_key(1, 2);
        store.put(&drop, b"stale").unwrap();
        let counter = Key::new(Subspace::Counter, 1, Collection::Email, 0).encode();

        store
            .batch(&[
                WriteOp::Set {
                    key: keep.clone(),
                    value: b"fresh".to_vec(),
                },
                WriteOp::Delete { key: drop.clone() },
                WriteOp::Add {
                    key: counter.clone(),
                    by: 3,
                },
            ])
            .unwrap();

        assert_eq!(store.get(&keep).unwrap().as_deref(), Some(&b"fresh"[..]));
        assert_eq!(store.get(&drop).unwrap(), None);
        assert_eq!(store.counter(&counter).unwrap(), 3);
    }

    #[test]
    fn counters_add_and_read_back() {
        let store = MemStore::default();
        let key = Key::new(Subspace::Counter, 5, Collection::Email, 0).encode();

        assert_eq!(store.counter(&key).unwrap(), 0);
        assert_eq!(store.add_and_get(&key, 10).unwrap(), 10);
        assert_eq!(store.add_and_get(&key, -4).unwrap(), 6);
        assert_eq!(store.counter(&key).unwrap(), 6);
    }

    #[test]
    fn write_op_reports_its_key() {
        let set = WriteOp::Set {
            key: vec![1, 2, 3],
            value: vec![9],
        };
        let delete = WriteOp::Delete { key: vec![4, 5] };
        let add = WriteOp::Add {
            key: vec![6],
            by: 1,
        };
        assert_eq!(set.key(), &[1, 2, 3]);
        assert_eq!(delete.key(), &[4, 5]);
        assert_eq!(add.key(), &[6]);
    }

    #[test]
    fn store_is_object_safe_and_shareable() {
        fn assert_shareable<T: Send + Sync + ?Sized>() {}
        assert_shareable::<dyn Store>();
        let _boxed: std::sync::Arc<dyn Store> = std::sync::Arc::new(MemStore::default());
    }
}
