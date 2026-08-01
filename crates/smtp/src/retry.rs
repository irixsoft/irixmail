use std::time::Duration;

use crate::queue_model::{Expiry, RetrySchedule};

pub const BASE_BACKOFF: Duration = Duration::from_secs(300);

pub const MAX_BACKOFF: Duration = Duration::from_secs(7_200);

const MAX_SHIFT: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry(RetrySchedule),
    Bounce,
}

impl RetryDecision {
    pub fn is_bounce(&self) -> bool {
        matches!(self, RetryDecision::Bounce)
    }

    pub fn schedule(&self) -> Option<RetrySchedule> {
        match self {
            RetryDecision::Retry(schedule) => Some(*schedule),
            RetryDecision::Bounce => None,
        }
    }
}

pub fn backoff(attempts: u32) -> Duration {
    let shift = attempts.min(MAX_SHIFT);
    let scaled = BASE_BACKOFF
        .as_secs()
        .saturating_mul(1u64 << shift)
        .min(MAX_BACKOFF.as_secs());
    Duration::from_secs(scaled)
}

pub fn next_after_deferral(retry: &RetrySchedule, expiry: &Expiry, now: u64) -> RetryDecision {
    let attempts = retry.attempts.saturating_add(1);

    if expiry.is_expired(attempts, now) {
        return RetryDecision::Bounce;
    }

    let due = now.saturating_add(backoff(retry.attempts).as_secs());
    RetryDecision::Retry(RetrySchedule { attempts, due })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_retry_waits_the_base_delay() {
        assert_eq!(backoff(0), BASE_BACKOFF);
    }

    #[test]
    fn each_further_attempt_doubles_the_wait_until_the_ceiling() {
        assert_eq!(backoff(1), BASE_BACKOFF * 2);
        assert_eq!(backoff(2), BASE_BACKOFF * 4);
        assert_eq!(backoff(8), MAX_BACKOFF);
        assert_eq!(backoff(20), MAX_BACKOFF);
    }

    #[test]
    fn the_backoff_never_overflows_on_an_extreme_attempt_count() {
        assert_eq!(backoff(u32::MAX), MAX_BACKOFF);
    }

    #[test]
    fn a_deferral_under_the_attempt_cap_reschedules_for_a_backoff_step() {
        let retry = RetrySchedule::first(1_000);
        let decision = next_after_deferral(&retry, &Expiry::Attempts(5), 1_000);
        match decision {
            RetryDecision::Retry(next) => {
                assert_eq!(next.attempts, 1);
                assert_eq!(next.due, 1_000 + BASE_BACKOFF.as_secs());
            }
            RetryDecision::Bounce => panic!("expected a retry, got a bounce"),
        }
    }

    #[test]
    fn the_wait_grows_across_successive_deferrals() {
        let first = RetrySchedule {
            attempts: 1,
            due: 0,
        };
        let decision = next_after_deferral(&first, &Expiry::Attempts(9), 2_000);
        match decision {
            RetryDecision::Retry(next) => {
                assert_eq!(next.attempts, 2);
                assert_eq!(next.due, 2_000 + (BASE_BACKOFF.as_secs() * 2));
            }
            RetryDecision::Bounce => panic!("expected a retry, got a bounce"),
        }
    }

    #[test]
    fn the_attempt_that_reaches_the_cap_bounces_the_recipient() {
        let retry = RetrySchedule {
            attempts: 4,
            due: 0,
        };
        let decision = next_after_deferral(&retry, &Expiry::Attempts(5), 5_000);
        assert!(decision.is_bounce());
        assert_eq!(decision.schedule(), None);
    }

    #[test]
    fn an_attempt_cap_grants_exactly_its_count_of_tries() {
        let expiry = Expiry::Attempts(3);
        let mut retry = RetrySchedule::first(0);
        let mut tries = 0;
        loop {
            match next_after_deferral(&retry, &expiry, 0) {
                RetryDecision::Retry(next) => {
                    tries += 1;
                    retry = next;
                }
                RetryDecision::Bounce => {
                    tries += 1;
                    break;
                }
            }
        }
        assert_eq!(tries, 3);
    }

    #[test]
    fn a_deferral_past_the_deadline_bounces_the_recipient() {
        let retry = RetrySchedule {
            attempts: 2,
            due: 0,
        };
        let decision = next_after_deferral(&retry, &Expiry::At(4_000), 4_000);
        assert!(decision.is_bounce());
    }

    #[test]
    fn a_deferral_before_the_deadline_reschedules() {
        let retry = RetrySchedule {
            attempts: 0,
            due: 0,
        };
        let decision = next_after_deferral(&retry, &Expiry::At(10_000), 1_000);
        match decision {
            RetryDecision::Retry(next) => {
                assert_eq!(next.attempts, 1);
                assert_eq!(next.due, 1_000 + BASE_BACKOFF.as_secs());
            }
            RetryDecision::Bounce => panic!("expected a retry, got a bounce"),
        }
    }
}
