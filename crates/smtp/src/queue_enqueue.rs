use irixmail_core::Result;
use irixmail_store::{BlobStore, Collection, Key, Store, Subspace};

use crate::queue_model::{Expiry, QueueRecipient, QueuedMessage};

const QUEUE_ACCOUNT: u32 = 0;

const QUEUE_ID_DOCUMENT: u32 = 0;

pub struct Enqueue<'a> {
    pub created: u64,
    pub return_path: &'a str,
    pub recipients: &'a [(String, Expiry)],
    pub first_due: u64,
}

pub struct Enqueued {
    pub id: u32,
    pub message: QueuedMessage,
}

pub fn enqueue(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    raw: &[u8],
    request: &Enqueue<'_>,
) -> Result<Enqueued> {
    // Body to the blob store first so the queue record never points at a missing blob.
    let hash = blobs.put(raw)?;
    let id = allocate_id(store)?;

    let recipients = request
        .recipients
        .iter()
        .map(|(address, expiry)| QueueRecipient::new(address.clone(), request.first_due, *expiry))
        .collect();

    let message = QueuedMessage::new(
        request.created,
        &hash,
        raw.len() as u64,
        request.return_path,
        recipients,
    );

    let bytes = irixmail_store::serialize::archive(&message)?;
    store.put(&message_key(id), &bytes)?;

    Ok(Enqueued { id, message })
}

pub fn load(store: &dyn Store, id: u32) -> Result<Option<QueuedMessage>> {
    match store.get(&message_key(id))? {
        Some(bytes) => Ok(Some(irixmail_store::serialize::deserialize(&bytes)?)),
        None => Ok(None),
    }
}

pub fn persist(store: &dyn Store, id: u32, message: &QueuedMessage) -> Result<()> {
    let bytes = irixmail_store::serialize::archive(message)?;
    store.put(&message_key(id), &bytes)
}

pub fn remove(store: &dyn Store, id: u32) -> Result<()> {
    store.delete(&message_key(id))
}

pub fn retry_now(store: &dyn Store, id: u32, now: u64) -> Result<bool> {
    let Some(mut message) = load(store, id)? else {
        return Ok(false);
    };
    for recipient in &mut message.recipients {
        if recipient.status.is_pending() {
            recipient.retry.due = now;
        }
    }
    persist(store, id, &message)?;
    Ok(true)
}

fn allocate_id(store: &dyn Store) -> Result<u32> {
    let next = store.add_and_get(&id_counter_key(), 1)?;
    Ok(next as u32)
}

fn message_key(id: u32) -> Vec<u8> {
    Key::new(
        Subspace::Queue,
        QUEUE_ACCOUNT,
        Collection::EmailSubmission,
        id,
    )
    .encode()
}

