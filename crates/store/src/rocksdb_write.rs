use std::thread::sleep;
use std::time::{Duration, Instant};

use irixmail_core::{Error, Result};
use rocksdb::{ErrorKind, OptimisticTransactionOptions, WriteOptions};

use crate::key::KeyPrefix;
use crate::rocksdb_store::RocksdbStore;
use crate::traits_store::{Flow, Store, ValueAssert, WriteOp};

const COMMIT_RETRY_LIMIT: u32 = 16;

const COMMIT_RETRY_BUDGET: Duration = Duration::from_secs(5);

const MIN_BACKOFF_MILLIS: u64 = 5;

const MAX_BACKOFF_MILLIS: u64 = 50;

impl RocksdbStore {
    pub fn apply_batch(&self, ops: &[WriteOp]) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }

        self.with_retry(|| {
            let mut txn_opts = OptimisticTransactionOptions::default();
            txn_opts.set_snapshot(true);
            let txn = self
                .db()
                .transaction_opt(&WriteOptions::default(), &txn_opts);

            for op in ops {
                match op {
                    WriteOp::Set { key, value } => {
                        let cf = self.cf(subspace_of(key)?)?;
                        txn.put_cf(&cf, key, value).map_err(CommitError::RocksDb)?;
                    }
                    WriteOp::Delete { key } => {
                        let cf = self.cf(subspace_of(key)?)?;
                        txn.delete_cf(&cf, key).map_err(CommitError::RocksDb)?;
                    }
                    WriteOp::Add { key, by } => {
                        let cf = self.cf(subspace_of(key)?)?;
                        txn.merge_cf(&cf, key, by.to_le_bytes())
                            .map_err(CommitError::RocksDb)?;
                    }
                }
            }

            txn.commit().map_err(CommitError::RocksDb)
        })
    }

    pub fn apply_batch_conditional(
        &self,
        asserts: &[ValueAssert],
        ops: &[WriteOp],
    ) -> Result<bool> {
        if asserts.is_empty() {
            self.apply_batch(ops)?;
            return Ok(true);
        }

        self.with_retry(|| {
            let mut txn_opts = OptimisticTransactionOptions::default();
            txn_opts.set_snapshot(true);
            let txn = self
                .db()
                .transaction_opt(&WriteOptions::default(), &txn_opts);

            for assert in asserts {
                let cf = self.cf(subspace_of(&assert.key)?)?;
                let current = txn
                    .get_pinned_for_update_cf(&cf, &assert.key, true)
                    .map_err(CommitError::RocksDb)?;
                let matches = match (&current, &assert.expected) {
                    (Some(bytes), Some(expected)) => bytes.as_ref() == expected.as_slice(),
                    (None, None) => true,
                    _ => false,
                };
                if !matches {
                    return Ok(false);
                }
            }

            for op in ops {
                match op {
                    WriteOp::Set { key, value } => {
                        let cf = self.cf(subspace_of(key)?)?;
                        txn.put_cf(&cf, key, value).map_err(CommitError::RocksDb)?;
                    }
                    WriteOp::Delete { key } => {
                        let cf = self.cf(subspace_of(key)?)?;
                        txn.delete_cf(&cf, key).map_err(CommitError::RocksDb)?;
                    }
                    WriteOp::Add { key, by } => {
                        let cf = self.cf(subspace_of(key)?)?;
                        txn.merge_cf(&cf, key, by.to_le_bytes())
                            .map_err(CommitError::RocksDb)?;
                    }
                }
            }

            txn.commit().map_err(CommitError::RocksDb)?;
            Ok(true)
        })
    }

    pub fn add_counter(&self, key: &[u8], by: i64) -> Result<i64> {
        self.with_retry(|| {
            let cf = self.cf(subspace_of(key)?)?;
            let mut txn_opts = OptimisticTransactionOptions::default();
            txn_opts.set_snapshot(true);
            let txn = self
                .db()
                .transaction_opt(&WriteOptions::default(), &txn_opts);

            let current = txn
                .get_pinned_for_update_cf(&cf, key, true)
                .map_err(CommitError::RocksDb)?
                .map(|bytes| decode_counter(&bytes))
                .transpose()?
                .unwrap_or(0);

            let next = current.wrapping_add(by);
            txn.put_cf(&cf, key, next.to_le_bytes())
                .map_err(CommitError::RocksDb)?;
            txn.commit().map_err(CommitError::RocksDb)?;
            Ok(next)
        })
    }

    pub fn read_counter(&self, key: &[u8]) -> Result<i64> {
        match self.get(key)? {
            Some(bytes) => decode_counter(&bytes),
            None => Ok(0),
        }
    }

    fn with_retry<T>(
        &self,
        mut commit: impl FnMut() -> std::result::Result<T, CommitError>,
    ) -> Result<T> {
        let start = Instant::now();
        let mut attempt = 0u32;
        loop {
            match commit() {
                Ok(value) => return Ok(value),
                Err(CommitError::Internal(err)) => return Err(err),
                Err(CommitError::RocksDb(err)) => {
                    let recoverable = matches!(
                        err.kind(),
                        ErrorKind::Busy | ErrorKind::TryAgain | ErrorKind::MergeInProgress
                    );
                    if recoverable
                        && attempt < COMMIT_RETRY_LIMIT
                        && start.elapsed() < COMMIT_RETRY_BUDGET
                    {
                        sleep(Duration::from_millis(backoff_millis(attempt)));
                        attempt += 1;
                    } else {
                        return Err(Error::store(format!("rocksdb write failed: {err}")));
                    }
                }
            }
        }
    }
}

