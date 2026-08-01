use std::time::{Duration, SystemTime, UNIX_EPOCH};

use irixmail_core::Result;
use irixmail_store::{BlobStore, Store};

use crate::queue_enqueue::{enqueue, Enqueue, Enqueued};
use crate::queue_model::Expiry;

pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(5 * 86_400);

pub struct Submission<'a> {
    pub return_path: &'a str,
    pub recipients: &'a [String],
}

pub fn enqueue_submission(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    signed: &[u8],
    submission: &Submission<'_>,
) -> Result<Enqueued> {
    enqueue_submission_at(store, blobs, signed, submission, now_seconds())
}

fn enqueue_submission_at(
    store: &dyn Store,
    blobs: &dyn BlobStore,
    signed: &[u8],
    submission: &Submission<'_>,
    now: u64,
) -> Result<Enqueued> {
    let expiry = Expiry::At(now.saturating_add(DEFAULT_MAX_AGE.as_secs()));
    let recipients: Vec<(String, Expiry)> = submission
        .recipients
        .iter()
        .map(|address| (address.clone(), expiry))
        .collect();

    let request = Enqueue {
        created: now,
        return_path: submission.return_path,
        recipients: &recipients,
        first_due: now,
    };

    enqueue(store, blobs, signed, &request)
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue_enqueue::load;
    use crate::queue_model::RecipientStatus;
    use irixmail_core::Result;
    use irixmail_store::{BlobHash, Flow, KeyPrefix, Subspace, WriteOp};
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

    const SIGNED: &[u8] =
        b"DKIM-Signature: v=1\r\nFrom: alice@irixsoft.com\r\nTo: bob@example.org\r\n\r\nHi.\r\n";

    fn submission<'a>(recipients: &'a [String]) -> Submission<'a> {
        Submission {
            return_path: "alice@irixsoft.com",
            recipients,
        }
    }

    #[test]
    fn the_signed_body_is_queued_verbatim_under_its_blob() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec!["bob@example.org".to_string()];

        let accepted =
            enqueue_submission(&store, &blobs, SIGNED, &submission(&recipients)).expect("enqueue");

        let stored = blobs
            .get_all(&accepted.message.blob_hash())
            .expect("blob get")
            .expect("blob present");
        assert_eq!(stored, SIGNED);
        assert_eq!(accepted.message.size, SIGNED.len() as u64);
    }

    #[test]
    fn the_return_path_is_the_authenticated_sender() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec!["bob@example.org".to_string()];

        let accepted =
            enqueue_submission(&store, &blobs, SIGNED, &submission(&recipients)).expect("enqueue");

        assert_eq!(accepted.message.return_path, "alice@irixsoft.com");
    }

    #[test]
    fn every_recipient_is_admitted_and_due_at_once() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec![
            "bob@example.org".to_string(),
            "carol@example.net".to_string(),
        ];

        let accepted =
            enqueue_submission_at(&store, &blobs, SIGNED, &submission(&recipients), 1_000)
                .expect("enqueue");

        assert_eq!(accepted.message.recipients.len(), 2);
        assert_eq!(accepted.message.recipients[0].address, "bob@example.org");
        assert_eq!(accepted.message.recipients[1].address, "carol@example.net");
        for rcpt in &accepted.message.recipients {
            assert_eq!(rcpt.status, RecipientStatus::Scheduled);
        }
        assert!(accepted.message.has_due_recipient(1_000));
    }

    #[test]
    fn each_recipient_expires_a_max_age_past_acceptance() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec!["bob@example.org".to_string()];

        let accepted =
            enqueue_submission_at(&store, &blobs, SIGNED, &submission(&recipients), 1_000)
                .expect("enqueue");

        let deadline = 1_000 + DEFAULT_MAX_AGE.as_secs();
        assert_eq!(accepted.message.recipients[0].expiry, Expiry::At(deadline));
        assert!(!accepted.message.recipients[0].has_expired(deadline - 1));
        assert!(accepted.message.recipients[0].has_expired(deadline));
    }

    #[test]
    fn the_accepted_instant_is_the_submission_now() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec!["bob@example.org".to_string()];

        let accepted =
            enqueue_submission_at(&store, &blobs, SIGNED, &submission(&recipients), 4_242)
                .expect("enqueue");

        assert_eq!(accepted.message.created, 4_242);
    }

    #[test]
    fn the_queued_record_reads_back_from_the_store() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec!["bob@example.org".to_string()];

        let accepted =
            enqueue_submission(&store, &blobs, SIGNED, &submission(&recipients)).expect("enqueue");

        let loaded = load(&store, accepted.id).expect("load").expect("present");
        assert_eq!(loaded, accepted.message);
    }

    #[test]
    fn a_message_with_no_recipients_is_complete_at_once() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients: Vec<String> = Vec::new();

        let accepted =
            enqueue_submission(&store, &blobs, SIGNED, &submission(&recipients)).expect("enqueue");

        assert!(accepted.message.recipients.is_empty());
        assert!(accepted.message.is_complete());
    }

    #[test]
    fn the_record_lands_in_the_queue_partition() {
        let store = MemStore::default();
        let blobs = MemBlobStore::default();
        let recipients = vec!["bob@example.org".to_string()];

        enqueue_submission(&store, &blobs, SIGNED, &submission(&recipients)).expect("enqueue");

        let mut found = 0;
        store
            .iterate(
                &KeyPrefix::subspace(Subspace::Queue),
                &mut |_key, _value| {
                    found += 1;
                    Ok(Flow::Continue)
                },
            )
            .expect("iterate");
        assert_eq!(found, 1);
    }
}
