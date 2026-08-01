pub fn noop_response() -> &'static [u8] {
    b"+OK\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_is_acknowledged() {
        assert_eq!(noop_response(), b"+OK\r\n");
    }
}
