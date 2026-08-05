use std::time::{SystemTime, UNIX_EPOCH};

use irixmail_core::Result;
use irixmail_directory::Account;
use irixmail_mail::{
    evaluate_vacation, last_vacation_reply, record_vacation_reply, DeliveryOutcome,
    DeliveryRequest, MailServices, VacationConfig, VacationDecision, DEFAULT_PERIOD_SECONDS,
};
use irixmail_store::{Collection, Key, Store, Subspace};

use crate::dsn::build_dsn;
use crate::queue_enqueue::{enqueue, Enqueue};
use crate::queue_model::{Expiry, QueueRecipient, QueuedMessage, RecipientStatus};
use crate::sub_enqueue::DEFAULT_MAX_AGE;

const METRICS_ACCOUNT: u32 = 0;

const INBOUND_SUFFIX: u8 = b'i';

const SECONDS_PER_DAY: u64 = 86_400;

const RELAY_MAX_ATTEMPTS: u32 = 25;

pub struct InboundOutcome {
    pub deliveries: Vec<DeliveryOutcome>,
    pub daily_total: i64,
}

pub fn deliver_inbound(
    services: &MailServices,
    requests: &[DeliveryRequest<'_>],
) -> Result<InboundOutcome> {
    let mut deliveries = Vec::with_capacity(requests.len());
    for request in requests {
        deliveries.push(services.deliver(request)?);
    }

    let daily_total = record_inbound(services.store().as_ref(), now_seconds())?;
    Ok(InboundOutcome {
        deliveries,
        daily_total,
    })
}

pub fn enqueue_relays(services: &MailServices, outcome: &InboundOutcome) {
    let now = now_seconds();
    for delivery in &outcome.deliveries {
        for relay in &delivery.relays {
            let recipients = [(relay.rcpt_to.clone(), Expiry::Attempts(RELAY_MAX_ATTEMPTS))];
            let request = Enqueue {
                created: now,
                return_path: &relay.mail_from,
                recipients: &recipients,
                first_due: now,
            };
            if let Err(err) = enqueue(
                services.store().as_ref(),
                services.blobs().as_ref(),
                &relay.message,
                &request,
            ) {
                tracing::error!(error = %err, recipient = %relay.rcpt_to, "a relay copy could not be queued");
            }
        }
    }
}

pub fn enqueue_forward(
    services: &MailServices,
    mail_from: &str,
    destination: &str,
    raw: &[u8],
    now: u64,
) -> Result<()> {
    let recipients = [(
        destination.to_string(),
        Expiry::Attempts(RELAY_MAX_ATTEMPTS),
    )];
    let request = Enqueue {
        created: now,
        return_path: mail_from,
        recipients: &recipients,
        first_due: now,
    };
    enqueue(
        services.store().as_ref(),
        services.blobs().as_ref(),
        raw,
        &request,
    )?;
    Ok(())
}

const OVER_QUOTA_REASON: &str = "552 5.2.2 Mailbox full";

const REPORTING_MTA: &str = "irixmail";

pub fn bounce_over_quota(
    services: &MailServices,
    recipients: &[&str],
    outcome: &InboundOutcome,
    mail_from: &str,
    raw: &[u8],
    now: u64,
) {
    if mail_from.is_empty() {
        return;
    }
    let bounced: Vec<QueueRecipient> = recipients
        .iter()
        .zip(&outcome.deliveries)
        .filter(|(_, delivery)| delivery.is_over_quota())
        .map(|(address, _)| {
            let mut rcpt = QueueRecipient::new(*address, now, Expiry::Attempts(1));
            rcpt.status = RecipientStatus::Bounced(OVER_QUOTA_REASON.to_string());
            rcpt
        })
        .collect();
    if bounced.is_empty() {
        return;
    }

    let report = QueuedMessage {
        created: now,
        blob_hash: Vec::new(),
        size: raw.len() as u64,
        return_path: mail_from.to_string(),
        recipients: bounced,
    };
    let reporting_mta = services.hostname().unwrap_or(REPORTING_MTA);
    let Some(dsn) = build_dsn(&report, reporting_mta, raw, now) else {
        return;
    };
    let queued = [(
        mail_from.to_string(),
        Expiry::At(now.saturating_add(DEFAULT_MAX_AGE.as_secs())),
    )];
    let request = Enqueue {
        created: now,
        return_path: "",
        recipients: &queued,
        first_due: now,
    };
    if let Err(err) = enqueue(
        services.store().as_ref(),
        services.blobs().as_ref(),
        &dsn,
        &request,
    ) {
        tracing::error!(error = %err, sender = %mail_from, "an over-quota bounce could not be queued");
    }
}

pub fn respond_vacations(
    services: &MailServices,
    recipients: &[(&Account, &str)],
    outcome: &InboundOutcome,
    mail_from: &str,
    raw: &[u8],
    now: u64,
) {
    let store = services.store().as_ref();
    for ((account, recipient), delivery) in recipients.iter().zip(&outcome.deliveries) {
        if delivery.is_over_quota() || !account.vacation.enabled {
            continue;
        }
        let config = VacationConfig {
            enabled: true,
            start: account.vacation.active_from,
            end: account.vacation.active_to,
            period_seconds: DEFAULT_PERIOD_SECONDS,
            subject: (!account.vacation.subject.is_empty())
                .then(|| account.vacation.subject.clone()),
            body: account.vacation.body.clone(),
        };
        let last = match last_vacation_reply(store, account.id, mail_from) {
            Ok(last) => last,
            Err(err) => {
                tracing::error!(error = %err, "the vacation reply log could not be read");
                continue;
            }
        };
        let decision = match evaluate_vacation(&config, raw, mail_from, recipient, now, last) {
            Ok(decision) => decision,
            Err(err) => {
                tracing::error!(error = %err, "the vacation responder could not evaluate a message");
                continue;
            }
        };
        let VacationDecision::Reply(reply) = decision else {
            continue;
        };
        let queued = [(reply.to.clone(), Expiry::Attempts(RELAY_MAX_ATTEMPTS))];
        let request = Enqueue {
            created: now,
            return_path: "",
            recipients: &queued,
            first_due: now,
        };
        match enqueue(store, services.blobs().as_ref(), &reply.message, &request) {
            Ok(_) => {
                if let Err(err) = record_vacation_reply(store, account.id, &reply.to, reply.sent_at)
                {
                    tracing::error!(error = %err, "a sent vacation reply could not be recorded");
                }
            }
            Err(err) => {
                tracing::error!(error = %err, recipient = %reply.to, "a vacation reply could not be queued");
            }
        }
    }
}

pub fn record_inbound(store: &dyn Store, seconds_since_epoch: u64) -> Result<i64> {
    let day = day_number(seconds_since_epoch);
    store.add_and_get(&daily_inbound_key(day), 1)
}

pub fn inbound_total(store: &dyn Store, seconds_since_epoch: u64) -> Result<i64> {
    let day = day_number(seconds_since_epoch);
    store.counter(&daily_inbound_key(day))
}

pub fn day_number(seconds_since_epoch: u64) -> u32 {
    (seconds_since_epoch / SECONDS_PER_DAY) as u32
}

fn daily_inbound_key(day: u32) -> Vec<u8> {
    Key::new(Subspace::Counter, METRICS_ACCOUNT, Collection::Email, day)
        .with_suffix(vec![INBOUND_SUFFIX])
        .encode()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_core::Result;
    use irixmail_directory::{Account, Forwarding, Role, VacationResponder};
    use irixmail_mail::{Mailbox, SpecialUse};
    use irixmail_store::{BlobHash, BlobStore, ChangeNotifier, Flow, KeyPrefix, WriteOp};
    use std::collections::BTreeMap;
    use std::ops::Range;
    use std::sync::{Arc, Mutex};

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

    const INBOX_ID: u32 = 1;

    const MESSAGE: &[u8] = concat!(
        "From: someone@example.com\r\n",
        "To: me@example.org\r\n",
        "Subject: Hello\r\n",
        "\r\n",
        "Body text.\r\n",
    )
    .as_bytes();

    fn services() -> (Arc<MemStore>, MailServices) {
        let store = Arc::new(MemStore::default());
        let mail = MailServices::new(
            store.clone(),
            Arc::new(MemBlobStore::default()),
            Arc::new(ChangeNotifier::new()),
        );
        (store, mail)
    }

    fn account(id: u64) -> Account {
        Account {
            id,
            local_part: "me".to_string(),
            domain_id: 1,
            display_name: String::new(),
            enabled: true,
            role: Role::User,
            aliases: Vec::new(),
            forwarding: Forwarding::default(),
            quota_bytes: 0,
            quota_messages: 0,
            locale: String::new(),
            timezone: String::new(),
            signature: String::new(),
            vacation: VacationResponder::default(),
            created_at: 0,
        }
    }

    fn mailboxes() -> Vec<Mailbox> {
        vec![Mailbox::new(INBOX_ID, "Inbox", SpecialUse::Inbox, 1)]
    }

    fn request<'a>(
        account: &'a Account,
        mailboxes: &'a [Mailbox],
        document_id: u32,
    ) -> DeliveryRequest<'a> {
        DeliveryRequest {
            account,
            mailboxes,
            sieve: None,
            mail_from: "someone@example.com",
            recipient: "me@example.org",
            document_id,
            raw: MESSAGE,
            target_override: None,
            received_at: 1_700_000_000,
        }
    }

    #[test]
    fn the_over_quota_bounce_reports_the_configured_hostname() {
        use irixmail_mail::{DeliveryOutcome, QuotaVerdict};

        let blobs = Arc::new(MemBlobStore::default());
        let store = Arc::new(MemStore::default());
        let mail = MailServices::new(
            store.clone(),
            blobs.clone(),
            Arc::new(ChangeNotifier::new()),
        )
        .with_hostname("mx.real.example");

        let delivered = DeliveryOutcome {
            filed_into: vec![INBOX_ID],
            over_quota: None,
            ..DeliveryOutcome::default()
        };
        let refused = DeliveryOutcome {
            over_quota: Some(QuotaVerdict::OverByteQuota {
                limit: 1,
                would_use: 2,
            }),
            ..DeliveryOutcome::default()
        };
        let outcome = InboundOutcome {
            deliveries: vec![delivered, refused],
            daily_total: 1,
        };

        bounce_over_quota(
            &mail,
            &["ok@d.example", "full@d.example"],
            &outcome,
            "sender@remote.example",
            MESSAGE,
            1_700_000_000,
        );

        let queued = crate::queue_enqueue::load(store.as_ref(), 1)
            .unwrap()
            .expect("the bounce is queued");
        let dsn = blobs
            .get(
                &irixmail_store::BlobHash::from_bytes(queued.blob_hash.clone()),
                0..queued.size as usize,
            )
            .unwrap()
            .expect("the DSN body is stored");
        let text = String::from_utf8_lossy(&dsn);
        assert!(
            text.contains("Reporting-MTA: dns;mx.real.example"),
            "got: {text}"
        );
        assert!(!text.contains("dns;irixmail"), "got: {text}");
    }

    #[test]
    fn a_day_number_buckets_an_instant_by_whole_days() {
        assert_eq!(day_number(0), 0);
        assert_eq!(day_number(SECONDS_PER_DAY - 1), 0);
        assert_eq!(day_number(SECONDS_PER_DAY), 1);
        assert_eq!(day_number(SECONDS_PER_DAY * 3 + 17), 3);
    }

    #[test]
    fn one_day_keeps_its_own_counter_apart_from_the_next() {
        let store = MemStore::default();
        record_inbound(&store, SECONDS_PER_DAY * 3).unwrap();
        record_inbound(&store, SECONDS_PER_DAY * 3 + 10).unwrap();
        record_inbound(&store, SECONDS_PER_DAY * 4).unwrap();

        assert_eq!(inbound_total(&store, SECONDS_PER_DAY * 3).unwrap(), 2);
        assert_eq!(inbound_total(&store, SECONDS_PER_DAY * 4).unwrap(), 1);
        assert_eq!(inbound_total(&store, SECONDS_PER_DAY * 5).unwrap(), 0);
    }

    #[test]
    fn recording_returns_the_running_total_for_the_day() {
        let store = MemStore::default();
        assert_eq!(record_inbound(&store, 0).unwrap(), 1);
        assert_eq!(record_inbound(&store, 100).unwrap(), 2);
        assert_eq!(record_inbound(&store, 200).unwrap(), 3);
    }

    #[test]
    fn the_inbound_and_outbound_keys_for_a_day_are_distinct() {
        let inbound = daily_inbound_key(7);
        let outbound = Key::new(Subspace::Counter, METRICS_ACCOUNT, Collection::Email, 7)
            .with_suffix(vec![b'o'])
            .encode();
        assert_ne!(inbound, outbound);
        assert_eq!(inbound[0], Subspace::Counter.as_byte());
    }

    #[test]
    fn an_accepted_message_is_delivered_and_counted_once() {
        let (store, mail) = services();
        let account = account(7);
        let mailboxes = mailboxes();
        let requests = vec![request(&account, &mailboxes, 10)];

        let outcome = deliver_inbound(&mail, &requests).expect("deliver");

        assert_eq!(outcome.deliveries.len(), 1);
        assert_eq!(outcome.deliveries[0].filed_into, vec![INBOX_ID]);
        assert_eq!(outcome.daily_total, 1);
        assert_eq!(inbound_total(store.as_ref(), now_seconds()).unwrap(), 1);
    }

    #[test]
    fn a_message_fanned_to_several_recipients_counts_as_one_arrival() {
        let (_store, mail) = services();
        let first = account(7);
        let second = account(8);
        let first_boxes = mailboxes();
        let second_boxes = mailboxes();
        let requests = vec![
            request(&first, &first_boxes, 10),
            request(&second, &second_boxes, 10),
        ];

        let outcome = deliver_inbound(&mail, &requests).expect("deliver");

        assert_eq!(outcome.deliveries.len(), 2);
        assert!(outcome.deliveries.iter().all(|d| d.was_filed()));
        assert_eq!(outcome.daily_total, 1);
    }

    #[test]
    fn a_failed_delivery_leaves_the_tally_untouched() {
        let (store, mail) = services();
        let account = account(7);
        let no_inbox = vec![Mailbox::new(2, "Archive", SpecialUse::None, 1)];
        let requests = vec![request(&account, &no_inbox, 10)];

        let result = deliver_inbound(&mail, &requests);
        assert!(result.is_err());
        assert_eq!(inbound_total(store.as_ref(), now_seconds()).unwrap(), 0);
    }
}
