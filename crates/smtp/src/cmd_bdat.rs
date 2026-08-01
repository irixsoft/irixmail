const NEED_RCPT: &[u8] = b"503 5.5.1 Need RCPT before BDAT\r\n";
const TOO_LARGE: &[u8] = b"552 5.3.4 Message size exceeds the fixed limit\r\n";
const CHUNK_OK: &[u8] = b"250 2.6.0 Chunk accepted\r\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BdatOutcome {
    Reject(&'static [u8]),
    TooLarge(&'static [u8]),
    Receive { chunk_size: usize, is_last: bool },
}

pub fn bdat_reply(
    has_recipient: bool,
    chunk_size: usize,
    accumulated: usize,
    max_size: usize,
    is_last: bool,
) -> BdatOutcome {
    if !has_recipient {
        BdatOutcome::Reject(NEED_RCPT)
    } else if max_size > 0 && accumulated.saturating_add(chunk_size) > max_size {
        BdatOutcome::TooLarge(TOO_LARGE)
    } else {
        BdatOutcome::Receive {
            chunk_size,
            is_last,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkStep {
    Accepted,
    Complete,
    TooLarge,
}

#[derive(Default)]
pub struct ChunkReceiver {
    message: Vec<u8>,
    max_size: usize,
}

impl ChunkReceiver {
    pub fn new(max_size: usize) -> Self {
        Self {
            message: Vec::new(),
            max_size,
        }
    }

    pub fn push_chunk(&mut self, chunk: &[u8], is_last: bool) -> ChunkStep {
        if self.max_size > 0 && self.message.len() + chunk.len() > self.max_size {
            return ChunkStep::TooLarge;
        }
        self.message.extend_from_slice(chunk);
        if is_last {
            ChunkStep::Complete
        } else {
            ChunkStep::Accepted
        }
    }

    pub fn len(&self) -> usize {
        self.message.len()
    }

    pub fn is_empty(&self) -> bool {
        self.message.is_empty()
    }

    pub fn into_message(self) -> Vec<u8> {
        self.message
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkDisposal {
    Drain(usize),
    Close,
}

pub fn chunk_disposal(chunk_size: usize, max_size: usize) -> ChunkDisposal {
    if max_size > 0 && chunk_size > max_size {
        ChunkDisposal::Close
    } else {
        ChunkDisposal::Drain(chunk_size)
    }
}

pub fn chunk_ok_reply() -> &'static [u8] {
    CHUNK_OK
}

pub fn too_large_reply() -> &'static [u8] {
    TOO_LARGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bdat_without_a_recipient_is_refused() {
        assert_eq!(
            bdat_reply(false, 16, 0, 64, false),
            BdatOutcome::Reject(NEED_RCPT)
        );
    }

    #[test]
    fn bdat_with_a_recipient_receives_the_declared_bytes() {
        assert_eq!(
            bdat_reply(true, 16, 0, 64, true),
            BdatOutcome::Receive {
                chunk_size: 16,
                is_last: true
            }
        );
    }

    #[test]
    fn an_oversized_declared_chunk_is_refused_before_receipt() {
        assert_eq!(
            bdat_reply(true, 65, 0, 64, true),
            BdatOutcome::TooLarge(TOO_LARGE)
        );
    }

    #[test]
    fn the_declared_bound_counts_previously_accumulated_chunks() {
        assert_eq!(
            bdat_reply(true, 33, 32, 64, false),
            BdatOutcome::TooLarge(TOO_LARGE)
        );
        assert_eq!(
            bdat_reply(true, 32, 32, 64, false),
            BdatOutcome::Receive {
                chunk_size: 32,
                is_last: false
            }
        );
    }

    #[test]
    fn a_declared_size_near_usize_max_does_not_wrap_past_the_bound() {
        assert_eq!(
            bdat_reply(true, usize::MAX, 32, 64, true),
            BdatOutcome::TooLarge(TOO_LARGE)
        );
    }

    #[test]
    fn a_zero_max_size_leaves_the_declared_size_unbounded() {
        assert_eq!(
            bdat_reply(true, usize::MAX, 0, 0, true),
            BdatOutcome::Receive {
                chunk_size: usize::MAX,
                is_last: true
            }
        );
    }

    #[test]
    fn a_refused_chunk_within_the_limit_is_drained() {
        assert_eq!(chunk_disposal(64, 64), ChunkDisposal::Drain(64));
    }

    #[test]
    fn a_refused_chunk_over_the_limit_closes_the_connection() {
        assert_eq!(chunk_disposal(65, 64), ChunkDisposal::Close);
    }

    #[test]
    fn a_zero_max_size_always_drains() {
        assert_eq!(
            chunk_disposal(usize::MAX, 0),
            ChunkDisposal::Drain(usize::MAX)
        );
    }

    #[test]
    fn chunks_are_concatenated_verbatim() {
        let mut receiver = ChunkReceiver::new(0);
        assert_eq!(receiver.push_chunk(b"hello ", false), ChunkStep::Accepted);
        assert_eq!(receiver.push_chunk(b"world", true), ChunkStep::Complete);
        assert_eq!(receiver.into_message(), b"hello world");
    }

    #[test]
    fn a_leading_dot_is_kept_intact() {
        let mut receiver = ChunkReceiver::new(0);
        assert_eq!(
            receiver.push_chunk(b".not stuffed\r\n", true),
            ChunkStep::Complete
        );
        assert_eq!(receiver.into_message(), b".not stuffed\r\n");
    }

    #[test]
    fn a_single_last_chunk_completes_immediately() {
        let mut receiver = ChunkReceiver::new(0);
        assert_eq!(receiver.push_chunk(b"body", true), ChunkStep::Complete);
        assert_eq!(receiver.into_message(), b"body");
    }

    #[test]
    fn an_empty_last_chunk_completes_an_empty_message() {
        let mut receiver = ChunkReceiver::new(0);
        assert_eq!(receiver.push_chunk(b"", true), ChunkStep::Complete);
        assert!(receiver.is_empty());
        assert_eq!(receiver.into_message(), b"");
    }

    #[test]
    fn a_message_over_the_limit_is_reported_too_large() {
        let mut receiver = ChunkReceiver::new(8);
        assert_eq!(receiver.push_chunk(b"abcd", false), ChunkStep::Accepted);
        assert_eq!(receiver.push_chunk(b"efghij", true), ChunkStep::TooLarge);
        assert_eq!(receiver.len(), 4);
    }

    #[test]
    fn a_message_within_the_limit_keeps_accumulating() {
        let mut receiver = ChunkReceiver::new(16);
        assert_eq!(receiver.push_chunk(b"four", false), ChunkStep::Accepted);
        assert_eq!(receiver.push_chunk(b"five", false), ChunkStep::Accepted);
        assert_eq!(receiver.len(), 8);
    }

    #[test]
    fn a_zero_limit_leaves_the_message_unbounded() {
        let mut receiver = ChunkReceiver::new(0);
        for _ in 0..1000 {
            assert_eq!(receiver.push_chunk(b"chunk", false), ChunkStep::Accepted);
        }
        assert_eq!(receiver.len(), 5000);
    }
}
