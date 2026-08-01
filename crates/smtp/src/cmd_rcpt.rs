use crate::parser::RcptParams;

const NEED_MAIL: &[u8] = b"503 5.5.1 Send MAIL before RCPT\r\n";
const TOO_MANY: &[u8] = b"452 4.5.3 Too many recipients\r\n";
const NO_MAILBOX: &[u8] = b"550 5.1.1 Mailbox does not exist\r\n";
const RELAY_DENIED: &[u8] = b"550 5.7.1 Relaying not allowed\r\n";
const ACCEPTED: &[u8] = b"250 2.1.5 Recipient OK\r\n";

pub const DEFAULT_MAX_RECIPIENTS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipient {
    Local,
    LocalUnknown,
    Remote,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForwardPath {
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RcptOutcome {
    Reject(&'static [u8]),
    Accept {
        path: ForwardPath,
        reply: &'static [u8],
    },
}

pub fn rcpt_reply(
    params: &RcptParams,
    sender_pending: bool,
    recipient: Recipient,
    authenticated: bool,
    accepted_count: usize,
    max_recipients: usize,
) -> RcptOutcome {
    if !sender_pending {
        return RcptOutcome::Reject(NEED_MAIL);
    }
    if max_recipients > 0 && accepted_count >= max_recipients {
        return RcptOutcome::Reject(TOO_MANY);
    }
    match recipient {
        Recipient::Local => RcptOutcome::Accept {
            path: ForwardPath {
                address: params.address.clone(),
            },
            reply: ACCEPTED,
        },
        Recipient::LocalUnknown => RcptOutcome::Reject(NO_MAILBOX),
        Recipient::Remote => {
            if authenticated {
                RcptOutcome::Accept {
                    path: ForwardPath {
                        address: params.address.clone(),
                    },
                    reply: ACCEPTED,
                }
            } else {
                RcptOutcome::Reject(RELAY_DENIED)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(address: &str) -> RcptParams {
        RcptParams {
            address: address.to_string(),
        }
    }

    #[test]
    fn a_local_recipient_is_accepted_when_the_mailbox_exists() {
        let outcome = rcpt_reply(
            &params("c@local.example"),
            true,
            Recipient::Local,
            false,
            0,
            DEFAULT_MAX_RECIPIENTS,
        );
        match outcome {
            RcptOutcome::Accept { path, reply } => {
                assert_eq!(path.address, "c@local.example");
                assert!(reply.starts_with(b"250"));
            }
            _ => panic!("expected the recipient to be accepted"),
        }
    }

    #[test]
    fn rcpt_before_a_sender_is_refused() {
        assert_eq!(
            rcpt_reply(&params("c@d.example"), false, Recipient::Local, false, 0, 0),
            RcptOutcome::Reject(NEED_MAIL)
        );
    }

    #[test]
    fn an_unknown_local_mailbox_is_rejected() {
        assert_eq!(
            rcpt_reply(
                &params("ghost@local.example"),
                true,
                Recipient::LocalUnknown,
                false,
                0,
                0
            ),
            RcptOutcome::Reject(NO_MAILBOX)
        );
    }

    #[test]
    fn an_unauthenticated_remote_recipient_is_refused_as_relaying() {
        assert_eq!(
            rcpt_reply(
                &params("user@remote.example"),
                true,
                Recipient::Remote,
                false,
                0,
                0
            ),
            RcptOutcome::Reject(RELAY_DENIED)
        );
    }

    #[test]
    fn an_authenticated_session_may_relay_to_a_remote_recipient() {
        let outcome = rcpt_reply(
            &params("user@remote.example"),
            true,
            Recipient::Remote,
            true,
            0,
            DEFAULT_MAX_RECIPIENTS,
        );
        assert!(matches!(outcome, RcptOutcome::Accept { .. }));
    }

    #[test]
    fn a_full_recipient_list_is_refused() {
        assert_eq!(
            rcpt_reply(
                &params("c@local.example"),
                true,
                Recipient::Local,
                false,
                100,
                100
            ),
            RcptOutcome::Reject(TOO_MANY)
        );
    }

    #[test]
    fn a_recipient_below_the_cap_is_still_accepted() {
        let outcome = rcpt_reply(
            &params("c@local.example"),
            true,
            Recipient::Local,
            false,
            99,
            100,
        );
        assert!(matches!(outcome, RcptOutcome::Accept { .. }));
    }

    #[test]
    fn an_unlimited_cap_skips_the_recipient_count_check() {
        let outcome = rcpt_reply(
            &params("c@local.example"),
            true,
            Recipient::Local,
            false,
            10_000,
            0,
        );
        assert!(matches!(outcome, RcptOutcome::Accept { .. }));
    }
}
