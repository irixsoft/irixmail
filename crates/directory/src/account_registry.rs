use std::sync::Arc;

use irixmail_core::{Error, IdGenerator, Result};
use irixmail_store::{Flow, KeyPrefix, Store, Subspace, WriteOp};

use crate::account::{Account, Forwarding, Role, VacationResponder};
use crate::address_index::AddressIndex;
use crate::domain_registry::DomainRegistry;

const TAG_ACCOUNT_RECORD: u8 = 0x20;

const TAG_ACCOUNT_ADDRESS: u8 = 0x21;

#[derive(Clone)]
struct AddressMaintenance {
    domains: DomainRegistry,
    addresses: AddressIndex,
}

impl AddressMaintenance {
    fn addresses_of(&self, account: &Account) -> Vec<String> {
        let mut addresses: Vec<String> = account
            .aliases
            .iter()
            .filter(|alias| !alias.trim().is_empty())
            .cloned()
            .collect();
        if let Ok(domain) = self.domains.get(account.domain_id) {
            addresses.push(format!("{}@{}", account.local_part, domain.name));
        }
        addresses
    }
}

#[derive(Clone)]
pub struct AccountRegistry {
    store: Arc<dyn Store>,
    ids: Arc<IdGenerator>,
    maintenance: Option<AddressMaintenance>,
}

impl AccountRegistry {
    pub fn new(store: Arc<dyn Store>, ids: Arc<IdGenerator>) -> Self {
        Self {
            store,
            ids,
            maintenance: None,
        }
    }

    pub fn with_address_maintenance(
        mut self,
        domains: DomainRegistry,
        addresses: AddressIndex,
    ) -> Self {
        self.maintenance = Some(AddressMaintenance { domains, addresses });
        self
    }

    pub fn create(
        &self,
        local_part: &str,
        domain_id: u64,
        display_name: impl Into<String>,
        role: Role,
    ) -> Result<Account> {
        self.create_with_extra_ops(local_part, domain_id, display_name, role, |_, _| Vec::new())
    }

    pub fn create_with_extra_ops(
        &self,
        local_part: &str,
        domain_id: u64,
        display_name: impl Into<String>,
        role: Role,
        extra_ops: impl FnOnce(u64, u64) -> Vec<WriteOp>,
    ) -> Result<Account> {
        let local_part = normalize_local_part(local_part);
        if local_part.is_empty() {
            return Err(Error::invalid_input(
                "an account local part must not be empty",
            ));
        }
        if self.lookup_id_by_address(domain_id, &local_part)?.is_some() {
            return Err(Error::invalid_input(format!(
                "an account {local_part} already exists in domain {domain_id}"
            )));
        }

        let id = self.ids.generate();
        let created_at = IdGenerator::timestamp_of(id) * 1_000;
        let account = Account {
            id,
            local_part,
            domain_id,
            display_name: display_name.into(),
            enabled: true,
            role,
            aliases: Vec::new(),
            forwarding: Forwarding::default(),
            quota_bytes: 0,
            quota_messages: 0,
            locale: String::new(),
            timezone: String::new(),
            signature: String::new(),
            vacation: VacationResponder::default(),
            created_at,
        };

        let mut ops = vec![
            WriteOp::Set {
                key: record_key(account.id),
                value: encode(&account)?,
            },
            WriteOp::Set {
                key: address_key(account.domain_id, &account.local_part),
                value: account.id.to_be_bytes().to_vec(),
            },
        ];
        ops.extend(extra_ops(account.id, account.created_at));
        if let Some(maintenance) = &self.maintenance {
            ops.extend(maintenance.addresses.account_address_ops(
                account.id,
                &[],
                &maintenance.addresses_of(&account),
            )?);
        }
        self.store.batch(&ops)?;
        Ok(account)
    }

    pub fn get(&self, id: u64) -> Result<Account> {
        match self.store.get(&record_key(id))? {
            Some(bytes) => decode(&bytes),
            None => Err(Error::not_found(format!("account {id}"))),
        }
    }

    pub fn get_by_address(&self, local_part: &str, domain_id: u64) -> Result<Option<Account>> {
        match self.lookup_id_by_address(domain_id, &normalize_local_part(local_part))? {
            Some(id) => self.get(id).map(Some),
            None => Ok(None),
        }
    }

    pub fn list(&self) -> Result<Vec<Account>> {
        self.collect(|_| true)
    }

    pub fn list_for_domain(&self, domain_id: u64) -> Result<Vec<Account>> {
        self.collect(|account| account.domain_id == domain_id)
    }

