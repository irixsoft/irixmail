use std::sync::Arc;
use std::time::Duration;

use irixmail_store::{settings_key, Store, TtlStore};

pub const DEFAULT_MAX_PER_SENDER: u32 = 500;
pub const DEFAULT_MAX_PER_DOMAIN: u32 = 2000;
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutboundLimits {
    pub max_per_sender: u32,
    pub max_per_domain: u32,
    pub window: Duration,
}

impl Default for OutboundLimits {
    fn default() -> Self {
        Self {
            max_per_sender: DEFAULT_MAX_PER_SENDER,
            max_per_domain: DEFAULT_MAX_PER_DOMAIN,
            window: DEFAULT_WINDOW,
        }
    }
}

impl OutboundLimits {
    pub fn is_disabled(&self) -> bool {
        self.window.is_zero() || (self.max_per_sender == 0 && self.max_per_domain == 0)
    }

    pub fn from_settings(store: &dyn Store) -> Self {
        let defaults = Self::default();
        let Ok(Some(bytes)) = store.get(&settings_key()) else {
            return defaults;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return defaults;
        };
        Self {
            max_per_sender: value["rateLimits"]["maxMessagesPerSenderPerHour"]
                .as_u64()
                .map(|max| max as u32)
                .unwrap_or(defaults.max_per_sender),
            max_per_domain: value["rateLimits"]["maxMessagesPerDomainPerHour"]
                .as_u64()
                .map(|max| max as u32)
                .unwrap_or(defaults.max_per_domain),
            window: defaults.window,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboundDecision {
    Allow,
    Deny(Axis),
}

impl OutboundDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, OutboundDecision::Allow)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Sender,
    Domain,
}

impl Axis {
    fn tag(self) -> u8 {
        match self {
            Axis::Sender => b's',
            Axis::Domain => b'd',
        }
    }
}

pub struct OutboundLimiter {
    store: Arc<TtlStore>,
    limits: OutboundLimits,
}

impl OutboundLimiter {
    pub fn new(store: Arc<TtlStore>, limits: OutboundLimits) -> Self {
        Self { store, limits }
    }

    pub fn limits(&self) -> OutboundLimits {
        self.limits
    }

    pub fn window_end(&self, now: u64) -> u64 {
        let window = self.limits.window.as_secs().max(1);
        (now / window + 1) * window
    }

    pub fn check(&self, sender: &str) -> OutboundDecision {
        if self.limits.is_disabled() {
            return OutboundDecision::Allow;
        }
        let sender = sender.to_ascii_lowercase();
        let domain = domain_of(&sender);

        if self.is_over(Axis::Sender, &sender, self.limits.max_per_sender) {
            return OutboundDecision::Deny(Axis::Sender);
        }
        if self.is_over(Axis::Domain, domain, self.limits.max_per_domain) {
            return OutboundDecision::Deny(Axis::Domain);
        }
        self.record(Axis::Sender, &sender, self.limits.max_per_sender);
        self.record(Axis::Domain, domain, self.limits.max_per_domain);
        OutboundDecision::Allow
    }

    fn is_over(&self, axis: Axis, identity: &str, limit: u32) -> bool {
        if limit == 0 {
            return false;
        }
        self.count(axis, identity) >= limit
    }

    fn record(&self, axis: Axis, identity: &str, limit: u32) {
        if limit == 0 {
            return;
        }
        let key = window_key(axis, identity, self.limits.window);
        let next = self.count(axis, identity) + 1;
        self.store
            .set(key, next.to_be_bytes().to_vec(), self.limits.window);
    }

    fn count(&self, axis: Axis, identity: &str) -> u32 {
        let key = window_key(axis, identity, self.limits.window);
        self.store
            .get(&key)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .unwrap_or(0)
    }
}

fn domain_of(sender: &str) -> &str {
    sender
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or("")
}

fn window_key(axis: Axis, identity: &str, window: Duration) -> Vec<u8> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bucket = now / window.as_secs().max(1);

    let mut key = Vec::with_capacity(16 + identity.len());
    key.extend_from_slice(b"ro:");
    key.push(axis.tag());
    key.extend_from_slice(&bucket.to_be_bytes());
    key.extend_from_slice(identity.as_bytes());
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(max_per_sender: u32, max_per_domain: u32) -> OutboundLimiter {
        OutboundLimiter::new(
            Arc::new(TtlStore::new()),
            OutboundLimits {
                max_per_sender,
                max_per_domain,
                window: Duration::from_secs(3600),
            },
        )
    }

    #[test]
    fn sends_within_the_sender_allowance_are_admitted() {
        let rl = limiter(3, 0);
        assert!(rl.check("a@example.com").is_allowed());
        assert!(rl.check("a@example.com").is_allowed());
        assert!(rl.check("a@example.com").is_allowed());
    }

