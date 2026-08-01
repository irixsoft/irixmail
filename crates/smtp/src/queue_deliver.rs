use std::sync::Arc;

use irixmail_core::{RelayConfig, Result};
use irixmail_dns::Resolver;
use irixmail_store::{BlobStore, Store, TtlStore};

use crate::deliver_out::{deliver, deliver_via_relay, record_outbound, DeliveryAttempt};
use crate::dsn::build_dsn;
use crate::mx_resolve::{resolve, MxResolution};
use crate::queue_enqueue::{enqueue, persist, remove, Enqueue};
use crate::queue_local::{deliver_local, hosted_domains, route, LocalDelivery, LocalRoute};
use crate::queue_manager::DueBatch;
use crate::queue_model::{Expiry, QueueRecipient, QueuedMessage, RecipientStatus};
use crate::ratelimit_out::{OutboundLimiter, OutboundLimits};
use crate::retry::{next_after_deferral, RetryDecision};
use crate::sub_enqueue::DEFAULT_MAX_AGE;

const REPORTING_MTA: &str = "irixmail";

#[derive(Clone)]
pub struct OutboundDelivery {
    store: Arc<dyn Store>,
    blobs: Arc<dyn BlobStore>,
    resolver: Resolver,
    relay: Option<RelayConfig>,
    hostname: Option<String>,
    rate_counters: Arc<TtlStore>,
    local: Option<LocalDelivery>,
}

impl OutboundDelivery {
    pub fn new(store: Arc<dyn Store>, blobs: Arc<dyn BlobStore>, resolver: Resolver) -> Self {
        Self {
            store,
            blobs,
            resolver,
            relay: None,
            hostname: None,
            rate_counters: Arc::new(TtlStore::new()),
            local: None,
        }
    }

    pub fn with_relay(mut self, relay: Option<RelayConfig>) -> Self {
        self.relay = relay;
        self
    }

    pub fn with_local_delivery(mut self, local: LocalDelivery) -> Self {
        self.local = Some(local);
        self
    }

    pub fn with_hostname(mut self, hostname: impl Into<String>) -> Self {
        self.hostname = Some(hostname.into());
        self
    }

    pub async fn process(&self, batch: DueBatch, now: u64) -> Result<()> {
        for due in batch.messages {
            if let Err(err) = self.process_message(due.id, due.message, now).await {
                tracing::warn!(queue_id = due.id, error = %err, "delivery pass failed; message stays queued");
            }
        }
        Ok(())
    }

