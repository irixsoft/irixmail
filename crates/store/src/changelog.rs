use irixmail_core::{Error, Result};

use crate::key::{Collection, Key, KeyPrefix, Subspace};
use crate::traits_store::{Flow, Store, WriteOp};

const CHANGE_ID_LEN: usize = std::mem::size_of::<u64>();

const CHANGELOG_DOCUMENT_ID: u32 = 0;

pub const FIRST_CHANGE_ID: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChangeKind {
    Insert = 0,
    Update = 1,
    Delete = 2,
}

impl ChangeKind {
    fn as_byte(self) -> u8 {
        self as u8
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(ChangeKind::Insert),
            1 => Some(ChangeKind::Update),
            2 => Some(ChangeKind::Delete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeLogEntry {
    pub change_id: u64,
    pub document_id: u32,
    pub kind: ChangeKind,
}

impl ChangeLogEntry {
    fn encode_value(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(std::mem::size_of::<u32>() + 1);
        buf.extend_from_slice(&self.document_id.to_be_bytes());
        buf.push(self.kind.as_byte());
        buf
    }

    fn decode(change_id: u64, value: &[u8]) -> Result<Self> {
        let expected = std::mem::size_of::<u32>() + 1;
        if value.len() != expected {
            return Err(Error::store(format!(
                "change-log entry has {} bytes, expected {expected}",
                value.len()
            )));
        }
        let mut document_bytes = [0u8; std::mem::size_of::<u32>()];
        document_bytes.copy_from_slice(&value[..std::mem::size_of::<u32>()]);
        let document_id = u32::from_be_bytes(document_bytes);
        let kind = ChangeKind::from_byte(value[std::mem::size_of::<u32>()])
            .ok_or_else(|| Error::store("change-log entry has an unknown change kind"))?;
        Ok(Self {
            change_id,
            document_id,
            kind,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VanishedEntry {
    pub change_id: u64,
    pub mailbox_id: u32,
    pub uid: u32,
}

pub struct ChangeLog<'a> {
    store: &'a dyn Store,
}

impl<'a> ChangeLog<'a> {
    pub fn new(store: &'a dyn Store) -> Self {
        Self { store }
    }

    pub fn record(
        &self,
        account_id: u32,
        collection: Collection,
        document_id: u32,
        kind: ChangeKind,
    ) -> Result<u64> {
        let change_id = self.allocate_change_id(account_id, collection)?;
        let entry = ChangeLogEntry {
            change_id,
            document_id,
            kind,
        };
        let key = entry_key(account_id, collection, change_id);
        self.store.put(&key, &entry.encode_value())?;
        Ok(change_id)
    }

    pub fn record_op(
        &self,
        account_id: u32,
        collection: Collection,
        document_id: u32,
        kind: ChangeKind,
    ) -> Result<(u64, WriteOp)> {
        let change_id = self.allocate_change_id(account_id, collection)?;
        let entry = ChangeLogEntry {
            change_id,
            document_id,
            kind,
        };
        let op = WriteOp::Set {
            key: entry_key(account_id, collection, change_id),
            value: entry.encode_value(),
        };
        Ok((change_id, op))
    }

    pub fn record_vanished_op(
        &self,
        account_id: u32,
        mailbox_id: u32,
        uid: u32,
    ) -> Result<(u64, WriteOp)> {
        // Vanished tombstones share the Email change-id sequence so one modseq covers both.
        let change_id = self.allocate_change_id(account_id, Collection::Email)?;
        let mut value = Vec::with_capacity(8);
        value.extend_from_slice(&mailbox_id.to_be_bytes());
        value.extend_from_slice(&uid.to_be_bytes());
        let op = WriteOp::Set {
            key: entry_key(account_id, Collection::EmailVanished, change_id),
            value,
        };
        Ok((change_id, op))
    }

    pub fn vanished_since(&self, account_id: u32, since: u64) -> Result<Vec<VanishedEntry>> {
        let prefix =
            KeyPrefix::collection(Subspace::ChangeLog, account_id, Collection::EmailVanished);
        let Some(first) = since.checked_add(1) else {
            return Ok(Vec::new());
        };
        let start = entry_key(account_id, Collection::EmailVanished, first);
        let mut entries = Vec::new();
        let mut scan_error = None;
        self.store
            .iterate_from(&prefix, &start, &mut |key, value| {
                if key.len() != ENTRY_KEY_LEN {
                    return Ok(Flow::Continue);
                }
                let change_id = match change_id_of(key) {
                    Ok(change_id) => change_id,
                    Err(err) => {
                        scan_error = Some(err);
                        return Ok(Flow::Stop);
                    }
                };
                if change_id <= since {
                    return Ok(Flow::Continue);
                }
                if value.len() != 8 {
                    scan_error = Some(Error::store("vanished entry value is malformed"));
                    return Ok(Flow::Stop);
                }
                let mut mailbox = [0u8; 4];
                mailbox.copy_from_slice(&value[..4]);
                let mut uid = [0u8; 4];
                uid.copy_from_slice(&value[4..]);
                entries.push(VanishedEntry {
                    change_id,
                    mailbox_id: u32::from_be_bytes(mailbox),
                    uid: u32::from_be_bytes(uid),
                });
                Ok(Flow::Continue)
            })?;
        if let Some(err) = scan_error {
            return Err(err);
        }
        Ok(entries)
    }

    pub fn changes_since(
        &self,
        account_id: u32,
        collection: Collection,
        since: u64,
    ) -> Result<Vec<ChangeLogEntry>> {
        Ok(self
            .changes_page(account_id, collection, since, usize::MAX)?
            .0)
    }

    pub fn changes_page(
        &self,
        account_id: u32,
        collection: Collection,
        since: u64,
        max: usize,
    ) -> Result<(Vec<ChangeLogEntry>, bool)> {
        let prefix = KeyPrefix::collection(Subspace::ChangeLog, account_id, collection);
        let Some(first) = since.checked_add(1) else {
            return Ok((Vec::new(), false));
        };
        if max == 0 {
            return Ok((Vec::new(), true));
        }
        let start = entry_key(account_id, collection, first);
        let mut entries = Vec::new();
        let mut has_more = false;
        let mut scan_error = None;
        self.store
            .iterate_from(&prefix, &start, &mut |key, value| {
                if key.len() != ENTRY_KEY_LEN {
                    return Ok(Flow::Continue);
                }
                let change_id = match change_id_of(key) {
                    Ok(change_id) => change_id,
                    Err(err) => {
                        scan_error = Some(err);
                        return Ok(Flow::Stop);
                    }
                };
                if change_id <= since {
                    return Ok(Flow::Continue);
                }
                if entries.len() == max {
                    has_more = true;
                    return Ok(Flow::Stop);
                }
                match ChangeLogEntry::decode(change_id, value) {
                    Ok(entry) => entries.push(entry),
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
        Ok((entries, has_more))
    }

    pub fn latest_change_id(&self, account_id: u32, collection: Collection) -> Result<u64> {
        let counter = self.store.counter(&counter_key(account_id, collection))?;
        Ok(counter.max(0) as u64)
    }

    pub fn can_calculate(
        &self,
        account_id: u32,
        collection: Collection,
        since: u64,
    ) -> Result<bool> {
        Ok(since >= self.floor(account_id, collection)?)
    }

    pub fn prune(&self, account_id: u32, collection: Collection, keep_last: u64) -> Result<usize> {
        let keep_last = keep_last.max(1);
        // The vanished partition draws its ids from the Email counter.
        let counter_source = match collection {
            Collection::EmailVanished => Collection::Email,
            other => other,
        };
        let latest = self.latest_change_id(account_id, counter_source)?;
        let cutoff = latest.saturating_sub(keep_last);
        if cutoff == 0 {
            return Ok(0);
        }
        let prefix = KeyPrefix::collection(Subspace::ChangeLog, account_id, collection);
        let mut ops = Vec::new();
        let mut scan_error = None;
        self.store.iterate(&prefix, &mut |key, _value| {
            if key.len() != ENTRY_KEY_LEN {
                return Ok(Flow::Continue);
            }
            match change_id_of(key) {
                Ok(change_id) if change_id <= cutoff => {
                    ops.push(WriteOp::Delete { key: key.to_vec() });
                    Ok(Flow::Continue)
                }
                Ok(_) => Ok(Flow::Stop),
                Err(err) => {
                    scan_error = Some(err);
                    Ok(Flow::Stop)
                }
            }
        })?;
        if let Some(err) = scan_error {
            return Err(err);
        }
        if ops.is_empty() {
            return Ok(0);
        }
        let pruned = ops.len();
        let floor = self.floor(account_id, collection)?.max(cutoff);
        ops.push(WriteOp::Set {
            key: floor_key(account_id, collection),
            value: floor.to_be_bytes().to_vec(),
        });
        self.store.batch(&ops)?;
        Ok(pruned)
    }

    fn floor(&self, account_id: u32, collection: Collection) -> Result<u64> {
        Ok(self
            .store
            .get(&floor_key(account_id, collection))?
            .and_then(|bytes| bytes.as_slice().try_into().ok().map(u64::from_be_bytes))
            .unwrap_or(0))
    }

    fn allocate_change_id(&self, account_id: u32, collection: Collection) -> Result<u64> {
        let next = self
            .store
            .add_and_get(&counter_key(account_id, collection), 1)?;
        Ok(next.max(FIRST_CHANGE_ID as i64) as u64)
    }
}

pub fn prune_change_logs(store: &dyn Store, keep_last: u64) -> Result<usize> {
    let mut partitions = Vec::new();
    let mut current: Option<(u32, Collection)> = None;
    store.iterate(
        &KeyPrefix::subspace(Subspace::ChangeLog),
        &mut |key, _value| {
            if let Some(partition) = partition_of(key) {
                if current != Some(partition) {
                    current = Some(partition);
                    partitions.push(partition);
                }
            }
            Ok(Flow::Continue)
        },
    )?;
    let log = ChangeLog::new(store);
    let mut pruned = 0;
    for (account_id, collection) in partitions {
        pruned += log.prune(account_id, collection, keep_last)?;
    }
    Ok(pruned)
}

fn partition_of(key: &[u8]) -> Option<(u32, Collection)> {
    if key.len() != ENTRY_KEY_LEN {
        return None;
    }
    let mut account = [0u8; ACCOUNT_LEN];
    account.copy_from_slice(&key[1..1 + ACCOUNT_LEN]);
    Collection::from_byte(key[1 + ACCOUNT_LEN])
        .map(|collection| (u32::from_be_bytes(account), collection))
}

const ACCOUNT_LEN: usize = std::mem::size_of::<u32>();
const ENTRY_KEY_LEN: usize = 1 + ACCOUNT_LEN + 1 + std::mem::size_of::<u32>() + CHANGE_ID_LEN;

fn counter_key(account_id: u32, collection: Collection) -> Vec<u8> {
    Key::new(
        Subspace::Counter,
        account_id,
        collection,
        CHANGELOG_DOCUMENT_ID,
    )
    .encode()
}

fn floor_key(account_id: u32, collection: Collection) -> Vec<u8> {
    Key::new(
        Subspace::Counter,
        account_id,
        collection,
        CHANGELOG_DOCUMENT_ID,
    )
    .with_suffix(b"changelog-floor".to_vec())
    .encode()
}

fn entry_key(account_id: u32, collection: Collection, change_id: u64) -> Vec<u8> {
    Key::new(
        Subspace::ChangeLog,
        account_id,
        collection,
        CHANGELOG_DOCUMENT_ID,
    )
    .with_suffix(change_id.to_be_bytes().to_vec())
    .encode()
}

fn change_id_of(key: &[u8]) -> Result<u64> {
    if key.len() < CHANGE_ID_LEN {
        return Err(Error::store(
            "change-log key is too short to carry a change id",
        ));
    }
    let mut bytes = [0u8; CHANGE_ID_LEN];
    bytes.copy_from_slice(&key[key.len() - CHANGE_ID_LEN..]);
    Ok(u64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn change_kind_bytes_round_trip() {
        for kind in [ChangeKind::Insert, ChangeKind::Update, ChangeKind::Delete] {
            assert_eq!(ChangeKind::from_byte(kind.as_byte()), Some(kind));
        }
        assert_eq!(ChangeKind::from_byte(9), None);
    }

    #[test]
    fn change_ids_are_monotonic_per_partition() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        let first = log
            .record(1, Collection::Email, 10, ChangeKind::Insert)
            .unwrap();
        let second = log
            .record(1, Collection::Email, 11, ChangeKind::Insert)
            .unwrap();
        let third = log
            .record(1, Collection::Email, 10, ChangeKind::Update)
            .unwrap();

        assert_eq!(first, FIRST_CHANGE_ID);
        assert_eq!(second, 2);
        assert_eq!(third, 3);
    }

    #[test]
    fn partitions_have_independent_id_sequences() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        let email_one = log
            .record(1, Collection::Email, 1, ChangeKind::Insert)
            .unwrap();
        let mailbox_one = log
            .record(1, Collection::Mailbox, 1, ChangeKind::Insert)
            .unwrap();
        let other_account = log
            .record(2, Collection::Email, 1, ChangeKind::Insert)
            .unwrap();

        assert_eq!(email_one, FIRST_CHANGE_ID);
        assert_eq!(mailbox_one, FIRST_CHANGE_ID);
        assert_eq!(other_account, FIRST_CHANGE_ID);
    }

    #[test]
    fn changes_since_replays_in_order_after_the_cursor() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        log.record(1, Collection::Email, 10, ChangeKind::Insert)
            .unwrap();
        log.record(1, Collection::Email, 11, ChangeKind::Insert)
            .unwrap();
        log.record(1, Collection::Email, 10, ChangeKind::Update)
            .unwrap();
        log.record(1, Collection::Email, 11, ChangeKind::Delete)
            .unwrap();

        let all = log.changes_since(1, Collection::Email, 0).unwrap();
        assert_eq!(
            all,
            vec![
                ChangeLogEntry {
                    change_id: 1,
                    document_id: 10,
                    kind: ChangeKind::Insert
                },
                ChangeLogEntry {
                    change_id: 2,
                    document_id: 11,
                    kind: ChangeKind::Insert
                },
                ChangeLogEntry {
                    change_id: 3,
                    document_id: 10,
                    kind: ChangeKind::Update
                },
                ChangeLogEntry {
                    change_id: 4,
                    document_id: 11,
                    kind: ChangeKind::Delete
                },
            ]
        );

        let tail = log.changes_since(1, Collection::Email, 2).unwrap();
        assert_eq!(
            tail,
            vec![
                ChangeLogEntry {
                    change_id: 3,
                    document_id: 10,
                    kind: ChangeKind::Update
                },
                ChangeLogEntry {
                    change_id: 4,
                    document_id: 11,
                    kind: ChangeKind::Delete
                },
            ]
        );

        assert!(log
            .changes_since(1, Collection::Email, 4)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn changes_since_isolates_partitions() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        log.record(1, Collection::Email, 5, ChangeKind::Insert)
            .unwrap();
        log.record(1, Collection::Mailbox, 6, ChangeKind::Insert)
            .unwrap();
        log.record(2, Collection::Email, 7, ChangeKind::Insert)
            .unwrap();

        let email = log.changes_since(1, Collection::Email, 0).unwrap();
        assert_eq!(email.len(), 1);
        assert_eq!(email[0].document_id, 5);

        let mailbox = log.changes_since(1, Collection::Mailbox, 0).unwrap();
        assert_eq!(mailbox.len(), 1);
        assert_eq!(mailbox[0].document_id, 6);
    }

    #[test]
    fn latest_change_id_tracks_the_cursor() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        assert_eq!(log.latest_change_id(1, Collection::Email).unwrap(), 0);

        log.record(1, Collection::Email, 1, ChangeKind::Insert)
            .unwrap();
        log.record(1, Collection::Email, 2, ChangeKind::Insert)
            .unwrap();
        assert_eq!(log.latest_change_id(1, Collection::Email).unwrap(), 2);
    }

    #[test]
    fn record_op_defers_the_write_but_allocates_the_id_now() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        let (change_id, op) = log
            .record_op(1, Collection::Email, 42, ChangeKind::Insert)
            .unwrap();
        assert_eq!(change_id, FIRST_CHANGE_ID);

        let next = log
            .record(1, Collection::Email, 43, ChangeKind::Insert)
            .unwrap();
        assert_eq!(next, 2);

        assert_eq!(log.changes_since(1, Collection::Email, 0).unwrap().len(), 1);

        store.batch(&[op]).unwrap();
        let changes = log.changes_since(1, Collection::Email, 0).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(changes
            .iter()
            .any(|entry| entry.change_id == 1 && entry.document_id == 42));
    }

    struct CountingStore {
        inner: MemStore,
        visits: Mutex<usize>,
    }

    impl CountingStore {
        fn new() -> Self {
            Self {
                inner: MemStore::default(),
                visits: Mutex::new(0),
            }
        }

        fn visits(&self) -> usize {
            *self.visits.lock().unwrap()
        }
    }

    impl Store for CountingStore {
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
            self.inner.iterate(prefix, &mut |key, value| {
                *self.visits.lock().unwrap() += 1;
                visit(key, value)
            })
        }

        fn iterate_from(
            &self,
            prefix: &KeyPrefix,
            start: &[u8],
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            self.inner.iterate(prefix, &mut |key, value| {
                if key < start {
                    return Ok(Flow::Continue);
                }
                *self.visits.lock().unwrap() += 1;
                visit(key, value)
            })
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
    fn changes_since_seeks_to_the_cursor_instead_of_scanning_the_partition() {
        let store = CountingStore::new();
        let log = ChangeLog::new(&store);
        for document in 0..100u32 {
            log.record(1, Collection::Email, document, ChangeKind::Insert)
                .unwrap();
        }

        let tail = log.changes_since(1, Collection::Email, 99).unwrap();
        assert_eq!(tail.len(), 1);
        assert!(
            store.visits() <= 1,
            "reading one change visited {} store entries",
            store.visits()
        );
    }

    #[test]
    fn pruning_bounds_the_log_and_keeps_recent_changes_replayable() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);
        for document in 0..10u32 {
            log.record(1, Collection::Email, document, ChangeKind::Insert)
                .unwrap();
        }

        let pruned = log.prune(1, Collection::Email, 3).unwrap();
        assert_eq!(pruned, 7);

        let recent = log.changes_since(1, Collection::Email, 7).unwrap();
        assert_eq!(recent.len(), 3);
        assert!(log.can_calculate(1, Collection::Email, 7).unwrap());
        assert!(!log.can_calculate(1, Collection::Email, 0).unwrap());
        assert!(!log.can_calculate(1, Collection::Email, 6).unwrap());
    }

    #[test]
    fn an_unpruned_log_replays_from_the_beginning() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);
        log.record(1, Collection::Email, 1, ChangeKind::Insert)
            .unwrap();
        assert!(log.can_calculate(1, Collection::Email, 0).unwrap());
    }

    #[test]
    fn prune_change_logs_sweeps_every_partition() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);
        for document in 0..10u32 {
            log.record(1, Collection::Email, document, ChangeKind::Insert)
                .unwrap();
            log.record(2, Collection::Mailbox, document, ChangeKind::Insert)
                .unwrap();
        }

        let pruned = prune_change_logs(&store, 4).unwrap();
        assert_eq!(pruned, 12);
        assert_eq!(log.changes_since(1, Collection::Email, 6).unwrap().len(), 4);
        assert_eq!(
            log.changes_since(2, Collection::Mailbox, 6).unwrap().len(),
            4
        );
        assert!(!log.can_calculate(1, Collection::Email, 0).unwrap());
        assert!(!log.can_calculate(2, Collection::Mailbox, 5).unwrap());
        assert!(log.can_calculate(2, Collection::Mailbox, 6).unwrap());
    }

    #[test]
    fn a_vanished_uid_shares_the_email_change_sequence() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        log.record(1, Collection::Email, 10, ChangeKind::Insert)
            .unwrap();
        let (change_id, op) = log.record_vanished_op(1, 3, 42).unwrap();
        store.batch(&[op]).unwrap();

        assert_eq!(change_id, 2);
        let next = log
            .record(1, Collection::Email, 11, ChangeKind::Insert)
            .unwrap();
        assert_eq!(next, 3);
    }