    #[test]
    fn the_send_over_the_sender_allowance_is_refused() {
        let rl = limiter(2, 0);
        assert!(rl.check("a@example.com").is_allowed());
        assert!(rl.check("a@example.com").is_allowed());
        assert_eq!(
            rl.check("a@example.com"),
            OutboundDecision::Deny(Axis::Sender)
        );
    }

    #[test]
    fn the_send_over_the_domain_allowance_is_refused() {
        let rl = limiter(0, 2);
        assert!(rl.check("a@example.com").is_allowed());
        assert!(rl.check("b@example.com").is_allowed());
        assert_eq!(
            rl.check("c@example.com"),
            OutboundDecision::Deny(Axis::Domain)
        );
    }

    #[test]
    fn the_domain_allowance_pools_every_sender_in_the_domain() {
        let rl = limiter(0, 3);
        assert!(rl.check("a@example.com").is_allowed());
        assert!(rl.check("b@example.com").is_allowed());
        assert!(rl.check("c@example.com").is_allowed());
        assert_eq!(
            rl.check("d@example.com"),
            OutboundDecision::Deny(Axis::Domain)
        );
    }

    #[test]
    fn distinct_domains_keep_independent_tallies() {
        let rl = limiter(0, 1);
        assert!(rl.check("a@one.example").is_allowed());
        assert_eq!(
            rl.check("b@one.example"),
            OutboundDecision::Deny(Axis::Domain)
        );
        assert!(rl.check("c@two.example").is_allowed());
    }

    #[test]
    fn a_sender_refusal_leaves_the_domain_tally_untouched() {
        let rl = limiter(1, 5);
        assert!(rl.check("a@example.com").is_allowed());
        assert_eq!(
            rl.check("a@example.com"),
            OutboundDecision::Deny(Axis::Sender)
        );
        assert!(rl.check("b@example.com").is_allowed());
        assert!(rl.check("c@example.com").is_allowed());
        assert!(rl.check("d@example.com").is_allowed());
    }

    #[test]
    fn the_sender_is_matched_without_regard_to_case() {
        let rl = limiter(1, 0);
        assert!(rl.check("User@Example.com").is_allowed());
        assert_eq!(
            rl.check("user@example.com"),
            OutboundDecision::Deny(Axis::Sender)
        );
    }

    #[test]
    fn a_zero_limit_on_an_axis_disables_that_axis() {
        let rl = limiter(0, 2);
        assert!(rl.check("a@example.com").is_allowed());
        assert!(rl.check("a@example.com").is_allowed());
        assert_eq!(
            rl.check("a@example.com"),
            OutboundDecision::Deny(Axis::Domain)
        );
    }

    #[test]
    fn both_axes_zero_admits_every_send() {
        let rl = limiter(0, 0);
        assert!(rl.limits().is_disabled());
        for _ in 0..1000 {
            assert!(rl.check("a@example.com").is_allowed());
        }
    }

    #[test]
    fn a_disabled_window_admits_every_send() {
        let rl = OutboundLimiter::new(
            Arc::new(TtlStore::new()),
            OutboundLimits {
                max_per_sender: 1,
                max_per_domain: 1,
                window: Duration::ZERO,
            },
        );
        assert!(rl.limits().is_disabled());
        for _ in 0..1000 {
            assert!(rl.check("a@example.com").is_allowed());
        }
    }

    #[test]
    fn a_repeated_refusal_does_not_consume_further_allowance() {
        let rl = limiter(1, 0);
        assert!(rl.check("a@example.com").is_allowed());
        assert_eq!(
            rl.check("a@example.com"),
            OutboundDecision::Deny(Axis::Sender)
        );
        assert_eq!(
            rl.check("a@example.com"),
            OutboundDecision::Deny(Axis::Sender)
        );
    }

    #[test]
    fn a_lapsed_window_starts_a_fresh_tally() {
        let rl = OutboundLimiter::new(
            Arc::new(TtlStore::new()),
            OutboundLimits {
                max_per_sender: 1,
                max_per_domain: 0,
                window: Duration::from_millis(1),
            },
        );
        assert!(rl.check("a@example.com").is_allowed());
        assert_eq!(
            rl.check("a@example.com"),
            OutboundDecision::Deny(Axis::Sender)
        );
        std::thread::sleep(Duration::from_millis(5));
        assert!(rl.check("a@example.com").is_allowed());
    }

    #[test]
    fn a_bare_sender_falls_under_the_empty_domain() {
        let rl = limiter(0, 1);
        assert!(rl.check("postmaster").is_allowed());
        assert_eq!(rl.check("hostmaster"), OutboundDecision::Deny(Axis::Domain));
    }

    #[test]
    fn the_defaults_cap_both_axes() {
        let limits = OutboundLimits::default();
        assert_eq!(limits.max_per_sender, DEFAULT_MAX_PER_SENDER);
        assert_eq!(limits.max_per_domain, DEFAULT_MAX_PER_DOMAIN);
        assert_eq!(limits.window, DEFAULT_WINDOW);
        assert!(!limits.is_disabled());
    }
}
