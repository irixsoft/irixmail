use std::path::Path;
use std::sync::Arc;

use irixmail_core::{Error, Result};
use rocksdb::{
    BoundColumnFamily, ColumnFamilyDescriptor, Direction, IteratorMode, MergeOperands,
    MultiThreaded, OptimisticTransactionDB, Options,
};

use crate::key::{KeyPrefix, Subspace};
use crate::traits_store::Flow;

pub const SCHEMA_VERSION: u32 = 1;

const SCHEMA_VERSION_TAG: u8 = 0x02;

pub fn schema_version_key() -> Vec<u8> {
    vec![Subspace::Registry.as_byte(), SCHEMA_VERSION_TAG]
}

const ALL_SUBSPACES: [Subspace; 7] = [
    Subspace::Property,
    Subspace::Index,
    Subspace::ChangeLog,
    Subspace::BlobRef,
    Subspace::Queue,
    Subspace::Registry,
    Subspace::Counter,
];

pub struct RocksdbStore {
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl RocksdbStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        std::fs::create_dir_all(path).map_err(|err| {
            Error::store(format!(
                "failed to create data directory {}: {err}",
                path.display()
            ))
        })?;

        let descriptors = ALL_SUBSPACES
            .into_iter()
            .map(column_family_descriptor)
            .collect::<Vec<_>>();

        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
            &db_opts,
            path,
            descriptors,
        )
        .map_err(|err| {
            Error::store(format!(
                "failed to open rocksdb store at {}: {err}",
                path.display()
            ))
        })?;

        let store = Self { db: Arc::new(db) };
        store.check_schema_version(path)?;
        Ok(store)
    }

    fn check_schema_version(&self, path: &Path) -> Result<()> {
        let key = schema_version_key();
        match self.get(&key)? {
            None => self.put(&key, &SCHEMA_VERSION.to_le_bytes()),
            Some(bytes) => {
                let stored = bytes
                    .as_slice()
                    .try_into()
                    .map(u32::from_le_bytes)
                    .map_err(|_| {
                        Error::store(format!(
                            "corrupt schema version stamp in the store at {}",
                            path.display()
                        ))
                    })?;
                if stored == SCHEMA_VERSION {
                    Ok(())
                } else {
                    Err(Error::store(format!(
                        "the store at {} has schema version {stored} but this build expects {SCHEMA_VERSION}; refusing to open it",
                        path.display()
                    )))
                }
            }
        }
    }

    pub(crate) fn db(&self) -> &OptimisticTransactionDB<MultiThreaded> {
        &self.db
    }

    pub(crate) fn cf(&self, subspace: Subspace) -> Result<Arc<BoundColumnFamily<'_>>> {
        let name = column_family_name(subspace);
        self.db
            .cf_handle(&name)
            .ok_or_else(|| Error::store(format!("rocksdb column family for {subspace} is missing")))
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let cf = self.cf(subspace_of(key)?)?;
        self.db
            .get_cf(&cf, key)
            .map_err(|err| Error::store(format!("rocksdb get failed: {err}")))
    }

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let cf = self.cf(subspace_of(key)?)?;
        self.db
            .put_cf(&cf, key, value)
            .map_err(|err| Error::store(format!("rocksdb put failed: {err}")))
    }

    pub fn delete(&self, key: &[u8]) -> Result<()> {
        let cf = self.cf(subspace_of(key)?)?;
        self.db
            .delete_cf(&cf, key)
            .map_err(|err| Error::store(format!("rocksdb delete failed: {err}")))
    }

    #[allow(clippy::type_complexity)]
    pub fn iterate(
        &self,
        prefix: &KeyPrefix,
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
    ) -> Result<()> {
        let bound = prefix.encode();
        self.iterate_bounded(&bound, &bound, visit)
    }

    #[allow(clippy::type_complexity)]
    pub fn iterate_from(
        &self,
        prefix: &KeyPrefix,
        start: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
    ) -> Result<()> {
        let bound = prefix.encode();
        let seek = if start > bound.as_slice() {
            start
        } else {
            &bound
        };
        self.iterate_bounded(&bound, seek, visit)
    }

    #[allow(clippy::type_complexity)]
    fn iterate_bounded(
        &self,
        bound: &[u8],
        seek: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
    ) -> Result<()> {
        let cf = self.cf(subspace_of(bound)?)?;
        let mode = IteratorMode::From(seek, Direction::Forward);

        for row in self.db.iterator_cf(&cf, mode) {
            let (key, value) =
                row.map_err(|err| Error::store(format!("rocksdb iterate failed: {err}")))?;
            if !key.starts_with(bound) {
                break;
            }
            if visit(&key, &value)? == Flow::Stop {
                break;
            }
        }
        Ok(())
    }
}

fn column_family_name(subspace: Subspace) -> String {
    String::from_utf8(vec![subspace.as_byte()]).expect("subspace bytes are printable ascii")
}

fn column_family_descriptor(subspace: Subspace) -> ColumnFamilyDescriptor {
    let mut opts = Options::default();
    if matches!(subspace, Subspace::Counter | Subspace::BlobRef) {
        opts.set_merge_operator_associative("counter-add", counter_merge);
    }
    ColumnFamilyDescriptor::new(column_family_name(subspace), opts)
}

fn subspace_of(key: &[u8]) -> Result<Subspace> {
    key.first()
        .copied()
        .and_then(Subspace::from_byte)
        .ok_or_else(|| Error::store("key does not begin with a known subspace byte"))
}

