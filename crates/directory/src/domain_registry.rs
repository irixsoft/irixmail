use std::sync::Arc;

use irixmail_core::{Error, IdGenerator, Result};
use irixmail_store::{Flow, KeyPrefix, Store, Subspace, WriteOp};

use crate::account_registry::AccountRegistry;
use crate::address_index::AddressIndex;
use crate::domain::{DnsStatus, Domain};

const TAG_DOMAIN_RECORD: u8 = 0x10;

const TAG_DOMAIN_NAME: u8 = 0x11;

#[derive(Clone)]
pub struct DomainRegistry {
    store: Arc<dyn Store>,
    ids: Arc<IdGenerator>,
    catch_all_index: Option<AddressIndex>,
    accounts: Option<Box<AccountRegistry>>,
}

impl DomainRegistry {
    pub fn new(store: Arc<dyn Store>, ids: Arc<IdGenerator>) -> Self {
        Self {
            store,
            ids,
            catch_all_index: None,
            accounts: None,
        }
    }

    pub fn with_catch_all_index(mut self, addresses: AddressIndex) -> Self {
        self.catch_all_index = Some(addresses);
        self
    }

    pub fn with_account_maintenance(mut self, accounts: AccountRegistry) -> Self {
        self.accounts = Some(Box::new(accounts));
        self
    }

    fn catch_all_ops(
        &self,
        previous: &Domain,
        current: &Domain,
        ops: &mut Vec<WriteOp>,
    ) -> Result<()> {
        if let Some(addresses) = &self.catch_all_index {
            if previous.catch_all_account_id.is_some() {
                ops.push(addresses.clear_catch_all_op(&previous.name));
            }
            if let Some(account_id) = current.catch_all_account_id {
                ops.push(addresses.set_catch_all_op(&current.name, account_id)?);
            }
        }
        Ok(())
    }

    fn rekey_primary_ops(
        &self,
        old_name: &str,
        new_name: &str,
        domain_id: u64,
        ops: &mut Vec<WriteOp>,
    ) -> Result<()> {
        let (Some(accounts), Some(addresses)) = (&self.accounts, &self.catch_all_index) else {
            return Ok(());
        };
        for account in accounts.list_for_domain(domain_id)? {
            let old_address = format!("{}@{old_name}", account.local_part);
            let new_address = format!("{}@{new_name}", account.local_part);
            if let Some(entry) = addresses.resolve(&new_address)? {
                if entry.account_id() != Some(account.id) {
                    return Err(Error::invalid_input(format!(
                        "the address {new_address} is already in use"
                    )));
                }
            }
            ops.push(addresses.remove_entry_op(&old_address));
            ops.push(addresses.account_entry_op(&new_address, account.id)?);
        }
        Ok(())
    }

    pub fn create(&self, name: &str, aliases: Vec<String>) -> Result<Domain> {
        let name = normalize_name(name);
        if name.is_empty() {
            return Err(Error::invalid_input("a domain name must not be empty"));
        }
        if self.lookup_id_by_name(&name)?.is_some() {
            return Err(Error::invalid_input(format!(
                "a domain named {name} already exists"
            )));
        }

        let aliases = aliases
            .iter()
            .map(|alias| normalize_name(alias))
            .filter(|alias| !alias.is_empty())
            .collect();

        let id = self.ids.generate();
        let created_at = IdGenerator::timestamp_of(id) * 1_000;
        let domain = Domain {
            id,
            name,
            aliases,
            enabled: true,
            catch_all_account_id: None,
            dkim_key_ids: Vec::new(),
            dns_status: DnsStatus::Unverified,
            created_at,
        };

        self.store.batch(&[
            WriteOp::Set {
                key: record_key(domain.id),
                value: encode(&domain)?,
            },
            WriteOp::Set {
                key: name_key(&domain.name),
                value: domain.id.to_be_bytes().to_vec(),
            },
        ])?;
        Ok(domain)
    }

    pub fn get(&self, id: u64) -> Result<Domain> {
        match self.store.get(&record_key(id))? {
            Some(bytes) => decode(&bytes),
            None => Err(Error::not_found(format!("domain {id}"))),
        }
    }

