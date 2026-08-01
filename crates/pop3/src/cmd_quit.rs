pub fn quit_response() -> &'static [u8] {
    b"+OK IRIXMAIL POP3 signing off\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_farewell_is_ok() {
        assert!(quit_response().starts_with(b"+OK"));
    }
}
