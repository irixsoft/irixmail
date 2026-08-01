use anyhow::Result;
use irixmail_core::BootstrapConfig;

use crate::setup::prompt;

pub fn configure(config: &mut BootstrapConfig) -> Result<()> {
    loop {
        let label = if is_valid_hostname(&config.server.hostname) {
            format!("Server hostname (FQDN) [{}]: ", config.server.hostname)
        } else {
            "Server hostname (FQDN), e.g. mail.example.com: ".to_string()
        };
        let answer = prompt(&label)?;
        if let Some(hostname) = chosen_hostname(&answer, &config.server.hostname) {
            config.server.hostname = hostname;
            return Ok(());
        }
        println!("enter a fully qualified hostname like mail.example.com");
    }
}

fn is_valid_hostname(name: &str) -> bool {
    name.contains('.') && !name.starts_with('.') && !name.ends_with('.')
}

fn chosen_hostname(answer: &str, current: &str) -> Option<String> {
    if answer.is_empty() && is_valid_hostname(current) {
        return Some(current.to_string());
    }
    is_valid_hostname(answer).then(|| answer.to_string())
}

#[cfg(test)]
mod tests {
    use super::{chosen_hostname, is_valid_hostname};

    #[test]
    fn hostnames_without_a_domain_part_are_rejected() {
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname("localhost"));
        assert!(!is_valid_hostname("mail."));
    }

    #[test]
    fn a_fully_qualified_hostname_is_accepted() {
        assert!(is_valid_hostname("mail.example.com"));
    }

    #[test]
    fn an_empty_answer_keeps_a_valid_existing_hostname() {
        assert_eq!(
            chosen_hostname("", "mail.example.com"),
            Some("mail.example.com".into())
        );
        assert_eq!(chosen_hostname("", "localhost"), None);
    }

    #[test]
    fn an_explicit_answer_wins_when_valid() {
        assert_eq!(
            chosen_hostname("mail.new.com", "mail.example.com"),
            Some("mail.new.com".into())
        );
        assert_eq!(chosen_hostname("bad", "mail.example.com"), None);
    }
}
