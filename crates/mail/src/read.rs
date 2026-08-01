use irixmail_core::{Error, Result};
use irixmail_store::{
    serialize, BatchBuilder, BlobStore, ChangeKind, ChangeLog, ChangeNotifier, Collection, Key,
    Quota, Store, Subspace, ValueAssert, WriteOp,
};

use crate::message_data::MessageData;
use crate::metadata::MessageMetadata;

const COLLECTION: Collection = Collection::Email;
const METADATA_SUFFIX: u8 = b'm';

pub fn allocate_document_id(store: &dyn Store, account_id: u32) -> Result<u32> {
    let key = Key::new(Subspace::Counter, account_id, COLLECTION, 0).encode();
    Ok(store.add_and_get(&key, 1)? as u32)
}

pub fn update_message<F>(
    store: &dyn Store,
    notifier: &ChangeNotifier,
    account_id: u32,
    document_id: u32,
    mutate: F,
) -> Result<bool>
where
    F: FnOnce(&mut MessageData) -> Result<()>,
{
    let Some(mut data) = load_data(store, account_id, document_id)? else {
        return Ok(false);
    };
    let before = data.mailboxes.clone();
    mutate(&mut data)?;

    let mut batch = BatchBuilder::new();
    batch.set(
        data_key(account_id, document_id),
        serialize::archive(&data)?,
    );
    push_vanished_ops(store, &mut batch, account_id, &before, &data.mailboxes)?;
    let (change_id, change_op) =
        ChangeLog::new(store).record_op(account_id, COLLECTION, document_id, ChangeKind::Update)?;
    batch.push(change_op);
    store.batch(&batch.build())?;

    notifier.notify_change(account_id, COLLECTION, change_id);
    Ok(true)
}

fn push_vanished_ops(
    store: &dyn Store,
    batch: &mut BatchBuilder,
    account_id: u32,
    before: &[crate::message_data::MailboxUid],
    after: &[crate::message_data::MailboxUid],
) -> Result<()> {
    for entry in before {
        if !after.contains(entry) {
            let (_, op) = ChangeLog::new(store).record_vanished_op(
                account_id,
                entry.mailbox_id,
                entry.uid,
            )?;
            batch.push(op);
        }
    }
    Ok(())
}

const UPDATE_RACE_ATTEMPTS: usize = 4;

pub struct UpdatedMessage {
    pub document_id: u32,
    pub change_id: Option<u64>,
    pub data: MessageData,
}

pub fn update_messages<F>(
    store: &dyn Store,
    notifier: &ChangeNotifier,
    account_id: u32,
    document_ids: &[u32],
    mut mutate: F,
) -> Result<Vec<UpdatedMessage>>
where
    F: FnMut(u32, &mut MessageData) -> Result<()>,
{
    for _ in 0..UPDATE_RACE_ATTEMPTS {
        let mut results: Vec<UpdatedMessage> = Vec::new();
        let mut asserts: Vec<ValueAssert> = Vec::new();
        let mut batch = BatchBuilder::new();
        let mut change_ids: Vec<u64> = Vec::new();
        for &document_id in document_ids {
            let key = data_key(account_id, document_id);
            let Some(original) = store.get(&key)? else {
                continue;
            };
            let mut data = serialize::deserialize::<MessageData>(&original)?;
            let before = data.mailboxes.clone();
            mutate(document_id, &mut data)?;
            let updated = serialize::archive(&data)?;
            let mut change = None;
            if updated != original {
                asserts.push(ValueAssert {
                    key: key.clone(),
                    expected: Some(original),
                });
                batch.set(key, updated);
                push_vanished_ops(store, &mut batch, account_id, &before, &data.mailboxes)?;
                let (change_id, change_op) = ChangeLog::new(store).record_op(
                    account_id,
                    COLLECTION,
                    document_id,
                    ChangeKind::Update,
                )?;
                batch.push(change_op);
                change_ids.push(change_id);
                change = Some(change_id);
            }
            results.push(UpdatedMessage {
                document_id,
                change_id: change,
                data,
            });
        }
        if batch.is_empty() {
            return Ok(results);
        }
        if store.batch_conditional(&asserts, &batch.build())? {
            for change_id in change_ids {
                notifier.notify_change(account_id, COLLECTION, change_id);
            }
            return Ok(results);
        }
    }
    Err(Error::store(
        "message update kept losing to concurrent writers",
    ))
}

pub fn delete_message(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    notifier: &ChangeNotifier,
    account_id: u32,
    document_id: u32,
) -> Result<bool> {
    let Some(data) = load_data(store, account_id, document_id)? else {
        return Ok(false);
    };
    let metadata = load_metadata(store, account_id, document_id)?;

    let mut batch = BatchBuilder::new();
    if let Some(metadata) = &metadata {
        if let Some(raw) = blobs.get_all(&metadata.blob_hash())? {
            if let Ok(text) = crate::index::message_text(&raw) {
                batch.extend(crate::index::unindex_ops(
                    store,
                    account_id,
                    document_id,
                    &text,
                )?);
            }
        }
        batch.push(crate::blob_store_msg::account_link_op(
            account_id,
            &metadata.blob_hash(),
            -1,
        ));
    }
    batch.push(WriteOp::Delete {
        key: data_key(account_id, document_id),
    });
    batch.push(WriteOp::Delete {
        key: metadata_key(account_id, document_id),
    });
    batch.extend(Quota::new(store).adjust_ops(account_id, -(data.size as i64), -1));
    push_vanished_ops(store, &mut batch, account_id, &data.mailboxes, &[])?;
    let (change_id, change_op) =
        ChangeLog::new(store).record_op(account_id, COLLECTION, document_id, ChangeKind::Delete)?;
    batch.push(change_op);
    store.batch(&batch.build())?;

    // After the records are gone, drop the blob reference so a crash can only ever
    // orphan a blob, never leave a record pointing at a missing one.
    if let Some(metadata) = &metadata {
        crate::blob_store_msg::release_message(store, &metadata.blob_hash())?;
    }

    notifier.notify_change(account_id, COLLECTION, change_id);
    Ok(true)
}

pub fn load_data(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
) -> Result<Option<MessageData>> {
    match store.get(&data_key(account_id, document_id))? {
        Some(bytes) => Ok(Some(serialize::deserialize::<MessageData>(&bytes)?)),
        None => Ok(None),
    }
}

pub fn load_metadata(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
) -> Result<Option<MessageMetadata>> {
    match store.get(&metadata_key(account_id, document_id))? {
        Some(bytes) => Ok(Some(serialize::deserialize::<MessageMetadata>(&bytes)?)),
        None => Ok(None),
    }
}

pub fn load_raw(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    account_id: u32,
    document_id: u32,
) -> Result<Option<Vec<u8>>> {
    let Some(metadata) = load_metadata(store, account_id, document_id)? else {
        return Ok(None);
    };
    blobs.get_all(&metadata.blob_hash())
}

fn data_key(account_id: u32, document_id: u32) -> Vec<u8> {
    Key::new(Subspace::Property, account_id, COLLECTION, document_id).encode()
}

fn metadata_key(account_id: u32, document_id: u32) -> Vec<u8> {
    Key::new(Subspace::Property, account_id, COLLECTION, document_id)
        .with_suffix(vec![METADATA_SUFFIX])
        .encode()
}
