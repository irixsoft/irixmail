pub const BYE: &str = "* BYE IRIXMAIL logging out\r\n";

pub fn logout_response(tag: &str) -> String {
    format!("{tag} OK LOGOUT completed\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bye_is_untagged() {
        assert!(BYE.starts_with("* BYE"));
        assert!(BYE.ends_with("\r\n"));
    }

    #[test]
    fn logout_echoes_the_tag_with_a_tagged_ok() {
        assert_eq!(logout_response("z9"), "z9 OK LOGOUT completed\r\n");
    }
}
