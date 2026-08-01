use std::time::Duration;

use irixmail_store::TtlStore;

const KEY_PREFIX: &[u8] = b"auth-throttle:";

const SUBJECT_IP: &[u8] = b"ip:";

const SUBJECT_ACCOUNT: &[u8] = b"account:";

pub const DEFAULT_MAX_FAILURES: u32 = 5;

pub const DEFAULT_WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrottlePolicy {
    pub max_failures: u32,
    pub window: Duration,
}

impl ThrottlePolicy {
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            max_failures: max_failures.max(1),
            window,
        }
    }
}

impl Default for ThrottlePolicy {
    fn default() -> Self {
        Self {
            max_failures: DEFAULT_MAX_FAILURES,
            window: DEFAULT_WINDOW,
        }
    }
}

#[derive(Clone)]
pub struct Throttle {
    store: std::sync::Arc<TtlStore>,
    policy: ThrottlePolicy,
}

impl Throttle {
    pub fn new(store: std::sync::Arc<TtlStore>, policy: ThrottlePolicy) -> Self {
        Self { store, policy }
    }

    pub fn policy(&self) -> ThrottlePolicy {
        self.policy
    }

    pub fn record_failure(&self, ip: Option<&str>, account_id: Option<u64>) {
        if let Some(ip) = ip {
            self.bump(&ip_key(ip));
        }
        if let Some(account_id) = account_id {
            self.bump(&account_key(account_id));
        }
    }

    pub fn record_success(&self, ip: Option<&str>, account_id: Option<u64>) {
        if let Some(ip) = ip {
            self.store.remove(&ip_key(ip));
        }
        if let Some(account_id) = account_id {
            self.store.remove(&account_key(account_id));
        }
    }

    pub fn is_locked(&self, ip: Option<&str>, account_id: Option<u64>) -> bool {
        let ip_locked = ip
            .map(|ip| self.count(&ip_key(ip)) >= self.policy.max_failures)
            .unwrap_or(false);
        let account_locked = account_id
            .map(|id| self.count(&account_key(id)) >= self.policy.max_failures)
            .unwrap_or(false);
        ip_locked || account_locked
    }

    fn count(&self, key: &[u8]) -> u32 {
        self.store
            .get(key)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .unwrap_or(0)
    }

    fn bump(&self, key: &[u8]) {
        let next = self.count(key).saturating_add(1);
        self.store.set(
            key.to_vec(),
            next.to_be_bytes().to_vec(),
            self.policy.window,
        );
    }
}

fn ip_key(ip: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + SUBJECT_IP.len() + ip.len());
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(SUBJECT_IP);
    key.extend_from_slice(ip.as_bytes());
    key
}

