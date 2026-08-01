use std::sync::Arc;

use irixmail_core::{Error, Result};
use irixmail_store::{Flow, KeyPrefix, Store, Subspace, WriteOp};

use crate::address::{AddressEntry, Target};

const TAG_ADDRESS_ENTRY: u8 = 0x30;

const CATCH_ALL_PREFIX: char = '@';

#[derive(Clone)]
pub struct AddressIndex {
    store: Arc<dyn Store>,
}

impl AddressIndex {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    pub fn set(&self, entry: AddressEntry) -> Result<()> {
        if entry.address.trim().is_empty() {
            return Err(Error::invalid_input(
                "an address-index entry must name an address",
            ));
        }
        self.store.put(&entry_key(&entry.address), &encode(&entry)?)
    }

    pub fn resolve(&self, address: &str) -> Result<Option<AddressEntry>> {
        let address = normalize_address(address);
        match self.store.get(&entry_key(&address))? {
            Some(bytes) => decode(&bytes).map(Some),
            None => Ok(None),
        }
    }

    pub fn remove(&self, address: &str) -> Result<()> {
        let address = normalize_address(address);
        self.store.delete(&entry_key(&address))
    }

    pub fn set_account_addresses(
        &self,
        account_id: u64,
        previous: &[String],
        current: &[String],
    ) -> Result<()> {
        self.store
            .batch(&self.account_address_ops(account_id, previous, current)?)
    }

    pub fn account_address_ops(
        &self,
        account_id: u64,
        previous: &[String],
        current: &[String],
    ) -> Result<Vec<WriteOp>> {
        let current: Vec<String> = current
            .iter()
            .map(|address| normalize_address(address))
            .collect();
        let previous: Vec<String> = previous
            .iter()
            .map(|address| normalize_address(address))
            .collect();
        if current.iter().any(|address| address.is_empty())
            || previous.iter().any(|address| address.is_empty())
        {
            return Err(Error::invalid_input("an account address must not be blank"));
        }
        for address in &current {
            if previous.contains(address) {
                continue;
            }
            if let Some(entry) = self.resolve(address)? {
                if entry.account_id() != Some(account_id) {
                    return Err(Error::invalid_input(format!(
                        "the address {address} is already in use"
                    )));
                }
            }
        }

        let mut ops = Vec::with_capacity(previous.len() + current.len());
        for address in &previous {
            if !current.contains(address) {
                ops.push(WriteOp::Delete {
                    key: entry_key(address),
                });
            }
        }
        for address in &current {
            ops.push(WriteOp::Set {
                key: entry_key(address),
                value: encode(&AddressEntry {
                    address: address.clone(),
                    target: Target::Account { account_id },
                })?,
            });
        }
        Ok(ops)
    }

    pub(crate) fn account_entry_op(&self, address: &str, account_id: u64) -> Result<WriteOp> {
        let address = normalize_address(address);
        Ok(WriteOp::Set {
            key: entry_key(&address),
            value: encode(&AddressEntry {
                address,
                target: Target::Account { account_id },
            })?,
        })
    }

    pub(crate) fn remove_entry_op(&self, address: &str) -> WriteOp {
        WriteOp::Delete {
            key: entry_key(&normalize_address(address)),
        }
    }

    pub(crate) fn set_catch_all_op(&self, domain: &str, account_id: u64) -> Result<WriteOp> {
        let domain = normalize_address(domain);
        if domain.is_empty() {
            return Err(Error::invalid_input("a catch-all must name a domain"));
        }
        self.account_entry_op(&catch_all_address(&domain), account_id)
    }

    pub(crate) fn clear_catch_all_op(&self, domain: &str) -> WriteOp {
        self.remove_entry_op(&catch_all_address(&normalize_address(domain)))
    }

    pub fn set_catch_all(&self, domain: &str, account_id: u64) -> Result<()> {
        self.store
            .batch(&[self.set_catch_all_op(domain, account_id)?])
    }

    pub fn catch_all(&self, domain: &str) -> Result<Option<AddressEntry>> {
        let domain = normalize_address(domain);
        let wildcard = catch_all_address(&domain);
        match self.store.get(&entry_key(&wildcard))? {
            Some(bytes) => decode(&bytes).map(Some),
            None => Ok(None),
        }
    }

