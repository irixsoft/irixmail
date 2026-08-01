use irixmail_core::{Error, Result};
use irixmail_directory::Account;
use irixmail_store::{
    serialize, BatchBuilder, BlobStore, ChangeKind, ChangeLog, ChangeNotifier, Collection, Key,
    NewMailNotice, Quota, Store, Subspace,
};

use crate::blob_store_msg::{reference_op, store_blob};
use crate::forward::{plan_forward, ForwardRelay};
use crate::index::message_text;
use crate::mailbox::{Mailbox, SpecialUse};
use crate::message_data::{Keyword, MessageData};
use crate::quota_enforce::{enforce_quota, limits_for, QuotaVerdict};

const DELIVERY_COLLECTION: Collection = Collection::Email;

const METADATA_SUFFIX: u8 = b'm';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryTarget {
    Role(SpecialUse),
    Mailbox(u32),
}

pub struct DeliveryRequest<'a> {
    pub account: &'a Account,
    pub mailboxes: &'a [Mailbox],
    pub mail_from: &'a str,
    pub recipient: &'a str,
    pub document_id: u32,
    pub raw: &'a [u8],
    pub target_override: Option<DeliveryTarget>,
    pub received_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeliveryOutcome {
    pub filed_into: Vec<u32>,
    pub relays: Vec<ForwardRelay>,
    pub over_quota: Option<QuotaVerdict>,
}

impl DeliveryOutcome {
    pub fn was_filed(&self) -> bool {
        !self.filed_into.is_empty()
    }

    pub fn is_over_quota(&self) -> bool {
        self.over_quota.is_some()
    }
}

pub fn deliver(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    notifier: &ChangeNotifier,
    request: &DeliveryRequest<'_>,
) -> Result<DeliveryOutcome> {
    let account_id = request.account.id as u32;

    let mut metadata = crate::ingest::ingest(blob_placeholder(), request.raw)?;

    let forward = plan_forward(
        &request.account.forwarding,
        request.recipient,
        request.mail_from,
        request.raw,
    );
    let relays = forward.relays;

    if !forward.keep_local {
        return Ok(DeliveryOutcome {
            relays,
            ..DeliveryOutcome::default()
        });
    }

    let inbox = find_inbox(request.mailboxes)?;
    let targets = match request.target_override {
        Some(DeliveryTarget::Role(role)) => {
            vec![find_role(request.mailboxes, role).unwrap_or(inbox)]
        }
        Some(DeliveryTarget::Mailbox(id)) => vec![request
            .mailboxes
            .iter()
            .find(|m| m.id == id)
            .unwrap_or(inbox)],
        None => vec![inbox],
    };

    let size = request.raw.len() as u64;
    let limits = limits_for(request.account);
    let usage = Quota::new(store).usage(account_id)?;
    let verdict = enforce_quota(limits, usage, size);
    if verdict.is_over_quota() {
        return Ok(DeliveryOutcome {
            relays,
            over_quota: Some(verdict),
            ..DeliveryOutcome::default()
        });
    }

    // Body to the blob store first so the records never point at a missing blob.
    let blob_hash = store_blob(blobs, request.raw)?;
    metadata.blob_hash = blob_hash.clone().into_bytes();

    let threading =
        crate::threading::resolve_thread(store, account_id, request.document_id, request.raw)?;
    let mut data = MessageData::new(threading.thread_id, size as u32);
    data.received_at = request.received_at;
    data.sent_at = crate::index::message_sent_at(request.raw);
    let mut filed_into = Vec::with_capacity(targets.len());
    for mailbox in &targets {
        let uid = mailbox.next_uid(store, account_id)?;
        data.add_mailbox(mailbox.id, uid);
        filed_into.push(mailbox.id);
    }
    let mut batch = BatchBuilder::new();
    batch.set(
        metadata_key(account_id, request.document_id),
        serialize::archive(&metadata)?,
    );
    batch.set(
        data_key(account_id, request.document_id),
        serialize::archive(&data)?,
    );
    batch.extend(threading.ops);
    batch.push(reference_op(&blob_hash));
    batch.push(crate::blob_store_msg::account_link_op(
        account_id, &blob_hash, 1,
    ));
    batch.extend(Quota::new(store).adjust_ops(account_id, size as i64, 1));
    let text = message_text(request.raw).unwrap_or_default();
    batch.extend(crate::index::index_ops(
        store,
        account_id,
        request.document_id,
        &text,
    )?);
    let (change_id, change_op) = ChangeLog::new(store).record_op(
        account_id,
        DELIVERY_COLLECTION,
        request.document_id,
        ChangeKind::Insert,
    )?;
    batch.push(change_op);
    store.batch(&batch.build())?;

    notifier.notify_change(account_id, DELIVERY_COLLECTION, change_id);
    if request.target_override.is_none() {
        let noteworthy = targets.iter().find(|mailbox| {
            !matches!(
                mailbox.role,
                SpecialUse::Junk | SpecialUse::Trash | SpecialUse::Sent | SpecialUse::Drafts
            )
        });
        if let Some(mailbox) = noteworthy {
            notifier.notify_new_mail(NewMailNotice {
                account_id,
                document_id: request.document_id,
                mailbox_id: mailbox.id,
                sender: crate::index::message_sender(request.raw),
                subject: text.subject.clone(),
            });
        }
    }

    Ok(DeliveryOutcome {
        filed_into,
        relays,
        over_quota: None,
    })
}

