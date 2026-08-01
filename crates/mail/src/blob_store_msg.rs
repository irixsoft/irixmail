use irixmail_core::Result;
use irixmail_store::{BlobHash, BlobStore, Collection, Key, Store, Subspace, WriteOp};

const BLOB_REF_ACCOUNT: u32 = 0;

const BLOB_REF_COLLECTION: Collection = Collection::Email;

const BLOB_REF_DOCUMENT_ID: u32 = 0;

pub(crate) fn ref_count_key(hash: &BlobHash) -> Vec<u8> {
    Key::new(
        Subspace::BlobRef,
        BLOB_REF_ACCOUNT,
        BLOB_REF_COLLECTION,
        BLOB_REF_DOCUMENT_ID,
    )
    .with_suffix(hash.as_bytes().to_vec())
    .encode()
}

pub fn store_message(store: &dyn Store, blobs: &dyn BlobStore, raw: &[u8]) -> Result<BlobHash> {
    let hash = blobs.put(raw)?;
    store.add_and_get(&ref_count_key(&hash), 1)?;
    Ok(hash)
}

pub fn store_blob(blobs: &dyn BlobStore, raw: &[u8]) -> Result<BlobHash> {
    blobs.put(raw)
}

const RESERVATION_DOCUMENT_ID: u32 = 1;

pub const RESERVATION_TTL_SECS: u64 = 24 * 60 * 60;

pub(crate) fn reservation_key(account_id: u32, hash: &BlobHash) -> Vec<u8> {
    Key::new(
        Subspace::BlobRef,
        account_id,
        BLOB_REF_COLLECTION,
        RESERVATION_DOCUMENT_ID,
    )
    .with_suffix(hash.as_bytes().to_vec())
    .encode()
}

pub(crate) fn is_reservation_document(document: u32) -> bool {
    document == RESERVATION_DOCUMENT_ID
}

pub(crate) fn split_blob_ref_key(key: &[u8]) -> Option<(u32, u32, BlobHash)> {
    let prefix_len = ref_count_key(&BlobHash::from_bytes(Vec::new())).len();
    if key.len() <= prefix_len {
        return None;
    }
    let account = u32::from_be_bytes(key[1..5].try_into().ok()?);
    let document = u32::from_be_bytes(key[6..10].try_into().ok()?);
    Some((
        account,
        document,
        BlobHash::from_bytes(key[prefix_len..].to_vec()),
    ))
}

// blob first, then the reservation, so a crash strands a sweepable file
// rather than reserving a hash that was never written
pub fn reserve_upload(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    account_id: u32,
    raw: &[u8],
    now: u64,
) -> Result<BlobHash> {
    let hash = blobs.put(raw)?;
    store.put(
        &reservation_key(account_id, &hash),
        &(now + RESERVATION_TTL_SECS).to_le_bytes(),
    )?;
    Ok(hash)
}

const LINK_DOCUMENT_ID: u32 = 2;

pub(crate) fn is_link_document(document: u32) -> bool {
    document == LINK_DOCUMENT_ID
}

pub(crate) fn account_link_key(account_id: u32, hash: &BlobHash) -> Vec<u8> {
    Key::new(
        Subspace::BlobRef,
        account_id,
        BLOB_REF_COLLECTION,
        LINK_DOCUMENT_ID,
    )
    .with_suffix(hash.as_bytes().to_vec())
    .encode()
}

pub fn account_link_op(account_id: u32, hash: &BlobHash, by: i64) -> WriteOp {
    WriteOp::Add {
        key: account_link_key(account_id, hash),
        by,
    }
}

pub fn account_references_blob(
    store: &dyn Store,
    account_id: u32,
    hash: &BlobHash,
) -> Result<bool> {
    Ok(store.counter(&account_link_key(account_id, hash))? > 0)
}

pub fn has_live_reservation(
    store: &dyn Store,
    account_id: u32,
    hash: &BlobHash,
    now: u64,
) -> Result<bool> {
    match store.get(&reservation_key(account_id, hash))? {
        Some(bytes) => {
            let Ok(bytes) = <[u8; 8]>::try_from(bytes.as_slice()) else {
                return Ok(false);
            };
            Ok(u64::from_le_bytes(bytes) > now)
        }
        None => Ok(false),
    }
}

pub fn reference_op(hash: &BlobHash) -> WriteOp {
    WriteOp::Add {
        key: ref_count_key(hash),
        by: 1,
    }
}

pub fn add_reference(store: &dyn Store, hash: &BlobHash) -> Result<i64> {
    store.add_and_get(&ref_count_key(hash), 1)
}

// physical deletion is left to the purge sweep so a concurrent dedup delivery
// can never lose a blob it just re-referenced
pub fn release_message(store: &dyn Store, hash: &BlobHash) -> Result<i64> {
    let remaining = store.add_and_get(&ref_count_key(hash), -1)?;
    Ok(remaining.max(0))
}

