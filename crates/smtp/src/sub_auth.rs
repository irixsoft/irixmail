const AUTH_REQUIRED: &[u8] = b"530 5.7.0 Authentication required\r\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionGate {
    Proceed,
    Reject(&'static [u8]),
}

pub fn guard_submission(authenticated: bool) -> SubmissionGate {
    if authenticated {
        SubmissionGate::Proceed
    } else {
        SubmissionGate::Reject(AUTH_REQUIRED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unauthenticated_session_is_refused() {
        assert_eq!(
            guard_submission(false),
            SubmissionGate::Reject(AUTH_REQUIRED)
        );
    }

    #[test]
    fn an_authenticated_session_proceeds() {
        assert_eq!(guard_submission(true), SubmissionGate::Proceed);
    }

    #[test]
    fn the_refusal_demands_authentication() {
        let SubmissionGate::Reject(reply) = guard_submission(false) else {
            panic!("expected a refusal");
        };
        assert!(reply.starts_with(b"530"));
    }
}
