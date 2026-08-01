use irixmail_core::Result;
use irixmail_store::{ChangeNotifier, Collection, Flow, Key, KeyPrefix, Store, Subspace};

use crate::read::{load_data, load_metadata, update_message};
use crate::threading::resolve_thread;

const BACKFILL_MARKER_TAG: u8 = 0x33;

pub fn backfill_threads(
    store: &dyn Store,
    notifier: &ChangeNotifier,
    account_ids: &[u32],
) -> Result<usize> {
    let marker = marker_key();
    if store.get(&marker)?.is_some() {
        return Ok(0);
    }
    let mut updated = 0;
    for &account_id in account_ids {
        for document_id in document_ids(store, account_id)? {
            let Some(metadata) = load_metadata(store, account_id, document_id)? else {
                continue;
            };
            let resolution = resolve_thread(store, account_id, document_id, &metadata.raw_headers)?;
            store.batch(&resolution.ops)?;
            let Some(data) = load_data(store, account_id, document_id)? else {
                continue;
            };
            if data.thread_id != resolution.thread_id {
                update_message(store, notifier, account_id, document_id, |data| {
                    data.thread_id = resolution.thread_id;
                    Ok(())
                })?;
                updated += 1;
            }
        }
    }
    store.put(&marker, &[1])?;
    Ok(updated)
}

fn document_ids(store: &dyn Store, account_id: u32) -> Result<Vec<u32>> {
    let prefix = KeyPrefix::collection(Subspace::Property, account_id, Collection::Email);
    let bare_len = Key::new(Subspace::Property, account_id, Collection::Email, 0)
        .encode()
        .len();
    let mut ids = Vec::new();
    store.iterate(&prefix, &mut |key, _value| {
        if key.len() == bare_len {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&key[bare_len - 4..]);
            ids.push(u32::from_be_bytes(bytes));
        }
        Ok(Flow::Continue)
    })?;
    Ok(ids)
}

fn marker_key() -> Vec<u8> {
    vec![Subspace::Registry.as_byte(), BACKFILL_MARKER_TAG]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_data::MessageData;
    use irixmail_store::{serialize, Collection, Flow, Key, KeyPrefix, Subspace, WriteOp};
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
                    WriteOp::Add { key, by } => {
                        let current = map
                            .get(key)
                            .map(|bytes| {
                                let mut array = [0u8; 8];
                                array.copy_from_slice(bytes);
                                i64::from_le_bytes(array)
                            })
                            .unwrap_or(0);
                        map.insert(key.clone(), (current + by).to_le_bytes().to_vec());
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
            let mut map = self.map.lock().unwrap();
            let current = map
                .get(key)
                .map(|bytes| {
                    let mut array = [0u8; 8];
                    array.copy_from_slice(bytes);
                    i64::from_le_bytes(array)
                })
                .unwrap_or(0);
            map.insert(key.to_vec(), (current + by).to_le_bytes().to_vec());
            Ok(current + by)
        }

        fn counter(&self, key: &[u8]) -> Result<i64> {
            Ok(self
                .map
                .lock()
                .unwrap()
                .get(key)
                .map(|bytes| {
                    let mut array = [0u8; 8];
                    array.copy_from_slice(bytes);
                    i64::from_le_bytes(array)
                })
                .unwrap_or(0))
        }
    }

    fn data_key(account_id: u32, document_id: u32) -> Vec<u8> {
        Key::new(
            Subspace::Property,
            account_id,
            Collection::Email,
            document_id,
        )
        .encode()
    }

    fn metadata_key(account_id: u32, document_id: u32) -> Vec<u8> {
        Key::new(
            Subspace::Property,
            account_id,
            Collection::Email,
            document_id,
        )
        .with_suffix(vec![b'm'])
        .encode()
    }

    fn seed(store: &MemStore, document_id: u32, thread_id: u32, raw: &[u8]) {
        let metadata =
            crate::ingest::ingest(irixmail_store::BlobHash::from_bytes(Vec::new()), raw).unwrap();
        let mut data = MessageData::new(thread_id, raw.len() as u32);
        data.add_mailbox(1, document_id);
        store
            .put(
                &metadata_key(7, document_id),
                &serialize::archive(&metadata).unwrap(),
            )
            .unwrap();
        store
            .put(
                &data_key(7, document_id),
                &serialize::archive(&data).unwrap(),
            )
            .unwrap();
    }

    fn thread_of(store: &MemStore, document_id: u32) -> u32 {
        let bytes = store.get(&data_key(7, document_id)).unwrap().unwrap();
        serialize::deserialize::<MessageData>(&bytes)
            .unwrap()
            .thread_id
    }

    const PARENT: &[u8] =
        b"From: a@example.com\r\nSubject: hi\r\nMessage-ID: <root@example.com>\r\n\r\nbody\r\n";
    const REPLY: &[u8] = b"From: b@example.com\r\nSubject: Re: hi\r\nMessage-ID: <re@example.com>\r\nIn-Reply-To: <root@example.com>\r\n\r\nbody\r\n";

    #[test]
    fn backfill_rethreads_existing_messages() {
        let store = MemStore::default();
        let notifier = ChangeNotifier::new();
        seed(&store, 1, 1, PARENT);
        seed(&store, 2, 2, REPLY);

        let updated = backfill_threads(&store, &notifier, &[7]).unwrap();

        assert_eq!(updated, 1);
        assert_eq!(thread_of(&store, 1), 1);
        assert_eq!(thread_of(&store, 2), 1);
    }

    #[test]
    fn backfill_runs_only_once() {
        let store = MemStore::default();
        let notifier = ChangeNotifier::new();
        seed(&store, 1, 1, PARENT);
        seed(&store, 2, 2, REPLY);

        backfill_threads(&store, &notifier, &[7]).unwrap();
        seed(&store, 3, 3, REPLY);
        let second = backfill_threads(&store, &notifier, &[7]).unwrap();

        assert_eq!(second, 0);
        assert_eq!(thread_of(&store, 3), 3);
    }

    #[test]
    fn a_rethreaded_message_records_a_replayable_change() {
        let store = MemStore::default();
        let notifier = ChangeNotifier::new();
        seed(&store, 1, 1, PARENT);
        seed(&store, 2, 2, REPLY);

        backfill_threads(&store, &notifier, &[7]).unwrap();

        let changes = irixmail_store::ChangeLog::new(&store)
            .changes_since(7, Collection::Email, 0)
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].document_id, 2);
    }
}
