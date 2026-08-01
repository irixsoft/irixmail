use irixmail_core::Result;
use irixmail_store::{
    BatchBuilder, BlobStore, ChangeKind, ChangeLog, ChangeNotifier, Collection, Key, Store,
    Subspace,
};

use crate::cache::MessageStoreCache;
use crate::mailbox::{Mailbox, SpecialUse};
use crate::provision::{load_mailboxes, mailbox_key, mailbox_ops, SPAM_ID};
use crate::read::{delete_message, update_message};

const COLLECTION: Collection = Collection::Mailbox;
// Document id 0 in this counter subspace belongs to the Mailbox ChangeLog.
const MAILBOX_ID_COUNTER: u32 = 1;

fn allocate_mailbox_id(store: &dyn Store, account_id: u32) -> Result<u32> {
    let key = Key::new(
        Subspace::Counter,
        account_id,
        COLLECTION,
        MAILBOX_ID_COUNTER,
    )
    .encode();
    let allocated = store.add_and_get(&key, 1)? as u32;
    let floor = load_mailboxes(store, account_id)?
        .iter()
        .map(|mailbox| mailbox.id)
        .max()
        .unwrap_or(0)
        .max(SPAM_ID);
    if allocated > floor {
        Ok(allocated)
    } else {
        Ok(store.add_and_get(&key, i64::from(floor + 1 - allocated))? as u32)
    }
}

pub fn create_mailbox(
    store: &dyn Store,
    notifier: &ChangeNotifier,
    account_id: u32,
    name: &str,
    uid_validity: u32,
) -> Result<Mailbox> {
    let next_id = allocate_mailbox_id(store, account_id)?;
    let role = if name.eq_ignore_ascii_case("Archive") {
        SpecialUse::Archive
    } else {
        SpecialUse::None
    };
    let mailbox = Mailbox::new(next_id, name, role, uid_validity);

    let mut batch = BatchBuilder::new();
    batch.extend(mailbox_ops(account_id, std::slice::from_ref(&mailbox)));
    let (change_id, change_op) =
        ChangeLog::new(store).record_op(account_id, COLLECTION, next_id, ChangeKind::Insert)?;
    batch.push(change_op);
    store.batch(&batch.build())?;

    notifier.notify_change(account_id, COLLECTION, change_id);
    Ok(mailbox)
}

pub fn rename_mailbox(
    store: &dyn Store,
    notifier: &ChangeNotifier,
    account_id: u32,
    mailbox_id: u32,
    new_name: &str,
) -> Result<bool> {
    let Some(mut mailbox) = load_mailboxes(store, account_id)?
        .into_iter()
        .find(|mailbox| mailbox.id == mailbox_id)
    else {
        return Ok(false);
    };
    mailbox.name = new_name.to_string();

    let mut batch = BatchBuilder::new();
    batch.extend(mailbox_ops(account_id, std::slice::from_ref(&mailbox)));
    let (change_id, change_op) =
        ChangeLog::new(store).record_op(account_id, COLLECTION, mailbox_id, ChangeKind::Update)?;
    batch.push(change_op);
    store.batch(&batch.build())?;

    notifier.notify_change(account_id, COLLECTION, change_id);
    Ok(true)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxDelete {
    Deleted,
    HasMail,
}

pub fn delete_mailbox(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    notifier: &ChangeNotifier,
    account_id: u32,
    mailbox_id: u32,
    remove_emails: bool,
) -> Result<MailboxDelete> {
    let cache = MessageStoreCache::build(store, account_id)?;
    let members: Vec<(u32, usize)> = cache
        .in_mailbox(mailbox_id)
        .map(|entry| (entry.document_id, entry.mailboxes.len()))
        .collect();
    if !remove_emails && !members.is_empty() {
        return Ok(MailboxDelete::HasMail);
    }
    for (document_id, mailbox_count) in members {
        if mailbox_count <= 1 {
            delete_message(store, blobs, notifier, account_id, document_id)?;
        } else {
            update_message(store, notifier, account_id, document_id, |data| {
                data.remove_mailbox(mailbox_id);
                Ok(())
            })?;
        }
    }

    let mut batch = BatchBuilder::new();
    batch.push(irixmail_store::WriteOp::Delete {
        key: mailbox_key(account_id, mailbox_id),
    });
    let (change_id, change_op) =
        ChangeLog::new(store).record_op(account_id, COLLECTION, mailbox_id, ChangeKind::Delete)?;
    batch.push(change_op);
    store.batch(&batch.build())?;

    notifier.notify_change(account_id, COLLECTION, change_id);
    Ok(MailboxDelete::Deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use irixmail_store::{FsBlobStore, RocksdbStore};

    use crate::provision::{provision_ops, FIRST_USER_MAILBOX_ID};

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "irixmail-mailbox-admin-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_deleted_mailbox_id_is_never_reused() {
        let dir = temp_dir();
        let store = RocksdbStore::open(dir.join("db")).unwrap();
        let blobs = FsBlobStore::open(dir.join("blobs")).unwrap();
        let notifier = ChangeNotifier::new();
        store.batch(&provision_ops(7, 1_700_000_000_000)).unwrap();

        let first = create_mailbox(&store, &notifier, 7, "Projects", 100).unwrap();
        assert!(first.id >= FIRST_USER_MAILBOX_ID);

        delete_mailbox(&store, &blobs, &notifier, 7, first.id, false).unwrap();

        let second = create_mailbox(&store, &notifier, 7, "Archive", 100).unwrap();
        assert!(
            second.id > first.id,
            "mailbox id {} was reused after delete",
            second.id
        );
    }
}
