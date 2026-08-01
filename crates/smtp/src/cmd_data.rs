const NEED_RCPT: &[u8] = b"503 5.5.1 Need RCPT before DATA\r\n";
const READY: &[u8] = b"354 Start mail input; end with <CRLF>.<CRLF>\r\n";
const TOO_LARGE: &[u8] = b"552 5.3.4 Message size exceeds the fixed limit\r\n";
const ACCEPTED: &[u8] = b"250 2.0.0 Message accepted\r\n";
const MAILBOX_FULL: &[u8] = b"452 4.2.2 Mailbox full, try again later\r\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataOutcome {
    Reject(&'static [u8]),
    Ready(&'static [u8]),
}

pub fn data_reply(has_recipient: bool) -> DataOutcome {
    if has_recipient {
        DataOutcome::Ready(READY)
    } else {
        DataOutcome::Reject(NEED_RCPT)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyStep {
    Continue,
    Complete,
    TooLarge,
}

#[derive(Default)]
pub struct BodyReceiver {
    body: Vec<u8>,
    max_size: usize,
}

impl BodyReceiver {
    pub fn new(max_size: usize) -> Self {
        Self {
            body: Vec::new(),
            max_size,
        }
    }

    pub fn push_line(&mut self, line: &[u8]) -> BodyStep {
        if line == b"." {
            return BodyStep::Complete;
        }
        let content = if line.first() == Some(&b'.') {
            &line[1..]
        } else {
            line
        };
        if self.max_size > 0 && self.body.len() + content.len() + 2 > self.max_size {
            return BodyStep::TooLarge;
        }
        self.body.extend_from_slice(content);
        self.body.extend_from_slice(b"\r\n");
        BodyStep::Continue
    }

    pub fn len(&self) -> usize {
        self.body.len()
    }

    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    pub fn into_body(self) -> Vec<u8> {
        self.body
    }
}

pub fn accepted_reply() -> &'static [u8] {
    ACCEPTED
}

pub fn too_large_reply() -> &'static [u8] {
    TOO_LARGE
}

pub fn mailbox_full_reply() -> &'static [u8] {
    MAILBOX_FULL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_without_a_recipient_is_refused() {
        assert_eq!(data_reply(false), DataOutcome::Reject(NEED_RCPT));
    }

    #[test]
    fn data_with_a_recipient_invites_the_body() {
        match data_reply(true) {
            DataOutcome::Ready(reply) => assert!(reply.starts_with(b"354")),
            _ => panic!("expected the body input to be invited"),
        }
    }

    #[test]
    fn a_plain_body_is_collected_with_crlf_endings() {
        let mut receiver = BodyReceiver::new(0);
        assert_eq!(receiver.push_line(b"hello"), BodyStep::Continue);
        assert_eq!(receiver.push_line(b"world"), BodyStep::Continue);
        assert_eq!(receiver.push_line(b"."), BodyStep::Complete);
        assert_eq!(receiver.into_body(), b"hello\r\nworld\r\n");
    }

    #[test]
    fn a_leading_dot_is_unstuffed() {
        let mut receiver = BodyReceiver::new(0);
        assert_eq!(receiver.push_line(b"..dotted"), BodyStep::Continue);
        assert_eq!(receiver.push_line(b"."), BodyStep::Complete);
        assert_eq!(receiver.into_body(), b".dotted\r\n");
    }

    #[test]
    fn an_empty_body_terminates_immediately() {
        let mut receiver = BodyReceiver::new(0);
        assert_eq!(receiver.push_line(b"."), BodyStep::Complete);
        assert!(receiver.is_empty());
        assert_eq!(receiver.into_body(), b"");
    }

    #[test]
    fn a_blank_line_is_preserved_as_a_crlf() {
        let mut receiver = BodyReceiver::new(0);
        assert_eq!(receiver.push_line(b"head"), BodyStep::Continue);
        assert_eq!(receiver.push_line(b""), BodyStep::Continue);
        assert_eq!(receiver.push_line(b"body"), BodyStep::Continue);
        receiver.push_line(b".");
        assert_eq!(receiver.into_body(), b"head\r\n\r\nbody\r\n");
    }

    #[test]
    fn a_body_over_the_limit_is_reported_too_large() {
        let mut receiver = BodyReceiver::new(8);
        assert_eq!(receiver.push_line(b"ok"), BodyStep::Continue);
        assert_eq!(receiver.push_line(b"overflowing"), BodyStep::TooLarge);
    }

    #[test]
    fn a_body_within_the_limit_keeps_accumulating() {
        let mut receiver = BodyReceiver::new(16);
        assert_eq!(receiver.push_line(b"four"), BodyStep::Continue);
        assert_eq!(receiver.push_line(b"five"), BodyStep::Continue);
        assert_eq!(receiver.len(), 12);
    }

    #[test]
    fn a_zero_limit_leaves_the_body_unbounded() {
        let mut receiver = BodyReceiver::new(0);
        for _ in 0..1000 {
            assert_eq!(receiver.push_line(b"line"), BodyStep::Continue);
        }
        assert_eq!(receiver.len(), 6000);
    }
}
