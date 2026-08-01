use irixmail_core::Result;
use irixmail_store::{BlobHash, BlobStore};

const ENCODING_NONE: u8 = 0;
const ENCODING_QUOTED_PRINTABLE: u8 = 1;
const ENCODING_BASE64: u8 = 2;

pub fn decode_blob_id(blob_id: &str) -> Option<BlobHash> {
    if blob_id.is_empty() || !blob_id.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..blob_id.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&blob_id[index..index + 2], 16).ok())
        .collect();
    bytes.map(BlobHash::from_bytes)
}

pub fn blob_hash_of(blob_id: &str) -> Option<BlobHash> {
    blob_id.split('-').next().and_then(decode_blob_id)
}

pub fn section_blob_id(blob_id: &str, start: u32, length: u32, encoding: u8) -> String {
    format!("{blob_id}-{start}-{length}-{encoding}")
}

pub fn fetch_blob(blobs: &dyn BlobStore, blob_id: &str) -> Result<Option<Vec<u8>>> {
    let mut fields = blob_id.split('-');
    let Some(hash) = fields.next().and_then(decode_blob_id) else {
        return Ok(None);
    };
    match (fields.next(), fields.next(), fields.next(), fields.next()) {
        (None, _, _, _) => blobs.get_all(&hash),
        (Some(start), Some(length), Some(encoding), None) => {
            let (Ok(start), Ok(length), Ok(encoding)) = (
                start.parse::<u32>(),
                length.parse::<u32>(),
                encoding.parse::<u8>(),
            ) else {
                return Ok(None);
            };
            let range = start as usize..(start as usize).saturating_add(length as usize);
            match blobs.get(&hash, range)? {
                Some(bytes) => Ok(decode_section(bytes, encoding)),
                None => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

fn decode_section(bytes: Vec<u8>, encoding: u8) -> Option<Vec<u8>> {
    match encoding {
        ENCODING_NONE => Some(bytes),
        ENCODING_QUOTED_PRINTABLE => {
            mail_parser::decoders::quoted_printable::quoted_printable_decode(&bytes)
        }
        ENCODING_BASE64 => mail_parser::decoders::base64::base64_decode(&bytes),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use irixmail_store::FsBlobStore;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-jmap-download-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn a_stored_blob_is_fetched_by_its_id() {
        let dir = TempDir::new();
        let blobs = FsBlobStore::open(dir.path.join("blobs")).unwrap();
        let hash = blobs.put(b"payload").unwrap();
        let fetched = fetch_blob(&blobs, &hash.to_hex()).unwrap();
        assert_eq!(fetched.as_deref(), Some(&b"payload"[..]));
    }

    #[test]
    fn a_section_blob_id_fetches_the_slice() {
        let dir = TempDir::new();
        let blobs = FsBlobStore::open(dir.path.join("blobs")).unwrap();
        let hash = blobs.put(b"head MIDDLE tail").unwrap();
        let id = section_blob_id(&hash.to_hex(), 5, 6, ENCODING_NONE);
        let fetched = fetch_blob(&blobs, &id).unwrap();
        assert_eq!(fetched.as_deref(), Some(&b"MIDDLE"[..]));
    }

    #[test]
    fn a_base64_section_is_decoded_on_fetch() {
        let dir = TempDir::new();
        let blobs = FsBlobStore::open(dir.path.join("blobs")).unwrap();
        let hash = blobs.put(b"prefix:aGVsbG8=").unwrap();
        let id = section_blob_id(&hash.to_hex(), 7, 8, ENCODING_BASE64);
        let fetched = fetch_blob(&blobs, &id).unwrap();
        assert_eq!(fetched.as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn a_quoted_printable_section_is_decoded_on_fetch() {
        let dir = TempDir::new();
        let blobs = FsBlobStore::open(dir.path.join("blobs")).unwrap();
        let hash = blobs.put(b"caf=C3=A9").unwrap();
        let id = section_blob_id(&hash.to_hex(), 0, 9, ENCODING_QUOTED_PRINTABLE);
        let fetched = fetch_blob(&blobs, &id).unwrap();
        assert_eq!(fetched.as_deref(), Some("café".as_bytes()));
    }

    #[test]
    fn a_malformed_blob_id_is_not_found() {
        let dir = TempDir::new();
        let blobs = FsBlobStore::open(dir.path.join("blobs")).unwrap();
        assert!(fetch_blob(&blobs, "nothex!!").unwrap().is_none());
        assert!(decode_blob_id("abc").is_none());
        assert!(decode_blob_id("").is_none());
    }

    #[test]
    fn a_malformed_section_suffix_is_not_found() {
        let dir = TempDir::new();
        let blobs = FsBlobStore::open(dir.path.join("blobs")).unwrap();
        let hash = blobs.put(b"payload").unwrap();
        let hex = hash.to_hex();
        assert!(fetch_blob(&blobs, &format!("{hex}-1")).unwrap().is_none());
        assert!(fetch_blob(&blobs, &format!("{hex}-a-b-0"))
            .unwrap()
            .is_none());
        assert!(fetch_blob(&blobs, &format!("{hex}-0-4-9"))
            .unwrap()
            .is_none());
        assert!(fetch_blob(&blobs, &format!("{hex}-0-4-0-0"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn an_unknown_blob_id_is_not_found() {
        let dir = TempDir::new();
        let blobs = FsBlobStore::open(dir.path.join("blobs")).unwrap();
        assert!(fetch_blob(&blobs, "00112233").unwrap().is_none());
        assert!(fetch_blob(&blobs, "00112233-0-4-0").unwrap().is_none());
    }
}
