const LOOPING: &[u8] = b"450 4.4.6 Too many Received headers, a mail loop is suspected\r\n";

pub const DEFAULT_MAX_RECEIVED: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopConfig {
    pub max_received: usize,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_received: DEFAULT_MAX_RECEIVED,
        }
    }
}

impl LoopConfig {
    pub fn is_disabled(&self) -> bool {
        self.max_received == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopDecision {
    Allow,
    Reject(&'static [u8]),
}

impl LoopDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, LoopDecision::Allow)
    }
}

pub fn check(raw_message: &[u8], config: LoopConfig) -> LoopDecision {
    if config.is_disabled() {
        return LoopDecision::Allow;
    }
    if received_count(raw_message) > config.max_received {
        LoopDecision::Reject(LOOPING)
    } else {
        LoopDecision::Allow
    }
}

pub fn received_count(raw_message: &[u8]) -> usize {
    let mut count = 0;
    for line in raw_message.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line[0] == b' ' || line[0] == b'\t' {
            continue;
        }
        if is_received_field(line) {
            count += 1;
        }
    }
    count
}

fn is_received_field(line: &[u8]) -> bool {
    match line.iter().position(|&byte| byte == b':') {
        Some(colon) => line[..colon].eq_ignore_ascii_case(b"received"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_with_no_trace_headers_counts_zero() {
        let raw = b"From: a@b.example\r\nSubject: hi\r\n\r\nbody\r\n";
        assert_eq!(received_count(raw), 0);
    }

    #[test]
    fn each_received_header_is_tallied() {
        let raw = b"Received: from a\r\nReceived: from b\r\nReceived: from c\r\nFrom: a@b.example\r\n\r\nbody\r\n";
        assert_eq!(received_count(raw), 3);
    }

    #[test]
    fn the_field_name_is_matched_without_regard_to_case() {
        let raw = b"RECEIVED: from a\r\nreceived: from b\r\nReCeIvEd: from c\r\n\r\n";
        assert_eq!(received_count(raw), 3);
    }

    #[test]
    fn a_folded_continuation_line_is_not_counted_again() {
        let raw = b"Received: from relay.example\r\n\tby host.example with ESMTP\r\n id 1234; date\r\nFrom: a@b.example\r\n\r\n";
        assert_eq!(received_count(raw), 1);
    }

    #[test]
    fn a_received_token_in_the_body_is_ignored() {
        let raw = b"Received: from a\r\n\r\nReceived: this is body text\r\nReceived: more body\r\n";
        assert_eq!(received_count(raw), 1);
    }

    #[test]
    fn a_field_whose_name_merely_starts_with_received_is_not_a_trace_header() {
        let raw = b"Received-SPF: pass\r\nReceivedX: nope\r\n\r\n";
        assert_eq!(received_count(raw), 0);
    }

    #[test]
    fn bare_lf_line_endings_are_tolerated() {
        let raw = b"Received: from a\nReceived: from b\n\nbody\n";
        assert_eq!(received_count(raw), 2);
    }

    #[test]
    fn a_count_within_the_ceiling_is_admitted() {
        let raw = b"Received: from a\r\nReceived: from b\r\n\r\n";
        let config = LoopConfig { max_received: 5 };
        assert_eq!(check(raw, config), LoopDecision::Allow);
        assert!(check(raw, config).is_allowed());
    }

    #[test]
    fn a_count_at_the_ceiling_is_still_admitted() {
        let mut raw = Vec::new();
        for _ in 0..5 {
            raw.extend_from_slice(b"Received: from a\r\n");
        }
        raw.extend_from_slice(b"\r\n");
        let config = LoopConfig { max_received: 5 };
        assert_eq!(received_count(&raw), 5);
        assert!(check(&raw, config).is_allowed());
    }

    #[test]
    fn a_count_over_the_ceiling_is_refused_with_a_transient_negative() {
        let mut raw = Vec::new();
        for _ in 0..6 {
            raw.extend_from_slice(b"Received: from a\r\n");
        }
        raw.extend_from_slice(b"\r\n");
        let config = LoopConfig { max_received: 5 };
        match check(&raw, config) {
            LoopDecision::Reject(reply) => assert!(reply.starts_with(b"450")),
            LoopDecision::Allow => panic!("expected the looping message to be refused"),
        }
    }

    #[test]
    fn a_disabled_ceiling_admits_any_count() {
        let mut raw = Vec::new();
        for _ in 0..200 {
            raw.extend_from_slice(b"Received: from a\r\n");
        }
        raw.extend_from_slice(b"\r\n");
        let config = LoopConfig { max_received: 0 };
        assert!(config.is_disabled());
        assert!(check(&raw, config).is_allowed());
    }

    #[test]
    fn the_default_ceiling_is_fifty() {
        assert_eq!(LoopConfig::default().max_received, DEFAULT_MAX_RECEIVED);
        assert_eq!(DEFAULT_MAX_RECEIVED, 50);
        assert!(!LoopConfig::default().is_disabled());
    }
}
