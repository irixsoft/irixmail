use std::net::IpAddr;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use irixmail_core::{Error, IdGenerator, Result};
use irixmail_store::{Flow, KeyPrefix, Store, Subspace, WriteOp};

const TAG_IP_RULE: u8 = 0x31;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IpAction {
    Allow,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpRule {
    pub id: u64,
    pub cidr: String,
    pub action: IpAction,
}

#[derive(Clone, Copy)]
struct CompiledRule {
    network: IpAddr,
    prefix: u8,
    action: IpAction,
}

#[derive(Clone)]
pub struct IpRuleRegistry {
    store: Arc<dyn Store>,
    ids: Arc<IdGenerator>,
    cache: Arc<RwLock<Option<Arc<Vec<CompiledRule>>>>>,
}

impl IpRuleRegistry {
    pub fn new(store: Arc<dyn Store>, ids: Arc<IdGenerator>) -> Self {
        Self {
            store,
            ids,
            cache: Arc::new(RwLock::new(None)),
        }
    }

    pub fn create(&self, cidr: &str, action: IpAction) -> Result<IpRule> {
        parse_cidr(cidr)?;
        let rule = IpRule {
            id: self.ids.generate(),
            cidr: cidr.to_string(),
            action,
        };
        self.store.batch(&[WriteOp::Set {
            key: record_key(rule.id),
            value: encode(&rule)?,
        }])?;
        self.invalidate();
        Ok(rule)
    }

    pub fn list(&self) -> Result<Vec<IpRule>> {
        let mut rules = Vec::new();
        self.store.iterate(
            &KeyPrefix::subspace(Subspace::Registry),
            &mut |key, value| {
                if is_rule_key(key) {
                    rules.push(decode(value)?);
                }
                Ok(Flow::Continue)
            },
        )?;
        Ok(rules)
    }

    pub fn delete(&self, id: u64) -> Result<bool> {
        if self.store.get(&record_key(id))?.is_none() {
            return Ok(false);
        }
        self.store.batch(&[WriteOp::Delete {
            key: record_key(id),
        }])?;
        self.invalidate();
        Ok(true)
    }

    pub fn blocks(&self, ip: IpAddr) -> bool {
        matches!(self.decision(ip), Ok(Some(IpAction::Block)))
    }

    pub fn decision(&self, ip: IpAddr) -> Result<Option<IpAction>> {
        let ip = ip.to_canonical();
        let mut blocked = false;
        for rule in self.compiled()?.iter() {
            if !cidr_contains(rule.network, rule.prefix, ip) {
                continue;
            }
            match rule.action {
                IpAction::Allow => return Ok(Some(IpAction::Allow)),
                IpAction::Block => blocked = true,
            }
        }
        Ok(if blocked { Some(IpAction::Block) } else { None })
    }

    fn compiled(&self) -> Result<Arc<Vec<CompiledRule>>> {
        if let Some(rules) = self.cache.read().expect("ip rule cache poisoned").clone() {
            return Ok(rules);
        }
        let mut guard = self.cache.write().expect("ip rule cache poisoned");
        if let Some(rules) = guard.clone() {
            return Ok(rules);
        }
        let compiled = Arc::new(
            self.list()?
                .into_iter()
                .filter_map(|rule| {
                    let (network, prefix) = parse_cidr(&rule.cidr).ok()?;
                    Some(CompiledRule {
                        network,
                        prefix,
                        action: rule.action,
                    })
                })
                .collect::<Vec<_>>(),
        );
        *guard = Some(Arc::clone(&compiled));
        Ok(compiled)
    }

    fn invalidate(&self) {
        *self.cache.write().expect("ip rule cache poisoned") = None;
    }
}

pub fn parse_cidr(text: &str) -> Result<(IpAddr, u8)> {
    let (address, prefix) = match text.split_once('/') {
        Some((address, prefix)) => {
            let prefix: u8 = prefix
                .parse()
                .map_err(|_| Error::invalid_input(format!("invalid CIDR prefix in {text}")))?;
            (address, Some(prefix))
        }
        None => (text, None),
    };
    let network: IpAddr = address
        .parse()
        .map_err(|_| Error::invalid_input(format!("invalid IP address in {text}")))?;
    let width = match network {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    let prefix = prefix.unwrap_or(width);
    if prefix > width {
        return Err(Error::invalid_input(format!(
            "CIDR prefix /{prefix} exceeds /{width} in {text}"
        )));
    }
    Ok((network, prefix))
}

fn cidr_contains(network: IpAddr, prefix: u8, ip: IpAddr) -> bool {
    match (network, ip) {
        (IpAddr::V4(network), IpAddr::V4(ip)) => {
            masked(u32::from(network) as u128, prefix, 32)
                == masked(u32::from(ip) as u128, prefix, 32)
        }
        (IpAddr::V6(network), IpAddr::V6(ip)) => {
            masked(u128::from(network), prefix, 128) == masked(u128::from(ip), prefix, 128)
        }
        _ => false,
    }
}

fn masked(value: u128, prefix: u8, width: u8) -> u128 {
    if prefix == 0 {
        return 0;
    }
    value >> (width - prefix)
}

fn record_key(id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + std::mem::size_of::<u64>());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_IP_RULE);
    key.extend_from_slice(&id.to_be_bytes());
    key
}

