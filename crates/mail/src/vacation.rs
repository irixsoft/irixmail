use irixmail_core::{Error, Result};
use irixmail_store::{Store, Subspace};
use mail_parser::{Address, HeaderName, HeaderValue, MessageParser};
use serde::{Deserialize, Serialize};

pub const DEFAULT_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60;

const TAG_VACATION_REPLY: u8 = 0x29;

pub fn last_vacation_reply(
    store: &dyn Store,
    account_id: u64,
    sender: &str,
) -> Result<Option<u64>> {
    match store.get(&reply_key(account_id, sender))? {
        Some(bytes) => {
            let array: [u8; 8] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| Error::serialize("vacation reply timestamp is malformed"))?;
            Ok(Some(u64::from_be_bytes(array)))
        }
        None => Ok(None),
    }
}

pub fn record_vacation_reply(
    store: &dyn Store,
    account_id: u64,
    sender: &str,
    replied_at: u64,
) -> Result<()> {
    store.put(&reply_key(account_id, sender), &replied_at.to_be_bytes())
}

fn reply_key(account_id: u64, sender: &str) -> Vec<u8> {
    let sender = sender.trim().to_ascii_lowercase();
    let mut key = Vec::with_capacity(10 + sender.len());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_VACATION_REPLY);
    key.extend_from_slice(&account_id.to_be_bytes());
    key.extend_from_slice(sender.as_bytes());
    key
}

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    Eq,
)]
#[rkyv(derive(Debug), compare(PartialEq))]
pub struct VacationConfig {
    pub enabled: bool,
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub period_seconds: u64,
    pub subject: Option<String>,
    pub body: String,
}

