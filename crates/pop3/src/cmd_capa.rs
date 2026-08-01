pub fn capa_response(is_tls: bool) -> Vec<u8> {
    let mut out = String::from("+OK capabilities follow\r\n");
    if is_tls {
        out.push_str("USER\r\n");
        out.push_str("SASL PLAIN LOGIN\r\n");
    } else {
        out.push_str("STLS\r\n");
    }
    out.push_str("TOP\r\n");
    out.push_str("UIDL\r\n");
    out.push_str("RESP-CODES\r\n");
    out.push_str("PIPELINING\r\n");
    out.push_str("EXPIRE NEVER\r\n");
    out.push_str("UTF8\r\n");
    out.push_str("IMPLEMENTATION IRIXMAIL\r\n");
    out.push_str(".\r\n");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(is_tls: bool) -> String {
        String::from_utf8(capa_response(is_tls)).unwrap()
    }

    #[test]
    fn the_listing_opens_with_ok_and_ends_with_a_dot() {
        let text = rendered(false);
        assert!(text.starts_with("+OK"));
        assert!(text.ends_with("\r\n.\r\n"));
    }

    #[test]
    fn the_core_capabilities_are_listed() {
        for is_tls in [false, true] {
            let text = rendered(is_tls);
            for cap in [
                "TOP",
                "UIDL",
                "RESP-CODES",
                "PIPELINING",
                "EXPIRE NEVER",
                "UTF8",
                "IMPLEMENTATION IRIXMAIL",
            ] {
                assert!(
                    text.contains(&format!("{cap}\r\n")),
                    "missing {cap} (tls={is_tls})"
                );
            }
        }
    }

    #[test]
    fn stls_is_offered_only_before_tls() {
        assert!(rendered(false).contains("STLS\r\n"));
        assert!(!rendered(true).contains("STLS\r\n"));
    }

    #[test]
    fn user_and_sasl_are_offered_only_over_tls() {
        assert!(rendered(true).contains("USER\r\n"));
        assert!(rendered(true).contains("SASL PLAIN LOGIN\r\n"));
        assert!(!rendered(false).contains("USER\r\n"));
        assert!(!rendered(false).contains("SASL"));
    }
}
