pub fn noop_response(tag: &str) -> String {
    format!("{tag} OK NOOP completed\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_echoes_the_tag_with_a_tagged_ok() {
        assert_eq!(noop_response("a1"), "a1 OK NOOP completed\r\n");
    }

    #[test]
    fn the_response_is_a_single_tagged_line() {
        let response = noop_response("x");
        assert!(response.starts_with("x OK"));
        assert_eq!(response.matches("\r\n").count(), 1);
    }
}
