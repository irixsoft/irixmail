use irixmail_core::Result;

use crate::key::{Collection, Key, Subspace};
use crate::traits_store::{Store, WriteOp};

const QUOTA_COLLECTION: Collection = Collection::Email;

const QUOTA_DOCUMENT_ID: u32 = 0;

const BYTES_SUFFIX: u8 = b'b';

const MESSAGES_SUFFIX: u8 = b'c';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuotaLimits {
    pub bytes: u64,
    pub messages: u64,
}

impl QuotaLimits {
    pub const UNLIMITED: QuotaLimits = QuotaLimits {
        bytes: 0,
        messages: 0,
    };

    pub fn is_bounded(self) -> bool {
        self.bytes != 0 || self.messages != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuotaUsage {
    pub bytes: u64,
    pub messages: u64,
}

impl QuotaUsage {
    pub fn fits(self, limits: QuotaLimits, bytes: u64, messages: u64) -> bool {
        let bytes_ok = limits.bytes == 0 || self.bytes.saturating_add(bytes) <= limits.bytes;
        let messages_ok =
            limits.messages == 0 || self.messages.saturating_add(messages) <= limits.messages;
        bytes_ok && messages_ok
    }
}

pub struct Quota<'a> {
    store: &'a dyn Store,
}

impl<'a> Quota<'a> {
    pub fn new(store: &'a dyn Store) -> Self {
        Self { store }
    }

    pub fn usage(&self, account_id: u32) -> Result<QuotaUsage> {
        let bytes = self.store.counter(&bytes_key(account_id))?.max(0) as u64;
        let messages = self.store.counter(&messages_key(account_id))?.max(0) as u64;
        Ok(QuotaUsage { bytes, messages })
    }

    pub fn fits(
        &self,
        account_id: u32,
        limits: QuotaLimits,
        bytes: u64,
        messages: u64,
    ) -> Result<bool> {
        if !limits.is_bounded() {
            return Ok(true);
        }
        let usage = self.usage(account_id)?;
        Ok(usage.fits(limits, bytes, messages))
    }

    pub fn adjust(
        &self,
        account_id: u32,
        bytes_delta: i64,
        messages_delta: i64,
    ) -> Result<QuotaUsage> {
        let bytes = self
            .store
            .add_and_get(&bytes_key(account_id), bytes_delta)?
            .max(0) as u64;
        let messages = self
            .store
            .add_and_get(&messages_key(account_id), messages_delta)?
            .max(0) as u64;
        Ok(QuotaUsage { bytes, messages })
    }