    pub fn get_by_name(&self, name: &str) -> Result<Option<Domain>> {
        let name = normalize_name(name);
        if let Some(id) = self.lookup_id_by_name(&name)? {
            return self.get(id).map(Some);
        }
        Ok(self
            .list()?
            .into_iter()
            .find(|domain| domain.matches_name(&name)))
    }

    pub fn canonical_name(&self, name: &str) -> Result<Option<String>> {
        Ok(self.get_by_name(name)?.map(|domain| domain.name))
    }

    pub fn list(&self) -> Result<Vec<Domain>> {
        let mut domains = Vec::new();
        let mut scan_error: Option<Error> = None;
        self.store.iterate(
            &KeyPrefix::subspace(Subspace::Registry),
            &mut |key, value| {
                if !is_record_key(key) {
                    return Ok(Flow::Continue);
                }
                match decode(value) {
                    Ok(domain) => {
                        domains.push(domain);
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
        Ok(domains)
    }

    pub fn update(&self, mut domain: Domain) -> Result<()> {
        let existing = self.get(domain.id)?;
        domain.name = normalize_name(&domain.name);
        if domain.name.is_empty() {
            return Err(Error::invalid_input("a domain name must not be empty"));
        }
        domain.aliases = domain
            .aliases
            .iter()
            .map(|alias| normalize_name(alias))
            .filter(|alias| !alias.is_empty())
            .collect();

        let mut ops = vec![WriteOp::Set {
            key: record_key(domain.id),
            value: encode(&domain)?,
        }];
        if domain.name != existing.name {
            if let Some(other) = self.lookup_id_by_name(&domain.name)? {
                if other != domain.id {
                    return Err(Error::invalid_input(format!(
                        "a domain named {} already exists",
                        domain.name
                    )));
                }
            }
            ops.push(WriteOp::Delete {
                key: name_key(&existing.name),
            });
            ops.push(WriteOp::Set {
                key: name_key(&domain.name),
                value: domain.id.to_be_bytes().to_vec(),
            });
            self.rekey_primary_ops(&existing.name, &domain.name, domain.id, &mut ops)?;
        }
        self.catch_all_ops(&existing, &domain, &mut ops)?;
        self.store.batch(&ops)?;
        Ok(())
    }

    pub fn delete(&self, id: u64) -> Result<()> {
        let domain = self.get(id)?;
        if let Some(accounts) = &self.accounts {
            if !accounts.list_for_domain(id)?.is_empty() {
                return Err(Error::invalid_input(format!(
                    "the domain {} still has accounts",
                    domain.name
                )));
            }
        }
        let mut ops = vec![
            WriteOp::Delete {
                key: record_key(id),
            },
            WriteOp::Delete {
                key: name_key(&domain.name),
            },
        ];
        if let Some(addresses) = &self.catch_all_index {
            if domain.catch_all_account_id.is_some() {
                ops.push(addresses.clear_catch_all_op(&domain.name));
            }
        }
        self.store.batch(&ops)
    }

    fn lookup_id_by_name(&self, name: &str) -> Result<Option<u64>> {
        match self.store.get(&name_key(name))? {
            Some(bytes) => {
                let array: [u8; 8] = bytes.as_slice().try_into().map_err(|_| {
                    Error::store(format!("domain name index for {name} is corrupt"))
                })?;
                Ok(Some(u64::from_be_bytes(array)))
            }
            None => Ok(None),
        }
    }
}

fn record_key(id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + std::mem::size_of::<u64>());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_DOMAIN_RECORD);
    key.extend_from_slice(&id.to_be_bytes());
    key
}

fn name_key(name: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + name.len());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_DOMAIN_NAME);
    key.extend_from_slice(name.as_bytes());
    key
}

fn is_record_key(key: &[u8]) -> bool {
    key.len() == 2 + std::mem::size_of::<u64>()
        && key[0] == Subspace::Registry.as_byte()
        && key[1] == TAG_DOMAIN_RECORD
}

