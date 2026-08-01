use irixmail_core::Result;
use irixmail_store::{BlobStore, ChangeNotifier, Store, Subspace};

use crate::ingest::message_has_attachment;
use crate::message_data::Keyword;
use crate::read::{load_data, load_raw, update_message};
use crate::thread_backfill::document_ids;

const BACKFILL_MARKER_TAG: u8 = 0x36;

pub fn backfill_attachment_keywords(
    store: &dyn Store,
    blobs: &dyn BlobStore,
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
            let Some(data) = load_data(store, account_id, document_id)? else {
                continue;
            };
            if data.keywords.contains(&Keyword::has_attachment()) {
                continue;
            }
            let Some(raw) = load_raw(store, blobs, account_id, document_id)? else {
                continue;
            };
            if !message_has_attachment(&raw) {
                continue;
            }
            update_message(store, notifier, account_id, document_id, |data| {
                data.add_keyword(Keyword::has_attachment());
                Ok(())
            })?;
            updated += 1;
        }
    }
    store.put(&marker, &[1])?;
    Ok(updated)
}

fn marker_key() -> Vec<u8> {
    vec![Subspace::Registry.as_byte(), BACKFILL_MARKER_TAG]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_data::MessageData;
    use irixmail_store::{serialize, BlobHash, Collection, Flow, Key, KeyPrefix, WriteOp};
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

    #[derive(Default)]
    struct MemBlobStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
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
            let hash = BlobHash::from_bytes(vec![bytes.len() as u8]);
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

    fn key_for(account_id: u32, document_id: u32) -> Key {
        Key::new(
            Subspace::Property,
            account_id,
            Collection::Email,
            document_id,
        )
    }

    fn seed(store: &MemStore, blobs: &MemBlobStore, document_id: u32, raw: &[u8]) {
        let hash = blobs.put(raw).unwrap();
        let mut metadata = crate::ingest::ingest(hash, raw).unwrap();
        metadata.blob_hash = metadata.blob_hash().into_bytes();
        let mut data = MessageData::new(document_id, raw.len() as u32);
        data.add_mailbox(1, document_id);
        store
            .put(
                &key_for(7, document_id).with_suffix(vec![b'm']).encode(),
                &serialize::archive(&metadata).unwrap(),
            )
            .unwrap();
        store
            .put(
                &key_for(7, document_id).encode(),
                &serialize::archive(&data).unwrap(),
            )
            .unwrap();
    }

    fn keywords_of(store: &MemStore, document_id: u32) -> Vec<Keyword> {
        let bytes = store
            .get(&key_for(7, document_id).encode())
            .unwrap()
            .unwrap();
        serialize::deserialize::<MessageData>(&bytes)
            .unwrap()
            .keywords
    }

    const PLAIN: &[u8] = b"From: a@example.com\r\nSubject: hi\r\n\r\nbody\r\n";
    const ATTACHED: &[u8] = concat!(
        "From: a@example.com\r\n",
        "Subject: files\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"B\"\r\n",
        "\r\n",
        "--B\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "see attached\r\n",
        "--B\r\n",
        "Content-Type: application/pdf\r\n",
        "Content-Disposition: attachment; filename=\"x.pdf\"\r\n",
        "\r\n",
        "%PDF-1.4\r\n",
        "--B--\r\n",
    )
    .as_bytes();

    #[test]
    fn backfill_marks_stored_messages_with_attachments() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        seed(&store, &blobs, 1, PLAIN);
        seed(&store, &blobs, 2, ATTACHED);

        let updated = backfill_attachment_keywords(&store, &blobs, &notifier, &[7]).unwrap();

        assert_eq!(updated, 1);
        assert!(!keywords_of(&store, 1).contains(&Keyword::has_attachment()));
        assert!(keywords_of(&store, 2).contains(&Keyword::has_attachment()));
    }

    #[test]
    fn the_marker_makes_a_second_run_a_no_op() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let notifier = ChangeNotifier::new();
        seed(&store, &blobs, 1, ATTACHED);

        assert_eq!(
            backfill_attachment_keywords(&store, &blobs, &notifier, &[7]).unwrap(),
            1
        );

        seed(&store, &blobs, 2, ATTACHED);
        assert_eq!(
            backfill_attachment_keywords(&store, &blobs, &notifier, &[7]).unwrap(),
            0
        );
        assert!(!keywords_of(&store, 2).contains(&Keyword::has_attachment()));
    }
}