    pub fn adjust_ops(
        &self,
        account_id: u32,
        bytes_delta: i64,
        messages_delta: i64,
    ) -> Vec<WriteOp> {
        let mut ops = Vec::with_capacity(2);
        if bytes_delta != 0 {
            ops.push(WriteOp::Add {
                key: bytes_key(account_id),
                by: bytes_delta,
            });
        }
        if messages_delta != 0 {
            ops.push(WriteOp::Add {
                key: messages_key(account_id),
                by: messages_delta,
            });
        }
        ops
    }
}

fn bytes_key(account_id: u32) -> Vec<u8> {
    Key::new(
        Subspace::Counter,
        account_id,
        QUOTA_COLLECTION,
        QUOTA_DOCUMENT_ID,
    )
    .with_suffix(vec![BYTES_SUFFIX])
    .encode()
}

fn messages_key(account_id: u32) -> Vec<u8> {
    Key::new(
        Subspace::Counter,
        account_id,
        QUOTA_COLLECTION,
        QUOTA_DOCUMENT_ID,
    )
    .with_suffix(vec![MESSAGES_SUFFIX])
    .encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::KeyPrefix;
    use crate::traits_store::Flow;
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

    #[test]
    fn untouched_account_uses_nothing() {
        let store = MemStore::default();
        let quota = Quota::new(&store);
        assert_eq!(quota.usage(7).unwrap(), QuotaUsage::default());
    }

    #[test]
    fn adjust_raises_and_lowers_both_tallies() {
        let store = MemStore::default();
        let quota = Quota::new(&store);

        let after_first = quota.adjust(1, 500, 1).unwrap();
        assert_eq!(
            after_first,
            QuotaUsage {
                bytes: 500,
                messages: 1
            }
        );
        let after_second = quota.adjust(1, 1500, 1).unwrap();
        assert_eq!(
            after_second,
            QuotaUsage {
                bytes: 2000,
                messages: 2
            }
        );

        let after_delete = quota.adjust(1, -500, -1).unwrap();
        assert_eq!(
            after_delete,
            QuotaUsage {
                bytes: 1500,
                messages: 1
            }
        );
        assert_eq!(
            quota.usage(1).unwrap(),
            QuotaUsage {
                bytes: 1500,
                messages: 1
            }
        );
    }

    #[test]
    fn the_two_tallies_are_independent_per_account() {
        let store = MemStore::default();
        let quota = Quota::new(&store);

        quota.adjust(1, 100, 1).unwrap();
        quota.adjust(2, 999, 5).unwrap();

        assert_eq!(
            quota.usage(1).unwrap(),
            QuotaUsage {
                bytes: 100,
                messages: 1
            }
        );
        assert_eq!(
            quota.usage(2).unwrap(),
            QuotaUsage {
                bytes: 999,
                messages: 5
            }
        );
    }

    #[test]
    fn usage_never_reads_below_zero() {
        let store = MemStore::default();
        let quota = Quota::new(&store);

        quota.adjust(3, 100, 1).unwrap();
        let after = quota.adjust(3, -250, -3).unwrap();
        assert_eq!(after, QuotaUsage::default());
        assert_eq!(quota.usage(3).unwrap(), QuotaUsage::default());
    }

    #[test]
    fn adjust_ops_omit_zero_deltas() {
        let store = MemStore::default();
        let quota = Quota::new(&store);

        assert!(quota.adjust_ops(1, 0, 0).is_empty());
        assert_eq!(quota.adjust_ops(1, 10, 0).len(), 1);
        assert_eq!(quota.adjust_ops(1, 0, 1).len(), 1);
        assert_eq!(quota.adjust_ops(1, 10, 1).len(), 2);
    }

    #[test]
    fn adjust_ops_fold_into_a_batch_atomically() {
        let store = MemStore::default();
        let quota = Quota::new(&store);

        let mut ops = vec![WriteOp::Set {
            key: Key::new(Subspace::Property, 4, Collection::Email, 1).encode(),
            value: b"message".to_vec(),
        }];
        ops.extend(quota.adjust_ops(4, 2048, 1));
        store.batch(&ops).unwrap();

        assert_eq!(
            quota.usage(4).unwrap(),
            QuotaUsage {
                bytes: 2048,
                messages: 1
            }
        );
    }

    #[test]
    fn unlimited_quota_always_fits() {
        let store = MemStore::default();
        let quota = Quota::new(&store);
        quota.adjust(1, 1_000_000, 100).unwrap();

        assert!(quota
            .fits(1, QuotaLimits::UNLIMITED, u64::MAX, u64::MAX)
            .unwrap());
        assert!(!QuotaLimits::UNLIMITED.is_bounded());
    }

    #[test]
    fn byte_ceiling_is_enforced() {
        let store = MemStore::default();
        let quota = Quota::new(&store);
        quota.adjust(1, 900, 1).unwrap();
        let limits = QuotaLimits {
            bytes: 1000,
            messages: 0,
        };

        assert!(quota.fits(1, limits, 100, 1).unwrap());
        assert!(!quota.fits(1, limits, 101, 1).unwrap());
        assert!(quota.fits(1, limits, 50, 1_000_000).unwrap());
    }

    #[test]
    fn message_ceiling_is_enforced() {
        let store = MemStore::default();
        let quota = Quota::new(&store);
        quota.adjust(1, 10, 4).unwrap();
        let limits = QuotaLimits {
            bytes: 0,
            messages: 5,
        };

        assert!(quota.fits(1, limits, 10, 1).unwrap());
        assert!(!quota.fits(1, limits, 10, 2).unwrap());
        assert!(quota.fits(1, limits, u64::MAX - 10, 1).unwrap());
    }

    #[test]
    fn both_ceilings_must_be_satisfied() {
        let store = MemStore::default();
        let quota = Quota::new(&store);
        quota.adjust(1, 500, 2).unwrap();
        let limits = QuotaLimits {
            bytes: 1000,
            messages: 3,
        };

        assert!(quota.fits(1, limits, 400, 1).unwrap());
        assert!(!quota.fits(1, limits, 600, 1).unwrap());
        assert!(!quota.fits(1, limits, 100, 2).unwrap());
    }

    #[test]
    fn fits_uses_saturating_arithmetic() {
        let store = MemStore::default();
        let quota = Quota::new(&store);
        quota.adjust(1, 10, 1).unwrap();
        let limits = QuotaLimits {
            bytes: 1000,
            messages: 10,
        };

        assert!(!quota.fits(1, limits, u64::MAX, 1).unwrap());
        assert!(!QuotaUsage {
            bytes: 10,
            messages: 1
        }
        .fits(limits, u64::MAX, 1));
    }

    #[test]
    fn the_byte_and_message_keys_differ() {
        let bytes = bytes_key(42);
        let messages = messages_key(42);
        assert_ne!(bytes, messages);
        assert_eq!(bytes[0], Subspace::Counter.as_byte());
        assert_eq!(messages[0], Subspace::Counter.as_byte());
        assert_eq!(&bytes[1..5], &42u32.to_be_bytes());
        assert_eq!(&messages[1..5], &42u32.to_be_bytes());
        assert_eq!(bytes[..bytes.len() - 1], messages[..messages.len() - 1]);
    }
}
