const NOOP_OK: &[u8] = b"250 2.0.0 OK\r\n";

pub fn noop_reply() -> &'static [u8] {
    NOOP_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_noop_is_acknowledged() {
        assert_eq!(noop_reply(), NOOP_OK);
    }

    #[test]
    fn the_reply_is_a_positive_completion() {
        assert!(noop_reply().starts_with(b"250"));
    }
}
