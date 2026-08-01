use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::time::{interval, MissedTickBehavior};

use irixmail_core::shutdown::ShutdownSignal;

pub const DEFAULT_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

struct Entry {
    value: Vec<u8>,
    expires_at: Instant,
}

impl Entry {
    fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

#[derive(Default)]
pub struct TtlStore {
    entries: Mutex<HashMap<Vec<u8>, Entry>>,
}

impl TtlStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>, ttl: Duration) {
        let entry = Entry {
            value: value.into(),
            expires_at: Instant::now() + ttl,
        };
        self.entries.lock().insert(key.into(), entry);
    }

    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        match entries.get(key) {
            Some(entry) if entry.is_expired(now) => {
                entries.remove(key);
                None
            }
            Some(entry) => Some(entry.value.clone()),
            None => None,
        }
    }

    pub fn contains(&self, key: &[u8]) -> bool {
        let now = Instant::now();
        let entries = self.entries.lock();
        matches!(entries.get(key), Some(entry) if !entry.is_expired(now))
    }

    pub fn remove(&self, key: &[u8]) -> Option<Vec<u8>> {
        let now = Instant::now();
        self.entries
            .lock()
            .remove(key)
            .filter(|entry| !entry.is_expired(now))
            .map(|entry| entry.value)
    }

    pub fn sweep(&self) -> usize {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        let before = entries.len();
        entries.retain(|_, entry| !entry.is_expired(now));
        before - entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }

    pub fn clear(&self) {
        self.entries.lock().clear();
    }

    pub async fn run_sweeper(
        self: Arc<Self>,
        interval_period: Duration,
        mut shutdown: ShutdownSignal,
    ) {
        let mut ticker = interval(interval_period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    self.sweep();
                }
                _ = shutdown.recv() => {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_value_reads_back_within_its_lifetime() {
        let store = TtlStore::new();
        store.set(
            b"greylist:sender".to_vec(),
            b"deferred".to_vec(),
            Duration::from_secs(60),
        );

        assert_eq!(store.get(b"greylist:sender"), Some(b"deferred".to_vec()));
        assert!(store.contains(b"greylist:sender"));
    }

    #[test]
    fn a_missing_key_reads_as_absent() {
        let store = TtlStore::new();
        assert_eq!(store.get(b"never-set"), None);
        assert!(!store.contains(b"never-set"));
    }

    #[test]
    fn a_zero_lifetime_entry_is_already_expired() {
        let store = TtlStore::new();
        store.set(b"k".to_vec(), b"v".to_vec(), Duration::ZERO);

        assert_eq!(store.get(b"k"), None);
        assert!(!store.contains(b"k"));
    }

    #[test]
    fn an_expired_entry_reads_as_absent_and_is_dropped_on_read() {
        let store = TtlStore::new();
        store.set(b"k".to_vec(), b"v".to_vec(), Duration::ZERO);

        assert_eq!(store.len(), 1);
        assert_eq!(store.get(b"k"), None);
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn writing_an_existing_key_replaces_its_value_and_lifetime() {
        let store = TtlStore::new();
        store.set(b"window".to_vec(), b"1".to_vec(), Duration::ZERO);
        store.set(b"window".to_vec(), b"2".to_vec(), Duration::from_secs(60));

        assert_eq!(store.get(b"window"), Some(b"2".to_vec()));
    }

    #[test]
    fn remove_returns_a_live_value_and_clears_the_key() {
        let store = TtlStore::new();
        store.set(b"k".to_vec(), b"v".to_vec(), Duration::from_secs(60));

        assert_eq!(store.remove(b"k"), Some(b"v".to_vec()));
        assert_eq!(store.get(b"k"), None);
        assert!(store.is_empty());
    }

    #[test]
    fn remove_reports_nothing_for_an_expired_entry() {
        let store = TtlStore::new();
        store.set(b"k".to_vec(), b"v".to_vec(), Duration::ZERO);

        assert_eq!(store.remove(b"k"), None);
        assert!(store.is_empty());
    }

    #[test]
    fn sweep_discards_only_expired_entries() {
        let store = TtlStore::new();
        store.set(b"live".to_vec(), b"a".to_vec(), Duration::from_secs(60));
        store.set(b"dead-one".to_vec(), b"b".to_vec(), Duration::ZERO);
        store.set(b"dead-two".to_vec(), b"c".to_vec(), Duration::ZERO);

        let removed = store.sweep();
        assert_eq!(removed, 2, "both already-lapsed entries are reclaimed");

        assert_eq!(store.len(), 1);
        assert_eq!(store.get(b"live"), Some(b"a".to_vec()));
        assert_eq!(store.get(b"dead-one"), None);
    }

    #[test]
    fn sweep_of_an_all_live_partition_removes_nothing() {
        let store = TtlStore::new();
        store.set(b"a".to_vec(), b"1".to_vec(), Duration::from_secs(60));
        store.set(b"b".to_vec(), b"2".to_vec(), Duration::from_secs(60));

        assert_eq!(store.sweep(), 0);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn clear_empties_the_partition_regardless_of_expiry() {
        let store = TtlStore::new();
        store.set(b"a".to_vec(), b"1".to_vec(), Duration::from_secs(60));
        store.set(b"b".to_vec(), b"2".to_vec(), Duration::ZERO);

        store.clear();
        assert!(store.is_empty());
        assert_eq!(store.get(b"a"), None);
    }

    #[tokio::test]
    async fn the_sweeper_reclaims_lapsed_entries_on_each_tick() {
        use irixmail_core::shutdown::Shutdown;
        use tokio::time::{sleep, timeout};

        let store = Arc::new(TtlStore::new());
        store.set(b"short".to_vec(), b"x".to_vec(), Duration::ZERO);
        store.set(b"long".to_vec(), b"y".to_vec(), Duration::from_secs(3600));

        let shutdown = Shutdown::new();
        let signal = shutdown.subscribe();
        let sweeper = tokio::spawn(store.clone().run_sweeper(Duration::from_millis(10), signal));

        timeout(Duration::from_secs(5), async {
            loop {
                if store.get(b"short").is_none() {
                    break;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the lapsed entry should be swept within the timeout");

        assert_eq!(
            store.get(b"long"),
            Some(b"y".to_vec()),
            "the live entry remains"
        );

        shutdown.trigger(irixmail_core::shutdown::ShutdownCause::Internal);
        timeout(Duration::from_secs(5), sweeper)
            .await
            .expect("the sweeper should stop after shutdown")
            .unwrap();
    }

    #[tokio::test]
    async fn the_sweeper_stops_when_shutdown_is_signalled() {
        use irixmail_core::shutdown::{Shutdown, ShutdownCause};
        use tokio::time::timeout;

        let store = Arc::new(TtlStore::new());
        let shutdown = Shutdown::new();
        let sweeper = tokio::spawn(
            store
                .clone()
                .run_sweeper(DEFAULT_SWEEP_INTERVAL, shutdown.subscribe()),
        );

        shutdown.trigger(ShutdownCause::Internal);
        timeout(Duration::from_secs(5), sweeper)
            .await
            .expect("the loop should observe shutdown without waiting out the interval")
            .unwrap();
    }
}
