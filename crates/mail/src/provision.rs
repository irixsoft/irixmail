use irixmail_core::{Error, Result};
use irixmail_store::{Collection, Flow, Key, KeyPrefix, Store, Subspace, WriteOp};

use crate::mailbox::{assign_uid_validity, Mailbox, SpecialUse};

pub const INBOX_ID: u32 = 1;

pub const SENT_ID: u32 = 2;

pub const DRAFTS_ID: u32 = 3;

pub const TRASH_ID: u32 = 4;

pub const SPAM_ID: u32 = 5;

pub const FIRST_USER_MAILBOX_ID: u32 = SPAM_ID + 1;

pub const SYSTEM_MAILBOX_COUNT: usize = 5;

pub fn provision_mailboxes(created_at_millis: u64) -> Vec<Mailbox> {
    let uid_validity = assign_uid_validity(created_at_millis);
    [
        (INBOX_ID, "Inbox", SpecialUse::Inbox),
        (SENT_ID, "Sent", SpecialUse::Sent),
        (DRAFTS_ID, "Drafts", SpecialUse::Drafts),
        (TRASH_ID, "Trash", SpecialUse::Trash),
        (SPAM_ID, "Spam", SpecialUse::Junk),
    ]
    .into_iter()
    .map(|(id, name, role)| Mailbox::new(id, name, role, uid_validity))
    .collect()
}

pub fn mailbox_key(account_id: u32, mailbox_id: u32) -> Vec<u8> {
    Key::new(
        Subspace::Property,
        account_id,
        Collection::Mailbox,
        mailbox_id,
    )
    .encode()
}

pub fn mailbox_ops(account_id: u32, mailboxes: &[Mailbox]) -> Vec<WriteOp> {
    mailboxes
        .iter()
        .map(|mailbox| WriteOp::Set {
            key: mailbox_key(account_id, mailbox.id),
            value: serde_json::to_vec(mailbox).expect("a mailbox row always serializes"),
        })
        .collect()
}

pub fn provision_ops(account_id: u32, created_at_millis: u64) -> Vec<WriteOp> {
    mailbox_ops(account_id, &provision_mailboxes(created_at_millis))
}

pub fn load_mailboxes(store: &dyn Store, account_id: u32) -> Result<Vec<Mailbox>> {
    let prefix = KeyPrefix::collection(Subspace::Property, account_id, Collection::Mailbox);
    let mut mailboxes = Vec::new();
    let mut scan_error = None;
    store.iterate(
        &prefix,
        &mut |_key, value| match serde_json::from_slice::<Mailbox>(value) {
            Ok(mailbox) => {
                mailboxes.push(mailbox);
                Ok(Flow::Continue)
            }
            Err(err) => {
                scan_error = Some(Error::serialize(format!(
                    "could not decode mailbox row: {err}"
                )));
                Ok(Flow::Stop)
            }
        },
    )?;
    if let Some(err) = scan_error {
        return Err(err);
    }
    mailboxes.sort_by_key(|mailbox| mailbox.id);
    Ok(mailboxes)
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
                    WriteOp::Add { .. } => unreachable!("provisioning does not use counters"),
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("provisioning does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("provisioning does not use counters")
        }
    }

    #[test]
    fn provisioned_mailbox_rows_persist_and_load_from_the_store() {
        let store = MemStore::default();
        let created_at = 1_700_000_000_000u64;

        store.batch(&provision_ops(7, created_at)).unwrap();
        let loaded = load_mailboxes(&store, 7).unwrap();

        assert_eq!(loaded, provision_mailboxes(created_at));
    }

    #[test]
    fn provisioning_yields_the_five_system_folders() {
        let mailboxes = provision_mailboxes(1_700_000_000_000);
        assert_eq!(mailboxes.len(), SYSTEM_MAILBOX_COUNT);

        let names: Vec<&str> = mailboxes.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["Inbox", "Sent", "Drafts", "Trash", "Spam"]);
    }

    #[test]
    fn each_folder_carries_its_reserved_id_and_role() {
        let mailboxes = provision_mailboxes(1_700_000_000_000);
        let expected = [
            (INBOX_ID, "Inbox", SpecialUse::Inbox),
            (SENT_ID, "Sent", SpecialUse::Sent),
            (DRAFTS_ID, "Drafts", SpecialUse::Drafts),
            (TRASH_ID, "Trash", SpecialUse::Trash),
            (SPAM_ID, "Spam", SpecialUse::Junk),
        ];
        for (mailbox, (id, name, role)) in mailboxes.iter().zip(expected) {
            assert_eq!(mailbox.id, id);
            assert_eq!(mailbox.name, name);
            assert_eq!(mailbox.role, role);
        }
    }

    #[test]
    fn the_spam_folder_plays_the_junk_role() {
        let mailboxes = provision_mailboxes(1_700_000_000_000);
        let spam = mailboxes
            .iter()
            .find(|m| m.name == "Spam")
            .expect("a spam folder is provisioned");
        assert_eq!(spam.role, SpecialUse::Junk);
        assert_eq!(spam.id, SPAM_ID);
    }

    #[test]
    fn there_is_exactly_one_inbox() {
        let mailboxes = provision_mailboxes(1_700_000_000_000);
        let inboxes = mailboxes
            .iter()
            .filter(|m| m.role == SpecialUse::Inbox)
            .count();
        assert_eq!(inboxes, 1);
    }

    #[test]
    fn the_reserved_ids_are_distinct_and_below_the_user_floor() {
        let ids = [INBOX_ID, SENT_ID, DRAFTS_ID, TRASH_ID, SPAM_ID];
        for (index, &id) in ids.iter().enumerate() {
            assert!(id < FIRST_USER_MAILBOX_ID);
            for &other in &ids[index + 1..] {
                assert_ne!(id, other, "reserved mailbox ids must be unique");
            }
        }
    }

    #[test]
    fn all_folders_share_the_uid_validity_of_their_creation_moment() {
        let created_at = 1_700_000_000_000;
        let mailboxes = provision_mailboxes(created_at);
        let expected = assign_uid_validity(created_at);
        for mailbox in &mailboxes {
            assert_eq!(mailbox.uid_validity, expected);
        }
    }

    #[test]
    fn folders_created_later_carry_a_later_uid_validity() {
        let earlier = provision_mailboxes(1_700_000_000_000);
        let later = provision_mailboxes(1_700_000_001_000);
        assert!(later[0].uid_validity > earlier[0].uid_validity);
    }

    #[test]
    fn the_folders_are_returned_in_ascending_id_order() {
        let mailboxes = provision_mailboxes(1_700_000_000_000);
        for window in mailboxes.windows(2) {
            assert!(window[0].id < window[1].id);
        }
    }
}
