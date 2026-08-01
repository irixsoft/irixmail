use std::future::Future;
use std::time::Duration;

use irixmail_core::registry::Registry;

use crate::inspect::CertSummary;

pub struct RenewalSchedule {
    pub check_interval: Duration,
    pub renew_before: Duration,
}

impl Default for RenewalSchedule {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(12 * 60 * 60),
            renew_before: Duration::from_secs(30 * 24 * 60 * 60),
        }
    }
}

pub fn needs_renewal(not_after_unix: u64, now_unix: u64, renew_before: Duration) -> bool {
    not_after_unix.saturating_sub(now_unix) < renew_before.as_secs()
}

pub fn needs_issuance(
    summary: Option<&CertSummary>,
    now_unix: u64,
    renew_before: Duration,
) -> bool {
    match summary {
        Some(summary) => {
            summary.self_signed
                || needs_renewal(summary.not_after.max(0) as u64, now_unix, renew_before)
        }
        None => true,
    }
}

pub fn register_renewal<F, Fut>(registry: &Registry, schedule: RenewalSchedule, mut check: F)
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    registry.register_background("tls-renewal", move || async move {
        let mut ticker = tokio::time::interval(schedule.check_interval);
        loop {
            ticker.tick().await;
            check().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewal_is_due_only_within_the_window() {
        let before = Duration::from_secs(100);
        assert!(needs_renewal(150, 100, before));
        assert!(!needs_renewal(300, 100, before));
        assert!(needs_renewal(50, 100, before));
    }

    fn summary(not_after: i64, self_signed: bool) -> CertSummary {
        CertSummary {
            sans: vec!["mail.test".to_string()],
            not_after,
            issuer: "issuer".to_string(),
            self_signed,
        }
    }

    #[test]
    fn a_self_signed_certificate_is_due_despite_a_distant_expiry() {
        assert!(needs_issuance(
            Some(&summary(10_000, true)),
            100,
            Duration::from_secs(100)
        ));
    }

    #[test]
    fn a_ca_certificate_far_from_expiry_is_left_alone() {
        assert!(!needs_issuance(
            Some(&summary(10_000, false)),
            100,
            Duration::from_secs(100)
        ));
    }

    #[test]
    fn a_ca_certificate_inside_the_window_is_due() {
        assert!(needs_issuance(
            Some(&summary(150, false)),
            100,
            Duration::from_secs(100)
        ));
    }

    #[test]
    fn an_uninspectable_certificate_is_due() {
        assert!(needs_issuance(None, 100, Duration::from_secs(100)));
    }

    #[test]
    fn registering_appends_one_background_task() {
        let registry = Registry::new();
        register_renewal(&registry, RenewalSchedule::default(), || async {});
        assert_eq!(registry.len(), 1);
    }
}