fn id_counter_key() -> Vec<u8> {
    Key::new(
        Subspace::Counter,
        QUEUE_ACCOUNT,
        Collection::EmailSubmission,
        QUEUE_ID_DOCUMENT,
    )
    .encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_core::Result;
    use irixmail_store::{BlobHash, Flow, KeyPrefix, WriteOp};
    use std::collections::BTreeMap;
    use std::ops::Range;
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

    const BODY: &[u8] = b"From: s@example.com\r\nTo: a@one.example\r\n\r\nHello.\r\n";

    fn request<'a>(recipients: &'a [(String, Expiry)]) -> Enqueue<'a> {
        Enqueue {
            created: 1_000,
            return_path: "s@example.com",
            recipients,
            first_due: 1_000,
        }
    }

    #[test]
    fn enqueue_writes_the_body_to_the_blob_store_under_its_hash() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec![("a@one.example".to_string(), Expiry::Attempts(5))];

        let accepted = enqueue(&store, &blobs, BODY, &request(&recipients)).expect("enqueue");

        let stored = blobs
            .get_all(&accepted.message.blob_hash())
            .expect("blob get")
            .expect("blob present");
        assert_eq!(stored, BODY);
    }

    #[test]
    fn the_queued_record_carries_the_request_and_round_trips_from_the_store() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec![
            ("a@one.example".to_string(), Expiry::Attempts(5)),
            ("b@two.example".to_string(), Expiry::At(9_000)),
        ];

        let accepted = enqueue(&store, &blobs, BODY, &request(&recipients)).expect("enqueue");

        let loaded = load(&store, accepted.id).expect("load").expect("present");
        assert_eq!(loaded, accepted.message);
        assert_eq!(loaded.created, 1_000);
        assert_eq!(loaded.size, BODY.len() as u64);
        assert_eq!(loaded.return_path, "s@example.com");
        assert_eq!(loaded.recipients.len(), 2);
        assert_eq!(loaded.recipients[0].address, "a@one.example");
        assert_eq!(loaded.recipients[0].expiry, Expiry::Attempts(5));
        assert_eq!(loaded.recipients[1].expiry, Expiry::At(9_000));
        assert!(loaded.has_due_recipient(1_000));
        assert!(!loaded.has_due_recipient(999));
    }

    #[test]
    fn each_enqueue_takes_a_fresh_ascending_id() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec![("a@one.example".to_string(), Expiry::Attempts(1))];

        let first = enqueue(&store, &blobs, BODY, &request(&recipients)).expect("enqueue");
        let second =
            enqueue(&store, &blobs, b"a second message", &request(&recipients)).expect("enqueue");

        assert_eq!(first.id, 1);
        assert_eq!(second.id, 2);
        assert_ne!(message_key(first.id), message_key(second.id));
    }

    #[test]
    fn loading_an_unqueued_id_yields_nothing() {
        let store = MemStore::default();
        assert!(load(&store, 42).expect("load").is_none());
    }

    #[test]
    fn the_record_is_filed_in_the_queue_partition() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec![("a@one.example".to_string(), Expiry::Attempts(1))];

        let accepted = enqueue(&store, &blobs, BODY, &request(&recipients)).expect("enqueue");

        let mut found = 0;
        store
            .iterate(&KeyPrefix::subspace(Subspace::Queue), &mut |key, _value| {
                assert_eq!(key, message_key(accepted.id).as_slice());
                found += 1;
                Ok(Flow::Continue)
            })
            .expect("iterate");
        assert_eq!(found, 1);
    }

    #[test]
    fn a_retry_pulls_pending_recipients_forward_and_leaves_settled_ones_alone() {
        use crate::queue_model::RecipientStatus;

        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec![
            ("a@one.example".to_string(), Expiry::Attempts(5)),
            ("b@two.example".to_string(), Expiry::Attempts(5)),
        ];
        let request = Enqueue {
            created: 1_000,
            return_path: "s@example.com",
            recipients: &recipients,
            first_due: 5_000,
        };
        let accepted = enqueue(&store, &blobs, BODY, &request).expect("enqueue");
        let mut message = load(&store, accepted.id).expect("load").expect("present");
        message.recipients[1].status = RecipientStatus::Delivered;
        persist(&store, accepted.id, &message).expect("persist");

        assert!(retry_now(&store, accepted.id, 2_000).expect("retry"));

        let refreshed = load(&store, accepted.id).expect("load").expect("present");
        assert_eq!(refreshed.recipients[0].retry.due, 2_000);
        assert_eq!(refreshed.recipients[1].retry.due, 5_000);
    }

    #[test]
    fn retrying_an_unqueued_id_reports_nothing_to_do() {
        let store = MemStore::default();
        assert!(!retry_now(&store, 42, 1_000).expect("retry"));
    }

    #[test]
    fn a_message_with_no_recipients_is_complete_at_once() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients: Vec<(String, Expiry)> = Vec::new();

        let accepted = enqueue(&store, &blobs, BODY, &request(&recipients)).expect("enqueue");

        assert!(accepted.message.recipients.is_empty());
        assert!(accepted.message.is_complete());
        assert_eq!(accepted.message.next_due(), None);
    }
}
