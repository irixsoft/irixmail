use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const DEFAULT_MAX_PER_DESTINATION: u32 = 5;

#[derive(Clone)]
pub struct ConcurrencyLimiter {
    inner: Arc<Inner>,
}

struct Inner {
    max_per_destination: u32,
    in_flight: Mutex<HashMap<String, u32>>,
}

pub struct DeliverySlot {
    inner: Arc<Inner>,
    destination: String,
}

impl Default for ConcurrencyLimiter {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_PER_DESTINATION)
    }
}

impl ConcurrencyLimiter {
    pub fn new(max_per_destination: u32) -> Self {
        Self {
            inner: Arc::new(Inner {
                max_per_destination,
                in_flight: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn max_per_destination(&self) -> u32 {
        self.inner.max_per_destination
    }

    pub fn acquire(&self, destination: &str) -> Option<DeliverySlot> {
        let mut in_flight = self.inner.in_flight.lock().unwrap();
        if self.inner.max_per_destination != 0 {
            let current = in_flight.get(destination).copied().unwrap_or(0);
            if current >= self.inner.max_per_destination {
                return None;
            }
        }
        *in_flight.entry(destination.to_string()).or_insert(0) += 1;
        Some(DeliverySlot {
            inner: self.inner.clone(),
            destination: destination.to_string(),
        })
    }

    pub fn in_flight(&self, destination: &str) -> u32 {
        self.inner
            .in_flight
            .lock()
            .unwrap()
            .get(destination)
            .copied()
            .unwrap_or(0)
    }
}

impl Drop for DeliverySlot {
    fn drop(&mut self) {
        let mut in_flight = self.inner.in_flight.lock().unwrap();
        if let Some(count) = in_flight.get_mut(&self.destination) {
            *count -= 1;
            if *count == 0 {
                in_flight.remove(&self.destination);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempts_within_the_cap_each_take_a_slot() {
        let limiter = ConcurrencyLimiter::new(3);
        let _a = limiter.acquire("mail.example.com").expect("first");
        let _b = limiter.acquire("mail.example.com").expect("second");
        let _c = limiter.acquire("mail.example.com").expect("third");
        assert_eq!(limiter.in_flight("mail.example.com"), 3);
    }

    #[test]
    fn the_attempt_over_the_cap_is_refused() {
        let limiter = ConcurrencyLimiter::new(2);
        let _a = limiter.acquire("mail.example.com").expect("first");
        let _b = limiter.acquire("mail.example.com").expect("second");
        assert!(limiter.acquire("mail.example.com").is_none());
    }

    #[test]
    fn dropping_a_slot_frees_a_place_for_the_next_attempt() {
        let limiter = ConcurrencyLimiter::new(1);
        let slot = limiter.acquire("mail.example.com").expect("first");
        assert!(limiter.acquire("mail.example.com").is_none());
        drop(slot);
        assert!(limiter.acquire("mail.example.com").is_some());
    }

    #[test]
    fn distinct_destinations_are_capped_independently() {
        let limiter = ConcurrencyLimiter::new(1);
        let _one = limiter.acquire("mail.one.example").expect("one");
        assert!(limiter.acquire("mail.one.example").is_none());
        assert!(limiter.acquire("mail.two.example").is_some());
    }

    #[test]
    fn a_destination_with_no_live_attempt_is_forgotten() {
        let limiter = ConcurrencyLimiter::new(2);
        {
            let _slot = limiter.acquire("mail.example.com").expect("slot");
            assert_eq!(limiter.in_flight("mail.example.com"), 1);
        }
        assert_eq!(limiter.in_flight("mail.example.com"), 0);
        assert_eq!(limiter.inner.in_flight.lock().unwrap().len(), 0);
    }

    #[test]
    fn a_zero_cap_disables_the_limit() {
        let limiter = ConcurrencyLimiter::new(0);
        let mut slots = Vec::new();
        for _ in 0..1000 {
            slots.push(limiter.acquire("mail.example.com").expect("granted"));
        }
        assert_eq!(limiter.in_flight("mail.example.com"), 1000);
    }

    #[test]
    fn the_default_cap_bounds_a_single_destination() {
        let limiter = ConcurrencyLimiter::default();
        assert_eq!(limiter.max_per_destination(), DEFAULT_MAX_PER_DESTINATION);
        let mut slots = Vec::new();
        for _ in 0..DEFAULT_MAX_PER_DESTINATION {
            slots.push(limiter.acquire("mail.example.com").expect("granted"));
        }
        assert!(limiter.acquire("mail.example.com").is_none());
    }

    #[test]
    fn the_limiter_shares_one_tally_across_clones() {
        let limiter = ConcurrencyLimiter::new(1);
        let clone = limiter.clone();
        let _slot = limiter.acquire("mail.example.com").expect("first");
        assert!(clone.acquire("mail.example.com").is_none());
        assert_eq!(clone.in_flight("mail.example.com"), 1);
    }
}
