use irixmail_core::Result;
use irixmail_directory::{AccountRegistry, AddressIndex, DomainRegistry, Target};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Local {
        account_id: u64,
        via_catch_all: bool,
    },
    Forward {
        destination: String,
    },
    Rejected,
    Unknown,
}

impl Resolution {
    pub fn is_local(&self) -> bool {
        matches!(self, Resolution::Local { .. })
    }

    pub fn is_accepted(&self) -> bool {
        matches!(self, Resolution::Local { .. } | Resolution::Forward { .. })
    }

    pub fn account_id(&self) -> Option<u64> {
        match self {
            Resolution::Local { account_id, .. } => Some(*account_id),
            Resolution::Forward { .. } | Resolution::Rejected | Resolution::Unknown => None,
        }
    }
}

pub fn resolve(
    index: &AddressIndex,
    domains: &DomainRegistry,
    accounts: &AccountRegistry,
    recipient: &str,
) -> Result<Resolution> {
    let recipient = canonical_recipient(domains, recipient);
    let recipient = recipient.as_str();
    if let Some(entry) = index.resolve(recipient)? {
        return Ok(match entry.target {
            Target::Account { account_id } => Resolution::Local {
                account_id,
                via_catch_all: false,
            },
            Target::Forward { destination } => Resolution::Forward { destination },
            Target::Reject => Resolution::Rejected,
        });
    }

    if let Some(domain) = domain_of(recipient) {
        if let Some(entry) = index.catch_all(domain)? {
            if let Target::Account { account_id } = entry.target {
                return Ok(Resolution::Local {
                    account_id,
                    via_catch_all: true,
                });
            }
        }
    }

    if let Some(account_id) = role_fallback(accounts, domains, recipient)? {
        return Ok(Resolution::Local {
            account_id,
            via_catch_all: false,
        });
    }

    Ok(Resolution::Unknown)
}

fn role_fallback(
    accounts: &AccountRegistry,
    domains: &DomainRegistry,
    recipient: &str,
) -> Result<Option<u64>> {
    let (local, domain) = match recipient.rsplit_once('@') {
        Some((local, domain)) => (local, Some(domain)),
        None => (recipient, None),
    };
    let is_postmaster = local.eq_ignore_ascii_case("postmaster");
    let is_role = is_postmaster || local.eq_ignore_ascii_case("tlsrpt");
    if let Some(domain) = domain {
        if !is_role {
            return Ok(None);
        }
        let Some(domain) = domains.get_by_name(domain)? else {
            return Ok(None);
        };
        if let Some(admin) = admin_of(accounts.list_for_domain(domain.id)?) {
            return Ok(Some(admin));
        }
    } else if !is_postmaster {
        return Ok(None);
    }
    Ok(admin_of(accounts.list()?))
}

fn admin_of(accounts: Vec<irixmail_directory::Account>) -> Option<u64> {
    accounts
        .into_iter()
        .filter(|account| account.is_admin() && account.enabled)
        .map(|account| account.id)
        .min()
}

fn canonical_recipient(domains: &DomainRegistry, recipient: &str) -> String {
    let trimmed = recipient.trim();
    let Some((local, domain)) = trimmed.rsplit_once('@') else {
        return trimmed.to_string();
    };
    if domain.trim().is_empty() {
        return trimmed.to_string();
    }
    match domains.canonical_name(domain) {
        Ok(Some(canonical)) if !canonical.eq_ignore_ascii_case(domain) => {
            format!("{local}@{canonical}")
        }
        _ => trimmed.to_string(),
    }
}

