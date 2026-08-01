use irixmail_core::{Error, Result};
use irixmail_store::{
    BlobStore, Collection, Field, Flow, FtsIndex, KeyPrefix, Store, Subspace, WriteOp,
};
use mail_parser::{Address, Message, MessageParser};

const INDEX_COLLECTION: Collection = Collection::Email;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageText {
    pub subject: String,
    pub body: String,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
}

impl MessageText {
    fn fields(&self) -> [(Field, &str); 6] {
        [
            (Field::Subject, self.subject.as_str()),
            (Field::Body, self.body.as_str()),
            (Field::From, self.from.as_str()),
            (Field::To, self.to.as_str()),
            (Field::Cc, self.cc.as_str()),
            (Field::Bcc, self.bcc.as_str()),
        ]
    }

    fn spans(&self) -> impl Iterator<Item = &str> {
        self.fields()
            .into_iter()
            .filter_map(|(_, span)| (!span.is_empty()).then_some(span))
    }

    pub fn is_empty(&self) -> bool {
        self.fields().iter().all(|(_, span)| span.is_empty())
    }
}

pub fn message_text(raw: &[u8]) -> Result<MessageText> {
    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::invalid_input("message has no parseable headers"))?;
    Ok(gather(&parsed))
}

pub fn message_sender(raw: &[u8]) -> String {
    let Some(parsed) = MessageParser::default().parse(raw) else {
        return String::new();
    };
    let Some(first) = parsed.from().and_then(|address| address.iter().next()) else {
        return String::new();
    };
    first
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .or(first.address.as_deref())
        .unwrap_or_default()
        .to_string()
}

pub fn message_sent_at(raw: &[u8]) -> u64 {
    MessageParser::default()
        .parse(raw)
        .and_then(|message| message.date().map(|date| date.to_timestamp()))
        .filter(|timestamp| *timestamp > 0)
        .unwrap_or(0) as u64
}

fn gather(message: &Message<'_>) -> MessageText {
    let subject = message.subject().unwrap_or_default().to_string();

    let mut body = String::new();
    for pos in 0..message.text_body_count() {
        if let Some(text) = message.body_text(pos) {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(text.as_ref());
        }
    }

    MessageText {
        subject,
        body,
        from: addresses(message.from()),
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        bcc: addresses(message.bcc()),
    }
}

fn addresses(address: Option<&Address<'_>>) -> String {
    let mut out = String::new();
    if let Some(address) = address {
        for addr in address.iter() {
            if let Some(name) = &addr.name {
                push_with_space(&mut out, name.as_ref());
            }
            if let Some(value) = &addr.address {
                push_with_space(&mut out, value.as_ref());
            }
        }
    }
    out
}

fn push_with_space(out: &mut String, text: &str) {
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(text);
}

pub fn index_message(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
    raw: &[u8],
) -> Result<()> {
    let text = message_text(raw)?;
    write(store, account_id, document_id, &text, Direction::Add)
}

pub fn unindex_message(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
    raw: &[u8],
) -> Result<()> {
    let text = message_text(raw)?;
    write(store, account_id, document_id, &text, Direction::Remove)
}

pub fn reindex_account(store: &dyn Store, blobs: &dyn BlobStore, account_id: u32) -> Result<usize> {
    let prefix = KeyPrefix::collection(Subspace::Index, account_id, INDEX_COLLECTION);
    let mut wipe = Vec::new();
    store.iterate(&prefix, &mut |key, _value| {
        wipe.push(WriteOp::Delete { key: key.to_vec() });
        Ok(Flow::Continue)
    })?;
    if !wipe.is_empty() {
        store.batch(&wipe)?;
    }

    let mut reindexed = 0;
    for document_id in document_ids(store, account_id)? {
        let Some(raw) = crate::read::load_raw(store, blobs, account_id, document_id)? else {
            continue;
        };
        let Ok(text) = message_text(&raw) else {
            continue;
        };
        index_text(store, account_id, document_id, &text)?;
        reindexed += 1;
    }
    Ok(reindexed)
}

const DOCUMENT_ID_OFFSET: usize = 1 + std::mem::size_of::<u32>() + 1;
const DATA_KEY_LEN: usize = DOCUMENT_ID_OFFSET + std::mem::size_of::<u32>();