    pub fn clear_catch_all(&self, domain: &str) -> Result<()> {
        self.store.batch(&[self.clear_catch_all_op(domain)])
    }

    pub fn list(&self) -> Result<Vec<AddressEntry>> {
        let mut entries = Vec::new();
        let mut scan_error: Option<Error> = None;
        self.store.iterate(
            &KeyPrefix::subspace(Subspace::Registry),
            &mut |key, value| {
                if !is_entry_key(key) {
                    return Ok(Flow::Continue);
                }
                match decode(value) {
                    Ok(entry) => {
                        entries.push(entry);
                        Ok(Flow::Continue)
                    }
                    Err(err) => {
                        scan_error = Some(err);
                        Ok(Flow::Stop)
                    }
                }
            },
        )?;
        if let Some(err) = scan_error {
            return Err(err);
        }
        Ok(entries)
    }
}

fn entry_key(address: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + address.len());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_ADDRESS_ENTRY);
    key.extend_from_slice(address.as_bytes());
    key
}

fn is_entry_key(key: &[u8]) -> bool {
    key.len() > 2 && key[0] == Subspace::Registry.as_byte() && key[1] == TAG_ADDRESS_ENTRY
}

fn catch_all_address(domain: &str) -> String {
    format!("{CATCH_ALL_PREFIX}{domain}")
}

fn encode(entry: &AddressEntry) -> Result<Vec<u8>> {
    serde_json::to_vec(entry)
        .map_err(|err| Error::serialize(format!("could not encode address entry: {err}")))
}

fn decode(bytes: &[u8]) -> Result<AddressEntry> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::serialize(format!("could not decode address entry: {err}")))
}

