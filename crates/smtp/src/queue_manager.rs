use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;

use irixmail_core::registry::Registry;
use irixmail_core::shutdown::ShutdownSignal;
use irixmail_core::Result;
use irixmail_store::{Flow, KeyPrefix, Store, Subspace};

use crate::queue_model::QueuedMessage;

pub const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

const QUEUE_ID_OFFSET: usize = 1 + std::mem::size_of::<u32>() + 1;
const QUEUE_ID_LEN: usize = std::mem::size_of::<u32>();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueMessage {
    pub id: u32,
    pub message: QueuedMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueBatch {
    pub messages: Vec<DueMessage>,
    pub next_due: Option<u64>,
}

pub fn scan_due(store: &dyn Store, now: u64) -> Result<DueBatch> {
    let mut messages = Vec::new();
    let mut next_due: Option<u64> = None;

    store.iterate(&KeyPrefix::subspace(Subspace::Queue), &mut |key, value| {
        let message: QueuedMessage = irixmail_store::serialize::deserialize(value)?;

        if message.is_complete() {
            return Ok(Flow::Continue);
        }

        if message.has_due_recipient(now) {
            let id = queue_id_of(key)?;
            messages.push(DueMessage { id, message });
        } else if let Some(due) = message.next_due() {
            next_due = Some(match next_due {
                Some(earliest) => earliest.min(due),
                None => due,
            });
        }

        Ok(Flow::Continue)
    })?;

    Ok(DueBatch { messages, next_due })
}

pub fn scan_all(store: &dyn Store) -> Result<Vec<DueMessage>> {
    let mut messages = Vec::new();
    store.iterate(&KeyPrefix::subspace(Subspace::Queue), &mut |key, value| {
        let message: QueuedMessage = irixmail_store::serialize::deserialize(value)?;
        if !message.is_complete() {
            messages.push(DueMessage {
                id: queue_id_of(key)?,
                message,
            });
        }
        Ok(Flow::Continue)
    })?;
    Ok(messages)
}

pub fn next_wake(now: u64, next_due: Option<u64>) -> Instant {
    let refresh = REFRESH_INTERVAL.as_secs();
    let delay = match next_due {
        Some(due) => due.saturating_sub(now).min(refresh),
        None => refresh,
    };
    Instant::now() + Duration::from_secs(delay)
}

pub type Wakeup = mpsc::Sender<()>;

pub fn wakeup_channel() -> (Wakeup, mpsc::Receiver<()>) {
    mpsc::channel(1)
}

pub async fn run<F, Fut>(
    store: &dyn Store,
    clock: impl Fn() -> u64,
    mut wakeups: mpsc::Receiver<()>,
    mut shutdown: ShutdownSignal,
    deliver: F,
) -> Result<()>
where
    F: Fn(DueBatch) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    loop {
        let now = clock();
        let batch = match scan_due(store, now) {
            Ok(batch) => batch,
            Err(err) => {
                tracing::warn!(error = %err, "queue scan failed; retrying on the next wake");
                DueBatch {
                    messages: Vec::new(),
                    next_due: None,
                }
            }
        };
        let wake_at = next_wake(now, batch.next_due);
        if let Err(err) = deliver(batch).await {
            tracing::warn!(error = %err, "delivery pass failed; retrying on the next wake");
        }

        tokio::select! {
            biased;
            _ = shutdown.recv() => return Ok(()),
            signal = wakeups.recv() => {
                if signal.is_none() {
                    return Ok(());
                }
            }
            _ = tokio::time::sleep_until(wake_at) => {}
        }
    }
}

pub fn register_outbound<C, F, Fut>(
    registry: &Registry,
    store: Arc<dyn Store>,
    clock: C,
    wakeups: mpsc::Receiver<()>,
    shutdown: ShutdownSignal,
    deliver: F,
) where
    C: Fn() -> u64 + Send + 'static,
    F: Fn(DueBatch) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send,
{
    registry.register_background("smtp:queue", move || async move {
        if let Err(err) = run(store.as_ref(), clock, wakeups, shutdown, deliver).await {
            tracing::error!(error = %err, "outbound queue manager stopped");
        }
    });
}

