use std::collections::HashMap;

use irixmail_core::{Error, Result};
use irixmail_store::serialize;
use irixmail_store::{ChangeKind, ChangeLog, Collection, Flow, Key, KeyPrefix, Store, Subspace};

use crate::message_data::{Keyword, MailboxUid, MessageData};

const CACHE_COLLECTION: Collection = Collection::Email;

const DOCUMENT_ID_OFFSET: usize = 1 + std::mem::size_of::<u32>() + 1;

const DATA_KEY_LEN: usize = DOCUMENT_ID_OFFSET + std::mem::size_of::<u32>();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageCacheEntry {
    pub document_id: u32,
    pub mailboxes: Vec<MailboxUid>,
    pub keywords: Vec<Keyword>,
    pub thread_id: u32,
    pub size: u32,
    pub received_at: u64,
    pub sent_at: u64,
}

impl MessageCacheEntry {
    fn from_data(document_id: u32, data: &MessageData) -> Self {
        MessageCacheEntry {
            document_id,
            mailboxes: data.mailboxes.clone(),
            keywords: data.keywords.clone(),
            thread_id: data.thread_id,
            size: data.size,
            received_at: data.received_at,
            sent_at: data.sent_at,
        }
    }

    pub fn in_mailbox(&self, mailbox_id: u32) -> bool {
        self.mailboxes.iter().any(|m| m.mailbox_id == mailbox_id)
    }

    pub fn uid_in(&self, mailbox_id: u32) -> Option<u32> {
        self.mailboxes
            .iter()
            .find(|m| m.mailbox_id == mailbox_id)
            .map(|m| m.uid)
    }

