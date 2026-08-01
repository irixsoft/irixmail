use std::collections::HashSet;

use irixmail_core::Result;
use irixmail_store::{BlobStore, Flow, KeyPrefix, Store, Subspace};

use crate::blob_store_msg::{
    is_link_document, is_reservation_document, ref_count_key, split_blob_ref_key,
};

pub const PURGE_GRACE_SECS: u64 = 3600;

pub fn purge_orphans(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    now: u64,
    grace: u64,
) -> Result<usize> {
    let mut purged = 0;

    let mut reserved: HashSet<Vec<u8>> = HashSet::new();
    let mut dead_reservations = Vec::new();
    let mut orphans = Vec::new();
    store.iterate(
        &KeyPrefix::subspace(Subspace::BlobRef),
        &mut |key, value| {
            let Some((_, document, hash)) = split_blob_ref_key(key) else {
                return Ok(Flow::Continue);
            };
            if is_reservation_document(document) {
                match <[u8; 8]>::try_from(value) {
                    Ok(bytes) if u64::from_le_bytes(bytes) > now => {
                        reserved.insert(hash.as_bytes().to_vec());
                    }
                    _ => dead_reservations.push(key.to_vec()),
                }
                return Ok(Flow::Continue);
            }
            if is_link_document(document) {
                if let Ok(bytes) = <[u8; 8]>::try_from(value) {
                    if i64::from_le_bytes(bytes) <= 0 {
                        dead_reservations.push(key.to_vec());
                    }
                }
                return Ok(Flow::Continue);
            }
            // an unparseable refcount is treated as live, never as zero
            let Ok(bytes) = <[u8; 8]>::try_from(value) else {
                return Ok(Flow::Continue);
            };
            if i64::from_le_bytes(bytes) <= 0 {
                orphans.push((key.to_vec(), hash));
            }
            Ok(Flow::Continue)
        },
    )?;
    for key in &dead_reservations {
        store.delete(key)?;
    }
    for (key, hash) in &orphans {
        if reserved.contains(hash.as_bytes()) {
            continue;
        }
        if let Some(modified) = blobs.modified_at(hash)? {
            if now.saturating_sub(modified) < grace {
                continue;
            }
        }
        blobs.delete(hash)?;
        store.delete(key)?;
        purged += 1;
    }

    let mut stranded = Vec::new();
    blobs.for_each(&mut |hash, modified| {
        if now.saturating_sub(modified) >= grace.max(1)
            && !reserved.contains(hash.as_bytes())
            && store.get(&ref_count_key(hash))?.is_none()
        {
            stranded.push(hash.clone());
        }
        Ok(())
    })?;
    for hash in &stranded {
        blobs.delete(hash)?;
        purged += 1;
    }

    Ok(purged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use irixmail_store::{FsBlobStore, RocksdbStore};

    use crate::blob_store_msg::{ref_count_key, reference_count, store_message};

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("irixmail-purge-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn unix_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0)
    }

    #[test]
    fn an_orphaned_blob_is_purged_while_a_referenced_one_survives() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();

        let orphan = store_message(&store, &blobs, b"orphaned message").unwrap();
        store.add_and_get(&ref_count_key(&orphan), -1).unwrap();
        assert!(blobs.get_all(&orphan).unwrap().is_some());

        let live = store_message(&store, &blobs, b"live message").unwrap();

        let purged = purge_orphans(&store, &blobs, unix_now(), 0).unwrap();
        assert_eq!(purged, 1);
        assert!(blobs.get_all(&orphan).unwrap().is_none());
        assert!(store.get(&ref_count_key(&orphan)).unwrap().is_none());
        assert!(blobs.get_all(&live).unwrap().is_some());
        assert_eq!(reference_count(&store, &live).unwrap(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clean_store_purges_nothing() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();
        assert_eq!(purge_orphans(&store, &blobs, unix_now(), 0).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_refcount_never_deletes_the_blob() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();

        let hash = store_message(&store, &blobs, b"still referenced").unwrap();
        store.put(&ref_count_key(&hash), b"xx").unwrap();

        let purged = purge_orphans(&store, &blobs, unix_now(), 0).unwrap();
        assert_eq!(purged, 0);
        assert!(blobs.get_all(&hash).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn releasing_the_last_reference_defers_deletion_to_the_sweep() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();

        let hash = store_message(&store, &blobs, b"short lived").unwrap();
        let remaining = crate::blob_store_msg::release_message(&store, &hash).unwrap();
        assert_eq!(remaining, 0);
        assert!(
            blobs.get_all(&hash).unwrap().is_some(),
            "the decrement path must not delete the blob inline"
        );

        let purged = purge_orphans(&store, &blobs, unix_now(), 0).unwrap();
        assert_eq!(purged, 1);
        assert!(blobs.get_all(&hash).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blob_inside_the_grace_window_survives_the_sweep() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();

        let hash = store_message(&store, &blobs, b"just arrived").unwrap();
        store.add_and_get(&ref_count_key(&hash), -1).unwrap();

        let purged = purge_orphans(&store, &blobs, unix_now(), 3600).unwrap();
        assert_eq!(purged, 0);
        assert!(blobs.get_all(&hash).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_uploaded_blob_with_a_live_reservation_survives_the_sweep() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();
        let now = unix_now();

        let uploaded =
            crate::blob_store_msg::reserve_upload(&store, &blobs, 7, b"staged upload", now)
                .unwrap();

        let purged = purge_orphans(&store, &blobs, now + 7200, 3600).unwrap();
        assert_eq!(purged, 0);
        assert!(blobs.get_all(&uploaded).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_expired_reservation_is_swept_and_the_upload_reclaimed() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();
        let now = unix_now();

        let uploaded =
            crate::blob_store_msg::reserve_upload(&store, &blobs, 7, b"forgotten upload", now)
                .unwrap();
        let after_expiry = now + crate::blob_store_msg::RESERVATION_TTL_SECS + 7200;

        let purged = purge_orphans(&store, &blobs, after_expiry, 3600).unwrap();
        assert_eq!(purged, 1);
        assert!(blobs.get_all(&uploaded).unwrap().is_none());
        assert!(store
            .get(&crate::blob_store_msg::reservation_key(7, &uploaded))
            .unwrap()
            .is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_consumed_upload_survives_its_reservation_expiry() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();
        let now = unix_now();

        let uploaded =
            crate::blob_store_msg::reserve_upload(&store, &blobs, 7, b"imported upload", now)
                .unwrap();
        let imported = store_message(&store, &blobs, b"imported upload").unwrap();
        assert_eq!(uploaded, imported);
        let after_expiry = now + crate::blob_store_msg::RESERVATION_TTL_SECS + 7200;

        let purged = purge_orphans(&store, &blobs, after_expiry, 3600).unwrap();
        assert_eq!(purged, 0);
        assert!(blobs.get_all(&uploaded).unwrap().is_some());
        assert_eq!(reference_count(&store, &uploaded).unwrap(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blob_with_no_reference_entry_is_reclaimed_after_the_grace_window() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();

        let stranded =
            crate::blob_store_msg::store_blob(&blobs, b"crashed before the batch").unwrap();
        let live = store_message(&store, &blobs, b"live message").unwrap();

        assert_eq!(purge_orphans(&store, &blobs, unix_now(), 3600).unwrap(), 0);
        assert!(blobs.get_all(&stranded).unwrap().is_some());

        let purged = purge_orphans(&store, &blobs, unix_now() + 7200, 3600).unwrap();
        assert_eq!(purged, 1);
        assert!(blobs.get_all(&stranded).unwrap().is_none());
        assert!(blobs.get_all(&live).unwrap().is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