impl Default for VacationConfig {
    fn default() -> Self {
        VacationConfig {
            enabled: false,
            start: None,
            end: None,
            period_seconds: DEFAULT_PERIOD_SECONDS,
            subject: None,
            body: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressReason {
    Disabled,
    OutsideWindow,
    NoReturnPath,
    OwnAddress,
    AutomatedSender,
    AutomatedMessage,
    NotAddressed,
    RecentlyAnswered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VacationDecision {
    Suppress(SuppressReason),
    Reply(VacationReply),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VacationReply {
    pub to: String,
    pub message: Vec<u8>,
    pub sent_at: u64,
}

pub fn evaluate_vacation(
    config: &VacationConfig,
    raw: &[u8],
    mail_from: &str,
    recipient: &str,
    now: u64,
    last_replied_at: Option<u64>,
) -> Result<VacationDecision> {
    if !config.enabled {
        return Ok(VacationDecision::Suppress(SuppressReason::Disabled));
    }
    if !window_is_open(config, now) {
        return Ok(VacationDecision::Suppress(SuppressReason::OutsideWindow));
    }

    let sender = mail_from.trim();
    if sender.is_empty() {
        return Ok(VacationDecision::Suppress(SuppressReason::NoReturnPath));
    }
    let sender_lower = sender.to_ascii_lowercase();

    if sender_lower == recipient.trim().to_ascii_lowercase() {
        return Ok(VacationDecision::Suppress(SuppressReason::OwnAddress));
    }
    if looks_automated_sender(&sender_lower) {
        return Ok(VacationDecision::Suppress(SuppressReason::AutomatedSender));
    }

    if let Some(last) = last_replied_at {
        if now < last || now - last < config.period_seconds {
            return Ok(VacationDecision::Suppress(SuppressReason::RecentlyAnswered));
        }
    }

    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::invalid_input("vacation: message has no parseable headers"))?;

    if message_is_automated(&parsed) {
        return Ok(VacationDecision::Suppress(SuppressReason::AutomatedMessage));
    }
    if !account_is_addressed(&parsed, recipient) {
        return Ok(VacationDecision::Suppress(SuppressReason::NotAddressed));
    }

    let message = build_reply(config, &parsed, &sender_lower, recipient, now);
    Ok(VacationDecision::Reply(VacationReply {
        to: sender_lower,
        message,
        sent_at: now,
    }))
}

fn window_is_open(config: &VacationConfig, now: u64) -> bool {
    if let Some(start) = config.start {
        if now < start {
            return false;
        }
    }
    if let Some(end) = config.end {
        if now >= end {
            return false;
        }
    }
    true
}

fn looks_automated_sender(sender_lower: &str) -> bool {
    sender_lower.starts_with("mailer-daemon")
        || sender_lower.starts_with("owner-")
        || sender_lower.contains("-request@")
}

fn message_is_automated(message: &mail_parser::Message<'_>) -> bool {
    for header in message.headers() {
        match &header.name {
            HeaderName::ListId
            | HeaderName::ListArchive
            | HeaderName::ListHelp
            | HeaderName::ListOwner
            | HeaderName::ListPost
            | HeaderName::ListSubscribe
            | HeaderName::ListUnsubscribe => return true,
            HeaderName::Other(name) => {
                if name.eq_ignore_ascii_case("Auto-Submitted") {
                    let value = header_text(&header.value).trim();
                    if !value.is_empty() && !value.eq_ignore_ascii_case("no") {
                        return true;
                    }
                } else if name.eq_ignore_ascii_case("Precedence") {
                    if header_text(&header.value)
                        .trim()
                        .eq_ignore_ascii_case("bulk")
                    {
                        return true;
                    }
                } else if name.eq_ignore_ascii_case("X-Auto-Response-Suppress") {
                    let value = header_text(&header.value).to_ascii_lowercase();
                    if value
                        .split(',')
                        .map(str::trim)
                        .any(|token| token == "all" || token == "oof")
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn account_is_addressed(message: &mail_parser::Message<'_>, recipient: &str) -> bool {
    let recipient_lower = recipient.trim().to_ascii_lowercase();
    if recipient_lower.is_empty() {
        return false;
    }
    [message.to(), message.cc()]
        .into_iter()
        .flatten()
        .any(|address| address_contains(address, &recipient_lower))
}

fn address_contains(address: &Address<'_>, wanted: &str) -> bool {
    address
        .iter()
        .filter_map(|addr| addr.address())
        .any(|addr| addr.eq_ignore_ascii_case(wanted))
}

fn header_text<'x>(value: &'x HeaderValue<'x>) -> &'x str {
    value.as_text().unwrap_or("")
}

fn build_reply(
    config: &VacationConfig,
    message: &mail_parser::Message<'_>,
    sender_lower: &str,
    recipient: &str,
    now: u64,
) -> Vec<u8> {
    let subject = match &config.subject {
        Some(subject) if !subject.trim().is_empty() => subject.clone(),
        _ => derive_subject(message),
    };

    let mut out = String::new();
    push_header(&mut out, "From", recipient.trim());
    push_header(&mut out, "To", sender_lower);
    push_header(&mut out, "Subject", &collapse_folding(&subject));
    push_header(&mut out, "Date", &format_rfc5322_date(now));
    push_header(&mut out, "Auto-Submitted", "auto-replied");
    push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
    out.push_str("\r\n");
    out.push_str(&config.body);

    out.into_bytes()
}

fn derive_subject(message: &mail_parser::Message<'_>) -> String {
    match message.subject() {
        Some(subject) if !subject.trim().is_empty() => {
            let trimmed = subject.trim();
            if trimmed.len() >= 3 && trimmed[..3].eq_ignore_ascii_case("re:") {
                trimmed.to_string()
            } else {
                format!("Re: {trimmed}")
            }
        }
        _ => "Automatic reply".to_string(),
    }
}

fn push_header(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push_str(": ");
    out.push_str(value);
    out.push_str("\r\n");
}

fn collapse_folding(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch == '\r' || ch == '\n' { ' ' } else { ch })
        .collect()
}

fn format_rfc5322_date(now: u64) -> String {
    const DAYS_OF_WEEK: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let seconds_of_day = now % 86_400;
    let days_since_epoch = now / 86_400;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    let weekday = DAYS_OF_WEEK[(days_since_epoch % 7) as usize];
    let (year, month, day) = civil_from_days(days_since_epoch);

    format!(
        "{weekday}, {day:02} {month_name} {year} {hour:02}:{minute:02}:{second:02} +0000",
        month_name = MONTHS[(month - 1) as usize],
    )
}

fn civil_from_days(days_since_epoch: u64) -> (u64, u64, u64) {
    let z = days_since_epoch + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_781_827_200;

    fn config() -> VacationConfig {
        VacationConfig {
            enabled: true,
            body: "I am away until next week.".to_string(),
            ..VacationConfig::default()
        }
    }

    fn message(from: &str, to: &str, extra_headers: &str) -> Vec<u8> {
        format!(
            "From: {from}\r\nTo: {to}\r\nSubject: Question\r\n{extra_headers}\r\nA real question.\r\n"
        )
        .into_bytes()
    }

    fn evaluate(
        config: &VacationConfig,
        raw: &[u8],
        from: &str,
        rcpt: &str,
        last: Option<u64>,
    ) -> VacationDecision {
        evaluate_vacation(config, raw, from, rcpt, NOW, last).expect("evaluate")
    }

    #[test]
    fn a_disabled_responder_never_replies() {
        let config = VacationConfig::default();
        let raw = message("a@example.com", "me@example.org", "");
        let decision = evaluate(&config, &raw, "a@example.com", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::Disabled)
        );
    }

    #[test]
    fn a_first_message_from_a_person_earns_a_reply() {
        let raw = message("alice@example.com", "me@example.org", "");
        let decision = evaluate(&config(), &raw, "alice@example.com", "me@example.org", None);
        let VacationDecision::Reply(reply) = decision else {
            panic!("expected a reply, got {decision:?}");
        };
        assert_eq!(reply.to, "alice@example.com");
        assert_eq!(reply.sent_at, NOW);
        let text = String::from_utf8(reply.message).unwrap();
        assert!(text.contains("From: me@example.org\r\n"));
        assert!(text.contains("To: alice@example.com\r\n"));
        assert!(text.contains("Auto-Submitted: auto-replied\r\n"));
        assert!(text.contains("Subject: Re: Question\r\n"));
        assert!(text.ends_with("I am away until next week."));
    }

    #[test]
    fn a_configured_subject_is_used_verbatim() {
        let config = VacationConfig {
            subject: Some("Out of office".to_string()),
            ..config()
        };
        let raw = message("alice@example.com", "me@example.org", "");
        let VacationDecision::Reply(reply) =
            evaluate(&config, &raw, "alice@example.com", "me@example.org", None)
        else {
            panic!("expected a reply");
        };
        let text = String::from_utf8(reply.message).unwrap();
        assert!(text.contains("Subject: Out of office\r\n"));
    }

    #[test]
    fn an_empty_sender_is_a_bounce_and_is_left_alone() {
        let raw = message("alice@example.com", "me@example.org", "");
        let decision = evaluate(&config(), &raw, "", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::NoReturnPath)
        );
    }

    #[test]
    fn the_account_is_not_answered_when_it_writes_to_itself() {
        let raw = message("me@example.org", "me@example.org", "");
        let decision = evaluate(&config(), &raw, "me@example.org", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::OwnAddress)
        );
    }

    #[test]
    fn a_daemon_sender_is_left_alone() {
        let raw = message("MAILER-DAEMON@relay.example.net", "me@example.org", "");
        let decision = evaluate(
            &config(),
            &raw,
            "MAILER-DAEMON@relay.example.net",
            "me@example.org",
            None,
        );
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::AutomatedSender)
        );
    }

    #[test]
    fn a_list_request_sender_is_left_alone() {
        let raw = message("widgets-request@lists.example.net", "me@example.org", "");
        let decision = evaluate(
            &config(),
            &raw,
            "widgets-request@lists.example.net",
            "me@example.org",
            None,
        );
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::AutomatedSender)
        );
    }

    #[test]
    fn a_mailing_list_message_is_left_alone() {
        let raw = message(
            "alice@example.com",
            "me@example.org",
            "List-Id: <widgets.lists.example.net>\r\n",
        );
        let decision = evaluate(&config(), &raw, "alice@example.com", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::AutomatedMessage)
        );
    }

