#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassOutcome {
    Authenticated,
    Failed,
    Throttled,
    NeedUser,
    NeedTls,
    WrongState,
}

pub fn pass_response(outcome: PassOutcome) -> &'static [u8] {
    match outcome {
        PassOutcome::Authenticated => b"+OK mailbox ready\r\n",
        PassOutcome::Failed => b"-ERR authentication failed\r\n",
        PassOutcome::Throttled => b"-ERR [AUTH] too many failed authentication attempts\r\n",
        PassOutcome::NeedUser => b"-ERR send USER first\r\n",
        PassOutcome::NeedTls => b"-ERR [AUTH] STLS required before PASS\r\n",
        PassOutcome::WrongState => b"-ERR already in the transaction state\r\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_login_opens_the_mailbox() {
        assert!(pass_response(PassOutcome::Authenticated).starts_with(b"+OK"));
    }

    #[test]
    fn the_failure_paths_are_errors() {
        for outcome in [
            PassOutcome::Failed,
            PassOutcome::Throttled,
            PassOutcome::NeedUser,
            PassOutcome::NeedTls,
            PassOutcome::WrongState,
        ] {
            assert!(pass_response(outcome).starts_with(b"-ERR"));
        }
    }
}