fn queue_id_of(key: &[u8]) -> Result<u32> {
    let end = QUEUE_ID_OFFSET + QUEUE_ID_LEN;
    if key.len() < end {
        return Err(irixmail_core::Error::store(
            "queue key is too short to carry a queue id",
        ));
    }
    let mut bytes = [0u8; QUEUE_ID_LEN];
    bytes.copy_from_slice(&key[QUEUE_ID_OFFSET..end]);
    Ok(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue_enqueue::{enqueue, Enqueue};
    use crate::queue_model::{Expiry, RecipientStatus};
    use irixmail_core::Result;
    use irixmail_store::{BlobHash, BlobStore, KeyPrefix, WriteOp};
    use std::collections::BTreeMap;
    use std::ops::Range;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemStore {
        fn read_counter(map: &BTreeMap<Vec<u8>, Vec<u8>>, key: &[u8]) -> i64 {
            map.get(key)
                .map(|bytes| {
                    let mut array = [0u8; 8];
                    array.copy_from_slice(bytes);
                    i64::from_le_bytes(array)
                })
                .unwrap_or(0)
        }
    }

    impl Store for MemStore {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            Ok(self.map.lock().unwrap().get(key).cloned())
        }

        fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
            self.map
                .lock()
                .unwrap()
                .insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &[u8]) -> Result<()> {
            self.map.lock().unwrap().remove(key);
            Ok(())
        }

        fn iterate(
            &self,
            prefix: &KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            let bound = prefix.encode();
            let map = self.map.lock().unwrap();
            for (key, value) in map.iter() {
                if !key.starts_with(&bound) {
                    continue;
                }
                if visit(key, value)? == Flow::Stop {
                    break;
                }
            }
            Ok(())
        }

        fn batch(&self, ops: &[WriteOp]) -> Result<()> {
            let mut map = self.map.lock().unwrap();
            for op in ops {
                match op {
                    WriteOp::Set { key, value } => {
                        map.insert(key.clone(), value.clone());
                    }
                    WriteOp::Delete { key } => {
                        map.remove(key);
                    }
                    WriteOp::Add { key, by } => {
                        let next = Self::read_counter(&map, key) + by;
                        map.insert(key.clone(), next.to_le_bytes().to_vec());
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
            let mut map = self.map.lock().unwrap();
            let next = Self::read_counter(&map, key) + by;
            map.insert(key.to_vec(), next.to_le_bytes().to_vec());
            Ok(next)
        }

        fn counter(&self, key: &[u8]) -> Result<i64> {
            let map = self.map.lock().unwrap();
            Ok(Self::read_counter(&map, key))
        }
    }

    #[derive(Default)]
    struct MemBlobStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemBlobStore {
        fn digest(bytes: &[u8]) -> BlobHash {
            let sum = bytes
                .iter()
                .fold(0u32, |acc, b| acc.wrapping_add(*b as u32));
            let mut raw = (bytes.len() as u32).to_be_bytes().to_vec();
            raw.extend_from_slice(&sum.to_be_bytes());
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

    const BODY: &[u8] = b"From: s@example.com\r\nTo: a@one.example\r\n\r\nHello.\r\n";

    // leaks the coordinator so the signal can never fire mid-test
    fn idle_signal() -> ShutdownSignal {
        Box::leak(Box::new(irixmail_core::shutdown::Shutdown::new())).subscribe()
    }

    fn admit(store: &dyn Store, blobs: &dyn BlobStore, address: &str, first_due: u64) -> u32 {
        let recipients = vec![(address.to_string(), Expiry::Attempts(5))];
        let request = Enqueue {
            created: first_due,
            return_path: "s@example.com",
            recipients: &recipients,
            first_due,
        };
        enqueue(store, blobs, BODY, &request).expect("enqueue").id
    }

    #[test]
    fn a_scan_collects_messages_whose_recipients_have_come_due() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let early = admit(&store, &blobs, "a@one.example", 1_000);
        let late = admit(&store, &blobs, "b@two.example", 5_000);

        let batch = scan_due(&store, 1_000).expect("scan");

        assert_eq!(batch.messages.len(), 1);
        assert_eq!(batch.messages[0].id, early);
        assert_ne!(batch.messages[0].id, late);
        assert_eq!(batch.next_due, Some(5_000));
    }

    #[test]
    fn a_scan_with_nothing_due_reports_the_earliest_future_instant() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        admit(&store, &blobs, "a@one.example", 4_000);
        admit(&store, &blobs, "b@two.example", 2_500);

        let batch = scan_due(&store, 1_000).expect("scan");

        assert!(batch.messages.is_empty());
        assert_eq!(batch.next_due, Some(2_500));
    }

    #[test]
    fn a_completed_message_is_skipped_and_does_not_move_the_wakeup() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let id = admit(&store, &blobs, "a@one.example", 1_000);

        let mut message = crate::queue_enqueue::load(&store, id)
            .expect("load")
            .expect("present");
        message.recipients[0].status = RecipientStatus::Delivered;
        let bytes = irixmail_store::serialize::archive(&message).expect("archive");
        store
            .put(
                &irixmail_store::Key::new(
                    Subspace::Queue,
                    0,
                    irixmail_store::Collection::EmailSubmission,
                    id,
                )
                .encode(),
                &bytes,
            )
            .expect("put");

        let batch = scan_due(&store, 2_000).expect("scan");
        assert!(batch.messages.is_empty());
        assert_eq!(batch.next_due, None);
    }

    #[test]
    fn a_full_scan_lists_messages_that_are_not_yet_due() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let id = admit(&store, &blobs, "a@one.example", 5_000);

        let all = scan_all(&store).expect("scan");

        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].message.return_path, "s@example.com");
        assert!(scan_due(&store, 1_000).expect("scan").messages.is_empty());
    }

    #[test]
    fn a_full_scan_skips_completed_messages() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let id = admit(&store, &blobs, "a@one.example", 1_000);

        let mut message = crate::queue_enqueue::load(&store, id)
            .expect("load")
            .expect("present");
        message.recipients[0].status = RecipientStatus::Delivered;
        crate::queue_enqueue::persist(&store, id, &message).expect("persist");

        assert!(scan_all(&store).expect("scan").is_empty());
    }

    #[test]
    fn an_empty_queue_yields_an_empty_batch_with_no_wakeup() {
        let store = MemStore::default();
        let batch = scan_due(&store, 9_000).expect("scan");
        assert!(batch.messages.is_empty());
        assert_eq!(batch.next_due, None);
    }

    #[test]
    fn the_next_wakeup_takes_an_imminent_due_time_over_the_refresh_cadence() {
        let now = 1_000;
        let soon = next_wake(now, Some(now + 30));
        let cadence = next_wake(now, None);
        assert!(soon < cadence);
    }

    #[test]
    fn the_next_wakeup_caps_a_distant_due_time_at_the_refresh_cadence() {
        let now = 1_000;
        let distant = next_wake(now, Some(now + REFRESH_INTERVAL.as_secs() * 10));
        let cadence = next_wake(now, None);
        let skew = distant.saturating_duration_since(cadence);
        assert!(skew <= Duration::from_secs(1));
    }

    #[test]
    fn a_past_due_time_wakes_the_manager_at_once() {
        let now = 5_000;
        let wake = next_wake(now, Some(1_000));
        assert!(wake <= Instant::now() + Duration::from_millis(50));
    }

    #[tokio::test]
    async fn the_loop_scans_then_stops_when_the_wakeup_channel_closes() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        admit(&store, &blobs, "a@one.example", 1_000);

        let (wakeup, rx) = wakeup_channel();
        let seen = Arc::new(AtomicU64::new(0));
        let counter = seen.clone();

        let handle = tokio::spawn(async move {
            run(
                &store,
                || 2_000,
                rx,
                idle_signal(),
                move |batch| {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(batch.messages.len() as u64, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .await
        });

        tokio::task::yield_now().await;
        drop(wakeup);
        handle.await.expect("join").expect("run");

        assert!(seen.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn a_wakeup_nudge_triggers_another_scan_before_the_cadence() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        admit(&store, &blobs, "a@one.example", 1_000);

        let (wakeup, rx) = wakeup_channel();
        let passes = Arc::new(AtomicU64::new(0));
        let counter = passes.clone();

        let handle = tokio::spawn(async move {
            run(
                &store,
                || 2_000,
                rx,
                idle_signal(),
                move |_batch| {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .await
        });

        tokio::task::yield_now().await;
        let first = passes.load(Ordering::SeqCst);
        wakeup.send(()).await.expect("nudge");
        tokio::task::yield_now().await;
        assert!(passes.load(Ordering::SeqCst) > first);

        drop(wakeup);
        handle.await.expect("join").expect("run");
    }

    #[tokio::test]
    async fn the_loop_survives_a_failed_delivery_pass() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        admit(&store, &blobs, "a@one.example", 1_000);

        let (wakeup, rx) = wakeup_channel();
        let passes = Arc::new(AtomicU64::new(0));
        let counter = passes.clone();

        let handle = tokio::spawn(async move {
            run(
                &store,
                || 2_000,
                rx,
                idle_signal(),
                move |_batch| {
                    let counter = counter.clone();
                    async move {
                        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                            Err(irixmail_core::Error::store("transient delivery failure"))
                        } else {
                            Ok(())
                        }
                    }
                },
            )
            .await
        });

        tokio::task::yield_now().await;
        assert_eq!(passes.load(Ordering::SeqCst), 1);
        wakeup
            .send(())
            .await
            .expect("the loop must survive a failed pass");
        tokio::task::yield_now().await;
        assert!(passes.load(Ordering::SeqCst) >= 2);

        drop(wakeup);
        handle.await.expect("join").expect("run");
    }

    #[tokio::test]
    async fn the_loop_survives_a_transient_scan_error() {
        struct FlakyScanStore {
            inner: MemStore,
            failed_once: std::sync::atomic::AtomicBool,
        }

        impl Store for FlakyScanStore {
            fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
                self.inner.get(key)
            }
            fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
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
                if !self.failed_once.swap(true, Ordering::SeqCst) {
                    return Err(irixmail_core::Error::store("transient scan failure"));
                }
                self.inner.iterate(prefix, visit)
            }
            fn batch(&self, ops: &[WriteOp]) -> Result<()> {
                self.inner.batch(ops)
            }
            fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
                self.inner.add_and_get(key, by)
            }
            fn counter(&self, key: &[u8]) -> Result<i64> {
                self.inner.counter(key)
            }
        }

        let store = FlakyScanStore {
            inner: MemStore::default(),
            failed_once: std::sync::atomic::AtomicBool::new(false),
        };
        let blobs = MemBlobStore::default();
        admit(&store.inner, &blobs, "a@one.example", 1_000);

        let (wakeup, rx) = wakeup_channel();
        let seen = Arc::new(AtomicU64::new(0));
        let counter = seen.clone();

        let handle = tokio::spawn(async move {
            run(
                &store,
                || 2_000,
                rx,
                idle_signal(),
                move |batch| {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(batch.messages.len() as u64, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .await
        });

        tokio::task::yield_now().await;
        wakeup
            .send(())
            .await
            .expect("the loop must survive a scan error");
        tokio::task::yield_now().await;
        assert!(seen.load(Ordering::SeqCst) >= 1);

        drop(wakeup);
        handle.await.expect("join").expect("run");
    }

    #[tokio::test]
    async fn the_loop_exits_when_the_shutdown_signal_triggers() {
        let store = MemStore::default();
        let shutdown = irixmail_core::shutdown::Shutdown::new();
        let signal = shutdown.subscribe();
        let (wakeup, rx) = wakeup_channel();

        let handle = tokio::spawn(async move {
            run(&store, || 2_000, rx, signal, |_batch| async { Ok(()) }).await
        });

        tokio::task::yield_now().await;
        shutdown.trigger(irixmail_core::shutdown::ShutdownCause::Terminate);
        let joined = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("the loop must exit when shutdown triggers");
        joined.expect("join").expect("run");
        drop(wakeup);
    }

    #[test]
    fn the_queue_id_reads_back_from_an_encoded_key() {
        let key = irixmail_store::Key::new(
            Subspace::Queue,
            0,
            irixmail_store::Collection::EmailSubmission,
            0x0102_0304,
        )
        .encode();
        assert_eq!(queue_id_of(&key).expect("id"), 0x0102_0304);
    }

    #[test]
    fn a_truncated_key_is_rejected_rather_than_read_past_its_end() {
        assert!(queue_id_of(&[0u8; 3]).is_err());
    }

    #[tokio::test]
    async fn registering_appends_one_background_task() {
        let registry = irixmail_core::registry::Registry::new();
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let (_wakeup, rx) = wakeup_channel();

        register_outbound(
            &registry,
            store,
            || 0,
            rx,
            idle_signal(),
            |_batch| async { Ok(()) },
        );

        assert_eq!(registry.len(), 1);
        let registered = registry.registered();
        assert_eq!(registered[0].0, "smtp:queue");
        assert_eq!(
            registered[0].1,
            irixmail_core::registry::ServiceKind::Background
        );
    }

    #[tokio::test]
    async fn the_registered_loop_scans_then_stops_when_its_wakeup_closes() {
        let registry = irixmail_core::registry::Registry::new();
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs = MemBlobStore::default();
        admit(store.as_ref(), &blobs, "a@one.example", 1_000);

        let (wakeup, rx) = wakeup_channel();
        let seen = Arc::new(AtomicU64::new(0));
        let counter = seen.clone();

        register_outbound(
            &registry,
            store,
            || 2_000,
            rx,
            idle_signal(),
            move |batch| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(batch.messages.len() as u64, Ordering::SeqCst);
                    Ok(())
                }
            },
        );

        let mut tasks = registry.start_all();
        tokio::task::yield_now().await;
        drop(wakeup);
        while tasks.join_next().await.is_some() {}

        assert!(seen.load(Ordering::SeqCst) >= 1);
    }
}
