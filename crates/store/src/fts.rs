use std::collections::BTreeSet;

use irixmail_core::{Error, Result};
use roaring::RoaringBitmap;

use crate::key::{Collection, Key, KeyPrefix, Subspace};
use crate::traits_store::{Flow, Store, WriteOp};

pub const MAX_TERM_LENGTH: usize = 64;

pub fn tokenize(text: &str) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            push_term(&mut terms, &mut current);
        }
    }
    push_term(&mut terms, &mut current);
    terms
}

fn push_term(terms: &mut BTreeSet<String>, current: &mut String) {
    if !current.is_empty() {
        if current.len() <= MAX_TERM_LENGTH {
            terms.insert(std::mem::take(current));
        } else {
            current.clear();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Combined,
    Subject,
    Body,
    From,
    To,
    Cc,
    Bcc,
}

impl Field {
    fn tag(self) -> u32 {
        match self {
            Field::Combined => 0,
            Field::Subject => 1,
            Field::Body => 2,
            Field::From => 3,
            Field::To => 4,
            Field::Cc => 5,
            Field::Bcc => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    Term { field: Field, word: String },
    And(Vec<Query>),
    Or(Vec<Query>),
    Not(Box<Query>),
}

impl Query {
    pub fn term(word: impl Into<String>) -> Self {
        Query::Term {
            field: Field::Combined,
            word: word.into(),
        }
    }

    pub fn field(field: Field, word: impl Into<String>) -> Self {
        Query::Term {
            field,
            word: word.into(),
        }
    }

    pub fn all(queries: impl IntoIterator<Item = Query>) -> Self {
        Query::And(queries.into_iter().collect())
    }

    pub fn any(queries: impl IntoIterator<Item = Query>) -> Self {
        Query::Or(queries.into_iter().collect())
    }

    #[allow(clippy::should_implement_trait)]
    pub fn not(query: Query) -> Self {
        Query::Not(Box::new(query))
    }
}

pub struct FtsIndex<'a> {
    store: &'a dyn Store,
}

impl<'a> FtsIndex<'a> {
    pub fn new(store: &'a dyn Store) -> Self {
        Self { store }
    }

    pub fn index(
        &self,
        account_id: u32,
        collection: Collection,
        document_id: u32,
        field: Field,
        text: &str,
    ) -> Result<()> {
        let ops = self.index_ops(account_id, collection, document_id, &[(field, text)])?;
        self.apply(&ops)
    }

    pub fn remove(
        &self,
        account_id: u32,
        collection: Collection,
        document_id: u32,
        field: Field,
        text: &str,
    ) -> Result<()> {
        let ops = self.remove_ops(account_id, collection, document_id, &[(field, text)])?;
        self.apply(&ops)
    }

    pub fn index_ops(
        &self,
        account_id: u32,
        collection: Collection,
        document_id: u32,
        entries: &[(Field, &str)],
    ) -> Result<Vec<WriteOp>> {
        let mut ops = Vec::new();
        for key in term_keys(account_id, collection, entries) {
            let mut bitmap = self.read_bitmap(&key)?;
            if bitmap.insert(document_id) {
                ops.push(WriteOp::Set {
                    key,
                    value: serialize_bitmap(&bitmap),
                });
            }
        }
        Ok(ops)
    }

    pub fn remove_ops(
        &self,
        account_id: u32,
        collection: Collection,
        document_id: u32,
        entries: &[(Field, &str)],
    ) -> Result<Vec<WriteOp>> {
        let mut ops = Vec::new();
        for key in term_keys(account_id, collection, entries) {
            let mut bitmap = self.read_bitmap(&key)?;
            if bitmap.remove(document_id) {
                ops.push(if bitmap.is_empty() {
                    WriteOp::Delete { key }
                } else {
                    WriteOp::Set {
                        key,
                        value: serialize_bitmap(&bitmap),
                    }
                });
            }
        }
        Ok(ops)
    }

    fn apply(&self, ops: &[WriteOp]) -> Result<()> {
        for op in ops {
            match op {
                WriteOp::Set { key, value } => self.store.put(key, value)?,
                WriteOp::Delete { key } => self.store.delete(key)?,
                WriteOp::Add { .. } => {
                    return Err(Error::store("the search index does not use counters"))
                }
            }
        }
        Ok(())
    }

    pub fn search(
        &self,
        account_id: u32,
        collection: Collection,
        query: &Query,
        candidates: &[u32],
    ) -> Result<Vec<u32>> {
        let universe: RoaringBitmap = candidates.iter().copied().collect();
        let mut matched = self.evaluate(account_id, collection, query, &universe)?;
        matched &= &universe;
        Ok(matched.into_iter().collect())
    }

    fn evaluate(
        &self,
        account_id: u32,
        collection: Collection,
        query: &Query,
        universe: &RoaringBitmap,
    ) -> Result<RoaringBitmap> {
        match query {
            Query::Term { field, word } => {
                let mut tokens = tokenize(word).into_iter();
                let Some(first) = tokens.next() else {
                    return Ok(RoaringBitmap::new());
                };
                let mut matched = self.posting_list(account_id, collection, *field, &first)?;
                for token in tokens {
                    matched &= self.posting_list(account_id, collection, *field, &token)?;
                }
                Ok(matched)
            }
            Query::And(children) => {
                let mut iter = children.iter();
                let Some(first) = iter.next() else {
                    return Ok(universe.clone());
                };
                let mut matched = self.evaluate(account_id, collection, first, universe)?;
                for child in iter {
                    if matched.is_empty() {
                        break;
                    }
                    matched &= self.evaluate(account_id, collection, child, universe)?;
                }
                Ok(matched)
            }
            Query::Or(children) => {
                let mut matched = RoaringBitmap::new();
                for child in children {
                    matched |= self.evaluate(account_id, collection, child, universe)?;
                }
                Ok(matched)
            }
            Query::Not(inner) => {
                let matched = self.evaluate(account_id, collection, inner, universe)?;
                Ok(universe - matched)
            }
        }
    }

    fn posting_list(
        &self,
        account_id: u32,
        collection: Collection,
        field: Field,
        term: &str,
    ) -> Result<RoaringBitmap> {
        self.read_bitmap(&term_key(account_id, collection, field, term))
    }

    fn read_bitmap(&self, key: &[u8]) -> Result<RoaringBitmap> {
        match self.store.get(key)? {
            Some(bytes) => deserialize_bitmap(&bytes),
            None => Ok(RoaringBitmap::new()),
        }
    }
}

pub fn indexed_terms(
    store: &dyn Store,
    account_id: u32,
    collection: Collection,
) -> Result<Vec<String>> {
    let prefix = KeyPrefix::collection(Subspace::Index, account_id, collection);
    let prefix_len = prefix.encode().len();
    let mut terms = Vec::new();
    let mut scan_error = None;
    store.iterate(&prefix, &mut |key, _value| {
        match term_of(key, prefix_len) {
            Ok(term) => terms.push(term),
            Err(err) => {
                scan_error = Some(err);
                return Ok(Flow::Stop);
            }
        }
        Ok(Flow::Continue)
    })?;
    if let Some(err) = scan_error {
        return Err(err);
    }
    Ok(terms)
}

fn term_key(account_id: u32, collection: Collection, field: Field, term: &str) -> Vec<u8> {
    Key::new(Subspace::Index, account_id, collection, field.tag())
        .with_suffix(term.as_bytes().to_vec())
        .encode()
}

fn term_keys(
    account_id: u32,
    collection: Collection,
    entries: &[(Field, &str)],
) -> BTreeSet<Vec<u8>> {
    let mut keys = BTreeSet::new();
    for (field, text) in entries {
        for term in tokenize(text) {
            keys.insert(term_key(account_id, collection, *field, &term));
        }
    }
    keys
}

fn term_of(key: &[u8], prefix_len: usize) -> Result<String> {
    let suffix_start = prefix_len + std::mem::size_of::<u32>();
    let suffix = key
        .get(suffix_start..)
        .ok_or_else(|| Error::store("index key is too short to carry a term"))?;
    std::str::from_utf8(suffix)
        .map(|term| term.to_owned())
        .map_err(|_| Error::store("index key term is not valid utf-8"))
}

fn serialize_bitmap(bitmap: &RoaringBitmap) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(bitmap.serialized_size());
    let _ = bitmap.serialize_into(&mut bytes);
    bytes
}

fn deserialize_bitmap(bytes: &[u8]) -> Result<RoaringBitmap> {
    RoaringBitmap::deserialize_from(bytes)
        .map_err(|err| Error::store(format_args!("corrupt search posting list: {err}")))
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

        fn batch(&self, _ops: &[crate::traits_store::WriteOp]) -> Result<()> {
            unimplemented!("the index does not batch")
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unimplemented!("the index does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unimplemented!("the index does not use counters")
        }
    }

    const ACCOUNT: u32 = 7;
    const COLLECTION: Collection = Collection::Email;

    #[test]
    fn tokenize_lowercases_splits_and_dedupes() {
        let terms = tokenize("The Quick brown-FOX, the fox!");
        let collected: Vec<&str> = terms.iter().map(String::as_str).collect();
        assert_eq!(collected, vec!["brown", "fox", "quick", "the"]);
    }

    #[test]
    fn tokenize_keeps_alphanumeric_runs_and_drops_overlong_ones() {
        let long = "a".repeat(MAX_TERM_LENGTH + 1);
        let ok = "b".repeat(MAX_TERM_LENGTH);
        let text = format!("inv0ice {long} {ok}");
        let terms = tokenize(&text);
        assert!(terms.contains("inv0ice"));
        assert!(terms.contains(&ok));
        assert!(!terms.contains(&long));
    }

    #[test]
    fn tokenize_yields_nothing_for_separators_only() {
        assert!(tokenize("   ---  ... ").is_empty());
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn index_then_single_term_search_finds_the_document() {
        let store = MemStore::default();
        let index = FtsIndex::new(&store);
        index
            .index(
                ACCOUNT,
                COLLECTION,
                1,
                Field::Combined,
                "Quarterly invoice attached",
            )
            .unwrap();
        index
            .index(
                ACCOUNT,
                COLLECTION,
                2,
                Field::Combined,
                "Meeting notes for Monday",
            )
            .unwrap();

        let hits = index
            .search(ACCOUNT, COLLECTION, &Query::term("invoice"), &[1, 2])
            .unwrap();
        assert_eq!(hits, vec![1]);

        let hits = index
            .search(ACCOUNT, COLLECTION, &Query::term("INVOICE"), &[1, 2])
            .unwrap();
        assert_eq!(hits, vec![1]);
        let none = index
            .search(ACCOUNT, COLLECTION, &Query::term("absent"), &[1, 2])
            .unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn boolean_and_or_not_combine_posting_lists() {
        let store = MemStore::default();
        let index = FtsIndex::new(&store);
        index
            .index(ACCOUNT, COLLECTION, 1, Field::Combined, "alpha beta")
            .unwrap();
        index
            .index(ACCOUNT, COLLECTION, 2, Field::Combined, "beta gamma")
            .unwrap();
        index
            .index(ACCOUNT, COLLECTION, 3, Field::Combined, "gamma delta")
            .unwrap();
        let all = [1, 2, 3];

        let both = index
            .search(
                ACCOUNT,
                COLLECTION,
                &Query::all([Query::term("beta"), Query::term("alpha")]),
                &all,
            )
            .unwrap();
        assert_eq!(both, vec![1]);

        let either = index
            .search(
                ACCOUNT,
                COLLECTION,
                &Query::any([Query::term("alpha"), Query::term("delta")]),
                &all,
            )
            .unwrap();
        assert_eq!(either, vec![1, 3]);

        let without = index
            .search(ACCOUNT, COLLECTION, &Query::not(Query::term("gamma")), &all)
            .unwrap();
        assert_eq!(without, vec![1]);

        let refined = index
            .search(
                ACCOUNT,
                COLLECTION,
                &Query::all([Query::term("beta"), Query::not(Query::term("alpha"))]),
                &all,
            )
            .unwrap();
        assert_eq!(refined, vec![2]);
    }

    #[test]
    fn search_is_bounded_by_the_candidate_universe() {
        let store = MemStore::default();
        let index = FtsIndex::new(&store);
        index
            .index(ACCOUNT, COLLECTION, 1, Field::Combined, "shared word")
            .unwrap();
        index
            .index(ACCOUNT, COLLECTION, 2, Field::Combined, "shared word")
            .unwrap();

        let hits = index
            .search(ACCOUNT, COLLECTION, &Query::term("shared"), &[1])
            .unwrap();
        assert_eq!(hits, vec![1]);
    }

    #[test]
    fn removing_a_document_clears_it_from_results_and_empty_lists() {
        let store = MemStore::default();
        let index = FtsIndex::new(&store);
        let text = "uniqueword common";
        index
            .index(ACCOUNT, COLLECTION, 1, Field::Combined, text)
            .unwrap();
        index
            .index(ACCOUNT, COLLECTION, 2, Field::Combined, "common only")
            .unwrap();

        index
            .remove(ACCOUNT, COLLECTION, 1, Field::Combined, text)
            .unwrap();

        let gone = index
            .search(ACCOUNT, COLLECTION, &Query::term("uniqueword"), &[1, 2])
            .unwrap();
        assert!(gone.is_empty());
        let common = index
            .search(ACCOUNT, COLLECTION, &Query::term("common"), &[1, 2])
            .unwrap();
        assert_eq!(common, vec![2]);

        let terms = indexed_terms(&store, ACCOUNT, COLLECTION).unwrap();
        assert!(!terms.contains(&"uniqueword".to_string()));
        assert!(terms.contains(&"common".to_string()));
    }

    #[test]
    fn re_indexing_the_same_text_is_idempotent() {
        let store = MemStore::default();
        let index = FtsIndex::new(&store);
        index
            .index(
                ACCOUNT,
                COLLECTION,
                1,
                Field::Combined,
                "repeat repeat repeat",
            )
            .unwrap();
        index
            .index(
                ACCOUNT,
                COLLECTION,
                1,
                Field::Combined,
                "repeat repeat repeat",
            )
            .unwrap();

        let hits = index
            .search(ACCOUNT, COLLECTION, &Query::term("repeat"), &[1])
            .unwrap();
        assert_eq!(hits, vec![1]);
        let terms = indexed_terms(&store, ACCOUNT, COLLECTION).unwrap();
        assert_eq!(terms, vec!["repeat".to_string()]);
    }

    #[test]
    fn indexes_of_different_accounts_and_collections_do_not_mix() {
        let store = MemStore::default();
        let index = FtsIndex::new(&store);
        index
            .index(ACCOUNT, Collection::Email, 1, Field::Combined, "needle")
            .unwrap();
        index
            .index(ACCOUNT + 1, Collection::Email, 1, Field::Combined, "needle")
            .unwrap();
        index
            .index(ACCOUNT, Collection::Mailbox, 1, Field::Combined, "needle")
            .unwrap();

        let hits = index
            .search(ACCOUNT, Collection::Email, &Query::term("needle"), &[1])
            .unwrap();
        assert_eq!(hits, vec![1]);
        assert_eq!(
            indexed_terms(&store, ACCOUNT, Collection::Email).unwrap(),
            vec!["needle".to_string()]
        );
        assert_eq!(
            indexed_terms(&store, ACCOUNT, Collection::Mailbox).unwrap(),
            vec!["needle".to_string()]
        );
    }

    #[test]
    fn posting_lists_round_trip_through_serialization() {
        let mut bitmap = RoaringBitmap::new();
        for id in [1u32, 5, 9, 1_000, u32::MAX] {
            bitmap.insert(id);
        }
        let restored = deserialize_bitmap(&serialize_bitmap(&bitmap)).unwrap();
        assert_eq!(restored, bitmap);
    }

    #[test]
    fn corrupt_posting_list_bytes_are_reported() {
        assert!(deserialize_bitmap(&[0xFF, 0x00, 0x13, 0x37]).is_err());
    }
}