fn is_rule_key(key: &[u8]) -> bool {
    key.len() > 2 && key[0] == Subspace::Registry.as_byte() && key[1] == TAG_IP_RULE
}

fn encode(rule: &IpRule) -> Result<Vec<u8>> {
    serde_json::to_vec(rule)
        .map_err(|err| Error::serialize(format!("could not encode IP rule: {err}")))
}

fn decode(bytes: &[u8]) -> Result<IpRule> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::serialize(format!("could not decode IP rule: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
        iterations: AtomicUsize,
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
            self.iterations.fetch_add(1, Ordering::SeqCst);
            let bound = prefix.encode();
            let map = self.map.lock().unwrap();
            for (key, value) in map.iter() {
                if key.starts_with(&bound) && visit(key, value)? == Flow::Stop {
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
                        unreachable!("the IP rule registry does not use counters")
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the IP rule registry does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the IP rule registry does not use counters")
        }
    }

    fn registry_over(store: &Arc<dyn Store>) -> IpRuleRegistry {
        IpRuleRegistry::new(Arc::clone(store), Arc::new(IdGenerator::new(0)))
    }

    #[test]
    fn rules_persist_across_a_fresh_registry_over_the_same_store() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let registry = registry_over(&store);
        let rule = registry.create("203.0.113.0/24", IpAction::Block).unwrap();

        let fresh = registry_over(&store);
        let rules = fresh.list().unwrap();
        assert_eq!(rules, vec![rule.clone()]);

        assert!(fresh.delete(rule.id).unwrap());
        assert!(fresh.list().unwrap().is_empty());
        assert!(!fresh.delete(rule.id).unwrap());
    }

    #[test]
    fn an_invalid_cidr_is_refused_at_creation() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let registry = registry_over(&store);
        assert!(registry.create("not-an-ip", IpAction::Block).is_err());
        assert!(registry.create("10.0.0.0/33", IpAction::Block).is_err());
        assert!(registry.create("::1/129", IpAction::Block).is_err());
    }

    #[test]
    fn a_block_rule_matches_addresses_inside_its_prefix() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let registry = registry_over(&store);
        registry.create("203.0.113.0/24", IpAction::Block).unwrap();

        let inside: IpAddr = "203.0.113.77".parse().unwrap();
        let outside: IpAddr = "203.0.114.1".parse().unwrap();
        assert_eq!(registry.decision(inside).unwrap(), Some(IpAction::Block));
        assert_eq!(registry.decision(outside).unwrap(), None);
    }

    #[test]
    fn an_allow_rule_overrides_a_covering_block() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let registry = registry_over(&store);
        registry.create("10.0.0.0/8", IpAction::Block).unwrap();
        registry.create("10.1.2.3", IpAction::Allow).unwrap();

        let allowed: IpAddr = "10.1.2.3".parse().unwrap();
        let blocked: IpAddr = "10.1.2.4".parse().unwrap();
        assert_eq!(registry.decision(allowed).unwrap(), Some(IpAction::Allow));
        assert_eq!(registry.decision(blocked).unwrap(), Some(IpAction::Block));
    }

    #[test]
    fn ipv6_prefixes_match_within_their_family_only() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let registry = registry_over(&store);
        registry.create("2001:db8::/32", IpAction::Block).unwrap();

        let v6_inside: IpAddr = "2001:db8::1".parse().unwrap();
        let v6_outside: IpAddr = "2001:db9::1".parse().unwrap();
        let v4: IpAddr = "203.0.113.1".parse().unwrap();
        assert_eq!(registry.decision(v6_inside).unwrap(), Some(IpAction::Block));
        assert_eq!(registry.decision(v6_outside).unwrap(), None);
        assert_eq!(registry.decision(v4).unwrap(), None);
    }

    #[test]
    fn an_ipv4_mapped_ipv6_peer_matches_v4_rules() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let registry = registry_over(&store);
        registry.create("10.0.0.0/8", IpAction::Block).unwrap();
        registry.create("10.1.2.3", IpAction::Allow).unwrap();

        let mapped_blocked: IpAddr = "::ffff:10.9.9.9".parse().unwrap();
        let mapped_allowed: IpAddr = "::ffff:10.1.2.3".parse().unwrap();
        let mapped_outside: IpAddr = "::ffff:192.0.2.1".parse().unwrap();
        assert_eq!(
            registry.decision(mapped_blocked).unwrap(),
            Some(IpAction::Block)
        );
        assert_eq!(
            registry.decision(mapped_allowed).unwrap(),
            Some(IpAction::Allow)
        );
        assert_eq!(registry.decision(mapped_outside).unwrap(), None);
    }

    #[test]
    fn repeated_decisions_consult_a_cached_rule_set_not_the_store() {
        let mem = Arc::new(MemStore::default());
        let store: Arc<dyn Store> = Arc::clone(&mem) as Arc<dyn Store>;
        let registry = registry_over(&store);
        registry.create("10.0.0.0/8", IpAction::Block).unwrap();

        let ip: IpAddr = "10.1.2.3".parse().unwrap();
        registry.decision(ip).unwrap();
        let warm = mem.iterations.load(Ordering::SeqCst);
        for _ in 0..50 {
            registry.decision(ip).unwrap();
        }
        assert_eq!(mem.iterations.load(Ordering::SeqCst), warm);
    }

    #[test]
    fn create_and_delete_refresh_the_cached_decision() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let registry = registry_over(&store);
        let ip: IpAddr = "10.1.2.3".parse().unwrap();
        assert_eq!(registry.decision(ip).unwrap(), None);

        let rule = registry.create("10.0.0.0/8", IpAction::Block).unwrap();
        assert_eq!(registry.decision(ip).unwrap(), Some(IpAction::Block));

        let clone = registry.clone();
        assert_eq!(clone.decision(ip).unwrap(), Some(IpAction::Block));

        registry.delete(rule.id).unwrap();
        assert_eq!(registry.decision(ip).unwrap(), None);
        assert_eq!(clone.decision(ip).unwrap(), None);
    }

    #[test]
    fn a_zero_prefix_matches_every_address_in_its_family() {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let registry = registry_over(&store);
        registry.create("0.0.0.0/0", IpAction::Block).unwrap();
        let any: IpAddr = "198.51.100.9".parse().unwrap();
        assert_eq!(registry.decision(any).unwrap(), Some(IpAction::Block));
    }
}