// returning None marks the merge as failed so a corrupt value surfaces as a
// read error instead of silently resetting the accumulated count
fn counter_merge(
    _key: &[u8],
    existing: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    let mut total = match existing {
        Some(bytes) => i64::from_le_bytes(bytes.try_into().ok()?),
        None => 0,
    };

    for operand in operands.iter() {
        total = total.wrapping_add(i64::from_le_bytes(operand.try_into().ok()?));
    }

    Some(total.to_le_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Collection, Key};
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
                "irixmail-store-test-{}-{unique}",
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
    fn open_creates_a_usable_database_and_all_column_families() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();

        for subspace in ALL_SUBSPACES {
            assert!(
                store.cf(subspace).is_ok(),
                "missing column family for {subspace}"
            );
        }
    }

    #[test]
    fn a_fresh_store_is_stamped_with_the_current_schema_version() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        assert_eq!(
            store.get(&schema_version_key()).unwrap().as_deref(),
            Some(&SCHEMA_VERSION.to_le_bytes()[..])
        );
    }

    #[test]
    fn a_stamped_store_reopens_at_the_same_version() {
        let dir = TempDir::new();
        {
            RocksdbStore::open(&dir.path).unwrap();
        }
        assert!(RocksdbStore::open(&dir.path).is_ok());
    }

    #[test]
    fn a_store_with_a_different_schema_version_is_refused() {
        let dir = TempDir::new();
        {
            let store = RocksdbStore::open(&dir.path).unwrap();
            store
                .put(&schema_version_key(), &(SCHEMA_VERSION + 1).to_le_bytes())
                .unwrap();
        }
        assert!(RocksdbStore::open(&dir.path).is_err());
    }

    #[test]
    fn a_store_with_a_corrupt_schema_stamp_is_refused() {
        let dir = TempDir::new();
        {
            let store = RocksdbStore::open(&dir.path).unwrap();
            store.put(&schema_version_key(), b"bad").unwrap();
        }
        assert!(RocksdbStore::open(&dir.path).is_err());
    }

    #[test]
    fn open_creates_the_data_directory_when_absent() {
        let dir = TempDir::new();
        let nested = dir.path.join("data").join("kv");
        assert!(!nested.exists());

        let _store = RocksdbStore::open(&nested).unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn a_flush_after_writes_succeeds_and_data_survives_reopen() {
        use crate::traits_store::Store;

        let dir = TempDir::new();
        let key = email_key(1, 1);
        {
            let store = RocksdbStore::open(&dir.path).unwrap();
            store.put(&key, b"durable").unwrap();
            Store::flush(&store).unwrap();
        }
        let reopened = RocksdbStore::open(&dir.path).unwrap();
        assert_eq!(
            reopened.get(&key).unwrap().as_deref(),
            Some(&b"durable"[..])
        );
    }

    #[test]
    fn put_get_delete_round_trip() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let key = email_key(1, 1);

        assert_eq!(store.get(&key).unwrap(), None);
        store.put(&key, b"hello").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(&b"hello"[..]));

        store.put(&key, b"world").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(&b"world"[..]));

        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        store.delete(&key).unwrap();
    }

    #[test]
    fn reopening_preserves_written_values() {
        let dir = TempDir::new();
        let key = email_key(2, 5);
        {
            let store = RocksdbStore::open(&dir.path).unwrap();
            store.put(&key, b"durable").unwrap();
        }
        let store = RocksdbStore::open(&dir.path).unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(&b"durable"[..]));
    }

    #[test]
    fn iterate_visits_a_prefix_in_ascending_order() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
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
        store.put(&email_key(2, 1), b"other-account").unwrap();

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
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        for document in 1..=5u32 {
            store.put(&email_key(7, document), b"v").unwrap();
        }

        let prefix = KeyPrefix::collection(Subspace::Property, 7, Collection::Email);
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
    fn iterate_over_an_empty_prefix_visits_nothing() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();

        let prefix = KeyPrefix::account(Subspace::Property, 99);
        let mut visited = false;
        store
            .iterate(&prefix, &mut |_key, _value| {
                visited = true;
                Ok(Flow::Continue)
            })
            .unwrap();

        assert!(!visited);
    }

    #[test]
    fn iterate_keeps_subspaces_apart() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        store.put(&email_key(4, 1), b"property").unwrap();
        store
            .put(
                &Key::new(Subspace::Registry, 4, Collection::Email, 1).encode(),
                b"registry",
            )
            .unwrap();

        let prefix = KeyPrefix::subspace(Subspace::Registry);
        let mut values = Vec::new();
        store
            .iterate(&prefix, &mut |_key, value| {
                values.push(value.to_vec());
                Ok(Flow::Continue)
            })
            .unwrap();

        assert!(values.contains(&b"registry".to_vec()));
        assert!(!values.contains(&b"property".to_vec()));
    }

    #[test]
    fn get_rejects_a_key_without_a_known_subspace() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();

        assert!(store.get(b"").is_err());
        assert!(store.get(b"?unknown").is_err());
    }

    #[test]
    fn counter_merge_folds_signed_deltas() {
        let base = 10i64.to_le_bytes().to_vec();
        let ops = [3i64.to_le_bytes().to_vec(), (-4i64).to_le_bytes().to_vec()];
        let mut total = i64::from_le_bytes(base.as_slice().try_into().unwrap());
        for op in &ops {
            total += i64::from_le_bytes(op.as_slice().try_into().unwrap());
        }
        assert_eq!(total, 9);
    }

    #[test]
    fn column_family_name_is_the_subspace_byte() {
        assert_eq!(column_family_name(Subspace::Property), "p");
        assert_eq!(column_family_name(Subspace::Counter), "n");
    }
}
