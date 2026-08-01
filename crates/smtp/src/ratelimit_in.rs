use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use irixmail_store::TtlStore;

const TOO_MANY_CONNECTIONS: &[u8] = b"421 4.7.0 Too many connections, try again later\r\n";
const TOO_MANY_MESSAGES: &[u8] = b"452 4.7.0 Too many messages, try again later\r\n";

pub const DEFAULT_MAX_CONNECTIONS: u32 = 30;
pub const DEFAULT_MAX_MESSAGES: u32 = 200;
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimits {
    pub max_connections: u32,
    pub max_messages: u32,
    pub window: Duration,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_messages: DEFAULT_MAX_MESSAGES,
            window: DEFAULT_WINDOW,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateDecision {
    Allow,
    Deny(&'static [u8]),
}

impl RateDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, RateDecision::Allow)
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Connection,
    Message,
}

impl Kind {
    fn tag(self) -> u8 {
        match self {
            Kind::Connection => b'c',
            Kind::Message => b'm',
        }
    }
}

pub struct RateLimiter {
    store: Arc<TtlStore>,
    limits: RateLimits,
}

impl RateLimiter {
    pub fn new(store: Arc<TtlStore>, limits: RateLimits) -> Self {
        Self { store, limits }
    }

    pub fn reconfigured(&self, limits: RateLimits) -> Self {
        Self {
            store: Arc::clone(&self.store),
            limits,
        }
    }

    pub fn limits(&self) -> RateLimits {
        self.limits
    }

    pub fn on_connect(&self, ip: IpAddr) -> RateDecision {
        self.hit(
            Kind::Connection,
            ip,
            self.limits.max_connections,
            TOO_MANY_CONNECTIONS,
        )
    }

    pub fn on_message(&self, ip: IpAddr) -> RateDecision {
        self.hit(
            Kind::Message,
            ip,
            self.limits.max_messages,
            TOO_MANY_MESSAGES,
        )
    }

    fn hit(&self, kind: Kind, ip: IpAddr, limit: u32, reply: &'static [u8]) -> RateDecision {
        if limit == 0 || self.limits.window.is_zero() {
            return RateDecision::Allow;
        }
        let key = window_key(kind, ip, self.limits.window);
        let current = self
            .store
            .get(&key)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .unwrap_or(0);
        if current >= limit {
            return RateDecision::Deny(reply);
        }
        self.store.set(
            key,
            (current + 1).to_be_bytes().to_vec(),
            self.limits.window,
        );
        RateDecision::Allow
    }
}

fn window_key(kind: Kind, ip: IpAddr, window: Duration) -> Vec<u8> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bucket = now / window.as_secs().max(1);

    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(b"rl:");
    key.push(kind.tag());
    match ip {
        IpAddr::V4(v4) => {
            key.push(4);
            key.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            key.push(6);
            key.extend_from_slice(&v6.octets());
        }
    }
    key.extend_from_slice(&bucket.to_be_bytes());
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn limiter(max_connections: u32, max_messages: u32) -> RateLimiter {
        RateLimiter::new(
            Arc::new(TtlStore::new()),
            RateLimits {
                max_connections,
                max_messages,
                window: Duration::from_secs(60),
            },
        )
    }

    #[test]
    fn connections_within_the_allowance_are_admitted() {
        let rl = limiter(3, 0);
        let src = ip("198.51.100.7");
        assert!(rl.on_connect(src).is_allowed());
        assert!(rl.on_connect(src).is_allowed());
        assert!(rl.on_connect(src).is_allowed());
    }

    #[test]
    fn the_connection_over_the_allowance_is_refused() {
        let rl = limiter(2, 0);
        let src = ip("198.51.100.7");
        assert!(rl.on_connect(src).is_allowed());
        assert!(rl.on_connect(src).is_allowed());
        assert_eq!(rl.on_connect(src), RateDecision::Deny(TOO_MANY_CONNECTIONS));
    }

    #[test]
    fn the_message_over_the_allowance_is_deferred() {
        let rl = limiter(0, 1);
        let src = ip("198.51.100.7");
        assert!(rl.on_message(src).is_allowed());
        assert_eq!(rl.on_message(src), RateDecision::Deny(TOO_MANY_MESSAGES));
    }

    #[test]
    fn connection_and_message_tallies_do_not_share_a_bucket() {
        let rl = limiter(1, 1);
        let src = ip("203.0.113.9");
        assert!(rl.on_connect(src).is_allowed());
        assert_eq!(rl.on_connect(src), RateDecision::Deny(TOO_MANY_CONNECTIONS));
        assert!(rl.on_message(src).is_allowed());
    }

    #[test]
    fn each_source_keeps_its_own_tally() {
        let rl = limiter(1, 0);
        let one = ip("198.51.100.1");
        let two = ip("198.51.100.2");
        assert!(rl.on_connect(one).is_allowed());
        assert_eq!(rl.on_connect(one), RateDecision::Deny(TOO_MANY_CONNECTIONS));
        assert!(rl.on_connect(two).is_allowed());
    }

    #[test]
    fn ipv4_and_ipv6_sources_are_distinct() {
        let rl = limiter(1, 0);
        let v4 = ip("198.51.100.5");
        let v6 = ip("2001:db8::5");
        assert!(rl.on_connect(v4).is_allowed());
        assert!(rl.on_connect(v6).is_allowed());
        assert_eq!(rl.on_connect(v4), RateDecision::Deny(TOO_MANY_CONNECTIONS));
        assert_eq!(rl.on_connect(v6), RateDecision::Deny(TOO_MANY_CONNECTIONS));
    }

    #[test]
    fn a_zero_limit_disables_the_check() {
        let rl = limiter(0, 0);
        let src = ip("198.51.100.7");
        for _ in 0..1000 {
            assert!(rl.on_connect(src).is_allowed());
            assert!(rl.on_message(src).is_allowed());
        }
    }

    #[test]
    fn a_refusal_does_not_consume_further_allowance() {
        let rl = limiter(1, 0);
        let src = ip("198.51.100.7");
        assert!(rl.on_connect(src).is_allowed());
        assert_eq!(rl.on_connect(src), RateDecision::Deny(TOO_MANY_CONNECTIONS));
        assert_eq!(rl.on_connect(src), RateDecision::Deny(TOO_MANY_CONNECTIONS));
    }

    #[test]
    fn the_defaults_cap_both_axes() {
        let limits = RateLimits::default();
        assert_eq!(limits.max_connections, DEFAULT_MAX_CONNECTIONS);
        assert_eq!(limits.max_messages, DEFAULT_MAX_MESSAGES);
        assert_eq!(limits.window, DEFAULT_WINDOW);
    }

    #[test]
    fn the_connection_reply_is_a_transient_negative() {
        assert!(TOO_MANY_CONNECTIONS.starts_with(b"421"));
        assert!(TOO_MANY_MESSAGES.starts_with(b"452"));
    }
}
