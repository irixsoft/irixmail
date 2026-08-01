use std::path::Path;

use irixmail_core::{Error, Result};
use rocksdb::checkpoint::Checkpoint;

use crate::rocksdb_store::RocksdbStore;

impl RocksdbStore {
    pub fn checkpoint(&self, destination: impl AsRef<Path>) -> Result<()> {
        let destination = destination.as_ref();

        let checkpoint = Checkpoint::new(self.db()).map_err(|err| {
            Error::store(format!("failed to create rocksdb checkpoint object: {err}"))
        })?;

        checkpoint.create_checkpoint(destination).map_err(|err| {
            Error::store(format!(
                "failed to write rocksdb checkpoint to {}: {err}",
                destination.display()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Collection, Key, KeyPrefix, Subspace};
    use crate::traits_store::Flow;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-checkpoint-test-{}-{unique}",
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

    fn email_key(account: u32, document: u32) -> Vec<u8> {
        Key::new(Subspace::Property, account, Collection::Email, document).encode()
    }

    #[test]
    fn checkpoint_reopens_as_a_complete_database() {
        let dir = TempDir::new();
        let live = dir.path.join("live");
        let snapshot = dir.path.join("snapshot");

        let store = RocksdbStore::open(&live).unwrap();
        store.put(&email_key(1, 1), b"alpha").unwrap();
        store.put(&email_key(1, 2), b"beta").unwrap();

        store.checkpoint(&snapshot).unwrap();
        assert!(snapshot.exists());

        let restored = RocksdbStore::open(&snapshot).unwrap();
        assert_eq!(
            restored.get(&email_key(1, 1)).unwrap().as_deref(),
            Some(&b"alpha"[..])
        );
        assert_eq!(
            restored.get(&email_key(1, 2)).unwrap().as_deref(),
            Some(&b"beta"[..])
        );
    }

    #[test]
    fn checkpoint_is_frozen_at_the_moment_it_is_taken() {
        let dir = TempDir::new();
        let live = dir.path.join("live");
        let snapshot = dir.path.join("snapshot");

        let store = RocksdbStore::open(&live).unwrap();
        store.put(&email_key(2, 1), b"before").unwrap();

        store.checkpoint(&snapshot).unwrap();

        store.put(&email_key(2, 1), b"after").unwrap();
        store.put(&email_key(2, 2), b"new").unwrap();

        let restored = RocksdbStore::open(&snapshot).unwrap();
        assert_eq!(
            restored.get(&email_key(2, 1)).unwrap().as_deref(),
            Some(&b"before"[..])
        );
        assert_eq!(restored.get(&email_key(2, 2)).unwrap(), None);
    }

    #[test]
    fn checkpoint_preserves_every_subspace() {
        let dir = TempDir::new();
        let live = dir.path.join("live");
        let snapshot = dir.path.join("snapshot");

        let store = RocksdbStore::open(&live).unwrap();
        store
            .put(
                &Key::new(Subspace::Registry, 3, Collection::Email, 1).encode(),
                b"cfg",
            )
            .unwrap();
        store
            .put(
                &Key::new(Subspace::ChangeLog, 3, Collection::Email, 1).encode(),
                b"log",
            )
            .unwrap();

        store.checkpoint(&snapshot).unwrap();

        let restored = RocksdbStore::open(&snapshot).unwrap();
        let registry = restored
            .get(&Key::new(Subspace::Registry, 3, Collection::Email, 1).encode())
            .unwrap();
        let changelog = restored
            .get(&Key::new(Subspace::ChangeLog, 3, Collection::Email, 1).encode())
            .unwrap();
        assert_eq!(registry.as_deref(), Some(&b"cfg"[..]));
        assert_eq!(changelog.as_deref(), Some(&b"log"[..]));
    }

    #[test]
    fn checkpoint_keeps_the_live_store_writable() {
        let dir = TempDir::new();
        let live = dir.path.join("live");
        let snapshot = dir.path.join("snapshot");

        let store = RocksdbStore::open(&live).unwrap();
        store.put(&email_key(4, 1), b"v").unwrap();
        store.checkpoint(&snapshot).unwrap();

        store.put(&email_key(4, 2), b"still-writable").unwrap();
        assert_eq!(
            store.get(&email_key(4, 2)).unwrap().as_deref(),
            Some(&b"still-writable"[..])
        );

        let prefix = KeyPrefix::collection(Subspace::Property, 4, Collection::Email);
        let mut seen = 0;
        store
            .iterate(&prefix, &mut |_key, _value| {
                seen += 1;
                Ok(Flow::Continue)
            })
            .unwrap();
        assert_eq!(seen, 2);
    }

    #[test]
    fn checkpoint_rejects_a_destination_that_already_exists() {
        let dir = TempDir::new();
        let live = dir.path.join("live");
        let snapshot = dir.path.join("snapshot");
        std::fs::create_dir_all(&snapshot).unwrap();

        let store = RocksdbStore::open(&live).unwrap();
        assert!(store.checkpoint(&snapshot).is_err());
    }
}