pub struct AppendRequest<'a> {
    pub account: &'a Account,
    pub mailbox: &'a Mailbox,
    pub flags: Vec<Keyword>,
    pub received_at: u64,
    pub document_id: u32,
    pub raw: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendOutcome {
    pub uid: u32,
    pub over_quota: bool,
}

pub fn append_message(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    notifier: &ChangeNotifier,
    request: &AppendRequest<'_>,
) -> Result<AppendOutcome> {
    let account_id = request.account.id as u32;
    let mut metadata = crate::ingest::ingest(blob_placeholder(), request.raw)?;

    let size = request.raw.len() as u64;
    let usage = Quota::new(store).usage(account_id)?;
    if enforce_quota(limits_for(request.account), usage, size).is_over_quota() {
        return Ok(AppendOutcome {
            uid: 0,
            over_quota: true,
        });
    }

    let blob_hash = store_blob(blobs, request.raw)?;
    metadata.blob_hash = blob_hash.clone().into_bytes();

    let uid = request.mailbox.next_uid(store, account_id)?;
    let threading =
        crate::threading::resolve_thread(store, account_id, request.document_id, request.raw)?;
    let mut data = MessageData::new(threading.thread_id, size as u32);
    data.received_at = request.received_at;
    data.sent_at = crate::index::message_sent_at(request.raw);
    data.add_mailbox(request.mailbox.id, uid);
    for keyword in &request.flags {
        data.add_keyword(keyword.clone());
    }

    let mut batch = BatchBuilder::new();
    batch.set(
        metadata_key(account_id, request.document_id),
        serialize::archive(&metadata)?,
    );
    batch.set(
        data_key(account_id, request.document_id),
        serialize::archive(&data)?,
    );
    batch.extend(threading.ops);
    batch.push(reference_op(&blob_hash));
    batch.push(crate::blob_store_msg::account_link_op(
        account_id, &blob_hash, 1,
    ));
    batch.extend(Quota::new(store).adjust_ops(account_id, size as i64, 1));
    let text = message_text(request.raw).unwrap_or_default();
    batch.extend(crate::index::index_ops(
        store,
        account_id,
        request.document_id,
        &text,
    )?);
    let (change_id, change_op) = ChangeLog::new(store).record_op(
        account_id,
        DELIVERY_COLLECTION,
        request.document_id,
        ChangeKind::Insert,
    )?;
    batch.push(change_op);
    store.batch(&batch.build())?;

    notifier.notify_change(account_id, DELIVERY_COLLECTION, change_id);

    Ok(AppendOutcome {
        uid,
        over_quota: false,
    })
}

