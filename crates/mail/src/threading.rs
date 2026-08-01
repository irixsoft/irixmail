use irixmail_core::Result;
use irixmail_store::{Collection, Key, Store, Subspace, WriteOp};
use mail_parser::{HeaderValue, MessageParser};

const MAX_REFERENCES: usize = 20;

pub struct ThreadResolution {
    pub thread_id: u32,
    pub ops: Vec<WriteOp>,
}

pub fn resolve_thread(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
    raw: &[u8],
) -> Result<ThreadResolution> {
    let ids = message_ids(raw);
    let mut lookups = Vec::with_capacity(ids.len());
    let mut thread_id = None;
    for id in &ids {
        let found = lookup(store, account_id, id)?;
        if thread_id.is_none() {
            thread_id = found;
        }
        lookups.push(found);
    }
    let thread_id = thread_id.unwrap_or(document_id);
    let ops = ids
        .iter()
        .zip(&lookups)
        .filter(|(_, found)| found.is_none())
        .map(|(id, _)| WriteOp::Set {
            key: entry_key(account_id, id),
            value: thread_id.to_be_bytes().to_vec(),
        })
        .collect();
    Ok(ThreadResolution { thread_id, ops })
}

fn message_ids(raw: &[u8]) -> Vec<String> {
    let Some(parsed) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    let mut push = |id: &str| {
        let id = id
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim();
        if !id.is_empty() && ids.len() < MAX_REFERENCES && !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
    };
    collect(parsed.in_reply_to(), &mut push);
    match parsed.references() {
        HeaderValue::TextList(list) => {
            for id in list.iter().rev() {
                push(id);
            }
        }
        other => collect(other, &mut push),
    }
    if let Some(id) = parsed.message_id() {
        push(id);
    }
    ids
}

fn collect(value: &HeaderValue<'_>, push: &mut impl FnMut(&str)) {
    match value {
        HeaderValue::Text(id) => push(id),
        HeaderValue::TextList(list) => {
            for id in list {
                push(id);
            }
        }
        _ => {}
    }
}

fn lookup(store: &dyn Store, account_id: u32, message_id: &str) -> Result<Option<u32>> {
    Ok(store
        .get(&entry_key(account_id, message_id))?
        .and_then(|bytes| bytes.as_slice().try_into().ok().map(u32::from_be_bytes)))
}

fn entry_key(account_id: u32, message_id: &str) -> Vec<u8> {
    let mut suffix = vec![b't'];
    suffix.extend_from_slice(&blake3::hash(message_id.as_bytes()).as_bytes()[..16]);
    Key::new(Subspace::Property, account_id, Collection::Thread, 0)
        .with_suffix(suffix)
        .encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_store::{Flow, KeyPrefix};
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

    fn resolve_and_apply(store: &MemStore, document_id: u32, raw: &[u8]) -> u32 {
        let resolution = resolve_thread(store, 7, document_id, raw).unwrap();
        store.batch(&resolution.ops).unwrap();
        resolution.thread_id
    }

    fn message(message_id: &str, extra_headers: &str) -> Vec<u8> {
        format!(
            "From: a@example.com\r\nSubject: hi\r\nMessage-ID: <{message_id}>\r\n{extra_headers}\r\n\r\nbody\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn an_unrelated_message_starts_its_own_thread() {
        let store = MemStore::default();
        let thread = resolve_and_apply(&store, 10, &message("first@example.com", ""));
        assert_eq!(thread, 10);
    }

    #[test]
    fn a_reply_joins_its_parents_thread() {
        let store = MemStore::default();
        resolve_and_apply(&store, 10, &message("first@example.com", ""));
        let reply = resolve_and_apply(
            &store,
            11,
            &message("second@example.com", "In-Reply-To: <first@example.com>\r\n"),
        );
        assert_eq!(reply, 10);
    }

    #[test]
    fn a_references_chain_links_a_grandchild_to_the_root() {
        let store = MemStore::default();
        resolve_and_apply(&store, 10, &message("first@example.com", ""));
        resolve_and_apply(
            &store,
            11,
            &message("second@example.com", "References: <first@example.com>\r\n"),
        );
        let grandchild = resolve_and_apply(
            &store,
            12,
            &message(
                "third@example.com",
                "References: <first@example.com> <second@example.com>\r\n",
            ),
        );
        assert_eq!(grandchild, 10);
    }

    #[test]
    fn an_out_of_order_parent_joins_the_thread_its_reply_started() {
        let store = MemStore::default();
        let reply = resolve_and_apply(
            &store,
            10,
            &message("second@example.com", "In-Reply-To: <first@example.com>\r\n"),
        );
        assert_eq!(reply, 10);
        let parent = resolve_and_apply(&store, 11, &message("first@example.com", ""));
        assert_eq!(parent, 10);
    }

    #[test]
    fn a_message_without_a_message_id_still_gets_a_thread() {
        let store = MemStore::default();
        let raw = b"From: a@example.com\r\nSubject: bare\r\n\r\nbody\r\n";
        let thread = resolve_and_apply(&store, 42, raw);
        assert_eq!(thread, 42);
    }

    #[test]
    fn two_replies_to_the_same_parent_share_a_thread() {
        let store = MemStore::default();
        resolve_and_apply(&store, 10, &message("first@example.com", ""));
        let one = resolve_and_apply(
            &store,
            11,
            &message("a@example.com", "In-Reply-To: <first@example.com>\r\n"),
        );
        let two = resolve_and_apply(
            &store,
            12,
            &message("b@example.com", "In-Reply-To: <first@example.com>\r\n"),
        );
        assert_eq!(one, 10);
        assert_eq!(two, 10);
    }
}