    #[test]
    fn vanished_since_replays_tombstones_after_the_cursor() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        let (first, op) = log.record_vanished_op(1, 3, 42).unwrap();
        store.batch(&[op]).unwrap();
        let (_, op) = log.record_vanished_op(1, 3, 43).unwrap();
        store.batch(&[op]).unwrap();

        let all = log.vanished_since(1, 0).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].mailbox_id, 3);
        assert_eq!(all[0].uid, 42);
        assert_eq!(all[1].uid, 43);

        let tail = log.vanished_since(1, first).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].uid, 43);
    }

    #[test]
    fn vanished_tombstones_are_isolated_per_account() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        let (_, op) = log.record_vanished_op(1, 3, 42).unwrap();
        store.batch(&[op]).unwrap();

        assert!(log.vanished_since(2, 0).unwrap().is_empty());
    }

    #[test]
    fn pruning_sweeps_old_vanished_tombstones() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        for uid in 0..10u32 {
            let (_, op) = log.record_vanished_op(1, 3, uid).unwrap();
            store.batch(&[op]).unwrap();
        }

        let pruned = prune_change_logs(&store, 3).unwrap();
        assert!(pruned >= 7, "pruned {pruned} vanished entries");
        let remaining = log.vanished_since(1, 0).unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn corrupt_entry_value_is_reported() {
        let store = MemStore::default();
        let log = ChangeLog::new(&store);

        let key = entry_key(1, Collection::Email, 1);
        store.put(&key, b"too short").unwrap();
        let err = log.changes_since(1, Collection::Email, 0).unwrap_err();
        assert!(matches!(err, Error::Store(_)));
    }

    #[test]
    fn entry_value_round_trips_through_decode() {
        let entry = ChangeLogEntry {
            change_id: 99,
            document_id: 0x0A0B_0C0D,
            kind: ChangeKind::Update,
        };
        let value = entry.encode_value();
        assert_eq!(ChangeLogEntry::decode(99, &value).unwrap(), entry);
    }

    #[test]
    fn change_id_of_reads_the_trailing_bytes() {
        let key = entry_key(7, Collection::Mailbox, 0x0102_0304_0506_0708);
        assert_eq!(change_id_of(&key).unwrap(), 0x0102_0304_0506_0708);

        assert!(change_id_of(&[0, 1, 2]).is_err());
    }
}
