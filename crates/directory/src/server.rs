use std::sync::Arc;

use irixmail_core::{IdGenerator, Result, Server};
use irixmail_store::{RocksdbStore, Store, TtlStore};

use crate::account_registry::AccountRegistry;
use crate::address_index::AddressIndex;
use crate::api_key_registry::ApiKeyRegistry;
use crate::credential_registry::CredentialRegistry;
use crate::dkim_registry::DkimKeyRegistry;
use crate::domain_registry::DomainRegistry;
use crate::ip_rules::IpRuleRegistry;
use crate::recovery_admin::RecoveryAdmin;
use crate::throttle::{Throttle, ThrottlePolicy};

#[derive(Clone)]
pub struct Directory {
    store: Arc<dyn Store>,
    domains: DomainRegistry,
    accounts: AccountRegistry,
    credentials: CredentialRegistry,
    api_keys: ApiKeyRegistry,
    dkim: DkimKeyRegistry,
    addresses: AddressIndex,
    throttle: Throttle,
    recovery_admin: Option<RecoveryAdmin>,
    ip_rules: IpRuleRegistry,
    ids: Arc<IdGenerator>,
}

impl Directory {
    pub fn new(
        store: Arc<dyn Store>,
        ids: Arc<IdGenerator>,
        recovery_admin: Option<RecoveryAdmin>,
    ) -> Self {
        let throttle = Throttle::new(Arc::new(TtlStore::new()), ThrottlePolicy::default());
        let addresses = AddressIndex::new(Arc::clone(&store));
        let domains = DomainRegistry::new(Arc::clone(&store), Arc::clone(&ids))
            .with_catch_all_index(addresses.clone())
            .with_account_maintenance(AccountRegistry::new(Arc::clone(&store), Arc::clone(&ids)));
        let accounts = AccountRegistry::new(Arc::clone(&store), Arc::clone(&ids))
            .with_address_maintenance(domains.clone(), addresses.clone());
        Self {
            domains,
            accounts,
            credentials: CredentialRegistry::new(Arc::clone(&store), Arc::clone(&ids)),
            api_keys: ApiKeyRegistry::new(Arc::clone(&store), Arc::clone(&ids)),
            dkim: DkimKeyRegistry::new(Arc::clone(&store)),
            ip_rules: IpRuleRegistry::new(Arc::clone(&store), Arc::clone(&ids)),
            ids,
            addresses,
            throttle,
            recovery_admin,
            store,
        }
    }

    pub fn from_server(server: &Server) -> Result<Self> {
        let store: Arc<dyn Store> = server.storage().store::<RocksdbStore>()?;
        let ids = server.ids_handle();
        let recovery_admin = RecoveryAdmin::from_env()?;
        Ok(Self::new(store, ids, recovery_admin))
    }

    pub fn store(&self) -> Arc<dyn Store> {
        Arc::clone(&self.store)
    }

    pub fn ids(&self) -> &IdGenerator {
        &self.ids
    }

    pub fn domains(&self) -> &DomainRegistry {
        &self.domains
    }

    pub fn accounts(&self) -> &AccountRegistry {
        &self.accounts
    }

    pub fn credentials(&self) -> &CredentialRegistry {
        &self.credentials
    }

    pub fn api_keys(&self) -> &ApiKeyRegistry {
        &self.api_keys
    }

    pub fn dkim(&self) -> &DkimKeyRegistry {
        &self.dkim
    }

    pub fn addresses(&self) -> &AddressIndex {
        &self.addresses
    }

    pub fn throttle(&self) -> &Throttle {
        &self.throttle
    }

    pub fn recovery_admin(&self) -> Option<&RecoveryAdmin> {
        self.recovery_admin.as_ref()
    }