    async fn process_message(&self, id: u32, mut message: QueuedMessage, now: u64) -> Result<()> {
        if message.recipients.iter().any(|rcpt| rcpt.is_due(now)) {
            let limiter = OutboundLimiter::new(
                Arc::clone(&self.rate_counters),
                OutboundLimits::from_settings(self.store.as_ref()),
            );
            if !limiter.check(&message.return_path).is_allowed() {
                // waits out the rate window without consuming a retry attempt
                let due = limiter.window_end(now);
                for rcpt in message
                    .recipients
                    .iter_mut()
                    .filter(|rcpt| rcpt.is_due(now))
                {
                    rcpt.retry.due = due;
                    rcpt.status = RecipientStatus::Deferred("outbound rate limited".into());
                }
                return self.commit(id, message);
            }
        }

        let pending_before: Vec<bool> = message
            .recipients
            .iter()
            .map(|rcpt| rcpt.status.is_pending())
            .collect();

        let raw = match self.blobs.get_all(&message.blob_hash())? {
            Some(bytes) => bytes,
            None => {
                for rcpt in message
                    .recipients
                    .iter_mut()
                    .filter(|r| r.status.is_pending())
                {
                    rcpt.status =
                        RecipientStatus::Bounced("the message body is no longer available".into());
                }
                self.raise_bounces(&message, &pending_before, &[], now)?;
                return self.commit(id, message);
            }
        };

        let hosted = self
            .local
            .as_ref()
            .map(|local| hosted_domains(local.directory()));

        for rcpt in message.recipients.iter_mut() {
            if !rcpt.is_due(now) {
                continue;
            }
            let local_host = self.hostname.as_deref();

            let mut target = rcpt.address.clone();
            let mut local_attempt = None;
            if let (Some(local), Some(hosted)) = (&self.local, &hosted) {
                // at most one redirect hop, so an alias pointing at itself cannot spin
                for _ in 0..2 {
                    match route(local, hosted, &target) {
                        Ok(LocalRoute::Deliver { account_id }) => {
                            local_attempt = Some(deliver_local(
                                local,
                                account_id,
                                &message.return_path,
                                &target,
                                &raw,
                                now,
                            ));
                            break;
                        }
                        Ok(LocalRoute::Redirect { destination }) => target = destination,
                        Ok(LocalRoute::Unknown) => {
                            local_attempt = Some(DeliveryAttempt::Bounced(format!(
                                "550 5.1.1 <{target}>: recipient address rejected: user unknown"
                            )));
                            break;
                        }
                        Ok(LocalRoute::Remote) => break,
                        Err(err) => {
                            tracing::warn!(recipient = %target, error = %err, "local routing failed");
                            local_attempt = Some(DeliveryAttempt::Deferred(format!(
                                "local routing failed: {err}"
                            )));
                            break;
                        }
                    }
                }
            }

            let attempt = match local_attempt {
                Some(attempt) => attempt,
                None => match &self.relay {
                    Some(relay) => {
                        deliver_via_relay(relay, local_host, &message.return_path, &target, &raw)
                            .await
                    }
                    None => match resolve(&self.resolver, domain_of(&target)).await {
                        Ok(MxResolution::Targets(targets)) => {
                            deliver(&targets, local_host, &message.return_path, &target, &raw).await
                        }
                        Ok(MxResolution::NoMailAccepted) => DeliveryAttempt::Bounced(
                            "the destination domain does not accept mail".into(),
                        ),
                        Ok(MxResolution::Unresolvable) => {
                            DeliveryAttempt::Deferred("no reachable mail exchange".into())
                        }
                        Err(err) => {
                            DeliveryAttempt::Deferred(format!("MX resolution failed: {err}"))
                        }
                    },
                },
            };
            if attempt.is_delivered() {
                if let Err(err) = record_outbound(self.store.as_ref(), now) {
                    tracing::warn!(error = %err, "could not record the outbound total");
                }
            }
            apply_attempt(rcpt, attempt, now);
            if let Some((address, reason)) = rejection_note(rcpt) {
                tracing::warn!(recipient = %address, reason = %reason, "delivery permanently rejected");
            }
        }

        self.raise_bounces(&message, &pending_before, &raw, now)?;
        self.commit(id, message)
    }

    fn raise_bounces(
        &self,
        message: &QueuedMessage,
        pending_before: &[bool],
        original: &[u8],
        now: u64,
    ) -> Result<()> {
        if message.return_path.is_empty() {
            return Ok(());
        }
        let newly_bounced: Vec<QueueRecipient> = message
            .recipients
            .iter()
            .zip(pending_before)
            .filter(|(rcpt, was_pending)| {
                **was_pending && matches!(rcpt.status, RecipientStatus::Bounced(_))
            })
            .map(|(rcpt, _)| rcpt.clone())
            .collect();
        if newly_bounced.is_empty() {
            return Ok(());
        }

        let report = QueuedMessage {
            recipients: newly_bounced,
            ..message.clone()
        };
        let reporting_mta = self.hostname.as_deref().unwrap_or(REPORTING_MTA);
        let Some(dsn) = build_dsn(&report, reporting_mta, original, now) else {
            return Ok(());
        };

        let recipients = vec![(
            message.return_path.clone(),
            Expiry::At(now.saturating_add(DEFAULT_MAX_AGE.as_secs())),
        )];
        enqueue(
            self.store.as_ref(),
            self.blobs.as_ref(),
            &dsn,
            &Enqueue {
                created: now,
                return_path: "",
                recipients: &recipients,
                first_due: now,
            },
        )?;
        Ok(())
    }

    fn commit(&self, id: u32, message: QueuedMessage) -> Result<()> {
        if message.is_complete() {
            remove(self.store.as_ref(), id)
        } else {
            persist(self.store.as_ref(), id, &message)
        }
    }
}

fn domain_of(address: &str) -> &str {
    address
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or("")
}

fn rejection_note(rcpt: &QueueRecipient) -> Option<(&str, &str)> {
    match &rcpt.status {
        RecipientStatus::Bounced(reason) => Some((rcpt.address.as_str(), reason.as_str())),
        _ => None,
    }
}