fn domain_of(address: &str) -> Option<&str> {
    match address.rsplit_once('@') {
        Some((_, domain)) if !domain.trim().is_empty() => Some(domain),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_core::Error;
    use irixmail_directory::AddressEntry;
    use irixmail_store::{Flow, KeyPrefix, Store, WriteOp};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

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
                    WriteOp::Add { .. } => unreachable!("resolution does not use counters"),
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("resolution does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("resolution does not use counters")
        }
    }

    fn index() -> AddressIndex {
        AddressIndex::new(Arc::new(MemStore::default()))
    }

    fn domains() -> DomainRegistry {
        DomainRegistry::new(
            Arc::new(MemStore::default()),
            Arc::new(irixmail_core::IdGenerator::new(0)),
        )
    }

    fn accounts() -> AccountRegistry {
        AccountRegistry::new(
            Arc::new(MemStore::default()),
            Arc::new(irixmail_core::IdGenerator::new(0)),
        )
    }

    #[test]
    fn an_account_address_resolves_to_its_account() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        let resolution = resolve(&index, &domains(), &accounts(), "alice@irixsoft.com").unwrap();
        assert_eq!(
            resolution,
            Resolution::Local {
                account_id: 7,
                via_catch_all: false,
            }
        );
        assert!(resolution.is_local());
        assert!(resolution.is_accepted());
        assert_eq!(resolution.account_id(), Some(7));
    }

    #[test]
    fn an_alias_resolves_to_the_account_it_points_at() {
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
        let resolution = resolve(&index, &domains(), &accounts(), "a.adams@irixsoft.com").unwrap();
        assert_eq!(resolution.account_id(), Some(7));
        assert!(!matches!(
            resolution,
            Resolution::Local {
                via_catch_all: true,
                ..
            }
        ));
    }

    #[test]
    fn an_alias_domain_resolves_to_the_same_account_as_the_primary() {
        let index = index();
        let domains = domains();
        domains
            .create("irixsoft.com", vec!["irix.example".to_string()])
            .unwrap();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();

        let via_alias = resolve(&index, &domains, &accounts(), "alice@irix.example").unwrap();
        assert_eq!(via_alias.account_id(), Some(7));
        let via_primary = resolve(&index, &domains, &accounts(), "alice@irixsoft.com").unwrap();
        assert_eq!(via_alias, via_primary);
    }

    #[test]
    fn a_catch_all_on_the_primary_covers_the_alias_domain() {
        let index = index();
        let domains = domains();
        domains
            .create("irixsoft.com", vec!["irix.example".to_string()])
            .unwrap();
        index.set_catch_all("irixsoft.com", 9).unwrap();

        let resolution = resolve(&index, &domains, &accounts(), "anything@irix.example").unwrap();
        assert_eq!(
            resolution,
            Resolution::Local {
                account_id: 9,
                via_catch_all: true,
            }
        );
    }

    #[test]
    fn an_unrelated_domain_is_not_canonicalized_and_stays_unknown() {
        let index = index();
        let domains = domains();
        domains
            .create("irixsoft.com", vec!["irix.example".to_string()])
            .unwrap();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();

        let resolution = resolve(&index, &domains, &accounts(), "alice@elsewhere.example").unwrap();
        assert_eq!(resolution, Resolution::Unknown);
    }

    #[test]
    fn a_recipient_resolves_regardless_of_casing_or_whitespace() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        let resolution =
            resolve(&index, &domains(), &accounts(), "  ALICE@IriXSoft.CoM  ").unwrap();
        assert_eq!(resolution.account_id(), Some(7));
    }

    #[test]
    fn a_forward_address_resolves_to_its_external_destination() {
        let index = index();
        index
            .set(AddressEntry::forward(
                "info@irixsoft.com",
                "owner@example.org",
            ))
            .unwrap();
        let resolution = resolve(&index, &domains(), &accounts(), "info@irixsoft.com").unwrap();
        assert_eq!(
            resolution,
            Resolution::Forward {
                destination: "owner@example.org".to_string(),
            }
        );
        assert!(!resolution.is_local());
        assert!(resolution.is_accepted());
        assert_eq!(resolution.account_id(), None);
    }

    #[test]
    fn a_rejected_address_is_declined_but_known() {
        let index = index();
        index
            .set(AddressEntry::reject("blocked@irixsoft.com"))
            .unwrap();
        let resolution = resolve(&index, &domains(), &accounts(), "blocked@irixsoft.com").unwrap();
        assert_eq!(resolution, Resolution::Rejected);
        assert!(!resolution.is_accepted());
        assert_eq!(resolution.account_id(), None);
    }

    #[test]
    fn an_unknown_address_resolves_to_unknown() {
        let index = index();
        let resolution = resolve(&index, &domains(), &accounts(), "absent@irixsoft.com").unwrap();
        assert_eq!(resolution, Resolution::Unknown);
        assert!(!resolution.is_accepted());
    }

    #[test]
    fn an_unmatched_recipient_falls_back_to_the_domain_catch_all() {
        let index = index();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        let resolution = resolve(&index, &domains(), &accounts(), "anything@irixsoft.com").unwrap();
        assert_eq!(
            resolution,
            Resolution::Local {
                account_id: 9,
                via_catch_all: true,
            }
        );
        assert!(resolution.is_local());
        assert_eq!(resolution.account_id(), Some(9));
    }

    #[test]
    fn an_exact_entry_wins_over_the_catch_all() {
        let index = index();
        index
            .set(AddressEntry::account("alice@irixsoft.com", 7))
            .unwrap();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        let resolution = resolve(&index, &domains(), &accounts(), "alice@irixsoft.com").unwrap();
        assert_eq!(
            resolution,
            Resolution::Local {
                account_id: 7,
                via_catch_all: false,
            }
        );
    }

    #[test]
    fn an_exact_rejection_is_not_overridden_by_a_catch_all() {
        let index = index();
        index
            .set(AddressEntry::reject("blocked@irixsoft.com"))
            .unwrap();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        let resolution = resolve(&index, &domains(), &accounts(), "blocked@irixsoft.com").unwrap();
        assert_eq!(resolution, Resolution::Rejected);
    }

    #[test]
    fn the_catch_all_is_scoped_to_its_own_domain() {
        let index = index();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        let resolution = resolve(&index, &domains(), &accounts(), "anything@example.org").unwrap();
        assert_eq!(resolution, Resolution::Unknown);
    }

    #[test]
    fn an_address_without_a_domain_does_not_consult_a_catch_all() {
        let index = index();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        assert_eq!(
            resolve(&index, &domains(), &accounts(), "postmaster").unwrap(),
            Resolution::Unknown
        );
        assert_eq!(
            resolve(&index, &domains(), &accounts(), "trailing@").unwrap(),
            Resolution::Unknown
        );
    }

    #[test]
    fn domain_of_returns_the_part_after_the_last_at() {
        assert_eq!(domain_of("alice@irixsoft.com"), Some("irixsoft.com"));
        assert_eq!(domain_of("user@sub@irixsoft.com"), Some("irixsoft.com"));
        assert_eq!(domain_of("postmaster"), None);
        assert_eq!(domain_of("trailing@"), None);
        assert_eq!(domain_of("trailing@   "), None);
    }

    #[test]
    fn a_store_error_surfaces_rather_than_resolving() {
        struct FailingStore;
        impl Store for FailingStore {
            fn get(&self, _key: &[u8]) -> Result<Option<Vec<u8>>> {
                Err(Error::internal("store unavailable"))
            }
            fn put(&self, _key: &[u8], _value: &[u8]) -> Result<()> {
                unreachable!()
            }
            fn delete(&self, _key: &[u8]) -> Result<()> {
                unreachable!()
            }
            fn iterate(
                &self,
                _prefix: &KeyPrefix,
                _visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
            ) -> Result<()> {
                unreachable!()
            }
            fn batch(&self, _ops: &[WriteOp]) -> Result<()> {
                unreachable!()
            }
            fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
                unreachable!()
            }
            fn counter(&self, _key: &[u8]) -> Result<i64> {
                unreachable!()
            }
        }
        let index = AddressIndex::new(Arc::new(FailingStore));
        assert!(resolve(&index, &domains(), &accounts(), "alice@irixsoft.com").is_err());
    }

    use irixmail_directory::Role;

    #[test]
    fn postmaster_falls_back_to_the_domain_admin() {
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        let admin = accounts.create("boss", domain.id, "", Role::Admin).unwrap();
        let resolution = resolve(&index(), &domains, &accounts, "PostMaster@irixsoft.com").unwrap();
        assert_eq!(
            resolution,
            Resolution::Local {
                account_id: admin.id,
                via_catch_all: false,
            }
        );
    }

    #[test]
    fn tlsrpt_falls_back_to_the_domain_admin() {
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        let admin = accounts.create("boss", domain.id, "", Role::Admin).unwrap();
        let resolution = resolve(&index(), &domains, &accounts, "tlsrpt@irixsoft.com").unwrap();
        assert_eq!(resolution.account_id(), Some(admin.id));
    }

    #[test]
    fn an_explicit_postmaster_entry_wins_over_the_fallback() {
        let index = index();
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        accounts.create("boss", domain.id, "", Role::Admin).unwrap();
        index
            .set(AddressEntry::forward(
                "postmaster@irixsoft.com",
                "ops@example.org",
            ))
            .unwrap();
        let resolution = resolve(&index, &domains, &accounts, "postmaster@irixsoft.com").unwrap();
        assert_eq!(
            resolution,
            Resolution::Forward {
                destination: "ops@example.org".to_string(),
            }
        );
    }

    #[test]
    fn a_catch_all_wins_over_the_postmaster_fallback() {
        let index = index();
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        accounts.create("boss", domain.id, "", Role::Admin).unwrap();
        index.set_catch_all("irixsoft.com", 9).unwrap();
        let resolution = resolve(&index, &domains, &accounts, "postmaster@irixsoft.com").unwrap();
        assert_eq!(
            resolution,
            Resolution::Local {
                account_id: 9,
                via_catch_all: true,
            }
        );
    }

    #[test]
    fn a_secondary_domain_falls_back_to_the_global_admin() {
        let domains = domains();
        let accounts = accounts();
        let primary = domains.create("irixsoft.com", Vec::new()).unwrap();
        let secondary = domains.create("other.example", Vec::new()).unwrap();
        let admin = accounts
            .create("boss", primary.id, "", Role::Admin)
            .unwrap();
        accounts
            .create("bob", secondary.id, "", Role::User)
            .unwrap();
        let resolution =
            resolve(&index(), &domains, &accounts, "postmaster@other.example").unwrap();
        assert_eq!(resolution.account_id(), Some(admin.id));
    }

    #[test]
    fn the_fallback_needs_an_admin_account() {
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        accounts.create("bob", domain.id, "", Role::User).unwrap();
        let resolution = resolve(&index(), &domains, &accounts, "postmaster@irixsoft.com").unwrap();
        assert_eq!(resolution, Resolution::Unknown);
    }

    #[test]
    fn a_disabled_admin_is_not_a_fallback_target() {
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        let mut admin = accounts.create("boss", domain.id, "", Role::Admin).unwrap();
        admin.enabled = false;
        accounts.update(admin).unwrap();
        let resolution = resolve(&index(), &domains, &accounts, "postmaster@irixsoft.com").unwrap();
        assert_eq!(resolution, Resolution::Unknown);
    }

    #[test]
    fn a_bare_postmaster_routes_to_the_global_admin() {
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        let admin = accounts.create("boss", domain.id, "", Role::Admin).unwrap();
        let resolution = resolve(&index(), &domains, &accounts, "postmaster").unwrap();
        assert_eq!(resolution.account_id(), Some(admin.id));
    }

    #[test]
    fn an_unhosted_domain_gets_no_postmaster_fallback() {
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        accounts.create("boss", domain.id, "", Role::Admin).unwrap();
        let resolution = resolve(
            &index(),
            &domains,
            &accounts,
            "postmaster@elsewhere.example",
        )
        .unwrap();
        assert_eq!(resolution, Resolution::Unknown);
    }

    #[test]
    fn only_role_local_parts_reach_the_fallback() {
        let domains = domains();
        let accounts = accounts();
        let domain = domains.create("irixsoft.com", Vec::new()).unwrap();
        accounts.create("boss", domain.id, "", Role::Admin).unwrap();
        let resolution = resolve(&index(), &domains, &accounts, "ghost@irixsoft.com").unwrap();
        assert_eq!(resolution, Resolution::Unknown);
    }
}