fn document_ids(store: &dyn Store, account_id: u32) -> Result<Vec<u32>> {
    let prefix = KeyPrefix::collection(Subspace::Property, account_id, INDEX_COLLECTION);
    let mut ids = Vec::new();
    store.iterate(&prefix, &mut |key, _value| {
        if key.len() == DATA_KEY_LEN {
            let mut bytes = [0u8; std::mem::size_of::<u32>()];
            bytes.copy_from_slice(&key[DOCUMENT_ID_OFFSET..]);
            ids.push(u32::from_be_bytes(bytes));
        }
        Ok(Flow::Continue)
    })?;
    Ok(ids)
}

pub fn index_text(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
    text: &MessageText,
) -> Result<()> {
    write(store, account_id, document_id, text, Direction::Add)
}

pub fn index_ops(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
    text: &MessageText,
) -> Result<Vec<WriteOp>> {
    FtsIndex::new(store).index_ops(account_id, INDEX_COLLECTION, document_id, &entries(text))
}

pub fn unindex_ops(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
    text: &MessageText,
) -> Result<Vec<WriteOp>> {
    FtsIndex::new(store).remove_ops(account_id, INDEX_COLLECTION, document_id, &entries(text))
}

fn entries(text: &MessageText) -> Vec<(Field, &str)> {
    let mut entries: Vec<(Field, &str)> =
        text.spans().map(|span| (Field::Combined, span)).collect();
    entries.extend(
        text.fields()
            .into_iter()
            .filter(|(_, span)| !span.is_empty()),
    );
    entries
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Add,
    Remove,
}

