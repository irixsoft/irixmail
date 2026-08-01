use crate::parser::MailParams;

const NEED_GREETING: &[u8] = b"503 5.5.1 Send EHLO/HELO first\r\n";
const NESTED_MAIL: &[u8] = b"503 5.5.1 Sender already given\r\n";
const TOO_LARGE: &[u8] = b"552 5.3.4 Message size exceeds the fixed limit\r\n";
const ACCEPTED: &[u8] = b"250 2.1.0 Sender OK\r\n";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReversePath {
    pub address: String,
    pub smtputf8: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailOutcome {
    Reject(&'static [u8]),
    Accept {
        path: ReversePath,
        reply: &'static [u8],
    },
}

pub fn mail_reply(
    params: &MailParams,
    greeted: bool,
    sender_pending: bool,
    max_message_size: usize,
) -> MailOutcome {
    if !greeted {
        return MailOutcome::Reject(NEED_GREETING);
    }
    if sender_pending {
        return MailOutcome::Reject(NESTED_MAIL);
    }
    if params.size > 0 && max_message_size > 0 && params.size > max_message_size {
        return MailOutcome::Reject(TOO_LARGE);
    }
    MailOutcome::Accept {
        path: ReversePath {
            address: params.address.clone(),
            smtputf8: params.smtputf8,
        },
        reply: ACCEPTED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(address: &str) -> MailParams {
        MailParams {
            address: address.to_string(),
            size: 0,
            body_8bitmime: false,
            smtputf8: false,
        }
    }

    #[test]
    fn a_sender_is_accepted_after_a_greeting() {
        let outcome = mail_reply(&params("a@b.example"), true, false, 1024);
        match outcome {
            MailOutcome::Accept { path, reply } => {
                assert_eq!(path.address, "a@b.example");
                assert!(!path.smtputf8);
                assert!(reply.starts_with(b"250"));
            }
            _ => panic!("expected the sender to be accepted"),
        }
    }

    #[test]
    fn a_null_sender_is_accepted_with_an_empty_address() {
        let outcome = mail_reply(&params(""), true, false, 0);
        match outcome {
            MailOutcome::Accept { path, .. } => assert!(path.address.is_empty()),
            _ => panic!("expected the null sender to be accepted"),
        }
    }

    #[test]
    fn mail_before_a_greeting_is_refused() {
        assert_eq!(
            mail_reply(&params("a@b.example"), false, false, 0),
            MailOutcome::Reject(NEED_GREETING)
        );
    }

    #[test]
    fn a_second_sender_is_refused() {
        assert_eq!(
            mail_reply(&params("a@b.example"), true, true, 0),
            MailOutcome::Reject(NESTED_MAIL)
        );
    }

    #[test]
    fn a_size_over_the_limit_is_rejected() {
        let mut params = params("a@b.example");
        params.size = 2048;
        assert_eq!(
            mail_reply(&params, true, false, 1024),
            MailOutcome::Reject(TOO_LARGE)
        );
    }

    #[test]
    fn a_size_within_the_limit_is_accepted() {
        let mut params = params("a@b.example");
        params.size = 1000;
        assert!(matches!(
            mail_reply(&params, true, false, 1024),
            MailOutcome::Accept { .. }
        ));
    }

    #[test]
    fn a_size_equal_to_the_limit_is_accepted() {
        let mut params = params("a@b.example");
        params.size = 1024;
        assert!(matches!(
            mail_reply(&params, true, false, 1024),
            MailOutcome::Accept { .. }
        ));
    }

    #[test]
    fn an_unannounced_size_skips_the_limit_check() {
        let outcome = mail_reply(&params("a@b.example"), true, false, 0);
        assert!(matches!(outcome, MailOutcome::Accept { .. }));
    }

    #[test]
    fn the_smtputf8_flag_is_carried_onto_the_reverse_path() {
        let mut params = params("u@\u{00fc}ber.example");
        params.smtputf8 = true;
        match mail_reply(&params, true, false, 0) {
            MailOutcome::Accept { path, .. } => assert!(path.smtputf8),
            _ => panic!("expected acceptance"),
        }
    }
}
