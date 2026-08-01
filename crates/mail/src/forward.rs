use irixmail_directory::Forwarding;
use mail_parser::{HeaderName, HeaderValue, MessageParser};

const DELIVERED_TO: &str = "Delivered-To";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardPlan {
    pub relays: Vec<ForwardRelay>,
    pub keep_local: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardRelay {
    pub mail_from: String,
    pub rcpt_to: String,
    pub message: Vec<u8>,
}

pub fn plan_forward(
    forwarding: &Forwarding,
    delivered_to: &str,
    mail_from: &str,
    raw: &[u8],
) -> ForwardPlan {
    if !forwarding.is_active() {
        return ForwardPlan {
            relays: Vec::new(),
            keep_local: true,
        };
    }

    let relays = forward_to(&forwarding.destinations, delivered_to, mail_from, raw);
    ForwardPlan {
        keep_local: forwarding.keep_local_copy || relays.is_empty(),
        relays,
    }
}

pub fn forward_to(
    destinations: &[String],
    delivered_to: &str,
    mail_from: &str,
    raw: &[u8],
) -> Vec<ForwardRelay> {
    let delivered_to_lower = delivered_to.trim().to_ascii_lowercase();
    let already_delivered = delivered_to_addresses(raw);

    let message = prepend_delivered_to(&delivered_to_lower, raw);

    let mut relays = Vec::new();
    let mut seen = Vec::new();
    for destination in destinations {
        let rcpt = destination.trim().to_ascii_lowercase();
        if rcpt.is_empty() {
            continue;
        }
        if rcpt == delivered_to_lower || already_delivered.contains(&rcpt) || seen.contains(&rcpt) {
            continue;
        }
        seen.push(rcpt.clone());
        relays.push(ForwardRelay {
            mail_from: mail_from.to_string(),
            rcpt_to: rcpt,
            message: message.clone(),
        });
    }

    relays
}

fn delivered_to_addresses(raw: &[u8]) -> Vec<String> {
    let Some(parsed) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };

    let mut addresses = Vec::new();
    for header in parsed.headers() {
        if let HeaderName::Other(name) = &header.name {
            if name.eq_ignore_ascii_case(DELIVERED_TO) {
                let value = header_text(&header.value).trim().to_ascii_lowercase();
                if !value.is_empty() {
                    addresses.push(value);
                }
            }
        }
    }
    addresses
}

