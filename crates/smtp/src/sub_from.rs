const FROM_NOT_OWNED: &[u8] =
    b"550 5.7.1 Sender address not owned by the authenticated account\r\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipGate {
    Proceed,
    Reject(&'static [u8]),
}

impl OwnershipGate {
    pub fn is_allowed(&self) -> bool {
        matches!(self, OwnershipGate::Proceed)
    }
}

pub fn guard_from<'a>(declared: &str, owned: impl IntoIterator<Item = &'a str>) -> OwnershipGate {
    if declared.is_empty() {
        return OwnershipGate::Proceed;
    }
    for address in owned {
        if address.eq_ignore_ascii_case(declared) {
            return OwnershipGate::Proceed;
        }
    }
    OwnershipGate::Reject(FROM_NOT_OWNED)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNED: &[&str] = &["alice@irixsoft.com", "a.adams@irixsoft.com"];

    #[test]
    fn the_primary_address_is_owned() {
        assert_eq!(
            guard_from("alice@irixsoft.com", OWNED.iter().copied()),
            OwnershipGate::Proceed
        );
    }

    #[test]
    fn an_alias_is_owned() {
        assert_eq!(
            guard_from("a.adams@irixsoft.com", OWNED.iter().copied()),
            OwnershipGate::Proceed
        );
    }

    #[test]
    fn ownership_ignores_letter_case() {
        assert_eq!(
            guard_from("Alice@IriXSoft.CoM", OWNED.iter().copied()),
            OwnershipGate::Proceed
        );
    }

    #[test]
    fn a_foreign_address_is_refused() {
        let gate = guard_from("mallory@example.org", OWNED.iter().copied());
        assert_eq!(gate, OwnershipGate::Reject(FROM_NOT_OWNED));
        assert!(!gate.is_allowed());
    }

    #[test]
    fn a_null_sender_is_permitted_for_bounces() {
        assert_eq!(
            guard_from("", OWNED.iter().copied()),
            OwnershipGate::Proceed
        );
    }

    #[test]
    fn an_account_with_no_addresses_owns_nothing() {
        assert_eq!(
            guard_from("alice@irixsoft.com", std::iter::empty()),
            OwnershipGate::Reject(FROM_NOT_OWNED)
        );
    }

    #[test]
    fn the_refusal_is_a_permanent_policy_failure() {
        let OwnershipGate::Reject(reply) = guard_from("x@y.example", std::iter::empty()) else {
            panic!("expected a refusal");
        };
        assert!(reply.starts_with(b"550"));
    }
}
