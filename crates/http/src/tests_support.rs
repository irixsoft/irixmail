use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use irixmail_core::{IdGenerator, LogBuffer};
use irixmail_directory::{Directory, RecoveryAdmin, SecretCipher};
use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore, RocksdbStore, Store};

use crate::app::{AppState, TokenInfo};

pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("irixmail-http-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn state(dir: &TempDir) -> AppState {
    build_state(dir, None)
}

pub fn state_with_recovery(dir: &TempDir, admin: RecoveryAdmin) -> AppState {
    build_state(dir, Some(admin))
}

fn build_state(dir: &TempDir, recovery: Option<RecoveryAdmin>) -> AppState {
    let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir.path.join("db")).unwrap());
    let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), recovery);
    let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(dir.path.join("blobs")).unwrap());
    let notifier = Arc::new(ChangeNotifier::new());
    let state = AppState::new(
        directory,
        LogBuffer::new(),
        store,
        blobs,
        notifier,
        "mail.example.com",
        irixmail_dns::Resolver::empty(),
        test_cipher(),
    );
    state.ready.store(true, Ordering::Relaxed);
    state
}

pub fn test_cipher() -> SecretCipher {
    SecretCipher::from_master_key(&SecretCipher::generate_master_key().unwrap()).unwrap()
}

pub fn admin_token(state: &AppState) -> String {
    state.tokens.issue(TokenInfo {
        account_id: 1,
        username: "admin@example.com".into(),
        is_admin: true,
    })
}