fn normalize_address(address: &str) -> String {
    address.trim().to_ascii_lowercase()
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
                    WriteOp::Add { .. } => {
                        unreachable!("the address index does not use counters");
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the address index does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the address index does not use counters")
        }
    }

    fn index() -> AddressIndex {
        AddressIndex::new(Arc::new(MemStore::default()))
    }

    #[test]
    fn a_set_entry_resolves_by_its_address() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        let resolved = index.resolve("alice@irixsoft.com").unwrap().unwrap();
        assert_eq!(resolved.address, "alice@irixsoft.com");
        assert_eq!(resolved.account_id(), Some(7));
    }

    #[test]
    fn resolve_matches_an_address_typed_in_any_casing() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        let resolved = index.resolve("ALICE@IriXSoft.CoM").unwrap().unwrap();
        assert_eq!(resolved.account_id(), Some(7));
    }

    #[test]
    fn resolve_is_none_for_an_address_the_index_does_not_hold() {
        let index = index();
        assert!(index.resolve("absent@irixsoft.com").unwrap().is_none());
    }

    #[test]
    fn a_rejecting_entry_is_present_rather_than_absent() {
        let index = index();
        index
            .set(AddressEntry::reject("blocked@irixsoft.com"))
            .unwrap();
        let resolved = index.resolve("blocked@irixsoft.com").unwrap().unwrap();
        assert!(resolved.target.is_rejected());
    }

    #[test]
    fn a_forward_entry_round_trips_its_destination() {
        let index = index();
        index
            .set(AddressEntry::forward(
                "info@irixsoft.com",
                "owner@example.org",
            ))
            .unwrap();
        let resolved = index.resolve("info@irixsoft.com").unwrap().unwrap();
        assert_eq!(
            resolved.target,
            Target::Forward {
                destination: "owner@example.org".to_string(),
            }
        );
    }

    #[test]
    fn set_overwrites_an_existing_entry() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        index
            .set(AddressEntry::forward(
                "alice@irixsoft.com",
                "elsewhere@example.org",
            ))
            .unwrap();
        let resolved = index.resolve("alice@irixsoft.com").unwrap().unwrap();
        assert_eq!(resolved.account_id(), None);
        assert!(!resolved.target.is_local());
    }

    #[test]
    fn set_rejects_a_blank_address() {
        let index = index();
        let err = index.set(AddressEntry::reject("   ")).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn remove_clears_an_entry_and_is_a_no_op_when_absent() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        index.remove("ALICE@irixsoft.com").unwrap();
        assert!(index.resolve("alice@irixsoft.com").unwrap().is_none());
        index.remove("alice@irixsoft.com").unwrap();
    }

    #[test]
    fn set_account_addresses_indexes_the_primary_and_every_alias() {
        let index = index();
        index
            .set_account_addresses(
                7,
                &[],
                &[
                    "alice@irixsoft.com".to_string(),
                    "a.adams@irixsoft.com".to_string(),
                ],
            )
            .unwrap();
        assert_eq!(
            index
                .resolve("alice@irixsoft.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(7)
        );
        assert_eq!(
            index
                .resolve("a.adams@irixsoft.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(7)
        );
    }

    #[test]
    fn set_account_addresses_drops_an_alias_removed_from_the_set() {
        let index = index();
        let first = vec![
            "alice@irixsoft.com".to_string(),
            "old@irixsoft.com".to_string(),
        ];
        index.set_account_addresses(7, &[], &first).unwrap();

        let second = vec!["alice@irixsoft.com".to_string()];
        index.set_account_addresses(7, &first, &second).unwrap();

        assert!(index.resolve("old@irixsoft.com").unwrap().is_none());
        assert_eq!(
            index
                .resolve("alice@irixsoft.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(7)
        );
    }

    #[test]
    fn set_account_addresses_treats_casing_as_one_address() {
        let index = index();
        index
            .set_account_addresses(
                7,
                &["Alice@IriXSoft.com".to_string()],
                &["alice@irixsoft.com".to_string()],
            )
            .unwrap();
        assert_eq!(
            index
                .resolve("alice@irixsoft.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(7)
        );
    }

    #[test]
    fn account_address_ops_refuses_an_address_held_by_another_account() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        let err = index
            .account_address_ops(8, &[], &["alice@irixsoft.com".to_string()])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn account_address_ops_refuses_an_address_held_by_a_forward() {
        let index = index();
        index
            .set(AddressEntry::forward(
                "info@irixsoft.com",
                "owner@example.org",
            ))
            .unwrap();
        let err = index
            .account_address_ops(8, &[], &["info@irixsoft.com".to_string()])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn account_address_ops_allows_reclaiming_the_accounts_own_entry() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        let ops = index
            .account_address_ops(7, &[], &["alice@irixsoft.com".to_string()])
            .unwrap();
        assert!(!ops.is_empty());
    }

    #[test]
    fn set_account_addresses_rejects_a_blank_address() {
        let index = index();
        let err = index
            .set_account_addresses(7, &[], &["  ".to_string()])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn a_catch_all_resolves_for_its_domain_only() {
        let index = index();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        let resolved = index.catch_all("IriXSoft.com").unwrap().unwrap();
        assert_eq!(resolved.account_id(), Some(9));
        assert!(index.catch_all("example.org").unwrap().is_none());
    }

    #[test]
    fn a_catch_all_does_not_shadow_an_exact_address() {
        let index = index();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        assert!(index.resolve("alice@irixsoft.com").unwrap().is_none());
        assert!(index.resolve("irixsoft.com").unwrap().is_none());
    }

    #[test]
    fn clear_catch_all_removes_it_and_is_a_no_op_when_absent() {
        let index = index();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        index.clear_catch_all("IriXSoft.com").unwrap();
        assert!(index.catch_all("irixsoft.com").unwrap().is_none());
        index.clear_catch_all("irixsoft.com").unwrap();
    }

    #[test]
    fn set_catch_all_rejects_a_blank_domain() {
        let index = index();
        let err = index.set_catch_all("   ", 9).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn list_returns_every_entry_in_address_order() {
        let index = index();
        index
            .set(AddressEntry::account("bob@irixsoft.com", 2))
            .unwrap();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 1))
            .unwrap();
        index.set_catch_all("irixsoft.com", 9).unwrap();

        let listed = index.list().unwrap();
        let addresses: Vec<&str> = listed.iter().map(|entry| entry.address.as_str()).collect();
        assert_eq!(
            addresses,
            vec!["@irixsoft.com", "alice@irixsoft.com", "bob@irixsoft.com"]
        );
    }

    #[test]
    fn list_is_empty_on_a_fresh_index() {
        let index = index();
        assert!(index.list().unwrap().is_empty());
    }
}