    pub fn update(&self, mut account: Account) -> Result<()> {
        let existing = self.get(account.id)?;
        account.local_part = normalize_local_part(&account.local_part);
        if account.local_part.is_empty() {
            return Err(Error::invalid_input(
                "an account local part must not be empty",
            ));
        }
        account.aliases.retain(|alias| !alias.trim().is_empty());

        let address_changed =
            account.local_part != existing.local_part || account.domain_id != existing.domain_id;

        let mut ops = vec![WriteOp::Set {
            key: record_key(account.id),
            value: encode(&account)?,
        }];
        if address_changed {
            if let Some(other) =
                self.lookup_id_by_address(account.domain_id, &account.local_part)?
            {
                if other != account.id {
                    return Err(Error::invalid_input(format!(
                        "an account {} already exists in domain {}",
                        account.local_part, account.domain_id
                    )));
                }
            }
            ops.push(WriteOp::Delete {
                key: address_key(existing.domain_id, &existing.local_part),
            });
            ops.push(WriteOp::Set {
                key: address_key(account.domain_id, &account.local_part),
                value: account.id.to_be_bytes().to_vec(),
            });
        }
        if let Some(maintenance) = &self.maintenance {
            ops.extend(maintenance.addresses.account_address_ops(
                account.id,
                &maintenance.addresses_of(&existing),
                &maintenance.addresses_of(&account),
            )?);
        }
        self.store.batch(&ops)?;
        Ok(())
    }

    pub fn delete(&self, id: u64) -> Result<()> {
        let account = self.get(id)?;
        let mut ops = vec![
            WriteOp::Delete {
                key: record_key(id),
            },
            WriteOp::Delete {
                key: address_key(account.domain_id, &account.local_part),
            },
        ];
        if let Some(maintenance) = &self.maintenance {
            ops.extend(maintenance.addresses.account_address_ops(
                id,
                &maintenance.addresses_of(&account),
                &[],
            )?);
        }
        self.store.batch(&ops)?;
        Ok(())
    }