    pub fn has_keyword(&self, keyword: &Keyword) -> bool {
        self.keywords.contains(keyword)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageStoreCache {
    account_id: u32,
    entries: HashMap<u32, MessageCacheEntry>,
    last_change_id: u64,
}

impl MessageStoreCache {
    pub fn build(store: &dyn Store, account_id: u32) -> Result<Self> {
        let last_change_id =
            ChangeLog::new(store).latest_change_id(account_id, CACHE_COLLECTION)?;

        let mut entries = HashMap::new();
        let prefix = KeyPrefix::collection(Subspace::Property, account_id, CACHE_COLLECTION);
        let mut scan_error = None;
        store.iterate(&prefix, &mut |key, value| {
            // Only the suffix-free key holds a MessageData record; the longer
            // suffixed sibling under the same document id is a different archive.
            if key.len() != DATA_KEY_LEN {
                return Ok(Flow::Continue);
            }
            let document_id = match document_id_of(key) {
                Ok(document_id) => document_id,
                Err(err) => {
                    scan_error = Some(err);
                    return Ok(Flow::Stop);
                }
            };
            match serialize::deserialize::<MessageData>(value) {
                Ok(data) => {
                    entries.insert(
                        document_id,
                        MessageCacheEntry::from_data(document_id, &data),
                    );
                }
                Err(err) => {
                    scan_error = Some(err);
                    return Ok(Flow::Stop);
                }
            }
            Ok(Flow::Continue)
        })?;
        if let Some(err) = scan_error {
            return Err(err);
        }

        Ok(MessageStoreCache {
            account_id,
            entries,
            last_change_id,
        })
    }

    pub fn refresh(&mut self, store: &dyn Store) -> Result<usize> {
        let log = ChangeLog::new(store);
        if !log.can_calculate(self.account_id, CACHE_COLLECTION, self.last_change_id)? {
            let rebuilt = Self::build(store, self.account_id)?;
            let replayed = rebuilt.len();
            *self = rebuilt;
            return Ok(replayed);
        }
        let changes = log.changes_since(self.account_id, CACHE_COLLECTION, self.last_change_id)?;

        for entry in &changes {
            match entry.kind {
                ChangeKind::Insert | ChangeKind::Update => {
                    match self.load_entry(store, entry.document_id)? {
                        Some(cached) => {
                            self.entries.insert(entry.document_id, cached);
                        }
                        None => {
                            self.entries.remove(&entry.document_id);
                        }
                    }
                }
                ChangeKind::Delete => {
                    self.entries.remove(&entry.document_id);
                }
            }
            self.last_change_id = self.last_change_id.max(entry.change_id);
        }

        Ok(changes.len())
    }

    pub fn account_id(&self) -> u32 {
        self.account_id
    }

    pub fn last_change_id(&self) -> u64 {
        self.last_change_id
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, document_id: u32) -> Option<&MessageCacheEntry> {
        self.entries.get(&document_id)
    }

    pub fn entries(&self) -> impl Iterator<Item = &MessageCacheEntry> {
        self.entries.values()
    }

    pub fn in_mailbox(&self, mailbox_id: u32) -> impl Iterator<Item = &MessageCacheEntry> {
        self.entries
            .values()
            .filter(move |e| e.in_mailbox(mailbox_id))
    }

    fn load_entry(&self, store: &dyn Store, document_id: u32) -> Result<Option<MessageCacheEntry>> {
        let key = data_key(self.account_id, document_id);
        match store.get(&key)? {
            Some(value) => {
                let data = serialize::deserialize::<MessageData>(&value)?;
                Ok(Some(MessageCacheEntry::from_data(document_id, &data)))
            }
            None => Ok(None),
        }
    }
}

fn data_key(account_id: u32, document_id: u32) -> Vec<u8> {
    Key::new(
        Subspace::Property,
        account_id,
        CACHE_COLLECTION,
        document_id,
    )
    .encode()
}

fn document_id_of(key: &[u8]) -> Result<u32> {
    let end = DOCUMENT_ID_OFFSET + std::mem::size_of::<u32>();
    if key.len() < end {
        return Err(Error::store(
            "message property key is too short to carry a document id",
        ));
    }
    let mut bytes = [0u8; std::mem::size_of::<u32>()];
    bytes.copy_from_slice(&key[DOCUMENT_ID_OFFSET..end]);
    Ok(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_store::WriteOp;
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

    fn put_message(
        store: &MemStore,
        account_id: u32,
        document_id: u32,
        data: &MessageData,
        kind: ChangeKind,
    ) {
        let bytes = serialize::archive(data).expect("archive");
        store
            .put(&data_key(account_id, document_id), &bytes)
            .unwrap();
        ChangeLog::new(store)
            .record(account_id, CACHE_COLLECTION, document_id, kind)
            .unwrap();
    }

    fn metadata_key(account_id: u32, document_id: u32) -> Vec<u8> {
        Key::new(
            Subspace::Property,
            account_id,
            CACHE_COLLECTION,
            document_id,
        )
        .with_suffix(vec![b'm'])
        .encode()
    }

    fn message_in(mailbox_id: u32, uid: u32, keywords: &[Keyword]) -> MessageData {
        let mut data = MessageData::new(1, 100);
        data.add_mailbox(mailbox_id, uid);
        for keyword in keywords {
            data.add_keyword(keyword.clone());
        }
        data
    }

    #[test]
    fn document_id_is_read_from_the_property_key() {
        let key = data_key(7, 0x0102_0304);
        assert_eq!(document_id_of(&key).unwrap(), 0x0102_0304);

        assert!(document_id_of(&[b'p', 0, 0]).is_err());
    }

    #[test]
    fn a_full_build_summarises_every_message_in_the_account() {
        let store = MemStore::default();
        put_message(
            &store,
            1,
            10,
            &message_in(1, 5, &[Keyword::Seen]),
            ChangeKind::Insert,
        );
        put_message(&store, 1, 11, &message_in(1, 6, &[]), ChangeKind::Insert);
        put_message(
            &store,
            1,
            12,
            &message_in(2, 1, &[Keyword::Flagged]),
            ChangeKind::Insert,
        );

        let cache = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.last_change_id(), 3);

        let first = cache.get(10).expect("message present");
        assert_eq!(first.document_id, 10);
        assert_eq!(first.uid_in(1), Some(5));
        assert!(first.has_keyword(&Keyword::Seen));
    }

    #[test]
    fn a_cache_entry_carries_the_message_size() {
        let store = MemStore::default();
        put_message(&store, 1, 10, &message_in(1, 5, &[]), ChangeKind::Insert);
        let cache = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(cache.get(10).unwrap().size, 100);
    }

    #[test]
    fn a_cache_entry_carries_the_received_timestamp() {
        let store = MemStore::default();
        let mut data = message_in(1, 5, &[]);
        data.received_at = 482_374_938;
        put_message(&store, 1, 10, &data, ChangeKind::Insert);
        let cache = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(cache.get(10).unwrap().received_at, 482_374_938);
    }

    #[test]
    fn a_cache_entry_carries_the_sent_timestamp() {
        let store = MemStore::default();
        let mut data = message_in(1, 5, &[]);
        data.sent_at = 1_580_515_200;
        put_message(&store, 1, 10, &data, ChangeKind::Insert);
        let cache = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(cache.get(10).unwrap().sent_at, 1_580_515_200);
    }

    #[test]
    fn build_isolates_one_account_from_another() {
        let store = MemStore::default();
        put_message(&store, 1, 1, &message_in(1, 1, &[]), ChangeKind::Insert);
        put_message(&store, 2, 1, &message_in(1, 1, &[]), ChangeKind::Insert);
        put_message(&store, 2, 2, &message_in(1, 2, &[]), ChangeKind::Insert);

        let cache = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(cache.len(), 1);
        assert!(cache.get(1).is_some());
    }

    #[test]
    fn an_empty_account_builds_an_empty_cache_at_the_zero_cursor() {
        let store = MemStore::default();
        let cache = MessageStoreCache::build(&store, 1).unwrap();
        assert!(cache.is_empty());
        assert_eq!(cache.last_change_id(), 0);
    }

    #[test]
    fn refresh_picks_up_an_inserted_message() {
        let store = MemStore::default();
        let mut cache = MessageStoreCache::build(&store, 1).unwrap();
        assert!(cache.is_empty());

        put_message(
            &store,
            1,
            10,
            &message_in(1, 5, &[Keyword::Seen]),
            ChangeKind::Insert,
        );
        let replayed = cache.refresh(&store).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.last_change_id(), 1);
        assert_eq!(cache.get(10).unwrap().uid_in(1), Some(5));
    }

    #[test]
    fn refresh_reflects_an_updated_message() {
        let store = MemStore::default();
        put_message(&store, 1, 10, &message_in(1, 5, &[]), ChangeKind::Insert);
        let mut cache = MessageStoreCache::build(&store, 1).unwrap();
        assert!(!cache.get(10).unwrap().has_keyword(&Keyword::Seen));

        put_message(
            &store,
            1,
            10,
            &message_in(1, 5, &[Keyword::Seen]),
            ChangeKind::Update,
        );
        let replayed = cache.refresh(&store).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(cache.len(), 1);
        assert!(cache.get(10).unwrap().has_keyword(&Keyword::Seen));
    }

    #[test]
    fn refresh_drops_a_deleted_message() {
        let store = MemStore::default();
        put_message(&store, 1, 10, &message_in(1, 5, &[]), ChangeKind::Insert);
        let mut cache = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(cache.len(), 1);

        store.delete(&data_key(1, 10)).unwrap();
        ChangeLog::new(&store)
            .record(1, CACHE_COLLECTION, 10, ChangeKind::Delete)
            .unwrap();

        let replayed = cache.refresh(&store).unwrap();
        assert_eq!(replayed, 1);
        assert!(cache.is_empty());
        assert_eq!(cache.last_change_id(), 2);
    }

    #[test]
    fn refresh_rebuilds_when_the_log_was_pruned_past_the_cursor() {
        let store = MemStore::default();
        put_message(&store, 1, 10, &message_in(1, 5, &[]), ChangeKind::Insert);
        let mut cache = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(cache.len(), 1);

        store.delete(&data_key(1, 10)).unwrap();
        let log = ChangeLog::new(&store);
        log.record(1, CACHE_COLLECTION, 10, ChangeKind::Delete)
            .unwrap();
        for document in 20..30u32 {
            put_message(
                &store,
                1,
                document,
                &message_in(1, document, &[]),
                ChangeKind::Insert,
            );
        }
        log.prune(1, CACHE_COLLECTION, 2).unwrap();
        assert!(!log
            .can_calculate(1, CACHE_COLLECTION, cache.last_change_id())
            .unwrap());

        cache.refresh(&store).unwrap();
        assert!(
            cache.get(10).is_none(),
            "a ghost entry survived the pruned gap"
        );
        assert_eq!(cache.len(), 10);
    }

    #[test]
    fn an_update_whose_record_has_vanished_is_treated_as_a_delete() {
        let store = MemStore::default();
        put_message(&store, 1, 10, &message_in(1, 5, &[]), ChangeKind::Insert);
        let mut cache = MessageStoreCache::build(&store, 1).unwrap();

        ChangeLog::new(&store)
            .record(1, CACHE_COLLECTION, 10, ChangeKind::Update)
            .unwrap();
        store.delete(&data_key(1, 10)).unwrap();

        cache.refresh(&store).unwrap();
        assert!(cache.is_empty());
    }

    #[test]
    fn a_refresh_with_no_new_changes_is_a_no_op() {
        let store = MemStore::default();
        put_message(&store, 1, 10, &message_in(1, 5, &[]), ChangeKind::Insert);
        let mut cache = MessageStoreCache::build(&store, 1).unwrap();
        let cursor = cache.last_change_id();

        let replayed = cache.refresh(&store).unwrap();
        assert_eq!(replayed, 0);
        assert_eq!(cache.last_change_id(), cursor);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn an_incremental_refresh_matches_a_full_rebuild() {
        let store = MemStore::default();
        let mut incremental = MessageStoreCache::build(&store, 1).unwrap();

        put_message(&store, 1, 10, &message_in(1, 5, &[]), ChangeKind::Insert);
        put_message(
            &store,
            1,
            11,
            &message_in(1, 6, &[Keyword::Seen]),
            ChangeKind::Insert,
        );
        incremental.refresh(&store).unwrap();

        put_message(
            &store,
            1,
            10,
            &message_in(2, 9, &[Keyword::Flagged]),
            ChangeKind::Update,
        );
        store.delete(&data_key(1, 11)).unwrap();
        ChangeLog::new(&store)
            .record(1, CACHE_COLLECTION, 11, ChangeKind::Delete)
            .unwrap();
        incremental.refresh(&store).unwrap();

        let rebuilt = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(incremental, rebuilt);
        assert_eq!(incremental.len(), 1);
        assert_eq!(incremental.get(10).unwrap().uid_in(2), Some(9));
    }

    #[test]
    fn in_mailbox_returns_only_the_folder_members() {
        let store = MemStore::default();
        put_message(&store, 1, 10, &message_in(1, 1, &[]), ChangeKind::Insert);
        put_message(&store, 1, 11, &message_in(1, 2, &[]), ChangeKind::Insert);
        put_message(&store, 1, 12, &message_in(2, 1, &[]), ChangeKind::Insert);

        let cache = MessageStoreCache::build(&store, 1).unwrap();
        let mut in_inbox: Vec<u32> = cache.in_mailbox(1).map(|e| e.document_id).collect();
        in_inbox.sort_unstable();
        assert_eq!(in_inbox, vec![10, 11]);

        let in_sent: Vec<u32> = cache.in_mailbox(2).map(|e| e.document_id).collect();
        assert_eq!(in_sent, vec![12]);

        assert_eq!(cache.in_mailbox(99).count(), 0);
    }

    #[test]
    fn build_reads_the_data_record_and_skips_its_metadata_sibling() {
        use crate::metadata::MessageMetadata;
        use irixmail_store::BlobHash;

        let store = MemStore::default();
        put_message(
            &store,
            1,
            10,
            &message_in(3, 5, &[Keyword::Seen, Keyword::Flagged]),
            ChangeKind::Insert,
        );
        let metadata = MessageMetadata::new(BlobHash::from_bytes(vec![1, 2, 3, 4]), Vec::new());
        store
            .put(
                &metadata_key(1, 10),
                &serialize::archive(&metadata).expect("archive"),
            )
            .unwrap();

        let cache = MessageStoreCache::build(&store, 1).unwrap();
        assert_eq!(cache.len(), 1);
        let entry = cache.get(10).expect("data entry present");
        assert_eq!(entry.uid_in(3), Some(5));
        assert!(entry.has_keyword(&Keyword::Seen));
        assert!(entry.has_keyword(&Keyword::Flagged));
        assert_ne!(entry.thread_id, u32::MAX);
    }

    #[test]
    fn a_corrupt_record_stops_the_build() {
        let store = MemStore::default();
        put_message(&store, 1, 10, &message_in(1, 1, &[]), ChangeKind::Insert);
        store.put(&data_key(1, 11), b"not an archive").unwrap();

        let err = MessageStoreCache::build(&store, 1).unwrap_err();
        assert!(matches!(err, Error::Serialize(_)));
    }
}
