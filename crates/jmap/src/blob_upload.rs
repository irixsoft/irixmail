use serde_json::{json, Value};

use irixmail_core::Result;
use irixmail_store::{BlobStore, Store};

pub fn store_upload(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    account_id: u32,
    bytes: &[u8],
    now: u64,
) -> Result<String> {
    Ok(irixmail_mail::reserve_upload(store, blobs, account_id, bytes, now)?.to_hex())
}

pub fn upload_response(account_id: &str, blob_id: &str, content_type: &str, size: usize) -> Value {
    json!({
        "accountId": account_id,
        "blobId": blob_id,
        "type": content_type,
        "size": size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use irixmail_store::{FsBlobStore, RocksdbStore};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-jmap-upload-{}-{unique}",
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
    fn an_upload_is_stored_and_returns_a_blob_id() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(dir.path.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.path.join("blobs")).unwrap();
        let blob_id = store_upload(&store, &blobs, 1, b"hello world", 0).unwrap();
        assert!(!blob_id.is_empty());
        let again = store_upload(&store, &blobs, 1, b"hello world", 0).unwrap();
        assert_eq!(blob_id, again);
    }

    #[test]
    fn the_response_carries_the_metadata() {
        let response = upload_response("a1", "deadbeef", "text/plain", 11);
        assert_eq!(response["accountId"], "a1");
        assert_eq!(response["blobId"], "deadbeef");
        assert_eq!(response["type"], "text/plain");
        assert_eq!(response["size"], 11);
    }
}
