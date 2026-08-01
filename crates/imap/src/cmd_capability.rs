pub struct CapabilityContext {
    pub is_tls: bool,
    pub authenticated: bool,
}

impl CapabilityContext {
    pub fn new() -> Self {
        Self {
            is_tls: false,
            authenticated: false,
        }
    }
}

impl Default for CapabilityContext {
    fn default() -> Self {
        Self::new()
    }
}

pub fn capability_codes(ctx: &CapabilityContext) -> String {
    let mut caps: Vec<&str> = vec!["IMAP4rev1"];
    if !ctx.is_tls {
        caps.push("STARTTLS");
        caps.push("LOGINDISABLED");
    }
    caps.push("IDLE");
    caps.push("UIDPLUS");
    caps.push("MOVE");
    caps.push("LITERAL+");
    caps.push("NAMESPACE");
    caps.push("SPECIAL-USE");
    caps.push("CHILDREN");
    caps.push("ID");
    caps.push("UNSELECT");
    caps.push("ENABLE");
    caps.push("CONDSTORE");
    caps.push("QRESYNC");
    caps.push("ESEARCH");
    caps.push("SEARCHRES");
    caps.push("MULTIAPPEND");
    caps.push("QUOTA");
    caps.push("LIST-EXTENDED");
    caps.push("LIST-STATUS");
    caps.push("SORT");
    caps.push("THREAD=REFERENCES");
    caps.push("THREAD=ORDEREDSUBJECT");
    if ctx.is_tls && !ctx.authenticated {
        caps.push("AUTH=PLAIN");
        caps.push("AUTH=LOGIN");
        caps.push("SASL-IR");
    }
    caps.join(" ")
}

pub fn capability_line(ctx: &CapabilityContext) -> Vec<u8> {
    format!("* CAPABILITY {}\r\n", capability_codes(ctx)).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(ctx: &CapabilityContext) -> String {
        String::from_utf8(capability_line(ctx)).unwrap()
    }

    #[test]
    fn imap4rev1_is_always_advertised() {
        let text = rendered(&CapabilityContext::new());
        assert!(text.starts_with("* CAPABILITY IMAP4rev1"));
        assert!(text.ends_with("\r\n"));
    }

    #[test]
    fn plaintext_offers_starttls_and_disables_login() {
        let text = rendered(&CapabilityContext::new());
        assert!(text.contains(" STARTTLS"));
        assert!(text.contains(" LOGINDISABLED"));
        assert!(!text.contains("AUTH="));
    }

    #[test]
    fn the_advertised_extensions_are_present() {
        let text = rendered(&CapabilityContext::new());
        for cap in [
            "IDLE",
            "UIDPLUS",
            "MOVE",
            "LITERAL+",
            "NAMESPACE",
            "SPECIAL-USE",
            "ID",
            "UNSELECT",
        ] {
            assert!(text.contains(cap), "missing {cap}");
        }
    }

    #[test]
    fn sasl_ir_is_advertised_alongside_auth() {
        let ctx = CapabilityContext {
            is_tls: true,
            authenticated: false,
        };
        assert!(rendered(&ctx).contains("SASL-IR"));
        assert!(!rendered(&CapabilityContext::new()).contains("SASL-IR"));
    }

    #[test]
    fn tls_offers_auth_and_drops_starttls() {
        let ctx = CapabilityContext {
            is_tls: true,
            authenticated: false,
        };
        let text = rendered(&ctx);
        assert!(text.contains("AUTH=PLAIN"));
        assert!(text.contains("AUTH=LOGIN"));
        assert!(!text.contains("STARTTLS"));
        assert!(!text.contains("LOGINDISABLED"));
    }

    #[test]
    fn an_authenticated_session_offers_neither_starttls_nor_auth() {
        let ctx = CapabilityContext {
            is_tls: true,
            authenticated: true,
        };
        let text = rendered(&ctx);
        assert!(!text.contains("AUTH="));
        assert!(!text.contains("STARTTLS"));
    }
}
