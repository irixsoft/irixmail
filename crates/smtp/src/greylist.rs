use std::sync::Arc;
use std::time::Duration;

use irixmail_core::Result;
use irixmail_store::ExpiringStore;

const DEFERRED: &[u8] = b"452 4.2.2 Greylisted, please retry in a few moments\r\n";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GreylistConfig {
    pub window: Duration,
}

impl GreylistConfig {
    pub fn is_disabled(&self) -> bool {
        self.window.is_zero()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GreylistDecision {
    Allow,
    Defer(&'static [u8]),
}

impl GreylistDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, GreylistDecision::Allow)
    }
}

pub struct Greylist {
    store: Arc<ExpiringStore>,
    config: GreylistConfig,
}

impl Greylist {
    pub fn new(store: Arc<ExpiringStore>, config: GreylistConfig) -> Self {
        Self { store, config }
    }

    pub fn reconfigured(&self, config: GreylistConfig) -> Self {
        Self {
            store: Arc::clone(&self.store),
            config,
        }
    }

    pub fn config(&self) -> GreylistConfig {
        self.config
    }

    pub fn check(&self, from: &str, rcpt: &str, authenticated: bool) -> Result<GreylistDecision> {
        if authenticated || self.config.is_disabled() {
            return Ok(GreylistDecision::Allow);
        }
        let key = pair_key(from, rcpt);
        if self.store.contains(&key)? {
            return Ok(GreylistDecision::Allow);
        }
        self.store.set(&key, self.config.window)?;
        Ok(GreylistDecision::Defer(DEFERRED))
    }

    pub fn check_or_allow(&self, from: &str, rcpt: &str, authenticated: bool) -> GreylistDecision {
        match self.check(from, rcpt, authenticated) {
            Ok(decision) => decision,
            Err(err) => {
                tracing::warn!(error = %err, "greylist store failure, accepting");
                GreylistDecision::Allow
            }
        }
    }
}

fn pair_key(from: &str, rcpt: &str) -> Vec<u8> {
    let from = from.to_ascii_lowercase();
    let rcpt = rcpt.to_ascii_lowercase();

    let mut key = Vec::with_capacity(11 + from.len() + rcpt.len());
    key.extend_from_slice(b"gl:");
    key.extend_from_slice(&(from.len() as u32).to_be_bytes());
    key.extend_from_slice(from.as_bytes());
    key.extend_from_slice(&(rcpt.len() as u32).to_be_bytes());
    key.extend_from_slice(rcpt.as_bytes());
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use irixmail_core::Error;
    use irixmail_store::{Flow, KeyPrefix, Store, WriteOp};

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
                    WriteOp::Add { .. } => unreachable!(),
                }
            }
            Ok(())
        }
        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!()
        }
        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!()
        }
    }

    struct FailingStore;

    impl Store for FailingStore {
        fn get(&self, _key: &[u8]) -> Result<Option<Vec<u8>>> {
            Err(Error::internal("store down"))
        }
        fn put(&self, _key: &[u8], _value: &[u8]) -> Result<()> {
            Err(Error::internal("store down"))
        }
        fn delete(&self, _key: &[u8]) -> Result<()> {
            Err(Error::internal("store down"))
        }
        fn iterate(
            &self,
            _prefix: &KeyPrefix,
            _visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            Err(Error::internal("store down"))
        }
        fn batch(&self, _ops: &[WriteOp]) -> Result<()> {
            Err(Error::internal("store down"))
        }
        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            Err(Error::internal("store down"))
        }
        fn counter(&self, _key: &[u8]) -> Result<i64> {
            Err(Error::internal("store down"))
        }
    }

    fn greylist(window: Duration) -> Greylist {
        let store = ExpiringStore::new(Arc::new(MemStore::default()) as Arc<dyn Store>);
        Greylist::new(Arc::new(store), GreylistConfig { window })
    }

    const HOUR: Duration = Duration::from_secs(3600);

    #[test]
    fn a_first_sighting_is_deferred() {
        let gl = greylist(HOUR);
        assert_eq!(
            gl.check("a@b.example", "c@d.example", false).unwrap(),
            GreylistDecision::Defer(DEFERRED)
        );
    }

    #[test]
    fn an_immediate_retry_is_admitted_with_no_minimum_delay() {
        let gl = greylist(HOUR);
        assert!(!gl
            .check("a@b.example", "c@d.example", false)
            .unwrap()
            .is_allowed());
        assert!(gl
            .check("a@b.example", "c@d.example", false)
            .unwrap()
            .is_allowed());
    }

    #[test]
    fn an_authenticated_session_is_exempt() {
        let gl = greylist(HOUR);
        assert!(gl
            .check("a@b.example", "c@d.example", true)
            .unwrap()
            .is_allowed());
    }

    #[test]
    fn the_default_config_is_disabled_and_admits_everything() {
        let gl = greylist(GreylistConfig::default().window);
        assert!(gl.config().is_disabled());
        for _ in 0..100 {
            assert!(gl
                .check("a@b.example", "c@d.example", false)
                .unwrap()
                .is_allowed());
        }
    }

    #[test]
    fn a_different_sender_starts_a_fresh_challenge() {
        let gl = greylist(HOUR);
        assert!(!gl
            .check("one@b.example", "c@d.example", false)
            .unwrap()
            .is_allowed());
        assert!(!gl
            .check("two@b.example", "c@d.example", false)
            .unwrap()
            .is_allowed());
    }

    #[test]
    fn a_different_recipient_starts_a_fresh_challenge() {
        let gl = greylist(HOUR);
        assert!(!gl
            .check("a@b.example", "one@d.example", false)
            .unwrap()
            .is_allowed());
        assert!(!gl
            .check("a@b.example", "two@d.example", false)
            .unwrap()
            .is_allowed());
    }

    #[test]
    fn the_pair_is_matched_without_regard_to_case() {
        let gl = greylist(HOUR);
        assert!(!gl
            .check("A@B.Example", "C@D.Example", false)
            .unwrap()
            .is_allowed());
        assert!(gl
            .check("a@b.example", "c@d.example", false)
            .unwrap()
            .is_allowed());
    }

    #[test]
    fn adjacent_address_bytes_do_not_collide_on_one_key() {
        let gl = greylist(HOUR);
        assert!(!gl.check("ab", "cd", false).unwrap().is_allowed());
        assert!(!gl.check("a", "bcd", false).unwrap().is_allowed());
    }

    #[test]
    fn a_store_error_surfaces_and_the_wrapper_accepts() {
        let store = ExpiringStore::new(Arc::new(FailingStore) as Arc<dyn Store>);
        let gl = Greylist::new(Arc::new(store), GreylistConfig { window: HOUR });
        assert!(gl.check("a@b.example", "c@d.example", false).is_err());
        assert!(gl
            .check_or_allow("a@b.example", "c@d.example", false)
            .is_allowed());
    }

    #[test]
    fn the_deferral_is_a_transient_negative() {
        assert!(DEFERRED.starts_with(b"452"));
    }
}