fn apply_attempt(rcpt: &mut QueueRecipient, attempt: DeliveryAttempt, now: u64) {
    match attempt {
        DeliveryAttempt::Delivered => rcpt.status = RecipientStatus::Delivered,
        DeliveryAttempt::Bounced(reason) => rcpt.status = RecipientStatus::Bounced(reason),
        DeliveryAttempt::Deferred(reason) => {
            match next_after_deferral(&rcpt.retry, &rcpt.expiry, now) {
                RetryDecision::Retry(schedule) => {
                    rcpt.retry = schedule;
                    rcpt.status = RecipientStatus::Deferred(reason);
                }
                RetryDecision::Bounce => rcpt.status = RecipientStatus::Bounced(reason),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue_model::Expiry;

    use std::collections::BTreeMap;
    use std::ops::Range;
    use std::sync::Mutex;

    use irixmail_store::{BlobHash, Flow, KeyPrefix, WriteOp};

    fn recipient() -> QueueRecipient {
        QueueRecipient::new("user@remote.example", 1_000, Expiry::Attempts(3))
    }

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
                if key.starts_with(&bound) && visit(key, value)? == Flow::Stop {
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
            Ok(Self::read_counter(&self.map.lock().unwrap(), key))
        }
    }

    #[derive(Default)]
    struct MemBlobStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
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
            let sum = bytes
                .iter()
                .fold(0u32, |acc, b| acc.wrapping_add(*b as u32));
            let mut raw = (bytes.len() as u32).to_be_bytes().to_vec();
            raw.extend_from_slice(&sum.to_be_bytes());
            let hash = BlobHash::from_bytes(raw);
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

    fn queue_records(store: &dyn Store) -> Vec<QueuedMessage> {
        let mut records = Vec::new();
        store
            .iterate(
                &KeyPrefix::subspace(irixmail_store::Subspace::Queue),
                &mut |_key, value| {
                    records.push(irixmail_store::serialize::deserialize(value)?);
                    Ok(Flow::Continue)
                },
            )
            .unwrap();
        records
    }

    fn batch_for(id: u32, message: QueuedMessage) -> crate::queue_manager::DueBatch {
        crate::queue_manager::DueBatch {
            messages: vec![crate::queue_manager::DueMessage { id, message }],
            next_due: None,
        }
    }

    fn relay(port: u16) -> Option<irixmail_core::RelayConfig> {
        Some(irixmail_core::RelayConfig {
            host: "127.0.0.1".to_string(),
            port,
            ..irixmail_core::RelayConfig::default()
        })
    }

    const ORIGINAL: &[u8] =
        b"From: alice@d.example\r\nTo: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n";

    #[tokio::test]
    async fn a_transient_blob_error_on_one_message_does_not_abort_the_batch() {
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        struct FlakyBlobStore {
            inner: MemBlobStore,
            poisoned: Mutex<Option<BlobHash>>,
        }

        impl BlobStore for FlakyBlobStore {
            fn get(&self, hash: &BlobHash, range: Range<usize>) -> Result<Option<Vec<u8>>> {
                if self.poisoned.lock().unwrap().as_ref() == Some(hash) {
                    return Err(irixmail_core::Error::store("transient blob failure"));
                }
                self.inner.get(hash, range)
            }
            fn put(&self, bytes: &[u8]) -> Result<BlobHash> {
                self.inner.put(bytes)
            }
            fn delete(&self, hash: &BlobHash) -> Result<()> {
                self.inner.delete(hash)
            }
        }

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs = Arc::new(FlakyBlobStore {
            inner: MemBlobStore::default(),
            poisoned: Mutex::new(None),
        });

        let flaky_rcpts = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let flaky = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &flaky_rcpts,
                first_due: 1_000,
            },
        )
        .unwrap();

        let healthy_rcpts = vec![("carol@remote.example".to_string(), Expiry::Attempts(3))];
        let healthy = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            b"Subject: other\r\n\r\nother body\r\n",
            &Enqueue {
                created: 1_000,
                return_path: "",
                recipients: &healthy_rcpts,
                first_due: 1_000,
            },
        )
        .unwrap();

        *blobs.poisoned.lock().unwrap() = Some(flaky.message.blob_hash());
        blobs.inner.delete(&healthy.message.blob_hash()).unwrap();

        let delivery = OutboundDelivery::new(
            Arc::clone(&store),
            blobs.clone() as Arc<dyn BlobStore>,
            Resolver::empty(),
        );
        let batch = crate::queue_manager::DueBatch {
            messages: vec![
                crate::queue_manager::DueMessage {
                    id: flaky.id,
                    message: flaky.message.clone(),
                },
                crate::queue_manager::DueMessage {
                    id: healthy.id,
                    message: healthy.message.clone(),
                },
            ],
            next_due: None,
        };

        delivery
            .process(batch, 1_000)
            .await
            .expect("one bad message must not abort the batch");

        let untouched = load(store.as_ref(), flaky.id)
            .unwrap()
            .expect("still queued");
        assert_eq!(untouched.recipients[0].status, RecipientStatus::Scheduled);
        assert!(load(store.as_ref(), healthy.id).unwrap().is_none());
    }

    #[tokio::test]
    async fn outbound_sends_over_the_sender_rate_are_deferred_not_blasted() {
        use crate::deliver_out::test_sink::relay_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};
        use crate::queue_manager::{DueBatch, DueMessage};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, capture) = relay_sink(false).await;

        store
            .put(
                &irixmail_store::settings_key(),
                serde_json::json!({ "rateLimits": { "maxMessagesPerSenderPerHour": 1 } })
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();

        let first_rcpts = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let first = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &first_rcpts,
                first_due: 1_000,
            },
        )
        .unwrap();
        let second_rcpts = vec![("carol@remote.example".to_string(), Expiry::Attempts(3))];
        let second = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            b"Subject: second\r\n\r\nsecond body\r\n",
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &second_rcpts,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port));
        let batch = DueBatch {
            messages: vec![
                DueMessage {
                    id: first.id,
                    message: first.message,
                },
                DueMessage {
                    id: second.id,
                    message: second.message,
                },
            ],
            next_due: None,
        };
        delivery.process(batch, 1_000).await.unwrap();

        assert!(
            load(store.as_ref(), first.id).unwrap().is_none(),
            "the first send goes out"
        );
        let survivor = load(store.as_ref(), second.id)
            .unwrap()
            .expect("the over-rate message stays queued");
        let rcpt = &survivor.recipients[0];
        assert!(
            matches!(rcpt.status, RecipientStatus::Deferred(_)),
            "over-rate sends defer instead of delivering: {:?}",
            rcpt.status
        );
        assert_eq!(
            rcpt.retry.attempts, 0,
            "a rate deferral must not consume a delivery attempt"
        );
        assert!(
            rcpt.retry.due > 1_000,
            "the retry waits for the next rate window"
        );

        let seen = capture.lock().unwrap().clone();
        assert_eq!(
            seen.iter()
                .filter(|line| line.starts_with("MAIL FROM:"))
                .count(),
            1,
            "only one message may reach the wire: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_permanent_rejection_enqueues_a_bounce_to_the_sender() {
        use crate::deliver_out::test_sink::rcpt_verdict_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let port = rcpt_verdict_sink(|_| "550 5.1.1 no such user").await;

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port));
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
        let records = queue_records(store.as_ref());
        assert_eq!(
            records.len(),
            1,
            "expected exactly one bounce record: {records:?}"
        );
        let bounce = &records[0];
        assert_eq!(bounce.return_path, "");
        assert_eq!(bounce.recipients.len(), 1);
        assert_eq!(bounce.recipients[0].address, "alice@d.example");
        assert!(bounce.recipients[0].status.is_pending());

        let body = blobs.get_all(&bounce.blob_hash()).unwrap().unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("multipart/report"));
        assert!(text.contains("To: <alice@d.example>"));
        assert!(text.contains("550"));
        assert!(text.contains("Subject: hi"));
    }

    #[tokio::test]
    async fn a_null_sender_message_that_fails_is_never_re_bounced() {
        use crate::deliver_out::test_sink::rcpt_verdict_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let port = rcpt_verdict_sink(|_| "550 5.1.1 no such user").await;

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port));
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
        assert!(queue_records(store.as_ref()).is_empty());
    }

    #[tokio::test]
    async fn an_expired_recipient_bounces_with_a_dsn() {
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());

        let recipients = vec![("bob@remote.example".to_string(), Expiry::At(500))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 400,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 400,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty());
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
        let records = queue_records(store.as_ref());
        assert_eq!(
            records.len(),
            1,
            "expected exactly one bounce record: {records:?}"
        );
        assert_eq!(records[0].return_path, "");
        assert_eq!(records[0].recipients[0].address, "alice@d.example");

        let body = blobs.get_all(&records[0].blob_hash()).unwrap().unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("Status: 5.0.0"));
        assert!(text.contains("Action: failed"));
    }

    #[tokio::test]
    async fn a_recipient_bounced_on_an_earlier_pass_is_not_reported_again() {
        use crate::deliver_out::test_sink::rcpt_verdict_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let port = rcpt_verdict_sink(|line| {
            if line.contains("bob@") {
                "550 5.1.1 no such user"
            } else {
                "452 4.2.2 try again later"
            }
        })
        .await;

        let recipients = vec![
            ("bob@remote.example".to_string(), Expiry::Attempts(9)),
            ("carol@remote.example".to_string(), Expiry::At(2_000)),
        ];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port));
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        let survivor = load(store.as_ref(), enqueued.id)
            .unwrap()
            .expect("still queued");
        delivery
            .process(batch_for(enqueued.id, survivor), 2_000)
            .await
            .unwrap();

        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
        let bounces: Vec<QueuedMessage> = queue_records(store.as_ref())
            .into_iter()
            .filter(|record| record.return_path.is_empty())
            .collect();
        assert_eq!(bounces.len(), 2, "one DSN per pass: {bounces:?}");

        let mut carol_reports = 0;
        for bounce in &bounces {
            let body = blobs.get_all(&bounce.blob_hash()).unwrap().unwrap();
            let text = String::from_utf8_lossy(&body).to_string();
            if text.contains("Final-Recipient: rfc822;carol@remote.example") {
                carol_reports += 1;
                assert!(
                    !text.contains("Final-Recipient: rfc822;bob@remote.example"),
                    "the second DSN must not re-report bob"
                );
            }
        }
        assert_eq!(carol_reports, 1);
    }

    #[tokio::test]
    async fn the_outbound_client_ehlos_with_the_configured_hostname() {
        use crate::deliver_out::test_sink::relay_sink;
        use crate::queue_enqueue::{enqueue, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, capture) = relay_sink(false).await;

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port))
                .with_hostname("mx.d.example");
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        let seen = capture.lock().unwrap().clone();
        assert!(
            seen.iter().any(|line| line == "EHLO mx.d.example"),
            "the outbound EHLO must advertise the configured mail FQDN, not the OS hostname: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_configured_relay_bypasses_mx_resolution_for_queued_mail() {
        use crate::deliver_out::test_sink::relay_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};
        use crate::queue_manager::DueMessage;

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, capture) = relay_sink(false).await;

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            b"Subject: q\r\n\r\nqueued body\r\n",
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(Some(RelayConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    ..RelayConfig::default()
                }));
        let batch = DueBatch {
            messages: vec![DueMessage {
                id: enqueued.id,
                message: enqueued.message,
            }],
            next_due: None,
        };
        delivery.process(batch, 1_000).await.unwrap();

        let seen = capture.lock().unwrap().clone();
        assert!(
            seen.iter().any(|line| line == "DATA:queued body"),
            "sink saw: {seen:?}"
        );
        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_delivered_message_counts_toward_todays_outbound_total() {
        use crate::deliver_out::{outbound_total, test_sink::relay_sink};
        use crate::queue_enqueue::{enqueue, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, _capture) = relay_sink(false).await;

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port));
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        assert_eq!(outbound_total(store.as_ref(), 1_000).unwrap(), 1);
    }

    #[tokio::test]
    async fn a_deferred_message_does_not_count() {
        use crate::deliver_out::{outbound_total, test_sink::rcpt_verdict_sink};
        use crate::queue_enqueue::{enqueue, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let port = rcpt_verdict_sink(|_| "451 4.3.0 try later").await;

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port));
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        assert_eq!(outbound_total(store.as_ref(), 1_000).unwrap(), 0);
    }

    #[tokio::test]
    async fn a_permanent_rejection_does_not_count() {
        use crate::deliver_out::{outbound_total, test_sink::rcpt_verdict_sink};
        use crate::queue_enqueue::{enqueue, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let port = rcpt_verdict_sink(|_| "550 5.1.1 no such user").await;

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port));
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        assert_eq!(outbound_total(store.as_ref(), 1_000).unwrap(), 0);
    }

    #[tokio::test]
    async fn two_recipients_delivered_in_one_pass_count_twice() {
        use crate::deliver_out::{outbound_total, test_sink::relay_sink};
        use crate::queue_enqueue::{enqueue, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, _capture) = relay_sink(false).await;

        let recipients = vec![
            ("bob@remote.example".to_string(), Expiry::Attempts(3)),
            ("carol@remote.example".to_string(), Expiry::Attempts(3)),
        ];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port));
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        assert_eq!(outbound_total(store.as_ref(), 1_000).unwrap(), 2);
    }

    fn hosted_setup(
        store: &Arc<dyn Store>,
        blobs: &Arc<dyn BlobStore>,
    ) -> (
        irixmail_directory::Directory,
        crate::queue_local::LocalDelivery,
    ) {
        let directory = irixmail_directory::Directory::new(
            Arc::clone(store),
            Arc::new(irixmail_core::IdGenerator::new(1)),
            None,
        );
        let mail = irixmail_mail::MailServices::new(
            Arc::clone(store),
            Arc::clone(blobs),
            Arc::new(irixmail_store::ChangeNotifier::new()),
        );
        let local = crate::queue_local::LocalDelivery::new(directory.clone(), mail);
        (directory, local)
    }

    #[tokio::test]
    async fn relay_mode_delivers_a_hosted_recipient_into_the_local_mailbox() {
        use crate::deliver_out::test_sink::relay_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, capture) = relay_sink(false).await;
        let (directory, local) = hosted_setup(&store, &blobs);
        let domain = directory
            .domains()
            .create("hosted.example", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("bob", domain.id, "Bob", irixmail_directory::Role::User)
            .unwrap();

        let recipients = vec![("bob@hosted.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@hosted.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port))
                .with_local_delivery(local);
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        let seen = capture.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|line| line.starts_with("MAIL FROM:")),
            "mail for a hosted domain must never reach the relay: {seen:?}"
        );
        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
        let filed =
            irixmail_mail::load_raw(store.as_ref(), blobs.as_ref(), account.id as u32, 1).unwrap();
        assert!(filed.is_some(), "the message must land in the mailbox");
    }

    #[tokio::test]
    async fn an_unknown_recipient_at_a_hosted_domain_bounces_instead_of_relaying() {
        use crate::deliver_out::test_sink::relay_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, capture) = relay_sink(false).await;
        let (directory, local) = hosted_setup(&store, &blobs);
        directory
            .domains()
            .create("hosted.example", Vec::new())
            .unwrap();

        let recipients = vec![("nobody@hosted.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@remote.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port))
                .with_local_delivery(local);
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        let seen = capture.lock().unwrap().clone();
        assert!(seen.is_empty(), "nothing may reach the relay: {seen:?}");
        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
        let records = queue_records(store.as_ref());
        assert_eq!(records.len(), 1, "expected one DSN: {records:?}");
        assert_eq!(records[0].return_path, "");
        assert_eq!(records[0].recipients[0].address, "alice@remote.example");
        let body = blobs.get_all(&records[0].blob_hash()).unwrap().unwrap();
        assert!(String::from_utf8_lossy(&body).contains("550"));
    }

    #[tokio::test]
    async fn a_remote_recipient_still_goes_to_the_relay_when_local_delivery_is_configured() {
        use crate::deliver_out::test_sink::relay_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, capture) = relay_sink(false).await;
        let (directory, local) = hosted_setup(&store, &blobs);
        directory
            .domains()
            .create("hosted.example", Vec::new())
            .unwrap();

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            b"Subject: q\r\n\r\nqueued body\r\n",
            &Enqueue {
                created: 1_000,
                return_path: "alice@hosted.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port))
                .with_local_delivery(local);
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        let seen = capture.lock().unwrap().clone();
        assert!(
            seen.iter().any(|line| line == "DATA:queued body"),
            "sink saw: {seen:?}"
        );
        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_hosted_alias_domain_is_delivered_locally() {
        use crate::deliver_out::test_sink::relay_sink;
        use crate::queue_enqueue::{enqueue, load, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (port, capture) = relay_sink(false).await;
        let (directory, local) = hosted_setup(&store, &blobs);
        let domain = directory
            .domains()
            .create("hosted.example", vec!["alt.example".to_string()])
            .unwrap();
        let mut account = directory
            .accounts()
            .create("bob", domain.id, "Bob", irixmail_directory::Role::User)
            .unwrap();
        account.aliases = vec!["bob@alt.example".to_string()];
        directory.accounts().update(account.clone()).unwrap();

        let recipients = vec![("bob@alt.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@hosted.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port))
                .with_local_delivery(local);
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        let seen = capture.lock().unwrap().clone();
        assert!(seen.is_empty(), "an alias domain is ours: {seen:?}");
        assert!(load(store.as_ref(), enqueued.id).unwrap().is_none());
        let filed =
            irixmail_mail::load_raw(store.as_ref(), blobs.as_ref(), account.id as u32, 1).unwrap();
        assert!(filed.is_some(), "the message must land in the mailbox");
    }

    #[tokio::test]
    async fn a_locally_delivered_recipient_counts_as_sent() {
        use crate::deliver_out::outbound_total;
        use crate::queue_enqueue::{enqueue, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let (directory, local) = hosted_setup(&store, &blobs);
        let domain = directory
            .domains()
            .create("hosted.example", Vec::new())
            .unwrap();
        directory
            .accounts()
            .create("bob", domain.id, "Bob", irixmail_directory::Role::User)
            .unwrap();

        let recipients = vec![("bob@hosted.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@hosted.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_local_delivery(local);
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        assert_eq!(outbound_total(store.as_ref(), 1_000).unwrap(), 1);
    }

    #[tokio::test]
    async fn the_bounce_report_names_the_configured_hostname() {
        use crate::deliver_out::test_sink::rcpt_verdict_sink;
        use crate::queue_enqueue::{enqueue, Enqueue};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let port = rcpt_verdict_sink(|_| "550 5.1.1 no such user").await;

        let recipients = vec![("bob@remote.example".to_string(), Expiry::Attempts(3))];
        let enqueued = enqueue(
            store.as_ref(),
            blobs.as_ref(),
            ORIGINAL,
            &Enqueue {
                created: 1_000,
                return_path: "alice@d.example",
                recipients: &recipients,
                first_due: 1_000,
            },
        )
        .unwrap();

        let delivery =
            OutboundDelivery::new(Arc::clone(&store), Arc::clone(&blobs), Resolver::empty())
                .with_relay(relay(port))
                .with_hostname("mx.d.example");
        delivery
            .process(batch_for(enqueued.id, enqueued.message), 1_000)
            .await
            .unwrap();

        let records = queue_records(store.as_ref());
        let body = blobs.get_all(&records[0].blob_hash()).unwrap().unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("MAILER-DAEMON@mx.d.example"));
        assert!(text.contains("Reporting-MTA: dns;mx.d.example"));
    }

    #[test]
    fn a_bounced_recipient_produces_a_rejection_note() {
        let mut rcpt = recipient();
        rcpt.status = RecipientStatus::Bounced("550 no such user".into());
        assert_eq!(
            rejection_note(&rcpt),
            Some(("user@remote.example", "550 no such user"))
        );
    }

    #[test]
    fn a_delivered_recipient_produces_no_note() {
        let mut rcpt = recipient();
        rcpt.status = RecipientStatus::Delivered;
        assert_eq!(rejection_note(&rcpt), None);
    }

    #[test]
    fn a_delivered_attempt_settles_the_recipient() {
        let mut rcpt = recipient();
        apply_attempt(&mut rcpt, DeliveryAttempt::Delivered, 1_000);
        assert_eq!(rcpt.status, RecipientStatus::Delivered);
        assert!(rcpt.status.is_settled());
    }

    #[test]
    fn a_bounce_settles_with_the_reason() {
        let mut rcpt = recipient();
        apply_attempt(
            &mut rcpt,
            DeliveryAttempt::Bounced("550 no such user".into()),
            1_000,
        );
        assert!(matches!(rcpt.status, RecipientStatus::Bounced(_)));
    }

    #[test]
    fn a_deferral_reschedules_and_advances_attempts() {
        let mut rcpt = recipient();
        apply_attempt(
            &mut rcpt,
            DeliveryAttempt::Deferred("451 try later".into()),
            1_000,
        );
        assert!(matches!(rcpt.status, RecipientStatus::Deferred(_)));
        assert_eq!(rcpt.retry.attempts, 1);
        assert!(rcpt.retry.due > 1_000);
    }

    #[test]
    fn a_deferral_past_the_attempt_limit_bounces() {
        let mut rcpt = recipient();
        rcpt.retry.attempts = 3;
        apply_attempt(
            &mut rcpt,
            DeliveryAttempt::Deferred("451 try later".into()),
            1_000,
        );
        assert!(matches!(rcpt.status, RecipientStatus::Bounced(_)));
    }
}
