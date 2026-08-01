use irixmail_directory::Account;
use irixmail_store::{QuotaLimits, QuotaUsage};

const MESSAGES_PER_DELIVERY: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaVerdict {
    Accepted,
    OverByteQuota { limit: u64, would_use: u64 },
    OverMessageQuota { limit: u64, would_use: u64 },
}

impl QuotaVerdict {
    pub fn is_accepted(self) -> bool {
        matches!(self, QuotaVerdict::Accepted)
    }

    pub fn is_over_quota(self) -> bool {
        !self.is_accepted()
    }
}

pub fn limits_for(account: &Account) -> QuotaLimits {
    QuotaLimits {
        bytes: account.quota_bytes,
        messages: account.quota_messages,
    }
}

pub fn enforce_quota(limits: QuotaLimits, usage: QuotaUsage, message_size: u64) -> QuotaVerdict {
    if !limits.is_bounded() {
        return QuotaVerdict::Accepted;
    }

    if limits.bytes != 0 {
        let would_use = usage.bytes.saturating_add(message_size);
        if would_use > limits.bytes {
            return QuotaVerdict::OverByteQuota {
                limit: limits.bytes,
                would_use,
            };
        }
    }

    if limits.messages != 0 {
        let would_use = usage.messages.saturating_add(MESSAGES_PER_DELIVERY);
        if would_use > limits.messages {
            return QuotaVerdict::OverMessageQuota {
                limit: limits.messages,
                would_use,
            };
        }
    }

    QuotaVerdict::Accepted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account_with_quota(quota_bytes: u64, quota_messages: u64) -> Account {
        Account {
            id: 1,
            local_part: "alice".to_string(),
            domain_id: 1,
            display_name: String::new(),
            enabled: true,
            role: irixmail_directory::Role::User,
            aliases: Vec::new(),
            forwarding: irixmail_directory::Forwarding::default(),
            quota_bytes,
            quota_messages,
            locale: String::new(),
            timezone: String::new(),
            signature: String::new(),
            vacation: irixmail_directory::VacationResponder::default(),
            created_at: 0,
        }
    }

    fn usage(bytes: u64, messages: u64) -> QuotaUsage {
        QuotaUsage { bytes, messages }
    }

    #[test]
    fn limits_carry_the_account_ceilings_through_unchanged() {
        let account = account_with_quota(2048, 50);
        let limits = limits_for(&account);
        assert_eq!(limits.bytes, 2048);
        assert_eq!(limits.messages, 50);
    }

    #[test]
    fn a_zero_ceiling_translates_to_an_unbounded_limit() {
        let account = account_with_quota(0, 0);
        let limits = limits_for(&account);
        assert!(!limits.is_bounded());
    }

    #[test]
    fn an_unbounded_account_accepts_any_message() {
        let limits = limits_for(&account_with_quota(0, 0));
        let verdict = enforce_quota(limits, usage(u64::MAX - 1, u64::MAX - 1), u64::MAX);
        assert_eq!(verdict, QuotaVerdict::Accepted);
        assert!(verdict.is_accepted());
        assert!(!verdict.is_over_quota());
    }

    #[test]
    fn a_message_filling_the_byte_ceiling_exactly_is_accepted() {
        let limits = QuotaLimits {
            bytes: 1000,
            messages: 0,
        };
        assert_eq!(
            enforce_quota(limits, usage(900, 1), 100),
            QuotaVerdict::Accepted
        );
    }

    #[test]
    fn a_message_one_byte_over_the_ceiling_is_refused() {
        let limits = QuotaLimits {
            bytes: 1000,
            messages: 0,
        };
        let verdict = enforce_quota(limits, usage(900, 1), 101);
        assert_eq!(
            verdict,
            QuotaVerdict::OverByteQuota {
                limit: 1000,
                would_use: 1001,
            }
        );
        assert!(verdict.is_over_quota());
    }

    #[test]
    fn a_message_filling_the_message_ceiling_exactly_is_accepted() {
        let limits = QuotaLimits {
            bytes: 0,
            messages: 5,
        };
        assert_eq!(
            enforce_quota(limits, usage(10, 4), 50),
            QuotaVerdict::Accepted
        );
    }

    #[test]
    fn a_message_past_the_message_ceiling_is_refused() {
        let limits = QuotaLimits {
            bytes: 0,
            messages: 5,
        };
        let verdict = enforce_quota(limits, usage(10, 5), 50);
        assert_eq!(
            verdict,
            QuotaVerdict::OverMessageQuota {
                limit: 5,
                would_use: 6,
            }
        );
    }

    #[test]
    fn both_ceilings_must_have_room_for_the_message() {
        let limits = QuotaLimits {
            bytes: 1000,
            messages: 3,
        };
        assert!(enforce_quota(limits, usage(500, 2), 400).is_accepted());
        assert!(enforce_quota(limits, usage(500, 2), 600).is_over_quota());
        assert!(enforce_quota(limits, usage(500, 3), 100).is_over_quota());
    }

    #[test]
    fn the_byte_dimension_is_reported_when_both_overflow() {
        let limits = QuotaLimits {
            bytes: 1000,
            messages: 3,
        };
        let verdict = enforce_quota(limits, usage(950, 3), 100);
        assert_eq!(
            verdict,
            QuotaVerdict::OverByteQuota {
                limit: 1000,
                would_use: 1050,
            }
        );
    }

    #[test]
    fn an_oversize_message_reports_over_quota_rather_than_wrapping() {
        let limits = QuotaLimits {
            bytes: 1000,
            messages: 0,
        };
        let verdict = enforce_quota(limits, usage(10, 1), u64::MAX);
        assert_eq!(
            verdict,
            QuotaVerdict::OverByteQuota {
                limit: 1000,
                would_use: u64::MAX,
            }
        );
    }

    #[test]
    fn a_full_account_refuses_a_message_of_no_size() {
        let limits = QuotaLimits {
            bytes: 0,
            messages: 2,
        };
        let verdict = enforce_quota(limits, usage(0, 2), 0);
        assert_eq!(
            verdict,
            QuotaVerdict::OverMessageQuota {
                limit: 2,
                would_use: 3,
            }
        );
    }

    #[test]
    fn enforce_quota_drives_off_the_account_limits() {
        let account = account_with_quota(1000, 0);
        let limits = limits_for(&account);
        assert!(enforce_quota(limits, usage(900, 1), 100).is_accepted());
        assert!(enforce_quota(limits, usage(900, 1), 200).is_over_quota());
    }
}
