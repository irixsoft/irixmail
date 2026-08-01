use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

pub const DEFAULT_LEASE: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lease {
    Granted,
    Held,
}

impl Lease {
    pub fn is_granted(&self) -> bool {
        matches!(self, Lease::Granted)
    }
}

pub struct LeaseRegistry {
    held: Mutex<HashMap<u32, u64>>,
    lease: Duration,
}

impl Default for LeaseRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_LEASE)
    }
}

impl LeaseRegistry {
    pub fn new(lease: Duration) -> Self {
        Self {
            held: Mutex::new(HashMap::new()),
            lease,
        }
    }

    pub fn lease(&self) -> Duration {
        self.lease
    }

    pub fn acquire(&self, id: u32, now: u64) -> Lease {
        let mut held = self.held.lock().unwrap();
        if let Some(expires) = held.get(&id) {
            if now < *expires {
                return Lease::Held;
            }
        }
        held.insert(id, now.saturating_add(self.lease.as_secs()));
        Lease::Granted
    }

    pub fn release(&self, id: u32) {
        self.held.lock().unwrap().remove(&id);
    }

    pub fn is_held(&self, id: u32, now: u64) -> bool {
        self.held
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|expires| now < *expires)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_message_is_granted_and_a_held_one_is_refused() {
        let registry = LeaseRegistry::new(Duration::from_secs(100));
        assert_eq!(registry.acquire(7, 1_000), Lease::Granted);
        assert_eq!(registry.acquire(7, 1_010), Lease::Held);
    }

    #[test]
    fn distinct_messages_lease_independently() {
        let registry = LeaseRegistry::new(Duration::from_secs(100));
        assert_eq!(registry.acquire(1, 1_000), Lease::Granted);
        assert_eq!(registry.acquire(2, 1_000), Lease::Granted);
    }

    #[test]
    fn a_released_message_can_be_leased_again() {
        let registry = LeaseRegistry::new(Duration::from_secs(100));
        assert_eq!(registry.acquire(5, 1_000), Lease::Granted);
        registry.release(5);
        assert_eq!(registry.acquire(5, 1_005), Lease::Granted);
    }

    #[test]
    fn a_lapsed_lease_is_reclaimed_by_the_next_attempt() {
        let registry = LeaseRegistry::new(Duration::from_secs(100));
        assert_eq!(registry.acquire(9, 1_000), Lease::Granted);
        assert_eq!(registry.acquire(9, 1_099), Lease::Held);
        assert_eq!(registry.acquire(9, 1_100), Lease::Granted);
    }

    #[test]
    fn reclaiming_a_lapsed_lease_resets_its_expiry() {
        let registry = LeaseRegistry::new(Duration::from_secs(100));
        registry.acquire(3, 1_000);
        assert_eq!(registry.acquire(3, 1_100), Lease::Granted);
        assert_eq!(registry.acquire(3, 1_150), Lease::Held);
        assert_eq!(registry.acquire(3, 1_200), Lease::Granted);
    }

    #[test]
    fn a_held_lease_reports_held_until_it_lapses() {
        let registry = LeaseRegistry::new(Duration::from_secs(50));
        assert!(!registry.is_held(4, 1_000));
        registry.acquire(4, 1_000);
        assert!(registry.is_held(4, 1_049));
        assert!(!registry.is_held(4, 1_050));
    }

    #[test]
    fn releasing_an_unleased_message_is_harmless() {
        let registry = LeaseRegistry::new(Duration::from_secs(50));
        registry.release(11);
        registry.acquire(11, 1_000);
        registry.release(11);
        registry.release(11);
        assert_eq!(registry.acquire(11, 1_001), Lease::Granted);
    }

    #[test]
    fn a_lease_expiry_at_the_far_edge_does_not_overflow() {
        let registry = LeaseRegistry::new(Duration::from_secs(300));
        assert_eq!(registry.acquire(1, u64::MAX - 300), Lease::Granted);
        assert_eq!(registry.acquire(1, u64::MAX - 1), Lease::Held);
    }
}