    fn collect(&self, keep: impl Fn(&Account) -> bool) -> Result<Vec<Account>> {
        let mut accounts = Vec::new();
        let mut scan_error: Option<Error> = None;
        self.store.iterate(
            &KeyPrefix::subspace(Subspace::Registry),
            &mut |key, value| {
                if !is_record_key(key) {
                    return Ok(Flow::Continue);
                }
                match decode(value) {
                    Ok(account) => {
                        if keep(&account) {
                            accounts.push(account);
                        }
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
        Ok(accounts)
    }

    fn lookup_id_by_address(&self, domain_id: u64, local_part: &str) -> Result<Option<u64>> {
        match self.store.get(&address_key(domain_id, local_part))? {
            Some(bytes) => {
                let array: [u8; 8] = bytes.as_slice().try_into().map_err(|_| {
                    Error::store(format!(
                        "account address index for {local_part} in domain {domain_id} is corrupt"
                    ))
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
    key.push(TAG_ACCOUNT_RECORD);
    key.extend_from_slice(&id.to_be_bytes());
    key
}

fn address_key(domain_id: u64, local_part: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + std::mem::size_of::<u64>() + local_part.len());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_ACCOUNT_ADDRESS);
    key.extend_from_slice(&domain_id.to_be_bytes());
    key.extend_from_slice(local_part.as_bytes());
    key
}

fn is_record_key(key: &[u8]) -> bool {
    key.len() == 2 + std::mem::size_of::<u64>()
        && key[0] == Subspace::Registry.as_byte()
        && key[1] == TAG_ACCOUNT_RECORD
}

fn encode(account: &Account) -> Result<Vec<u8>> {
    serde_json::to_vec(account)
        .map_err(|err| Error::serialize(format!("could not encode account: {err}")))
}

fn decode(bytes: &[u8]) -> Result<Account> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::serialize(format!("could not decode account: {err}")))
}

fn normalize_local_part(local_part: &str) -> String {
    local_part.trim().to_ascii_lowercase()
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
                        unreachable!("the account registry does not use counters");
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the account registry does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the account registry does not use counters")
        }
    }

    fn registry() -> AccountRegistry {
        AccountRegistry::new(Arc::new(MemStore::default()), Arc::new(IdGenerator::new(0)))
    }

    fn registry_with_maintenance() -> (AccountRegistry, DomainRegistry, AddressIndex) {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let ids = Arc::new(IdGenerator::new(0));
        let addresses = AddressIndex::new(Arc::clone(&store));
        let domains = DomainRegistry::new(Arc::clone(&store), Arc::clone(&ids));
        let accounts = AccountRegistry::new(store, ids)
            .with_address_maintenance(domains.clone(), addresses.clone());
        (accounts, domains, addresses)
    }

    #[test]
    fn an_alias_matching_another_accounts_address_is_refused() {
        let (accounts, domains, addresses) = registry_with_maintenance();
        let domain = domains.create("example.com", Vec::new()).unwrap();
        let alice = accounts.create("alice", domain.id, "", Role::User).unwrap();
        let mut bob = accounts.create("bob", domain.id, "", Role::User).unwrap();

        bob.aliases = vec!["Alice@Example.COM".to_string()];
        let err = accounts.update(bob).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(
            addresses
                .resolve("alice@example.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(alice.id)
        );
    }

    #[test]
    fn an_alias_matching_another_accounts_alias_is_refused() {
        let (accounts, domains, addresses) = registry_with_maintenance();
        let domain = domains.create("example.com", Vec::new()).unwrap();
        let mut alice = accounts.create("alice", domain.id, "", Role::User).unwrap();
        alice.aliases = vec!["sales@example.com".to_string()];
        accounts.update(alice.clone()).unwrap();

        let mut bob = accounts.create("bob", domain.id, "", Role::User).unwrap();
        bob.aliases = vec!["sales@example.com".to_string()];
        let err = accounts.update(bob).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(
            addresses
                .resolve("sales@example.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(alice.id)
        );
    }

    #[test]
    fn creating_an_account_over_another_accounts_alias_is_refused() {
        let (accounts, domains, addresses) = registry_with_maintenance();
        let domain = domains.create("example.com", Vec::new()).unwrap();
        let mut alice = accounts.create("alice", domain.id, "", Role::User).unwrap();
        alice.aliases = vec!["bob@example.com".to_string()];
        accounts.update(alice.clone()).unwrap();

        let err = accounts
            .create("bob", domain.id, "", Role::User)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(
            addresses
                .resolve("bob@example.com")
                .unwrap()
                .unwrap()
                .account_id(),
            Some(alice.id)
        );
    }

    #[test]
    fn an_account_keeps_its_own_alias_across_updates() {
        let (accounts, domains, _) = registry_with_maintenance();
        let domain = domains.create("example.com", Vec::new()).unwrap();
        let mut alice = accounts.create("alice", domain.id, "", Role::User).unwrap();
        alice.aliases = vec!["sales@example.com".to_string()];
        accounts.update(alice.clone()).unwrap();
        alice.display_name = "Alice".to_string();
        accounts.update(alice).unwrap();
    }

    #[test]
    fn create_with_extra_ops_folds_caller_ops_into_the_account_batch() {
        let store = Arc::new(MemStore::default());
        let registry = AccountRegistry::new(store.clone(), Arc::new(IdGenerator::new(0)));

        let account = registry
            .create_with_extra_ops("alice", 1, "Alice", Role::User, |id, created_at| {
                assert!(created_at > 0);
                vec![WriteOp::Set {
                    key: b"provision-marker".to_vec(),
                    value: id.to_be_bytes().to_vec(),
                }]
            })
            .unwrap();

        assert_eq!(
            store.get(b"provision-marker").unwrap(),
            Some(account.id.to_be_bytes().to_vec())
        );
        assert_eq!(registry.get(account.id).unwrap(), account);
    }

    #[test]
    fn a_created_account_is_readable_by_id_and_by_address() {
        let registry = registry();
        let created = registry
            .create("Alice", 42, "Alice Adams", Role::User)
            .unwrap();

        assert_eq!(created.local_part, "alice");
        assert_eq!(created.domain_id, 42);
        assert_eq!(created.display_name, "Alice Adams");
        assert!(created.enabled);
        assert_eq!(created.role, Role::User);
        assert_eq!(created.quota_bytes, 0);
        assert!(created.aliases.is_empty());
        assert_ne!(created.id, 0);
        assert!(created.created_at > 0);

        let by_id = registry.get(created.id).unwrap();
        assert_eq!(by_id, created);

        let by_address = registry.get_by_address("ALICE", 42).unwrap();
        assert_eq!(by_address, Some(created));
    }

    #[test]
    fn create_normalizes_the_local_part() {
        let registry = registry();
        let created = registry
            .create("  Bob.Smith  ", 1, "Bob", Role::Admin)
            .unwrap();
        assert_eq!(created.local_part, "bob.smith");
        assert_eq!(created.role, Role::Admin);
    }

    #[test]
    fn create_rejects_an_empty_local_part() {
        let registry = registry();
        let err = registry.create("   ", 1, "", Role::User).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn create_rejects_a_duplicate_address_regardless_of_case() {
        let registry = registry();
        registry.create("alice", 42, "", Role::User).unwrap();
        let err = registry.create("Alice", 42, "", Role::User).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn the_same_local_part_is_free_in_a_different_domain() {
        let registry = registry();
        let first = registry.create("alice", 1, "", Role::User).unwrap();
        let second = registry.create("alice", 2, "", Role::User).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(
            registry.get_by_address("alice", 1).unwrap().map(|a| a.id),
            Some(first.id)
        );
        assert_eq!(
            registry.get_by_address("alice", 2).unwrap().map(|a| a.id),
            Some(second.id)
        );
    }

    #[test]
    fn get_reports_not_found_for_an_unknown_id() {
        let registry = registry();
        let err = registry.get(999).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn get_by_address_returns_none_for_an_unknown_address() {
        let registry = registry();
        assert_eq!(registry.get_by_address("absent", 1).unwrap(), None);
    }

    #[test]
    fn list_returns_every_account_oldest_first() {
        let registry = registry();
        let first = registry.create("a", 1, "", Role::User).unwrap();
        let second = registry.create("b", 1, "", Role::User).unwrap();
        let third = registry.create("c", 2, "", Role::User).unwrap();

        let listed = registry.list().unwrap();
        let ids: Vec<u64> = listed.iter().map(|account| account.id).collect();
        assert_eq!(ids, vec![first.id, second.id, third.id]);
    }

    #[test]
    fn list_is_empty_on_a_fresh_registry() {
        let registry = registry();
        assert!(registry.list().unwrap().is_empty());
    }

    #[test]
    fn list_for_domain_returns_only_that_domains_accounts() {
        let registry = registry();
        let a = registry.create("a", 1, "", Role::User).unwrap();
        registry.create("b", 2, "", Role::User).unwrap();
        let c = registry.create("c", 1, "", Role::User).unwrap();

        let listed = registry.list_for_domain(1).unwrap();
        let ids: Vec<u64> = listed.iter().map(|account| account.id).collect();
        assert_eq!(ids, vec![a.id, c.id]);
        assert!(registry.list_for_domain(3).unwrap().is_empty());
    }

    #[test]
    fn update_persists_changes_to_an_existing_account() {
        let registry = registry();
        let mut account = registry.create("alice", 42, "Alice", Role::User).unwrap();
        account.enabled = false;
        account.role = Role::Admin;
        account.display_name = "Alice A.".to_string();
        account.quota_bytes = 1024;
        account.aliases = vec!["a.adams@irixsoft.com".to_string()];
        registry.update(account.clone()).unwrap();

        let reread = registry.get(account.id).unwrap();
        assert!(!reread.enabled);
        assert!(reread.is_admin());
        assert_eq!(reread.display_name, "Alice A.");
        assert_eq!(reread.quota_bytes, 1024);
        assert_eq!(reread.aliases, vec!["a.adams@irixsoft.com".to_string()]);
    }

    #[test]
    fn update_moves_the_address_index_when_the_local_part_changes() {
        let registry = registry();
        let mut account = registry.create("old", 42, "", Role::User).unwrap();
        account.local_part = "New".to_string();
        registry.update(account.clone()).unwrap();

        assert_eq!(registry.get_by_address("old", 42).unwrap(), None);
        let moved = registry.get_by_address("new", 42).unwrap().unwrap();
        assert_eq!(moved.id, account.id);
        assert_eq!(moved.local_part, "new");
    }

    #[test]
    fn update_moves_the_address_index_when_the_domain_changes() {
        let registry = registry();
        let mut account = registry.create("alice", 1, "", Role::User).unwrap();
        account.domain_id = 2;
        registry.update(account.clone()).unwrap();

        assert_eq!(registry.get_by_address("alice", 1).unwrap(), None);
        let moved = registry.get_by_address("alice", 2).unwrap().unwrap();
        assert_eq!(moved.id, account.id);
    }

    #[test]
    fn update_rejects_moving_onto_another_accounts_address() {
        let registry = registry();
        registry.create("taken", 1, "", Role::User).unwrap();
        let mut other = registry.create("free", 1, "", Role::User).unwrap();
        other.local_part = "taken".to_string();
        let err = registry.update(other).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn update_reports_not_found_for_a_missing_account() {
        let registry = registry();
        let phantom = Account {
            id: 4242,
            local_part: "ghost".to_string(),
            domain_id: 1,
            display_name: String::new(),
            enabled: true,
            role: Role::User,
            aliases: Vec::new(),
            forwarding: Forwarding::default(),
            quota_bytes: 0,
            quota_messages: 0,
            locale: String::new(),
            timezone: String::new(),
            signature: String::new(),
            vacation: VacationResponder::default(),
            created_at: 1,
        };
        let err = registry.update(phantom).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn delete_removes_the_record_and_frees_the_address() {
        let registry = registry();
        let account = registry.create("alice", 42, "", Role::User).unwrap();
        registry.delete(account.id).unwrap();

        assert!(matches!(
            registry.get(account.id).unwrap_err(),
            Error::NotFound(_)
        ));
        assert_eq!(registry.get_by_address("alice", 42).unwrap(), None);
        registry.create("alice", 42, "", Role::User).unwrap();
    }

    #[test]
    fn delete_reports_not_found_for_an_unknown_id() {
        let registry = registry();
        let err = registry.delete(123).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }
}
