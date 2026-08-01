const QUIT_OK: &[u8] = b"221 2.0.0 Bye\r\n";

pub fn quit_reply() -> &'static [u8] {
    QUIT_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quit_is_acknowledged() {
        assert_eq!(quit_reply(), QUIT_OK);
    }

    #[test]
    fn the_reply_is_a_closing_completion() {
        assert!(quit_reply().starts_with(b"221"));
    }
}