impl Store for RocksdbStore {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        RocksdbStore::get(self, key)
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        RocksdbStore::put(self, key, value)
    }

    fn delete(&self, key: &[u8]) -> Result<()> {
        RocksdbStore::delete(self, key)
    }

    fn iterate(
        &self,
        prefix: &KeyPrefix,
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
    ) -> Result<()> {
        RocksdbStore::iterate(self, prefix, visit)
    }

    fn iterate_from(
        &self,
        prefix: &KeyPrefix,
        start: &[u8],
        visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
    ) -> Result<()> {
        RocksdbStore::iterate_from(self, prefix, start, visit)
    }

    fn batch(&self, ops: &[WriteOp]) -> Result<()> {
        self.apply_batch(ops)
    }

    fn batch_conditional(&self, asserts: &[ValueAssert], ops: &[WriteOp]) -> Result<bool> {
        self.apply_batch_conditional(asserts, ops)
    }

    fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
        self.add_counter(key, by)
    }

    fn counter(&self, key: &[u8]) -> Result<i64> {
        self.read_counter(key)
    }

    fn flush(&self) -> Result<()> {
        self.db()
            .flush_wal(true)
            .map_err(|err| Error::store(format!("rocksdb WAL flush failed: {err}")))?;
        self.db()
            .flush()
            .map_err(|err| Error::store(format!("rocksdb memtable flush failed: {err}")))
    }
}

enum CommitError {
    RocksDb(rocksdb::Error),
    Internal(Error),
}

impl From<Error> for CommitError {
    fn from(err: Error) -> Self {
        CommitError::Internal(err)
    }
}

fn subspace_of(key: &[u8]) -> std::result::Result<crate::key::Subspace, CommitError> {
    key.first()
        .copied()
        .and_then(crate::key::Subspace::from_byte)
        .ok_or_else(|| {
            CommitError::Internal(Error::store(
                "key does not begin with a known subspace byte",
            ))
        })
}

fn decode_counter(bytes: &[u8]) -> Result<i64> {
    let array: [u8; 8] = bytes
        .try_into()
        .map_err(|_| Error::store("counter value is not eight bytes"))?;
    Ok(i64::from_le_bytes(array))
}

