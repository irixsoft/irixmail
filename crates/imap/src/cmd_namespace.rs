pub const NAMESPACE_LINE: &str = "* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n";

pub fn namespace_completion(tag: &str) -> String {
    format!("{tag} OK NAMESPACE completed\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_personal_namespace_uses_the_slash_delimiter() {
        assert!(NAMESPACE_LINE.starts_with("* NAMESPACE ((\"\" \"/\"))"));
        assert!(NAMESPACE_LINE.ends_with("NIL NIL\r\n"));
    }

    #[test]
    fn the_completion_echoes_the_tag() {
        assert_eq!(namespace_completion("a"), "a OK NAMESPACE completed\r\n");
    }
}
