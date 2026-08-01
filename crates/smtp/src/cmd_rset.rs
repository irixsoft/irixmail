const RESET_OK: &[u8] = b"250 2.0.0 Reset OK\r\n";

pub fn rset_reply() -> &'static [u8] {
    RESET_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reset_is_acknowledged() {
        assert_eq!(rset_reply(), RESET_OK);
    }

    #[test]
    fn the_reply_is_a_positive_completion() {
        assert!(rset_reply().starts_with(b"250"));
    }
}