fn backoff_millis(attempt: u32) -> u64 {
    let ceiling = MIN_BACKOFF_MILLIS
        .checked_shl(attempt)
        .map_or(MAX_BACKOFF_MILLIS, |grown| grown.min(MAX_BACKOFF_MILLIS));
    rand::Rng::random_range(&mut rand::rng(), MIN_BACKOFF_MILLIS..=ceiling)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Collection, Key, Subspace};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-store-write-test-{}-{unique}",
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

    fn counter_key(account: u32) -> Vec<u8> {
        Key::new(Subspace::Counter, account, Collection::Email, 0).encode()
    }

    fn blobref_key(account: u32) -> Vec<u8> {
        Key::new(Subspace::BlobRef, account, Collection::Email, 0).encode()
    }

    #[test]
    fn batch_applies_a_mixed_write_set_atomically() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let keep = email_key(1, 1);
        let stale = email_key(1, 2);
        store.put(&stale, b"old").unwrap();
        let counter = counter_key(1);

        store
            .apply_batch(&[
                WriteOp::Set {
                    key: keep.clone(),
                    value: b"fresh".to_vec(),
                },
                WriteOp::Delete { key: stale.clone() },
                WriteOp::Add {
                    key: counter.clone(),
                    by: 3,
                },
            ])
            .unwrap();

        assert_eq!(store.get(&keep).unwrap().as_deref(), Some(&b"fresh"[..]));
        assert_eq!(store.get(&stale).unwrap(), None);
        assert_eq!(store.read_counter(&counter).unwrap(), 3);
    }

    #[test]
    fn an_empty_batch_changes_nothing() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        store.apply_batch(&[]).unwrap();
    }

    #[test]
    fn batch_writes_persist_across_a_reopen() {
        let dir = TempDir::new();
        let key = email_key(2, 5);
        {
            let store = RocksdbStore::open(&dir.path).unwrap();
            store
                .apply_batch(&[WriteOp::Set {
                    key: key.clone(),
                    value: b"durable".to_vec(),
                }])
                .unwrap();
        }
        let store = RocksdbStore::open(&dir.path).unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(&b"durable"[..]));
    }

    #[test]
    fn batch_counter_adds_accumulate() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let counter = counter_key(4);

        store
            .apply_batch(&[WriteOp::Add {
                key: counter.clone(),
                by: 10,
            }])
            .unwrap();
        store
            .apply_batch(&[WriteOp::Add {
                key: counter.clone(),
                by: -4,
            }])
            .unwrap();

        assert_eq!(store.read_counter(&counter).unwrap(), 6);
    }

    #[test]
    fn blob_reference_counters_accumulate_through_add_and_batch() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let refs = blobref_key(7);

        assert_eq!(store.add_and_get(&refs, 1).unwrap(), 1);
        store
            .apply_batch(&[WriteOp::Add {
                key: refs.clone(),
                by: 1,
            }])
            .unwrap();

        assert_eq!(store.read_counter(&refs).unwrap(), 2);
    }

    #[test]
    fn batch_rejects_a_key_without_a_known_subspace() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();

        let result = store.apply_batch(&[WriteOp::Set {
            key: b"?bad".to_vec(),
            value: b"v".to_vec(),
        }]);
        assert!(result.is_err());
    }

    #[test]
    fn add_counter_returns_the_running_total() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let counter = counter_key(5);

        assert_eq!(store.read_counter(&counter).unwrap(), 0);
        assert_eq!(store.add_counter(&counter, 10).unwrap(), 10);
        assert_eq!(store.add_counter(&counter, -4).unwrap(), 6);
        assert_eq!(store.read_counter(&counter).unwrap(), 6);
    }

    #[test]
    fn read_counter_treats_an_untouched_key_as_zero() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        assert_eq!(store.read_counter(&counter_key(9)).unwrap(), 0);
    }

    #[test]
    fn a_merge_onto_a_corrupt_counter_errors_instead_of_zeroing() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let counter = counter_key(17);
        store.put(&counter, b"short").unwrap();

        let read = store
            .apply_batch(&[WriteOp::Add {
                key: counter.clone(),
                by: 3,
            }])
            .and_then(|_| store.read_counter(&counter));
        assert!(read.is_err());
    }

    #[test]
    fn read_counter_rejects_a_value_of_the_wrong_length() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let counter = counter_key(11);
        store.put(&counter, b"short").unwrap();
        assert!(store.read_counter(&counter).is_err());
    }

    #[test]
    fn merge_and_locked_reads_agree_on_the_same_counter() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let counter = counter_key(13);

        store
            .apply_batch(&[WriteOp::Add {
                key: counter.clone(),
                by: 7,
            }])
            .unwrap();
        assert_eq!(store.add_counter(&counter, 5).unwrap(), 12);
        assert_eq!(store.read_counter(&counter).unwrap(), 12);
    }

    #[test]
    fn concurrent_counter_adds_lose_no_increments() {
        let dir = TempDir::new();
        let store = Arc::new(RocksdbStore::open(&dir.path).unwrap());
        let counter = counter_key(21);

        let threads: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let counter = counter.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        store.add_counter(&counter, 1).unwrap();
                    }
                })
            })
            .collect();
        for handle in threads {
            handle.join().unwrap();
        }

        assert_eq!(store.read_counter(&counter).unwrap(), 8 * 50);
    }

    #[test]
    fn store_trait_routes_to_the_write_path() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let backend: &dyn Store = &store;
        let record = email_key(31, 1);
        let counter = counter_key(31);

        backend
            .batch(&[
                WriteOp::Set {
                    key: record.clone(),
                    value: b"v".to_vec(),
                },
                WriteOp::Add {
                    key: counter.clone(),
                    by: 2,
                },
            ])
            .unwrap();

        assert_eq!(backend.get(&record).unwrap().as_deref(), Some(&b"v"[..]));
        assert_eq!(backend.add_and_get(&counter, 3).unwrap(), 5);
        assert_eq!(backend.counter(&counter).unwrap(), 5);
        assert!(backend.exists(&record).unwrap());
    }

    #[test]
    fn a_conditional_batch_applies_when_the_asserted_value_matches() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let key = email_key(41, 1);
        store.put(&key, b"before").unwrap();

        let applied = store
            .apply_batch_conditional(
                &[ValueAssert {
                    key: key.clone(),
                    expected: Some(b"before".to_vec()),
                }],
                &[WriteOp::Set {
                    key: key.clone(),
                    value: b"after".to_vec(),
                }],
            )
            .unwrap();

        assert!(applied);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(&b"after"[..]));
    }

    #[test]
    fn a_conditional_batch_refuses_a_stale_assertion_without_writing() {
        let dir = TempDir::new();
        let store = RocksdbStore::open(&dir.path).unwrap();
        let key = email_key(42, 1);
        store.put(&key, b"current").unwrap();

        let applied = store
            .apply_batch_conditional(
                &[ValueAssert {
                    key: key.clone(),
                    expected: Some(b"stale".to_vec()),
                }],
                &[WriteOp::Set {
                    key: key.clone(),
                    value: b"clobbered".to_vec(),
                }],
            )
            .unwrap();

        assert!(!applied);
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(&b"current"[..]));

        let missing_expected = store
            .apply_batch_conditional(
                &[ValueAssert {
                    key: key.clone(),
                    expected: None,
                }],
                &[WriteOp::Set {
                    key: key.clone(),
                    value: b"clobbered".to_vec(),
                }],
            )
            .unwrap();
        assert!(!missing_expected);
    }

    #[test]
    fn concurrent_conditional_writers_lose_no_updates() {
        let dir = TempDir::new();
        let store = Arc::new(RocksdbStore::open(&dir.path).unwrap());
        let key = email_key(43, 1);
        store.put(&key, &0u64.to_be_bytes()).unwrap();

        let threads: Vec<_> = (0..4)
            .map(|_| {
                let store = Arc::clone(&store);
                let key = key.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        loop {
                            let current = store.get(&key).unwrap().unwrap();
                            let value = u64::from_be_bytes(current.as_slice().try_into().unwrap());
                            let next = (value + 1).to_be_bytes().to_vec();
                            let applied = store
                                .apply_batch_conditional(
                                    &[ValueAssert {
                                        key: key.clone(),
                                        expected: Some(current),
                                    }],
                                    &[WriteOp::Set {
                                        key: key.clone(),
                                        value: next,
                                    }],
                                )
                                .unwrap();
                            if applied {
                                break;
                            }
                        }
                    }
                })
            })
            .collect();
        for handle in threads {
            handle.join().unwrap();
        }

        let total = u64::from_be_bytes(
            store
                .get(&key)
                .unwrap()
                .unwrap()
                .as_slice()
                .try_into()
                .unwrap(),
        );
        assert_eq!(total, 4 * 50);
    }

    #[test]
    fn backoff_stays_within_the_window() {
        for attempt in 0..64u32 {
            let wait = backoff_millis(attempt);
            assert!((MIN_BACKOFF_MILLIS..=MAX_BACKOFF_MILLIS).contains(&wait));
        }
    }

    #[test]
    fn backoff_is_jittered_across_draws() {
        let draws: std::collections::HashSet<u64> = (0..64).map(|_| backoff_millis(4)).collect();
        assert!(draws.len() > 1);
    }
}