fn write(
    store: &dyn Store,
    account_id: u32,
    document_id: u32,
    text: &MessageText,
    direction: Direction,
) -> Result<()> {
    let ops = match direction {
        Direction::Add => index_ops(store, account_id, document_id, text)?,
        Direction::Remove => unindex_ops(store, account_id, document_id, text)?,
    };
    if ops.is_empty() {
        return Ok(());
    }
    store.batch(&ops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_store::{indexed_terms, Flow, KeyPrefix, Query, WriteOp};
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
                    WriteOp::Add { .. } => {
                        unimplemented!("the mail index does not use counters")
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unimplemented!("the mail index does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unimplemented!("the mail index does not use counters")
        }
    }

    const ACCOUNT: u32 = 3;
    const DOC: u32 = 1;

    fn search(store: &dyn Store, word: &str, candidates: &[u32]) -> Vec<u32> {
        FtsIndex::new(store)
            .search(ACCOUNT, INDEX_COLLECTION, &Query::term(word), candidates)
            .unwrap()
    }

    const PLAIN: &[u8] = concat!(
        "Subject: Quarterly invoice attached\r\n",
        "From: Alice Example <alice@example.com>\r\n",
        "To: bob@example.org\r\n",
        "\r\n",
        "Please find the invoice for review.\r\n",
    )
    .as_bytes();

    #[test]
    fn message_sender_prefers_the_display_name() {
        assert_eq!(message_sender(PLAIN), "Alice Example");
        let bare: &[u8] = b"From: alice@example.com\r\nSubject: x\r\n\r\nhi\r\n";
        assert_eq!(message_sender(bare), "alice@example.com");
        assert_eq!(message_sender(b"Subject: x\r\n\r\nhi\r\n"), "");
    }

    #[test]
    fn gathers_subject_body_and_addresses_per_field() {
        let text = message_text(PLAIN).unwrap();
        assert_eq!(text.subject, "Quarterly invoice attached");
        assert!(text.body.contains("Please find the invoice for review."));
        assert!(text.from.contains("Alice"));
        assert!(text.from.contains("alice@example.com"));
        assert!(text.to.contains("bob@example.org"));
        assert!(text.cc.is_empty());
        assert!(!text.is_empty());
    }

    #[test]
    fn field_scoped_search_matches_only_the_named_field() {
        let store = MemStore::default();
        index_message(&store, ACCOUNT, DOC, PLAIN).unwrap();

        let subject_hit = FtsIndex::new(&store)
            .search(
                ACCOUNT,
                INDEX_COLLECTION,
                &Query::field(Field::Subject, "quarterly"),
                &[DOC],
            )
            .unwrap();
        assert_eq!(subject_hit, vec![DOC]);

        let from_miss = FtsIndex::new(&store)
            .search(
                ACCOUNT,
                INDEX_COLLECTION,
                &Query::field(Field::From, "quarterly"),
                &[DOC],
            )
            .unwrap();
        assert!(from_miss.is_empty());

        let from_hit = FtsIndex::new(&store)
            .search(
                ACCOUNT,
                INDEX_COLLECTION,
                &Query::field(Field::From, "alice"),
                &[DOC],
            )
            .unwrap();
        assert_eq!(from_hit, vec![DOC]);
    }

    #[test]
    fn indexing_makes_every_field_searchable() {
        let store = MemStore::default();
        index_message(&store, ACCOUNT, DOC, PLAIN).unwrap();

        assert_eq!(search(&store, "quarterly", &[DOC]), vec![DOC]);
        assert_eq!(search(&store, "review", &[DOC]), vec![DOC]);
        assert_eq!(search(&store, "alice", &[DOC]), vec![DOC]);
        assert_eq!(search(&store, "example.org", &[DOC]), vec![DOC]);

        assert!(search(&store, "spreadsheet", &[DOC]).is_empty());
    }

    #[test]
    fn html_body_is_indexed_as_its_text() {
        let raw = concat!(
            "Subject: Newsletter\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "<p>Welcome to the <b>monthly</b> roundup</p>\r\n",
        )
        .as_bytes();
        let store = MemStore::default();
        index_message(&store, ACCOUNT, DOC, raw).unwrap();

        assert_eq!(search(&store, "monthly", &[DOC]), vec![DOC]);
        assert_eq!(search(&store, "roundup", &[DOC]), vec![DOC]);
        assert!(search(&store, "b", &[DOC]).is_empty());
    }

    #[test]
    fn unindexing_clears_the_message_from_results() {
        let store = MemStore::default();
        index_message(&store, ACCOUNT, DOC, PLAIN).unwrap();
        unindex_message(&store, ACCOUNT, DOC, PLAIN).unwrap();

        assert!(search(&store, "quarterly", &[DOC]).is_empty());
        assert!(search(&store, "alice", &[DOC]).is_empty());
        let terms = indexed_terms(&store, ACCOUNT, INDEX_COLLECTION).unwrap();
        assert!(terms.is_empty());
    }

    #[test]
    fn re_indexing_the_same_message_is_idempotent() {
        let store = MemStore::default();
        index_message(&store, ACCOUNT, DOC, PLAIN).unwrap();
        index_message(&store, ACCOUNT, DOC, PLAIN).unwrap();

        assert_eq!(search(&store, "invoice", &[DOC]), vec![DOC]);
    }

    #[test]
    fn index_text_matches_indexing_from_raw_bytes() {
        let store = MemStore::default();
        let text = message_text(PLAIN).unwrap();
        index_text(&store, ACCOUNT, DOC, &text).unwrap();

        assert_eq!(search(&store, "quarterly", &[DOC]), vec![DOC]);
        assert_eq!(search(&store, "alice", &[DOC]), vec![DOC]);
    }

    #[test]
    fn distinct_messages_are_found_independently() {
        let store = MemStore::default();
        index_message(&store, ACCOUNT, 1, PLAIN).unwrap();
        let other = concat!(
            "Subject: Meeting notes\r\n",
            "From: carol@example.net\r\n",
            "\r\n",
            "Notes from Monday's standup.\r\n",
        )
        .as_bytes();
        index_message(&store, ACCOUNT, 2, other).unwrap();

        assert_eq!(search(&store, "invoice", &[1, 2]), vec![1]);
        assert_eq!(search(&store, "standup", &[1, 2]), vec![2]);
        assert!(search(&store, "absent", &[1, 2]).is_empty());
    }

    #[test]
    fn message_with_no_headers_is_rejected() {
        assert!(matches!(message_text(b""), Err(Error::InvalidInput(_))));
    }

    #[test]
    fn sent_at_reads_the_date_header_and_defaults_to_zero() {
        let dated = concat!(
            "Subject: Dated\r\n",
            "Date: Sat, 01 Feb 2020 00:00:00 +0000\r\n",
            "\r\n",
            "body\r\n",
        )
        .as_bytes();
        assert_eq!(message_sent_at(dated), 1_580_515_200);
        assert_eq!(message_sent_at(PLAIN), 0);
    }

    #[test]
    fn message_with_no_searchable_text_indexes_nothing() {
        let raw = b"Date: Mon, 1 Jan 2024 00:00:00 +0000\r\n\r\n";
        let text = message_text(raw).unwrap();
        assert!(text.is_empty());

        let store = MemStore::default();
        index_message(&store, ACCOUNT, DOC, raw).unwrap();
        assert!(indexed_terms(&store, ACCOUNT, INDEX_COLLECTION)
            .unwrap()
            .is_empty());
    }
}