pub fn reference_count(store: &dyn Store, hash: &BlobHash) -> Result<i64> {
    store.counter(&ref_count_key(hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_store::{Flow, KeyPrefix, WriteOp};
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

    #[derive(Default)]
    struct MemBlobStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemBlobStore {
        fn digest(bytes: &[u8]) -> BlobHash {
            let sum = bytes
                .iter()
                .fold(0u32, |acc, b| acc.wrapping_add(*b as u32));
            let mut raw = (bytes.len() as u32).to_be_bytes().to_vec();
            raw.extend_from_slice(&sum.to_be_bytes());
            BlobHash::from_bytes(raw)
        }
    }

    impl BlobStore for MemBlobStore {
        fn get(&self, hash: &BlobHash, range: std::ops::Range<usize>) -> Result<Option<Vec<u8>>> {
            let map = self.map.lock().unwrap();
            let Some(data) = map.get(hash.as_bytes()) else {
                return Ok(None);
            };
            let start = range.start.min(data.len());
            let end = range.end.min(data.len()).max(start);
            Ok(Some(data[start..end].to_vec()))
        }

        fn put(&self, bytes: &[u8]) -> Result<BlobHash> {
            let hash = Self::digest(bytes);
            self.map
                .lock()
                .unwrap()
                .insert(hash.as_bytes().to_vec(), bytes.to_vec());
            Ok(hash)
        }

        fn delete(&self, hash: &BlobHash) -> Result<()> {
            self.map.lock().unwrap().remove(hash.as_bytes());
            Ok(())
        }
    }

    const MESSAGE: &[u8] = b"Subject: Hello\r\nFrom: alice@example.com\r\n\r\nbody\r\n";

    #[test]
    fn storing_a_message_persists_the_bytes_and_one_reference() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();

        let hash = store_message(&store, &blobs, MESSAGE).expect("store");

        assert_eq!(blobs.get_all(&hash).unwrap().as_deref(), Some(MESSAGE));
        assert_eq!(reference_count(&store, &hash).unwrap(), 1);
    }

    #[test]
    fn identical_messages_dedup_to_one_blob_with_two_references() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();

        let first = store_message(&store, &blobs, MESSAGE).expect("first");
        let second = store_message(&store, &blobs, MESSAGE).expect("second");

        assert_eq!(first, second);
        assert_eq!(reference_count(&store, &first).unwrap(), 2);
    }

    #[test]
    fn distinct_messages_get_separate_blobs() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();

        let one = store_message(&store, &blobs, MESSAGE).expect("one");
        let other =
            store_message(&store, &blobs, b"Subject: Other\r\n\r\ndifferent\r\n").expect("other");

        assert_ne!(one, other);
        assert_eq!(reference_count(&store, &one).unwrap(), 1);
        assert_eq!(reference_count(&store, &other).unwrap(), 1);
    }

    #[test]
    fn releasing_a_shared_blob_keeps_the_bytes_until_the_purge_sweep() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();

        let hash = store_message(&store, &blobs, MESSAGE).expect("first");
        store_message(&store, &blobs, MESSAGE).expect("second");

        assert_eq!(release_message(&store, &hash).unwrap(), 1);
        assert_eq!(reference_count(&store, &hash).unwrap(), 1);
        assert!(blobs.exists(&hash).unwrap());

        assert_eq!(release_message(&store, &hash).unwrap(), 0);
        assert_eq!(reference_count(&store, &hash).unwrap(), 0);
        assert!(blobs.exists(&hash).unwrap());
    }

    #[test]
    fn adding_a_reference_does_not_touch_the_blob_store() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();

        let hash = store_message(&store, &blobs, MESSAGE).expect("store");
        assert_eq!(add_reference(&store, &hash).unwrap(), 2);

        assert_eq!(release_message(&store, &hash).unwrap(), 1);
        assert!(blobs.exists(&hash).unwrap());
    }

    #[test]
    fn a_redundant_release_never_reports_a_negative_count() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();

        let hash = store_message(&store, &blobs, MESSAGE).expect("store");
        assert_eq!(release_message(&store, &hash).unwrap(), 0);
        assert!(blobs.exists(&hash).unwrap());

        assert_eq!(release_message(&store, &hash).unwrap(), 0);
        assert!(reference_count(&store, &hash).unwrap() <= 0);
    }

    #[test]
    fn reference_count_of_an_unknown_blob_is_zero() {
        let store = MemStore::default();
        let absent = BlobHash::from_bytes([9u8, 9, 9, 9]);
        assert_eq!(reference_count(&store, &absent).unwrap(), 0);
    }

    #[test]
    fn distinct_blobs_keep_independent_counters() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();

        let one = store_message(&store, &blobs, MESSAGE).expect("one");
        let other =
            store_message(&store, &blobs, b"Subject: Two\r\n\r\nsecond\r\n").expect("other");

        release_message(&store, &one).unwrap();
        assert_eq!(reference_count(&store, &one).unwrap(), 0);
        assert_eq!(reference_count(&store, &other).unwrap(), 1);
        assert!(blobs.exists(&other).unwrap());
    }
}
