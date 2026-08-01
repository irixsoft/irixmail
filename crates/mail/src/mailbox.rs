use irixmail_core::Result;
use irixmail_store::{Collection, Key, Store, Subspace};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpecialUse {
    None,
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
}

impl SpecialUse {
    pub fn attribute(self) -> Option<&'static str> {
        match self {
            SpecialUse::None | SpecialUse::Inbox => None,
            SpecialUse::Sent => Some("\\Sent"),
            SpecialUse::Drafts => Some("\\Drafts"),
            SpecialUse::Trash => Some("\\Trash"),
            SpecialUse::Junk => Some("\\Junk"),
            SpecialUse::Archive => Some("\\Archive"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub id: u32,
    pub name: String,
    pub role: SpecialUse,
    pub uid_validity: u32,
}

impl Mailbox {
    pub fn new(id: u32, name: impl Into<String>, role: SpecialUse, uid_validity: u32) -> Self {
        Mailbox {
            id,
            name: name.into(),
            role,
            uid_validity,
        }
    }

    pub fn next_uid(&self, store: &dyn Store, account_id: u32) -> Result<u32> {
        let next = store.add_and_get(&uid_counter_key(account_id, self.id), 1)?;
        Ok(next.max(FIRST_UID as i64) as u32)
    }

    pub fn last_uid(&self, store: &dyn Store, account_id: u32) -> Result<u32> {
        let counter = store.counter(&uid_counter_key(account_id, self.id))?;
        Ok(counter.max(0) as u32)
    }
}

const FIRST_UID: u32 = 1;

pub fn assign_uid_validity(created_at_millis: u64) -> u32 {
    (created_at_millis / 1_000) as u32
}

fn uid_counter_key(account_id: u32, mailbox_id: u32) -> Vec<u8> {
    Key::new(Subspace::Counter, account_id, Collection::Email, mailbox_id).encode()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn a_new_mailbox_carries_its_name_role_and_uid_validity() {
        let mailbox = Mailbox::new(3, "Archive", SpecialUse::Archive, 1_700);
        assert_eq!(mailbox.id, 3);
        assert_eq!(mailbox.name, "Archive");
        assert_eq!(mailbox.role, SpecialUse::Archive);
        assert_eq!(mailbox.uid_validity, 1_700);
    }

    #[test]
    fn uids_start_at_one_and_increase_strictly() {
        let store = MemStore::default();
        let mailbox = Mailbox::new(1, "Inbox", SpecialUse::Inbox, 42);

        assert_eq!(mailbox.next_uid(&store, 7).unwrap(), 1);
        assert_eq!(mailbox.next_uid(&store, 7).unwrap(), 2);
        assert_eq!(mailbox.next_uid(&store, 7).unwrap(), 3);
        assert_eq!(mailbox.last_uid(&store, 7).unwrap(), 3);
    }

    #[test]
    fn an_untouched_mailbox_reports_a_zero_high_water_mark() {
        let store = MemStore::default();
        let mailbox = Mailbox::new(1, "Drafts", SpecialUse::Drafts, 9);
        assert_eq!(mailbox.last_uid(&store, 1).unwrap(), 0);
    }

    #[test]
    fn each_mailbox_has_an_independent_uid_space() {
        let store = MemStore::default();
        let inbox = Mailbox::new(1, "Inbox", SpecialUse::Inbox, 1);
        let sent = Mailbox::new(2, "Sent", SpecialUse::Sent, 1);

        assert_eq!(inbox.next_uid(&store, 5).unwrap(), 1);
        assert_eq!(inbox.next_uid(&store, 5).unwrap(), 2);
        assert_eq!(sent.next_uid(&store, 5).unwrap(), 1);
        assert_eq!(inbox.next_uid(&store, 5).unwrap(), 3);
    }

    #[test]
    fn the_same_mailbox_id_in_different_accounts_counts_separately() {
        let store = MemStore::default();
        let mailbox = Mailbox::new(1, "Inbox", SpecialUse::Inbox, 1);

        assert_eq!(mailbox.next_uid(&store, 100).unwrap(), 1);
        assert_eq!(mailbox.next_uid(&store, 100).unwrap(), 2);
        assert_eq!(mailbox.next_uid(&store, 200).unwrap(), 1);
    }

    #[test]
    fn uid_validity_tracks_creation_time_and_changes_on_recreation() {
        let first = assign_uid_validity(1_700_000_000_000);
        let later = assign_uid_validity(1_700_000_001_000);
        assert_ne!(first, later);
        assert!(later > first);

        let same_second = assign_uid_validity(1_700_000_000_999);
        assert_eq!(first, same_second);
    }

    #[test]
    fn special_use_attributes_match_their_roles() {
        assert_eq!(SpecialUse::None.attribute(), None);
        assert_eq!(SpecialUse::Inbox.attribute(), None);
        assert_eq!(SpecialUse::Sent.attribute(), Some("\\Sent"));
        assert_eq!(SpecialUse::Drafts.attribute(), Some("\\Drafts"));
        assert_eq!(SpecialUse::Trash.attribute(), Some("\\Trash"));
        assert_eq!(SpecialUse::Junk.attribute(), Some("\\Junk"));
        assert_eq!(SpecialUse::Archive.attribute(), Some("\\Archive"));
    }

    #[test]
    fn a_mailbox_record_round_trips_through_json() {
        let mailbox = Mailbox::new(8, "Project X", SpecialUse::None, 123_456);
        let bytes = serde_json::to_vec(&mailbox).expect("encode");
        let restored: Mailbox = serde_json::from_slice(&bytes).expect("decode");
        assert_eq!(mailbox, restored);
    }
}
