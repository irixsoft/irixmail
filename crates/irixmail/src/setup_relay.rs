use anyhow::{Context, Result};
use irixmail_core::{BootstrapConfig, RelayConfig};

use crate::setup::prompt;

pub fn configure(config: &mut BootstrapConfig) -> Result<()> {
    println!(
        "Outbound mail can go direct to recipient servers (MX) or through an SMTP relay\n(e.g. Amazon SES) — use a relay when your host blocks outbound port 25."
    );
    let keep_relay = config.relay.is_some();
    loop {
        let default = if keep_relay { "relay" } else { "direct" };
        let answer = prompt(&format!(
            "Outbound delivery, \"direct\" (to MX) or \"relay\" [{default}]: "
        ))?;
        match wants_relay(&answer, keep_relay) {
            Some(true) => {
                config.relay = Some(collect_relay()?);
                return Ok(());
            }
            Some(false) => {
                config.relay = None;
                return Ok(());
            }
            None => println!("answer direct or relay"),
        }
    }
}

fn collect_relay() -> Result<RelayConfig> {
    let host = loop {
        let answer = prompt("Relay host (e.g. email-smtp.us-east-1.amazonaws.com): ")?;
        if !answer.is_empty() {
            break answer;
        }
        println!("the relay host cannot be empty");
    };
    let port = loop {
        let answer = prompt("Relay port [587]: ")?;
        match parse_port(&answer, 587) {
            Some(port) => break port,
            None => println!("enter a port number between 1 and 65535"),
        }
    };
    let username = prompt("Relay username (leave empty for none): ")?;
    let (username, password) = if username.is_empty() {
        (None, None)
    } else {
        let secret =
            rpassword::prompt_password("Relay password: ").context("reading the relay password")?;
        (Some(username), Some(secret))
    };
    Ok(relay_settings(host, port, username, password))
}

fn wants_relay(answer: &str, current_relay: bool) -> Option<bool> {
    match answer.to_ascii_lowercase().as_str() {
        "" => Some(current_relay),
        "direct" | "d" | "1" => Some(false),
        "relay" | "r" | "2" => Some(true),
        _ => None,
    }
}

fn parse_port(answer: &str, default: u16) -> Option<u16> {
    if answer.is_empty() {
        return Some(default);
    }
    answer.parse::<u16>().ok().filter(|port| *port != 0)
}

fn relay_settings(
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
) -> RelayConfig {
    let authenticated = username.is_some();
    RelayConfig {
        host,
        port,
        username,
        password,
        implicit_tls: port == 465,
        require_tls: authenticated,
        accept_invalid_certs: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delivery_mode_answer_is_parsed_with_the_current_mode_as_default() {
        assert_eq!(wants_relay("", false), Some(false));
        assert_eq!(wants_relay("", true), Some(true));
        assert_eq!(wants_relay("direct", true), Some(false));
        assert_eq!(wants_relay("RELAY", false), Some(true));
        assert_eq!(wants_relay("maybe", false), None);
    }

    #[test]
    fn the_relay_port_answer_falls_back_to_the_default() {
        assert_eq!(parse_port("", 587), Some(587));
        assert_eq!(parse_port("465", 587), Some(465));
        assert_eq!(parse_port("0", 587), None);
        assert_eq!(parse_port("70000", 587), None);
        assert_eq!(parse_port("abc", 587), None);
    }

    #[test]
    fn credentials_require_tls_and_port_465_means_implicit_tls() {
        let relay = relay_settings(
            "smtp.example.com".into(),
            465,
            Some("user".into()),
            Some("pass".into()),
        );
        assert!(relay.implicit_tls);
        assert!(relay.require_tls);
        assert_eq!(relay.username.as_deref(), Some("user"));
        assert_eq!(relay.password.as_deref(), Some("pass"));
        assert!(!relay.accept_invalid_certs);
    }

    #[test]
    fn an_open_relay_on_587_uses_starttls_without_requiring_it() {
        let relay = relay_settings("smtp.example.com".into(), 587, None, None);
        assert!(!relay.implicit_tls);
        assert!(!relay.require_tls);
        assert_eq!(relay.username, None);
        assert_eq!(relay.password, None);
    }
}
