#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserOutcome {
    Accepted,
    Empty,
    WrongState,
}

pub fn user_response(outcome: UserOutcome) -> &'static [u8] {
    match outcome {
        UserOutcome::Accepted => b"+OK send PASS\r\n",
        UserOutcome::Empty => b"-ERR username required\r\n",
        UserOutcome::WrongState => b"-ERR already in the transaction state\r\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accepted_user_is_asked_for_a_password() {
        assert_eq!(user_response(UserOutcome::Accepted), b"+OK send PASS\r\n");
    }

    #[test]
    fn an_empty_username_is_refused() {
        assert!(user_response(UserOutcome::Empty).starts_with(b"-ERR"));
    }

    #[test]
    fn a_user_after_login_is_refused() {
        assert!(user_response(UserOutcome::WrongState).starts_with(b"-ERR"));
    }
}
