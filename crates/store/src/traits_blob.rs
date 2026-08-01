use std::fmt;
use std::ops::Range;

use irixmail_core::Result;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobHash(Vec<u8>);

impl BlobHash {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(self.0.len() * 2);
        for byte in &self.0 {
            out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
            out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
        }
        out
    }
}

impl AsRef<[u8]> for BlobHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlobHash({})", self.to_hex())
    }
}

pub trait BlobStore: Send + Sync {
    fn get(&self, hash: &BlobHash, range: Range<usize>) -> Result<Option<Vec<u8>>>;

    fn get_all(&self, hash: &BlobHash) -> Result<Option<Vec<u8>>> {
        self.get(hash, 0..usize::MAX)
    }

    fn exists(&self, hash: &BlobHash) -> Result<bool> {
        Ok(self.get(hash, 0..0)?.is_some())
    }

    fn put(&self, bytes: &[u8]) -> Result<BlobHash>;

    fn delete(&self, hash: &BlobHash) -> Result<()>;

    fn modified_at(&self, _hash: &BlobHash) -> Result<Option<u64>> {
        Ok(None)
    }

    // backends that cannot enumerate stored blobs sweep nothing extra
    #[allow(clippy::type_complexity)]
    fn for_each(&self, _visit: &mut dyn FnMut(&BlobHash, u64) -> Result<()>) -> Result<()> {
        Ok(())
    }

    fn usage_bytes(&self) -> Result<u64> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemBlobStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemBlobStore {
        fn digest(bytes: &[u8]) -> BlobHash {
            let sum = bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
            let mut raw = (bytes.len() as u32).to_be_bytes().to_vec();
            raw.push(sum);
            BlobHash::from_bytes(raw)
        }
    }

    impl BlobStore for MemBlobStore {
        fn get(&self, hash: &BlobHash, range: Range<usize>) -> Result<Option<Vec<u8>>> {
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

    #[test]
    fn put_returns_a_content_derived_hash() {
        let store = MemBlobStore::default();
        let one = store.put(b"hello world").unwrap();
        let two = store.put(b"hello world").unwrap();
        let other = store.put(b"goodbye").unwrap();

        assert_eq!(one, two);
        assert_ne!(one, other);
    }

    #[test]
    fn get_all_reads_the_whole_payload() {
        let store = MemBlobStore::default();
        let hash = store.put(b"the quick brown fox").unwrap();

        assert_eq!(
            store.get_all(&hash).unwrap().as_deref(),
            Some(&b"the quick brown fox"[..])
        );
    }

    #[test]
    fn get_reads_a_byte_window() {
        let store = MemBlobStore::default();
        let hash = store.put(b"abcdefghij").unwrap();

        assert_eq!(
            store.get(&hash, 2..5).unwrap().as_deref(),
            Some(&b"cde"[..])
        );
    }

    #[test]
    fn get_clamps_a_range_past_the_end() {
        let store = MemBlobStore::default();
        let hash = store.put(b"abc").unwrap();

        assert_eq!(
            store.get(&hash, 1..99).unwrap().as_deref(),
            Some(&b"bc"[..])
        );
        assert_eq!(
            store.get(&hash, 0..usize::MAX).unwrap().as_deref(),
            Some(&b"abc"[..])
        );
    }

    #[test]
    fn get_missing_blob_is_none() {
        let store = MemBlobStore::default();
        let absent = BlobHash::from_bytes([0u8, 0, 0, 0, 0]);

        assert_eq!(store.get_all(&absent).unwrap(), None);
        assert!(!store.exists(&absent).unwrap());
    }

    #[test]
    fn exists_tracks_presence() {
        let store = MemBlobStore::default();
        let hash = store.put(b"present").unwrap();

        assert!(store.exists(&hash).unwrap());
    }

    #[test]
    fn delete_removes_then_is_a_no_op() {
        let store = MemBlobStore::default();
        let hash = store.put(b"transient").unwrap();
        assert!(store.exists(&hash).unwrap());

        store.delete(&hash).unwrap();
        assert!(!store.exists(&hash).unwrap());
        store.delete(&hash).unwrap();
    }

    #[test]
    fn blob_hash_renders_as_lowercase_hex() {
        let hash = BlobHash::from_bytes([0x00u8, 0x0f, 0xa0, 0xff]);
        assert_eq!(hash.to_hex(), "000fa0ff");
        assert_eq!(hash.to_string(), "000fa0ff");
        assert_eq!(format!("{hash:?}"), "BlobHash(000fa0ff)");
    }

    #[test]
    fn blob_hash_byte_accessors_round_trip() {
        let raw = vec![1u8, 2, 3, 4];
        let hash = BlobHash::from_bytes(raw.clone());
        assert_eq!(hash.len(), 4);
        assert!(!hash.is_empty());
        assert_eq!(hash.as_bytes(), raw.as_slice());
        assert_eq!(hash.as_ref(), raw.as_slice());
        assert_eq!(hash.into_bytes(), raw);

        assert!(BlobHash::from_bytes([]).is_empty());
    }

    #[test]
    fn blob_store_is_object_safe_and_shareable() {
        fn assert_shareable<T: Send + Sync + ?Sized>() {}
        assert_shareable::<dyn BlobStore>();
        let _boxed: std::sync::Arc<dyn BlobStore> = std::sync::Arc::new(MemBlobStore::default());
    }
}