fn prepend_delivered_to(delivered_to_lower: &str, raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + delivered_to_lower.len() + DELIVERED_TO.len() + 4);
    out.extend_from_slice(DELIVERED_TO.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(collapse_folding(delivered_to_lower).as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(raw);
    out
}

fn header_text<'x>(value: &'x HeaderValue<'x>) -> &'x str {
    value.as_text().unwrap_or("")
}

fn collapse_folding(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch == '\r' || ch == '\n' { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &[u8] = concat!(
        "From: sender@example.com\r\n",
        "To: alice@irixsoft.com\r\n",
        "Subject: Hello\r\n",
        "\r\n",
        "Body text.\r\n",
    )
    .as_bytes();

    fn forwarding(destinations: &[&str], keep_local: bool) -> Forwarding {
        Forwarding {
            destinations: destinations.iter().map(|d| d.to_string()).collect(),
            keep_local_copy: keep_local,
        }
    }

    #[test]
    fn an_inactive_forwarding_keeps_the_message_and_relays_nothing() {
        let plan = plan_forward(
            &Forwarding::default(),
            "alice@irixsoft.com",
            "sender@example.com",
            RAW,
        );
        assert!(plan.relays.is_empty());
        assert!(plan.keep_local);
    }

    #[test]
    fn an_active_forwarding_relays_to_each_destination() {
        let config = forwarding(&["alice@personal.example", "alice@work.example"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        let recipients: Vec<&str> = plan.relays.iter().map(|r| r.rcpt_to.as_str()).collect();
        assert_eq!(
            recipients,
            vec!["alice@personal.example", "alice@work.example"]
        );
        assert!(!plan.keep_local);
    }

    #[test]
    fn the_relay_preserves_the_envelope_sender() {
        let config = forwarding(&["alice@personal.example"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        assert_eq!(plan.relays[0].mail_from, "sender@example.com");
    }

    #[test]
    fn a_null_return_path_is_preserved_on_the_relay() {
        let config = forwarding(&["alice@personal.example"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "", RAW);
        assert_eq!(plan.relays[0].mail_from, "");
    }

    #[test]
    fn the_keep_local_flag_is_carried_through() {
        let config = forwarding(&["alice@personal.example"], true);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        assert_eq!(plan.relays.len(), 1);
        assert!(plan.keep_local);
    }

    #[test]
    fn the_relay_prepends_a_delivered_to_trace_header() {
        let config = forwarding(&["alice@personal.example"], false);
        let plan = plan_forward(&config, "Alice@IriXSoft.com", "sender@example.com", RAW);
        let message = String::from_utf8(plan.relays[0].message.clone()).unwrap();
        assert!(message.starts_with("Delivered-To: alice@irixsoft.com\r\n"));
        assert!(message.ends_with(std::str::from_utf8(RAW).unwrap()));
    }

    #[test]
    fn a_destination_equal_to_the_delivered_address_is_dropped_as_a_loop() {
        let config = forwarding(&["alice@irixsoft.com", "alice@personal.example"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        let recipients: Vec<&str> = plan.relays.iter().map(|r| r.rcpt_to.as_str()).collect();
        assert_eq!(recipients, vec!["alice@personal.example"]);
    }

    #[test]
    fn a_destination_already_in_the_delivered_to_trail_is_dropped_as_a_loop() {
        let raw = concat!(
            "Delivered-To: alice@personal.example\r\n",
            "From: sender@example.com\r\n",
            "To: alice@irixsoft.com\r\n",
            "\r\n",
            "Body.\r\n",
        )
        .as_bytes();
        let config = forwarding(&["alice@personal.example", "alice@work.example"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", raw);
        let recipients: Vec<&str> = plan.relays.iter().map(|r| r.rcpt_to.as_str()).collect();
        assert_eq!(recipients, vec!["alice@work.example"]);
    }

    #[test]
    fn a_delivered_to_trail_is_matched_case_insensitively() {
        let raw = concat!(
            "Delivered-To: Alice@Personal.Example\r\n",
            "From: sender@example.com\r\n",
            "\r\n",
            "Body.\r\n",
        )
        .as_bytes();
        let config = forwarding(&["alice@personal.example"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", raw);
        assert!(plan.relays.is_empty());
    }

    #[test]
    fn duplicate_destinations_are_relayed_once() {
        let config = forwarding(
            &[
                "alice@personal.example",
                "ALICE@personal.example",
                "  alice@personal.example  ",
            ],
            false,
        );
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        let recipients: Vec<&str> = plan.relays.iter().map(|r| r.rcpt_to.as_str()).collect();
        assert_eq!(recipients, vec!["alice@personal.example"]);
    }

    #[test]
    fn an_empty_destination_is_ignored() {
        let config = forwarding(&["", "   ", "alice@personal.example"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        let recipients: Vec<&str> = plan.relays.iter().map(|r| r.rcpt_to.as_str()).collect();
        assert_eq!(recipients, vec!["alice@personal.example"]);
    }

    #[test]
    fn forward_to_relays_an_explicit_redirect_list() {
        let relays = forward_to(
            &["elsewhere@example.net".to_string()],
            "alice@irixsoft.com",
            "sender@example.com",
            RAW,
        );
        assert_eq!(relays.len(), 1);
        assert_eq!(relays[0].rcpt_to, "elsewhere@example.net");
        assert_eq!(relays[0].mail_from, "sender@example.com");
    }

    #[test]
    fn every_relay_shares_the_same_built_message() {
        let config = forwarding(&["a@example.net", "b@example.net"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        assert_eq!(plan.relays.len(), 2);
        assert_eq!(plan.relays[0].message, plan.relays[1].message);
    }

    #[test]
    fn a_keep_local_account_keeps_the_copy_even_when_every_destination_loops() {
        let config = forwarding(&["alice@irixsoft.com"], true);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        assert!(plan.relays.is_empty());
        assert!(plan.keep_local);
    }

    #[test]
    fn a_forward_with_only_looping_destinations_keeps_the_copy_as_a_fail_safe() {
        let config = forwarding(&["alice@irixsoft.com"], false);
        let plan = plan_forward(&config, "alice@irixsoft.com", "sender@example.com", RAW);
        assert!(plan.relays.is_empty());
        assert!(
            plan.keep_local,
            "a forward that relays nowhere must not drop the message"
        );
    }

    #[test]
    fn a_trace_header_address_cannot_inject_extra_header_lines() {
        let relays = forward_to(
            &["dest@example.net".to_string()],
            "alice@irixsoft.com\r\nInjected: yes",
            "sender@example.com",
            RAW,
        );
        let message = String::from_utf8(relays[0].message.clone()).unwrap();
        assert!(message.starts_with("Delivered-To: alice@irixsoft.com  injected: yes\r\n"));
        assert!(!message.contains("\r\nInjected: yes\r\n"));
        assert!(!message.contains("\r\ninjected: yes\r\n"));
    }
}
