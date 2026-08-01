pub const CONTINUE: &str = "+ idling\r\n";

// Slightly under RFC 2177's 29-minute client re-issue guidance.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(28 * 60);

pub const IDLE_TIMED_OUT: &str = "* BYE IDLE timed out\r\n";

pub fn idle_done(line: &[u8]) -> bool {
    let trimmed = line
        .iter()
        .copied()
        .take_while(|byte| !matches!(byte, b'\r' | b'\n'))
        .collect::<Vec<u8>>();
    trimmed.eq_ignore_ascii_case(b"DONE")
}

pub fn idle_completion(tag: &str) -> String {
    format!("{tag} OK IDLE completed\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_is_recognized_case_insensitively() {
        assert!(idle_done(b"DONE\r\n"));
        assert!(idle_done(b"done\r\n"));
        assert!(idle_done(b"DoNe"));
    }

    #[test]
    fn other_lines_do_not_end_the_idle() {
        assert!(!idle_done(b"a NOOP\r\n"));
        assert!(!idle_done(b"\r\n"));
    }

    #[test]
    fn the_completion_echoes_the_tag() {
        assert_eq!(idle_completion("a"), "a OK IDLE completed\r\n");
    }
}
