use smtp_proto::{
    EhloResponse, AUTH_LOGIN, AUTH_PLAIN, EXT_8BIT_MIME, EXT_AUTH, EXT_CHUNKING,
    EXT_ENHANCED_STATUS_CODES, EXT_PIPELINING, EXT_SIZE, EXT_SMTP_UTF8, EXT_START_TLS,
};

const DEFAULT_MAX_MESSAGE_SIZE: usize = 25 * 1024 * 1024;

pub struct EhloContext<'a> {
    pub hostname: &'a str,
    pub max_message_size: usize,
    pub is_tls: bool,
    pub authenticated: bool,
}

impl<'a> EhloContext<'a> {
    pub fn new(hostname: &'a str) -> Self {
        Self {
            hostname,
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            is_tls: false,
            authenticated: false,
        }
    }
}

pub fn ehlo_response(ctx: &EhloContext<'_>) -> Vec<u8> {
    let mut response = EhloResponse::new(ctx.hostname);
    response.capabilities =
        EXT_ENHANCED_STATUS_CODES | EXT_8BIT_MIME | EXT_SMTP_UTF8 | EXT_PIPELINING | EXT_CHUNKING;

    if !ctx.is_tls {
        response.capabilities |= EXT_START_TLS;
    }

    if ctx.is_tls && !ctx.authenticated {
        response.auth_mechanisms = AUTH_PLAIN | AUTH_LOGIN;
        response.capabilities |= EXT_AUTH;
    }

    if ctx.max_message_size > 0 {
        response.size = ctx.max_message_size;
        response.capabilities |= EXT_SIZE;
    }

    let mut buf = Vec::with_capacity(128);
    response.write(&mut buf).ok();
    buf
}

pub fn helo_response(hostname: &str) -> Vec<u8> {
    format!("250 {hostname} at your service\r\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(ctx: &EhloContext<'_>) -> String {
        String::from_utf8(ehlo_response(ctx)).unwrap()
    }

    #[test]
    fn plaintext_session_advertises_starttls_but_not_auth() {
        let ctx = EhloContext::new("mail.example");
        let text = rendered(&ctx);
        assert!(text.starts_with("250-mail.example"));
        assert!(text.contains("STARTTLS\r\n"));
        assert!(!text.contains("AUTH "));
    }

    #[test]
    fn the_core_extensions_are_always_present() {
        let ctx = EhloContext::new("mail.example");
        let text = rendered(&ctx);
        assert!(text.contains("PIPELINING\r\n"));
        assert!(text.contains("8BITMIME\r\n"));
        assert!(text.contains("SMTPUTF8\r\n"));
        assert!(text.contains("ENHANCEDSTATUSCODES\r\n"));
    }

    #[test]
    fn the_size_limit_is_advertised() {
        let mut ctx = EhloContext::new("mail.example");
        ctx.max_message_size = 1024;
        let text = rendered(&ctx);
        assert!(text.contains("SIZE 1024\r\n"));
    }

    #[test]
    fn auth_is_offered_only_over_tls() {
        let mut ctx = EhloContext::new("mail.example");
        ctx.is_tls = true;
        let text = rendered(&ctx);
        assert!(text.contains("AUTH PLAIN LOGIN\r\n") || text.contains("AUTH LOGIN PLAIN\r\n"));
        assert!(!text.contains("STARTTLS\r\n"));
    }

    #[test]
    fn auth_is_withheld_once_authenticated() {
        let mut ctx = EhloContext::new("mail.example");
        ctx.is_tls = true;
        ctx.authenticated = true;
        let text = rendered(&ctx);
        assert!(!text.contains("AUTH "));
    }

    #[test]
    fn the_last_line_uses_a_space_separator() {
        let ctx = EhloContext::new("mail.example");
        let text = rendered(&ctx);
        let last = text.trim_end().rsplit("\r\n").next().unwrap();
        assert!(last.starts_with("250 "));
    }

    #[test]
    fn helo_yields_a_single_greeting_line() {
        let text = String::from_utf8(helo_response("mail.example")).unwrap();
        assert_eq!(text, "250 mail.example at your service\r\n");
        assert_eq!(text.matches("\r\n").count(), 1);
    }
}