    #[test]
    fn an_auto_submitted_message_is_left_alone() {
        let raw = message(
            "alice@example.com",
            "me@example.org",
            "Auto-Submitted: auto-generated\r\n",
        );
        let decision = evaluate(&config(), &raw, "alice@example.com", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::AutomatedMessage)
        );
    }

    #[test]
    fn an_auto_submitted_no_is_still_answered() {
        let raw = message(
            "alice@example.com",
            "me@example.org",
            "Auto-Submitted: no\r\n",
        );
        let decision = evaluate(&config(), &raw, "alice@example.com", "me@example.org", None);
        assert!(matches!(decision, VacationDecision::Reply(_)));
    }

    #[test]
    fn a_bulk_precedence_message_is_left_alone() {
        let raw = message(
            "alice@example.com",
            "me@example.org",
            "Precedence: bulk\r\n",
        );
        let decision = evaluate(&config(), &raw, "alice@example.com", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::AutomatedMessage)
        );
    }

    #[test]
    fn a_suppress_oof_message_is_left_alone() {
        let raw = message(
            "alice@example.com",
            "me@example.org",
            "X-Auto-Response-Suppress: DR, OOF, AutoReply\r\n",
        );
        let decision = evaluate(&config(), &raw, "alice@example.com", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::AutomatedMessage)
        );
    }

    #[test]
    fn a_message_not_addressed_to_the_account_is_left_alone() {
        let raw = message("alice@example.com", "someone-else@example.org", "");
        let decision = evaluate(&config(), &raw, "alice@example.com", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::NotAddressed)
        );
    }

    #[test]
    fn an_account_addressed_in_cc_is_answered() {
        let raw =
            b"From: alice@example.com\r\nTo: list@example.com\r\nCc: me@example.org\r\nSubject: Hi\r\n\r\nBody.\r\n"
                .to_vec();
        let decision = evaluate(&config(), &raw, "alice@example.com", "me@example.org", None);
        assert!(matches!(decision, VacationDecision::Reply(_)));
    }

    #[test]
    fn a_sender_answered_within_the_period_is_left_alone() {
        let raw = message("alice@example.com", "me@example.org", "");
        let recent = NOW - 60 * 60;
        let decision = evaluate(
            &config(),
            &raw,
            "alice@example.com",
            "me@example.org",
            Some(recent),
        );
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::RecentlyAnswered)
        );
    }

    #[test]
    fn a_sender_answered_before_the_period_is_answered_again() {
        let raw = message("alice@example.com", "me@example.org", "");
        let old = NOW - DEFAULT_PERIOD_SECONDS - 1;
        let decision = evaluate(
            &config(),
            &raw,
            "alice@example.com",
            "me@example.org",
            Some(old),
        );
        assert!(matches!(decision, VacationDecision::Reply(_)));
    }

    #[test]
    fn a_message_before_the_window_opens_is_left_alone() {
        let config = VacationConfig {
            start: Some(NOW + 86_400),
            ..config()
        };
        let raw = message("alice@example.com", "me@example.org", "");
        let decision = evaluate(&config, &raw, "alice@example.com", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::OutsideWindow)
        );
    }

    #[test]
    fn a_message_after_the_window_closes_is_left_alone() {
        let config = VacationConfig {
            end: Some(NOW - 86_400),
            ..config()
        };
        let raw = message("alice@example.com", "me@example.org", "");
        let decision = evaluate(&config, &raw, "alice@example.com", "me@example.org", None);
        assert_eq!(
            decision,
            VacationDecision::Suppress(SuppressReason::OutsideWindow)
        );
    }

    #[test]
    fn a_message_inside_the_window_is_answered() {
        let config = VacationConfig {
            start: Some(NOW - 86_400),
            end: Some(NOW + 86_400),
            ..config()
        };
        let raw = message("alice@example.com", "me@example.org", "");
        let decision = evaluate(&config, &raw, "alice@example.com", "me@example.org", None);
        assert!(matches!(decision, VacationDecision::Reply(_)));
    }

    #[test]
    fn unparseable_bytes_are_reported_as_invalid_input() {
        let result = evaluate_vacation(
            &config(),
            b"",
            "alice@example.com",
            "me@example.org",
            NOW,
            None,
        );
        assert!(matches!(result, Err(Error::InvalidInput(_))));
    }

    #[test]
    fn the_reply_date_is_a_well_formed_rfc5322_line() {
        assert_eq!(format_rfc5322_date(NOW), "Fri, 19 Jun 2026 00:00:00 +0000");
        assert_eq!(format_rfc5322_date(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(
            format_rfc5322_date(1_582_934_400),
            "Sat, 29 Feb 2020 00:00:00 +0000"
        );
    }

    #[test]
    fn the_config_round_trips_through_serde_json() {
        let config = VacationConfig {
            enabled: true,
            start: Some(10),
            end: Some(20),
            period_seconds: 3_600,
            subject: Some("Away".to_string()),
            body: "Back soon.".to_string(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: VacationConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, back);
    }
}
