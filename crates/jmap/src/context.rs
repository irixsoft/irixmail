use std::sync::Arc;

use irixmail_core::Result;
use irixmail_directory::Directory;
use irixmail_mail::MailServices;
use irixmail_store::{BlobStore, ChangeNotifier, Store};

pub type Submitter = Arc<dyn Fn(&[u8], &str, &[String]) -> Result<()> + Send + Sync>;

#[derive(Clone)]
pub struct JmapContext {
    pub store: Arc<dyn Store>,
    pub blobs: Arc<dyn BlobStore>,
    pub mail: MailServices,
    pub notifier: Arc<ChangeNotifier>,
    pub directory: Directory,
    pub account_id: u64,
    pub submitter: Option<Submitter>,
}

impl JmapContext {
    pub fn from_parts(
        store: Arc<dyn Store>,
        blobs: Arc<dyn BlobStore>,
        notifier: Arc<ChangeNotifier>,
        directory: Directory,
        account_id: u64,
        submitter: Option<Submitter>,
    ) -> Self {
        let mail = MailServices::new(
            Arc::clone(&store),
            Arc::clone(&blobs),
            Arc::clone(&notifier),
        );
        Self {
            store,
            blobs,
            mail,
            notifier,
            directory,
            account_id,
            submitter,
        }
    }
}

#[cfg(test)]
pub fn test_context() -> JmapContext {
    use irixmail_core::IdGenerator;
    use irixmail_store::{FsBlobStore, RocksdbStore};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("irixmail-jmap-ctx-{}-{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.join("db")).unwrap());
    let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(dir.join("blobs")).unwrap());
    let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
    JmapContext::from_parts(
        store,
        blobs,
        Arc::new(ChangeNotifier::new()),
        directory,
        1,
        None,
    )
}

// A context whose account_id is a real account persisted in the directory, so handlers that
// resolve the Account (e.g. Email/copy, Email/import) work end to end.
#[cfg(test)]
pub fn test_context_with_account() -> JmapContext {
    use irixmail_core::IdGenerator;
    use irixmail_directory::{password as pw, Role};
    use irixmail_store::{FsBlobStore, RocksdbStore};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("irixmail-jmap-acct-{}-{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.join("db")).unwrap());
    let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(dir.join("blobs")).unwrap());
    let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
    let domain = directory
        .domains()
        .create("example.com", Vec::new())
        .unwrap();
    let account = directory
        .accounts()
        .create("alice", domain.id, "Alice", Role::User)
        .unwrap();
    directory
        .credentials()
        .set_primary_password(account.id, pw::hash("secret").unwrap())
        .unwrap();
    JmapContext::from_parts(
        store,
        blobs,
        Arc::new(ChangeNotifier::new()),
        directory,
        account.id,
        None,
    )
}

#[cfg(test)]
pub(crate) mod test_flaky {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use irixmail_core::Result;
    use irixmail_store::{Flow, KeyPrefix, Store, WriteOp};

    pub(crate) struct FlakyStore {
        pub inner: Arc<dyn Store>,
        pub fail_puts: AtomicBool,
        pub fail_batches: AtomicBool,
        pub fail_iterates: AtomicBool,
    }

    impl FlakyStore {
        pub(crate) fn wrap(inner: Arc<dyn Store>) -> Arc<Self> {
            Arc::new(Self {
                inner,
                fail_puts: AtomicBool::new(false),
                fail_batches: AtomicBool::new(false),
                fail_iterates: AtomicBool::new(false),
            })
        }
    }

    impl Store for FlakyStore {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            self.inner.get(key)
        }
        fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
            if self.fail_puts.load(Ordering::SeqCst) {
                return Err(irixmail_core::Error::store("injected put failure"));
            }
            self.inner.put(key, value)
        }
        fn delete(&self, key: &[u8]) -> Result<()> {
            self.inner.delete(key)
        }
        fn iterate(
            &self,
            prefix: &KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            if self.fail_iterates.load(Ordering::SeqCst) {
                return Err(irixmail_core::Error::store("injected iterate failure"));
            }
            self.inner.iterate(prefix, visit)
        }
        fn batch(&self, ops: &[WriteOp]) -> Result<()> {
            if self.fail_batches.load(Ordering::SeqCst) {
                return Err(irixmail_core::Error::store("injected batch failure"));
            }
            self.inner.batch(ops)
        }
        fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
            self.inner.add_and_get(key, by)
        }
        fn counter(&self, key: &[u8]) -> Result<i64> {
            self.inner.counter(key)
        }
    }
}