fn account_key(account_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + SUBJECT_ACCOUNT.len() + 8);
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(SUBJECT_ACCOUNT);
    key.extend_from_slice(&account_id.to_be_bytes());
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn throttle(max_failures: u32, window: Duration) -> Throttle {
        Throttle::new(
            Arc::new(TtlStore::new()),
            ThrottlePolicy::new(max_failures, window),
        )
    }

    #[test]
    fn a_fresh_subject_is_not_locked() {
        let throttle = throttle(3, Duration::from_secs(60));
        assert!(!throttle.is_locked(Some("198.51.100.7"), Some(42)));
    }

    #[test]
    fn failures_below_the_limit_do_not_lock_the_account() {
        let throttle = throttle(3, Duration::from_secs(60));
        throttle.record_failure(None, Some(42));
        throttle.record_failure(None, Some(42));
        assert!(!throttle.is_locked(None, Some(42)));
    }

    #[test]
    fn reaching_the_limit_locks_the_account() {
        let throttle = throttle(3, Duration::from_secs(60));
        throttle.record_failure(None, Some(42));
        throttle.record_failure(None, Some(42));
        throttle.record_failure(None, Some(42));
        assert!(throttle.is_locked(None, Some(42)));
    }

    #[test]
    fn reaching_the_limit_locks_the_ip() {
        let throttle = throttle(2, Duration::from_secs(60));
        throttle.record_failure(Some("203.0.113.9"), None);
        throttle.record_failure(Some("203.0.113.9"), None);
        assert!(throttle.is_locked(Some("203.0.113.9"), None));
    }

    #[test]
    fn either_subject_over_its_limit_bars_the_login() {
        let throttle = throttle(2, Duration::from_secs(60));
        throttle.record_failure(Some("203.0.113.9"), Some(42));
        throttle.record_failure(Some("198.51.100.1"), Some(42));
        assert!(throttle.is_locked(Some("198.51.100.1"), Some(42)));
        assert!(!throttle.is_locked(Some("198.51.100.1"), None));
    }

    #[test]
    fn one_failure_tallies_both_subjects() {
        let throttle = throttle(1, Duration::from_secs(60));
        throttle.record_failure(Some("203.0.113.9"), Some(42));
        assert!(throttle.is_locked(Some("203.0.113.9"), None));
        assert!(throttle.is_locked(None, Some(42)));
    }

    #[test]
    fn distinct_subjects_count_separately() {
        let throttle = throttle(2, Duration::from_secs(60));
        throttle.record_failure(None, Some(1));
        throttle.record_failure(None, Some(1));
        assert!(throttle.is_locked(None, Some(1)));
        assert!(!throttle.is_locked(None, Some(2)));
    }

    #[test]
    fn distinct_ip_forms_count_separately() {
        let throttle = throttle(2, Duration::from_secs(60));
        throttle.record_failure(Some("203.0.113.9"), None);
        throttle.record_failure(Some("203.0.113.9"), None);
        assert!(throttle.is_locked(Some("203.0.113.9"), None));
        assert!(!throttle.is_locked(Some("203.0.113.10"), None));
    }

    #[test]
    fn a_success_clears_the_tally_for_both_subjects() {
        let throttle = throttle(3, Duration::from_secs(60));
        throttle.record_failure(Some("203.0.113.9"), Some(42));
        throttle.record_failure(Some("203.0.113.9"), Some(42));
        throttle.record_success(Some("203.0.113.9"), Some(42));
        throttle.record_failure(Some("203.0.113.9"), Some(42));
        assert!(!throttle.is_locked(Some("203.0.113.9"), Some(42)));
    }

    #[test]
    fn a_lapsed_window_forgets_the_failures() {
        let throttle = throttle(1, Duration::ZERO);
        throttle.record_failure(Some("203.0.113.9"), Some(42));
        assert!(!throttle.is_locked(Some("203.0.113.9"), Some(42)));
    }

    #[test]
    fn omitting_both_subjects_never_locks() {
        let throttle = throttle(1, Duration::from_secs(60));
        throttle.record_failure(None, None);
        assert!(!throttle.is_locked(None, None));
    }

    #[test]
    fn a_zero_limit_policy_is_clamped_to_one() {
        let policy = ThrottlePolicy::new(0, Duration::from_secs(60));
        assert_eq!(policy.max_failures, 1);
    }

    #[test]
    fn the_default_policy_uses_the_named_constants() {
        let policy = ThrottlePolicy::default();
        assert_eq!(policy.max_failures, DEFAULT_MAX_FAILURES);
        assert_eq!(policy.window, DEFAULT_WINDOW);
    }

    #[test]
    fn the_count_saturates_rather_than_wrapping() {
        let throttle = throttle(2, Duration::from_secs(60));
        let key = ip_key("203.0.113.9");
        throttle.store.set(
            key.clone(),
            u32::MAX.to_be_bytes().to_vec(),
            Duration::from_secs(60),
        );
        throttle.bump(&key);
        assert_eq!(throttle.count(&key), u32::MAX);
        assert!(throttle.is_locked(Some("203.0.113.9"), None));
    }
}