    pub fn ip_rules(&self) -> &IpRuleRegistry {
        &self.ip_rules
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use irixmail_store::{Flow, KeyPrefix, WriteOp};

    use crate::account::Role;

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
                        unreachable!("the directory bundle does not use counters")
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the directory bundle does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the directory bundle does not use counters")
        }
    }

    fn directory() -> Directory {
        Directory::new(
            Arc::new(MemStore::default()),
            Arc::new(IdGenerator::new(0)),
            None,
        )
    }

    #[test]
    fn creating_an_account_indexes_its_address_for_delivery_resolution() {
        let directory = directory();
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();

        let resolved = directory.addresses().resolve("alice@example.com").unwrap();
        assert_eq!(
            resolved.and_then(|entry| entry.account_id()),
            Some(account.id)
        );
    }

    #[test]
    fn updating_account_aliases_reindexes_them_for_resolution() {
        let directory = directory();
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let mut account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();

        account.aliases = vec!["a.adams@example.com".to_string()];
        directory.accounts().update(account.clone()).unwrap();
        assert_eq!(
            directory
                .addresses()
                .resolve("a.adams@example.com")
                .unwrap()
                .and_then(|entry| entry.account_id()),
            Some(account.id)
        );

        account.aliases.clear();
        directory.accounts().update(account.clone()).unwrap();
        assert!(directory
            .addresses()
            .resolve("a.adams@example.com")
            .unwrap()
            .is_none());
        assert_eq!(
            directory
                .addresses()
                .resolve("alice@example.com")
                .unwrap()
                .and_then(|entry| entry.account_id()),
            Some(account.id)
        );
    }

    #[test]
    fn deleting_an_account_removes_its_address_from_the_index() {
        let directory = directory();
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        assert!(directory
            .addresses()
            .resolve("alice@example.com")
            .unwrap()
            .is_some());

        directory.accounts().delete(account.id).unwrap();
        assert!(directory
            .addresses()
            .resolve("alice@example.com")
            .unwrap()
            .is_none());
    }

    #[test]
    fn setting_and_clearing_a_domain_catch_all_indexes_it() {
        let directory = directory();
        let mut domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();

        domain.catch_all_account_id = Some(account.id);
        directory.domains().update(domain.clone()).unwrap();
        assert_eq!(
            directory
                .addresses()
                .catch_all("example.com")
                .unwrap()
                .and_then(|entry| entry.account_id()),
            Some(account.id)
        );

        domain.catch_all_account_id = None;
        directory.domains().update(domain.clone()).unwrap();
        assert!(directory
            .addresses()
            .catch_all("example.com")
            .unwrap()
            .is_none());
    }

    #[test]
    fn updating_with_a_blank_alias_succeeds_and_drops_it() {
        let directory = directory();
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let mut account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();

        account.aliases = vec!["  ".to_string(), "a.adams@example.com".to_string()];
        directory.accounts().update(account.clone()).unwrap();

        let reread = directory.accounts().get(account.id).unwrap();
        assert_eq!(reread.aliases, vec!["a.adams@example.com".to_string()]);
        assert!(directory
            .addresses()
            .resolve("a.adams@example.com")
            .unwrap()
            .is_some());
    }

    #[test]
    fn the_services_share_the_one_store() {
        let directory = directory();
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();

        let listed = directory.domains().list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "example.com");
    }

    #[test]
    fn the_registries_share_the_one_id_generator() {
        let directory = directory();
        let first = directory
            .domains()
            .create("one.example", Vec::new())
            .unwrap();
        let second = directory
            .domains()
            .create("two.example", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("bob", first.id, "Bob", Role::User)
            .unwrap();

        assert_ne!(first.id, second.id);
        assert_ne!(first.id, account.id);
        assert_ne!(second.id, account.id);
    }

    #[test]
    fn the_credential_registry_shares_the_one_store() {
        let directory = directory();
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        directory
            .credentials()
            .set_primary_password(account.id, "$argon2id$primary")
            .unwrap();
        assert_eq!(directory.credentials().list(account.id).unwrap().len(), 1);
    }

    #[test]
    fn the_throttle_is_ready_to_record_failures() {
        let directory = directory();
        assert!(!directory.throttle().is_locked(Some("203.0.113.7"), None));
        assert_eq!(
            directory.throttle().policy().max_failures,
            ThrottlePolicy::default().max_failures
        );
    }

    #[test]
    fn no_recovery_admin_is_configured_by_default() {
        let directory = directory();
        assert!(directory.recovery_admin().is_none());
    }

    #[test]
    fn a_configured_recovery_admin_is_carried_through() {
        let admin = RecoveryAdmin::parse("root:$argon2id$v=19$m=1,t=1,p=1$c2FsdHNhbHQ$aGFzaA")
            .expect("a well-formed recovery-admin value parses");
        let directory = Directory::new(
            Arc::new(MemStore::default()),
            Arc::new(IdGenerator::new(0)),
            Some(admin),
        );
        assert!(directory.recovery_admin().is_some());
    }
}