fn encode(domain: &Domain) -> Result<Vec<u8>> {
    serde_json::to_vec(domain)
        .map_err(|err| Error::serialize(format!("could not encode domain: {err}")))
}

fn decode(bytes: &[u8]) -> Result<Domain> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::serialize(format!("could not decode domain: {err}")))
}

fn normalize_name(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
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
                        unreachable!("the domain registry does not use counters");
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the domain registry does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the domain registry does not use counters")
        }
    }

    fn registry() -> DomainRegistry {
        DomainRegistry::new(Arc::new(MemStore::default()), Arc::new(IdGenerator::new(0)))
    }

    struct BatchOnlyStore {
        inner: MemStore,
    }

    impl Store for BatchOnlyStore {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            self.inner.get(key)
        }

        fn put(&self, _key: &[u8], _value: &[u8]) -> Result<()> {
            Err(Error::store("direct put outside a batch"))
        }

        fn delete(&self, _key: &[u8]) -> Result<()> {
            Err(Error::store("direct delete outside a batch"))
        }

        fn iterate(
            &self,
            prefix: &KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            self.inner.iterate(prefix, visit)
        }

        fn batch(&self, ops: &[WriteOp]) -> Result<()> {
            self.inner.batch(ops)
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the domain registry does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the domain registry does not use counters")
        }
    }

    fn full_registry(
        store: Arc<dyn Store>,
    ) -> (
        DomainRegistry,
        crate::account_registry::AccountRegistry,
        AddressIndex,
    ) {
        let ids = Arc::new(IdGenerator::new(0));
        let addresses = AddressIndex::new(Arc::clone(&store));
        let domains = DomainRegistry::new(Arc::clone(&store), Arc::clone(&ids))
            .with_catch_all_index(addresses.clone())
            .with_account_maintenance(crate::account_registry::AccountRegistry::new(
                Arc::clone(&store),
                Arc::clone(&ids),
            ));
        let accounts = crate::account_registry::AccountRegistry::new(store, ids)
            .with_address_maintenance(domains.clone(), addresses.clone());
        (domains, accounts, addresses)
    }

    #[test]
    fn rename_rekeys_every_accounts_primary_address() {
        use crate::account::Role;
        let (domains, accounts, addresses) = full_registry(Arc::new(MemStore::default()));
        let mut domain = domains.create("example.com", Vec::new()).unwrap();
        let alice = accounts.create("alice", domain.id, "", Role::User).unwrap();
        let bob = accounts.create("bob", domain.id, "", Role::User).unwrap();

        domain.name = "example.net".to_string();
        domains.update(domain).unwrap();

        assert_eq!(
            addresses
                .resolve("alice@example.net")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(alice.id)
        );
        assert_eq!(
            addresses
                .resolve("bob@example.net")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(bob.id)
        );
        assert!(addresses.resolve("alice@example.com").unwrap().is_none());
        assert!(addresses.resolve("bob@example.com").unwrap().is_none());
    }

    #[test]
    fn rename_moves_the_catch_all_with_the_name() {
        use crate::account::Role;
        let (domains, accounts, addresses) = full_registry(Arc::new(MemStore::default()));
        let mut domain = domains.create("example.com", Vec::new()).unwrap();
        let alice = accounts.create("alice", domain.id, "", Role::User).unwrap();
        domain.catch_all_account_id = Some(alice.id);
        domains.update(domain.clone()).unwrap();

        domain.name = "example.net".to_string();
        domains.update(domain).unwrap();

        assert_eq!(
            addresses
                .catch_all("example.net")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(alice.id)
        );
        assert!(addresses.catch_all("example.com").unwrap().is_none());
    }

    #[test]
    fn rename_refuses_when_a_new_primary_collides_with_a_foreign_entry() {
        use crate::account::Role;
        let (domains, accounts, addresses) = full_registry(Arc::new(MemStore::default()));
        let mut domain = domains.create("example.com", Vec::new()).unwrap();
        accounts.create("alice", domain.id, "", Role::User).unwrap();

        let other = domains.create("other.example", Vec::new()).unwrap();
        let mut carol = accounts.create("carol", other.id, "", Role::User).unwrap();
        carol.aliases = vec!["alice@example.net".to_string()];
        accounts.update(carol.clone()).unwrap();

        domain.name = "example.net".to_string();
        let err = domains.update(domain).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(
            addresses
                .resolve("alice@example.net")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(carol.id)
        );
    }

    #[test]
    fn delete_is_blocked_while_the_domain_still_has_accounts() {
        use crate::account::Role;
        let (domains, accounts, addresses) = full_registry(Arc::new(MemStore::default()));
        let domain = domains.create("example.com", Vec::new()).unwrap();
        let alice = accounts.create("alice", domain.id, "", Role::User).unwrap();

        let err = domains.delete(domain.id).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert!(domains.get(domain.id).is_ok());
        assert_eq!(
            addresses
                .resolve("alice@example.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(alice.id)
        );

        accounts.delete(alice.id).unwrap();
        domains.delete(domain.id).unwrap();
        assert!(domains.get(domain.id).is_err());
    }

    #[test]
    fn delete_clears_the_catch_all_in_the_record_batch() {
        use crate::account::Role;
        let store: Arc<dyn Store> = Arc::new(BatchOnlyStore {
            inner: MemStore::default(),
        });
        let (domains, accounts, addresses) = full_registry(Arc::clone(&store));
        let other = domains.create("other.example", Vec::new()).unwrap();
        let carol = accounts.create("carol", other.id, "", Role::User).unwrap();

        let mut domain = domains.create("example.com", Vec::new()).unwrap();
        domain.catch_all_account_id = Some(carol.id);
        domains.update(domain.clone()).unwrap();

        domains.delete(domain.id).unwrap();
        assert!(addresses.catch_all("example.com").unwrap().is_none());
    }

    #[test]
    fn catch_all_changes_ride_the_record_batch() {
        use crate::account::Role;
        let store: Arc<dyn Store> = Arc::new(BatchOnlyStore {
            inner: MemStore::default(),
        });
        let (domains, accounts, addresses) = full_registry(Arc::clone(&store));
        let mut domain = domains.create("example.com", Vec::new()).unwrap();
        let alice = accounts.create("alice", domain.id, "", Role::User).unwrap();

        domain.catch_all_account_id = Some(alice.id);
        domains.update(domain.clone()).unwrap();
        assert_eq!(
            addresses
                .catch_all("example.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(alice.id)
        );

        domain.catch_all_account_id = None;
        domains.update(domain).unwrap();
        assert!(addresses.catch_all("example.com").unwrap().is_none());
    }

    #[test]
    fn a_created_domain_is_readable_by_id_and_by_name() {
        let registry = registry();
        let created = registry.create("Example.COM", Vec::new()).unwrap();

        assert_eq!(created.name, "example.com");
        assert!(created.enabled);
        assert_eq!(created.dns_status, DnsStatus::Unverified);
        assert_ne!(created.id, 0);
        assert!(created.created_at > 0);

        let by_id = registry.get(created.id).unwrap();
        assert_eq!(by_id, created);

        let by_name = registry.get_by_name("EXAMPLE.com.").unwrap();
        assert_eq!(by_name, Some(created));
    }

    #[test]
    fn get_by_name_resolves_an_alias_to_its_domain() {
        let registry = registry();
        let created = registry
            .create("mail.example.com", vec!["alt.example.com".to_string()])
            .unwrap();

        let by_alias = registry.get_by_name("ALT.example.com.").unwrap();
        assert_eq!(by_alias, Some(created));
        assert_eq!(registry.get_by_name("other.example").unwrap(), None);
    }

    #[test]
    fn canonical_name_maps_an_alias_to_the_primary() {
        let registry = registry();
        registry
            .create("mail.example.com", vec!["alt.example.com".to_string()])
            .unwrap();

        assert_eq!(
            registry.canonical_name("alt.example.com").unwrap(),
            Some("mail.example.com".to_string())
        );
        assert_eq!(
            registry.canonical_name("mail.example.com").unwrap(),
            Some("mail.example.com".to_string())
        );
        assert_eq!(registry.canonical_name("other.example").unwrap(), None);
    }

    #[test]
    fn create_normalizes_the_name_and_its_aliases() {
        let registry = registry();
        let created = registry
            .create(
                "  Mail.Example.Com.  ",
                vec!["ALT.example.com.".to_string(), "  ".to_string()],
            )
            .unwrap();
        assert_eq!(created.name, "mail.example.com");
        assert_eq!(created.aliases, vec!["alt.example.com".to_string()]);
    }

    #[test]
    fn create_rejects_an_empty_name() {
        let registry = registry();
        let err = registry.create("  .  ", Vec::new()).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn create_rejects_a_duplicate_name_regardless_of_case() {
        let registry = registry();
        registry.create("example.com", Vec::new()).unwrap();
        let err = registry.create("Example.Com", Vec::new()).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn get_reports_not_found_for_an_unknown_id() {
        let registry = registry();
        let err = registry.get(999).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn get_by_name_returns_none_for_an_unknown_name() {
        let registry = registry();
        assert_eq!(registry.get_by_name("absent.example").unwrap(), None);
    }

    #[test]
    fn list_returns_every_domain_oldest_first() {
        let registry = registry();
        let first = registry.create("a.example", Vec::new()).unwrap();
        let second = registry.create("b.example", Vec::new()).unwrap();
        let third = registry.create("c.example", Vec::new()).unwrap();

        let listed = registry.list().unwrap();
        let ids: Vec<u64> = listed.iter().map(|domain| domain.id).collect();
        assert_eq!(ids, vec![first.id, second.id, third.id]);
    }

    #[test]
    fn list_is_empty_on_a_fresh_registry() {
        let registry = registry();
        assert!(registry.list().unwrap().is_empty());
    }

    #[test]
    fn update_persists_changes_to_an_existing_domain() {
        let registry = registry();
        let mut domain = registry.create("example.com", Vec::new()).unwrap();
        domain.enabled = false;
        domain.catch_all_account_id = Some(7);
        domain.dns_status = DnsStatus::Verified {
            checked_at: 1_700_000_500_000,
        };
        registry.update(domain.clone()).unwrap();

        let reread = registry.get(domain.id).unwrap();
        assert!(!reread.enabled);
        assert_eq!(reread.catch_all_account_id, Some(7));
        assert!(reread.dns_status.is_verified());
    }

    #[test]
    fn update_moves_the_name_index_when_the_name_changes() {
        let registry = registry();
        let mut domain = registry.create("old.example", Vec::new()).unwrap();
        domain.name = "New.Example".to_string();
        registry.update(domain.clone()).unwrap();

        assert_eq!(registry.get_by_name("old.example").unwrap(), None);
        let moved = registry.get_by_name("new.example").unwrap().unwrap();
        assert_eq!(moved.id, domain.id);
        assert_eq!(moved.name, "new.example");
    }

    #[test]
    fn update_rejects_renaming_onto_another_domains_name() {
        let registry = registry();
        registry.create("taken.example", Vec::new()).unwrap();
        let mut other = registry.create("free.example", Vec::new()).unwrap();
        other.name = "taken.example".to_string();
        let err = registry.update(other).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn update_reports_not_found_for_a_missing_domain() {
        let registry = registry();
        let phantom = Domain {
            id: 4242,
            name: "ghost.example".to_string(),
            aliases: Vec::new(),
            enabled: true,
            catch_all_account_id: None,
            dkim_key_ids: Vec::new(),
            dns_status: DnsStatus::Unverified,
            created_at: 1,
        };
        let err = registry.update(phantom).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn delete_removes_the_record_and_frees_the_name() {
        let registry = registry();
        let domain = registry.create("example.com", Vec::new()).unwrap();
        registry.delete(domain.id).unwrap();

        assert!(matches!(
            registry.get(domain.id).unwrap_err(),
            Error::NotFound(_)
        ));
        assert_eq!(registry.get_by_name("example.com").unwrap(), None);
        registry.create("example.com", Vec::new()).unwrap();
    }

    #[test]
    fn delete_reports_not_found_for_an_unknown_id() {
        let registry = registry();
        let err = registry.delete(123).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }
}