fn find_inbox(mailboxes: &[Mailbox]) -> Result<&Mailbox> {
    mailboxes
        .iter()
        .find(|m| m.role == SpecialUse::Inbox)
        .ok_or_else(|| Error::not_found("account has no inbox to deliver into"))
}

fn find_role(mailboxes: &[Mailbox], role: SpecialUse) -> Option<&Mailbox> {
    mailboxes.iter().find(|m| m.role == role)
}

fn metadata_key(account_id: u32, document_id: u32) -> Vec<u8> {
    Key::new(
        Subspace::Property,
        account_id,
        DELIVERY_COLLECTION,
        document_id,
    )
    .with_suffix(vec![METADATA_SUFFIX])
    .encode()
}

fn data_key(account_id: u32, document_id: u32) -> Vec<u8> {
    Key::new(
        Subspace::Property,
        account_id,
        DELIVERY_COLLECTION,
        document_id,
    )
    .encode()
}

fn blob_placeholder() -> irixmail_store::BlobHash {
    irixmail_store::BlobHash::from_bytes(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::{Mailbox, SpecialUse};
    use crate::message_data::Keyword;
    use irixmail_directory::{Forwarding, Role, VacationResponder};
    use irixmail_store::{Flow, KeyPrefix, WriteOp};
    use std::collections::BTreeMap;
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
        fn get(&self, hash: &BlobHash, range: std::ops::Range<usize>) -> Result<Option<Vec<u8>>> {
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

    use irixmail_store::BlobHash;

    const INBOX_ID: u32 = 1;
    const NEWSLETTERS_ID: u32 = 2;

    const MESSAGE: &[u8] = concat!(
        "From: newsletter@example.com\r\n",
        "To: me@example.org\r\n",
        "Subject: Weekly deals\r\n",
        "\r\n",
        "Body text with the word invoice in it.\r\n",
    )
    .as_bytes();

    fn account(quota_bytes: u64, quota_messages: u64, forwarding: Forwarding) -> Account {
        Account {
            id: 7,
            local_part: "me".to_string(),
            domain_id: 1,
            display_name: String::new(),
            enabled: true,
            role: Role::User,
            aliases: Vec::new(),
            forwarding,
            quota_bytes,
            quota_messages,
            locale: String::new(),
            timezone: String::new(),
            signature: String::new(),
            vacation: VacationResponder::default(),
            created_at: 0,
        }
    }

    fn mailboxes() -> Vec<Mailbox> {
        vec![
            Mailbox::new(INBOX_ID, "Inbox", SpecialUse::Inbox, 1),
            Mailbox::new(NEWSLETTERS_ID, "Newsletters", SpecialUse::None, 1),
        ]
    }

    fn request<'a>(
        account: &'a Account,
        mailboxes: &'a [Mailbox],
        document_id: u32,
    ) -> DeliveryRequest<'a> {
        DeliveryRequest {
            account,
            mailboxes,
            mail_from: "newsletter@example.com",
            recipient: "me@example.org",
            document_id,
            raw: MESSAGE,
            target_override: None,
            received_at: 1_700_000_000,
        }
    }

    fn read_data(store: &MemStore, account_id: u32, document_id: u32) -> MessageData {
        let bytes = store
            .get(&data_key(account_id, document_id))
            .unwrap()
            .unwrap();
        serialize::deserialize::<MessageData>(&bytes).unwrap()
    }

    #[test]
    fn inbound_delivery_emits_a_new_mail_notice() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let mut mail_feed = notifier.subscribe_new_mail();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();

        let notice = mail_feed.try_recv().unwrap();
        assert_eq!(notice.account_id, 7);
        assert_eq!(notice.document_id, 10);
        assert_eq!(notice.mailbox_id, INBOX_ID);
        assert_eq!(notice.sender, "newsletter@example.com");
        assert_eq!(notice.subject, "Weekly deals");
    }

    #[test]
    fn overridden_deliveries_emit_no_new_mail_notice() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let mut mail_feed = notifier.subscribe_new_mail();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();
        let mut request = request(&account, &mailboxes, 10);
        request.target_override = Some(DeliveryTarget::Role(SpecialUse::Junk));

        deliver(&store, &blobs, &notifier, &request).unwrap();

        assert!(mail_feed.try_recv().is_err());
    }

    #[test]
    fn delivery_links_the_blob_to_the_recipient_account() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(&store, &blobs, &notifier, &request(&account, &mailboxes, 1)).unwrap();

        let hash = MemBlobStore::digest(MESSAGE);
        assert!(crate::blob_store_msg::account_references_blob(&store, 7, &hash).unwrap());
        assert!(!crate::blob_store_msg::account_references_blob(&store, 8, &hash).unwrap());
    }

    #[test]
    fn deleting_the_message_unlinks_the_blob_from_the_account() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(&store, &blobs, &notifier, &request(&account, &mailboxes, 1)).unwrap();
        crate::read::delete_message(&store, &blobs, &notifier, 7, 1).unwrap();

        let hash = MemBlobStore::digest(MESSAGE);
        assert!(!crate::blob_store_msg::account_references_blob(&store, 7, &hash).unwrap());
    }

    #[test]
    fn a_plain_message_with_no_filter_lands_in_the_inbox() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        let outcome = deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .expect("deliver");

        assert_eq!(outcome.filed_into, vec![INBOX_ID]);
        assert!(outcome.was_filed());
        assert!(!outcome.is_over_quota());

        let data = read_data(&store, 7, 10);
        assert_eq!(data.uid_in(INBOX_ID), Some(1));
        assert!(store.get(&metadata_key(7, 10)).unwrap().is_some());
    }

    #[test]
    fn a_deleted_message_leaves_a_vanished_tombstone_per_mailbox() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();
        crate::read::delete_message(&store, &blobs, &notifier, 7, 10).unwrap();

        let vanished = ChangeLog::new(&store).vanished_since(7, 0).unwrap();
        assert_eq!(vanished.len(), 1);
        assert_eq!(vanished[0].mailbox_id, INBOX_ID);
        assert_eq!(vanished[0].uid, 1);
    }

    #[test]
    fn removing_a_message_from_a_mailbox_records_a_tombstone() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();
        crate::read::update_messages(&store, &notifier, 7, &[10], |_, data| {
            data.remove_mailbox(INBOX_ID);
            data.add_mailbox(NEWSLETTERS_ID, 5);
            Ok(())
        })
        .unwrap();

        let vanished = ChangeLog::new(&store).vanished_since(7, 0).unwrap();
        assert_eq!(vanished.len(), 1);
        assert_eq!(vanished[0].mailbox_id, INBOX_ID);
        assert_eq!(vanished[0].uid, 1);
    }

    #[test]
    fn a_flag_only_update_records_no_tombstone() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();
        crate::read::update_messages(&store, &notifier, 7, &[10], |_, data| {
            data.add_keyword(Keyword::Seen);
            Ok(())
        })
        .unwrap();

        assert!(ChangeLog::new(&store)
            .vanished_since(7, 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn delivery_threads_a_reply_with_its_parent() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        let parent: &[u8] = concat!(
            "From: a@example.com\r\n",
            "Subject: hello\r\n",
            "Message-ID: <root@example.com>\r\n",
            "\r\n",
            "first\r\n",
        )
        .as_bytes();
        let reply: &[u8] = concat!(
            "From: b@example.com\r\n",
            "Subject: Re: hello\r\n",
            "Message-ID: <reply@example.com>\r\n",
            "In-Reply-To: <root@example.com>\r\n",
            "\r\n",
            "second\r\n",
        )
        .as_bytes();

        let mut first = request(&account, &mailboxes, 10);
        first.raw = parent;
        deliver(&store, &blobs, &notifier, &first).unwrap();
        let mut second = request(&account, &mailboxes, 11);
        second.raw = reply;
        deliver(&store, &blobs, &notifier, &second).unwrap();

        assert_eq!(read_data(&store, 7, 10).thread_id, 10);
        assert_eq!(read_data(&store, 7, 11).thread_id, 10);
    }

    #[test]
    fn an_appended_reply_joins_the_delivered_parents_thread() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        let parent: &[u8] = concat!(
            "From: a@example.com\r\n",
            "Subject: hello\r\n",
            "Message-ID: <root@example.com>\r\n",
            "\r\n",
            "first\r\n",
        )
        .as_bytes();
        let reply: &[u8] = concat!(
            "From: b@example.com\r\n",
            "Subject: Re: hello\r\n",
            "References: <root@example.com>\r\n",
            "\r\n",
            "second\r\n",
        )
        .as_bytes();

        let mut first = request(&account, &mailboxes, 10);
        first.raw = parent;
        deliver(&store, &blobs, &notifier, &first).unwrap();
        append_message(
            &store,
            &blobs,
            &notifier,
            &AppendRequest {
                account: &account,
                mailbox: &mailboxes[0],
                flags: Vec::new(),
                received_at: 1_700_000_000,
                document_id: 11,
                raw: reply,
            },
        )
        .unwrap();

        assert_eq!(read_data(&store, 7, 11).thread_id, 10);
    }

    #[test]
    fn delivery_stamps_the_received_timestamp_on_the_record() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();
        let mut req = request(&account, &mailboxes, 10);
        req.received_at = 482_374_938;

        deliver(&store, &blobs, &notifier, &req).expect("deliver");

        assert_eq!(read_data(&store, 7, 10).received_at, 482_374_938);
    }

    #[test]
    fn delivery_stamps_the_sent_timestamp_from_the_date_header() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();
        let dated: &[u8] = concat!(
            "From: newsletter@example.com\r\n",
            "Subject: Weekly deals\r\n",
            "Date: Sat, 01 Feb 2020 00:00:00 +0000\r\n",
            "\r\n",
            "invoice body\r\n",
        )
        .as_bytes();
        let mut req = request(&account, &mailboxes, 10);
        req.raw = dated;

        deliver(&store, &blobs, &notifier, &req).expect("deliver");

        assert_eq!(read_data(&store, 7, 10).sent_at, 1_580_515_200);
    }

    #[test]
    fn delivery_without_a_date_header_leaves_the_sent_timestamp_zero() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .expect("deliver");

        assert_eq!(read_data(&store, 7, 10).sent_at, 0);
    }

    #[test]
    fn a_target_override_files_into_the_named_special_use_folder() {
        const SPAM_ID: u32 = 5;
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = vec![
            Mailbox::new(INBOX_ID, "Inbox", SpecialUse::Inbox, 1),
            Mailbox::new(SPAM_ID, "Spam", SpecialUse::Junk, 1),
        ];
        let mut req = request(&account, &mailboxes, 11);
        req.target_override = Some(DeliveryTarget::Role(SpecialUse::Junk));

        let outcome = deliver(&store, &blobs, &notifier, &req).expect("deliver");

        assert_eq!(outcome.filed_into, vec![SPAM_ID]);
        assert!(!outcome.filed_into.contains(&INBOX_ID));
    }

    #[test]
    fn a_mailbox_target_files_into_that_mailbox_by_id() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();
        let mut req = request(&account, &mailboxes, 11);
        req.target_override = Some(DeliveryTarget::Mailbox(NEWSLETTERS_ID));

        let outcome = deliver(&store, &blobs, &notifier, &req).expect("deliver");

        assert_eq!(outcome.filed_into, vec![NEWSLETTERS_ID]);
        assert!(!outcome.filed_into.contains(&INBOX_ID));
    }

    #[test]
    fn the_message_becomes_searchable_after_delivery() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();

        let hits = FtsIndex::new(&store)
            .search(7, DELIVERY_COLLECTION, &Query::term("invoice"), &[10])
            .unwrap();
        assert_eq!(hits, vec![10]);
    }

    #[derive(Default)]
    struct BatchOnlyStore {
        inner: MemStore,
    }

    impl Store for BatchOnlyStore {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            self.inner.get(key)
        }

        fn put(&self, _key: &[u8], _value: &[u8]) -> Result<()> {
            Err(Error::store("standalone put outside the delivery batch"))
        }

        fn delete(&self, _key: &[u8]) -> Result<()> {
            Err(Error::store("standalone delete outside the delivery batch"))
        }

        fn iterate(
            &self,
            prefix: &KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
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

    #[test]
    fn delivery_writes_fts_postings_only_through_the_message_batch() {
        let store = BatchOnlyStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .expect("delivery must not write postings outside the batch");

        let hits = FtsIndex::new(&store)
            .search(7, DELIVERY_COLLECTION, &Query::term("invoice"), &[10])
            .unwrap();
        assert_eq!(hits, vec![10]);
    }

    #[test]
    fn append_writes_fts_postings_only_through_the_message_batch() {
        let store = BatchOnlyStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        append_message(
            &store,
            &blobs,
            &notifier,
            &AppendRequest {
                account: &account,
                mailbox: &mailboxes[0],
                flags: Vec::new(),
                received_at: 1_700_000_000,
                document_id: 10,
                raw: MESSAGE,
            },
        )
        .expect("append must not write postings outside the batch");

        let hits = FtsIndex::new(&store)
            .search(7, DELIVERY_COLLECTION, &Query::term("invoice"), &[10])
            .unwrap();
        assert_eq!(hits, vec![10]);
    }

    #[test]
    fn reindex_account_rebuilds_a_wiped_search_index() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();
        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();

        let prefix = KeyPrefix::collection(Subspace::Index, 7, DELIVERY_COLLECTION);
        let mut keys = Vec::new();
        store
            .iterate(&prefix, &mut |key, _| {
                keys.push(key.to_vec());
                Ok(Flow::Continue)
            })
            .unwrap();
        assert!(!keys.is_empty());
        for key in keys {
            store.delete(&key).unwrap();
        }
        let index = FtsIndex::new(&store);
        assert!(index
            .search(7, DELIVERY_COLLECTION, &Query::term("invoice"), &[10])
            .unwrap()
            .is_empty());

        let reindexed = crate::index::reindex_account(&store, &blobs, 7).unwrap();
        assert_eq!(reindexed, 1);
        let hits = index
            .search(7, DELIVERY_COLLECTION, &Query::term("invoice"), &[10])
            .unwrap();
        assert_eq!(hits, vec![10]);
    }

    #[test]
    fn the_quota_tallies_advance_by_the_message_size_and_one() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();

        let usage = Quota::new(&store).usage(7).unwrap();
        assert_eq!(usage.bytes, MESSAGE.len() as u64);
        assert_eq!(usage.messages, 1);
    }

    #[test]
    fn a_filed_message_raises_the_blob_reference_count_in_the_commit() {
        use crate::blob_store_msg::reference_count;

        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();

        let hash = MemBlobStore::digest(MESSAGE);
        assert_eq!(reference_count(&store, &hash).unwrap(), 1);
    }

    #[test]
    fn the_delivery_records_a_change_a_cache_can_replay() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .unwrap();

        let changes = ChangeLog::new(&store)
            .changes_since(7, DELIVERY_COLLECTION, 0)
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].document_id, 10);
        assert_eq!(changes[0].kind, ChangeKind::Insert);
    }

    #[test]
    fn an_over_quota_message_is_refused_and_stores_nothing() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(10, 0, Forwarding::default());
        let mailboxes = mailboxes();

        let outcome = deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .expect("deliver");

        assert!(outcome.is_over_quota());
        assert!(!outcome.was_filed());
        assert!(matches!(
            outcome.over_quota,
            Some(QuotaVerdict::OverByteQuota { .. })
        ));
        assert!(store.get(&data_key(7, 10)).unwrap().is_none());
        assert_eq!(Quota::new(&store).usage(7).unwrap().bytes, 0);
    }

    #[test]
    fn an_active_forwarding_relays_a_copy_and_still_files_locally() {
        let forwarding = Forwarding {
            destinations: vec!["elsewhere@example.net".to_string()],
            keep_local_copy: true,
        };
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, forwarding);
        let mailboxes = mailboxes();

        let outcome = deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .expect("deliver");

        assert_eq!(outcome.filed_into, vec![INBOX_ID]);
        assert_eq!(outcome.relays.len(), 1);
        assert_eq!(outcome.relays[0].rcpt_to, "elsewhere@example.net");
        assert_eq!(outcome.relays[0].mail_from, "newsletter@example.com");
    }

    #[test]
    fn a_forward_only_account_relays_without_filing_locally() {
        let forwarding = Forwarding {
            destinations: vec!["elsewhere@example.net".to_string()],
            keep_local_copy: false,
        };
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, forwarding);
        let mailboxes = mailboxes();

        let outcome = deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .expect("deliver");

        assert_eq!(outcome.relays.len(), 1);
        assert_eq!(outcome.relays[0].rcpt_to, "elsewhere@example.net");
        assert!(!outcome.was_filed());
        assert!(outcome.filed_into.is_empty());
        assert!(store.get(&data_key(7, 10)).unwrap().is_none());
        assert!(store.get(&metadata_key(7, 10)).unwrap().is_none());
        assert!(ChangeLog::new(&store)
            .changes_since(7, DELIVERY_COLLECTION, 0)
            .unwrap()
            .is_empty());
        assert_eq!(Quota::new(&store).usage(7).unwrap().messages, 0);
    }

    #[test]
    fn a_forward_that_yields_no_relays_keeps_the_local_copy() {
        let forwarding = Forwarding {
            destinations: vec!["me@example.org".to_string()],
            keep_local_copy: false,
        };
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, forwarding);
        let mailboxes = mailboxes();

        let outcome = deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        )
        .expect("deliver");

        assert!(
            outcome.relays.is_empty(),
            "the only destination loops back to the recipient"
        );
        assert_eq!(
            outcome.filed_into,
            vec![INBOX_ID],
            "mail must not vanish when every forward destination is dropped as a loop"
        );
    }

    #[test]
    fn a_message_with_no_inbox_is_refused() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = vec![Mailbox::new(
            NEWSLETTERS_ID,
            "Newsletters",
            SpecialUse::None,
            1,
        )];

        let result = deliver(
            &store,
            &blobs,
            &notifier,
            &request(&account, &mailboxes, 10),
        );
        assert!(matches!(result, Err(Error::NotFound(_))));
    }

    #[test]
    fn an_unparseable_message_is_rejected_before_anything_is_stored() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        let account = account(0, 0, Forwarding::default());
        let mailboxes = mailboxes();

        let mut req = request(&account, &mailboxes, 10);
        req.raw = b"";
        let result = deliver(&store, &blobs, &notifier, &req);
        assert!(matches!(result, Err(Error::InvalidInput(_))));
        assert!(store.get(&data_key(7, 10)).unwrap().is_none());
    }

    #[test]
    fn the_metadata_and_data_records_use_distinct_keys() {
        assert_ne!(metadata_key(7, 10), data_key(7, 10));
        assert!(metadata_key(7, 10).ends_with(&[METADATA_SUFFIX]));
    }

    use irixmail_store::{FtsIndex, Query};
}
