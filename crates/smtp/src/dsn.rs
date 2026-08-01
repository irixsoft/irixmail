use std::fmt::Write as _;

use crate::queue_model::{QueuedMessage, RecipientStatus};

const DAEMON_NAME: &str = "Mail Delivery Subsystem";
const DAEMON_MAILBOX: &str = "MAILER-DAEMON";

const MAX_RETURNED_HEADERS: usize = 4096;

pub fn build_dsn(
    message: &QueuedMessage,
    reporting_mta: &str,
    original_headers: &[u8],
    now: u64,
) -> Option<Vec<u8>> {
    if message.return_path.is_empty() {
        return None;
    }

    let mut failures = Vec::new();
    for rcpt in &message.recipients {
        if let RecipientStatus::Bounced(reason) = &rcpt.status {
            failures.push((rcpt.address.as_str(), reason.as_str()));
        }
    }
    if failures.is_empty() {
        return None;
    }

    let boundary = boundary_for(message);
    let message_id = format!("<{boundary}.{now:x}@{reporting_mta}>");
    let date = Rfc822Date::from_timestamp(now as i64).to_string();

    let human = human_summary(&failures);
    let status_part = status_part(message, reporting_mta, &failures);
    let returned = returned_headers(original_headers);

    let mut out = String::with_capacity(
        human.len() + status_part.len() + returned.len() + boundary.len() * 4 + 512,
    );

    let _ = write!(
        out,
        "From: {DAEMON_NAME} <{DAEMON_MAILBOX}@{reporting_mta}>\r\n"
    );
    let _ = write!(out, "To: <{}>\r\n", message.return_path);
    let _ = write!(out, "Subject: {}\r\n", subject(&failures));
    let _ = write!(out, "Date: {date}\r\n");
    let _ = write!(out, "Message-ID: {message_id}\r\n");
    out.push_str("Auto-Submitted: auto-replied\r\n");
    out.push_str("MIME-Version: 1.0\r\n");
    let _ = write!(
        out,
        "Content-Type: multipart/report; report-type=delivery-status;\r\n boundary=\"{boundary}\"\r\n"
    );
    out.push_str("\r\n");
    out.push_str("This is a delivery status notification, automatically generated.\r\n\r\n");

    let _ = write!(out, "--{boundary}\r\n");
    out.push_str("Content-Type: text/plain; charset=utf-8\r\n\r\n");
    out.push_str(&human);
    out.push_str("\r\n");

    let _ = write!(out, "--{boundary}\r\n");
    out.push_str("Content-Type: message/delivery-status\r\n\r\n");
    out.push_str(&status_part);

    let _ = write!(out, "--{boundary}\r\n");
    out.push_str("Content-Type: text/rfc822-headers\r\n\r\n");
    out.push_str(&returned);
    out.push_str("\r\n");

    let _ = write!(out, "--{boundary}--\r\n");

    Some(out.into_bytes())
}

fn subject(failures: &[(&str, &str)]) -> &'static str {
    if failures.len() == 1 {
        "Undelivered mail returned to sender"
    } else {
        "Undelivered mail returned to sender (multiple recipients)"
    }
}

fn human_summary(failures: &[(&str, &str)]) -> String {
    let mut text = String::new();
    text.push_str(
        "Your message could not be delivered to the following recipients and has been returned:\r\n\r\n",
    );
    for (address, reason) in failures {
        let _ = write!(text, "  <{address}>: {reason}\r\n");
    }
    text
}

fn status_part(message: &QueuedMessage, reporting_mta: &str, failures: &[(&str, &str)]) -> String {
    let mut text = String::new();
    let _ = write!(text, "Reporting-MTA: dns;{reporting_mta}\r\n");
    let _ = write!(
        text,
        "Arrival-Date: {}\r\n",
        Rfc822Date::from_timestamp(message.created as i64)
    );
    text.push_str("\r\n");

    for (address, reason) in failures {
        let _ = write!(text, "Final-Recipient: rfc822;{address}\r\n");
        text.push_str("Action: failed\r\n");
        let _ = write!(text, "Status: {}\r\n", status_code(reason));
        let _ = write!(text, "Diagnostic-Code: smtp;{}\r\n", one_line(reason));
        text.push_str("\r\n");
    }
    text
}

fn status_code(reason: &str) -> &'static str {
    match leading_code(reason) {
        Some(code) if (500..600).contains(&code) => match code {
            550 => "5.1.1",
            551 => "5.1.6",
            552 => "5.2.2",
            553 => "5.1.3",
            554 => "5.0.0",
            _ => "5.0.0",
        },
        _ => "5.0.0",
    }
}

fn leading_code(reason: &str) -> Option<u16> {
    let digits: String = reason.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() == 3 {
        digits.parse().ok()
    } else {
        None
    }
}

fn one_line(reason: &str) -> String {
    let mut folded = String::with_capacity(reason.len());
    let mut last_was_space = false;
    for ch in reason.chars() {
        if ch == '\r' || ch == '\n' {
            if !last_was_space {
                folded.push(' ');
                last_was_space = true;
            }
        } else {
            folded.push(ch);
            last_was_space = false;
        }
    }
    folded
}

fn returned_headers(original: &[u8]) -> String {
    let slice = &original[..original.len().min(MAX_RETURNED_HEADERS)];
    let text = String::from_utf8_lossy(slice);

    if let Some(end) = text.find("\r\n\r\n") {
        return text[..end + 2].to_string();
    }
    match text.rfind('\n') {
        Some(last) => text[..last + 1].to_string(),
        None => text.into_owned(),
    }
}

fn boundary_for(message: &QueuedMessage) -> String {
    let mut acc = message.created;
    for byte in &message.blob_hash {
        acc = acc.wrapping_mul(31).wrapping_add(*byte as u64);
    }
    format!("=_irixmail_dsn_{acc:016x}")
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(crate) struct Rfc822Date {
    weekday: usize,
    day: u8,
    month: usize,
    year: i64,
    hour: u8,
    minute: u8,
    second: u8,
}

impl Rfc822Date {
    pub(crate) fn from_timestamp(timestamp: i64) -> Self {
        let days = timestamp.div_euclid(86_400);
        let seconds = timestamp.rem_euclid(86_400);

        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let year = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as usize;

        Rfc822Date {
            weekday: (days.rem_euclid(7) + 4).rem_euclid(7) as usize,
            day,
            month,
            year: year + i64::from(month <= 2),
            hour: (seconds / 3_600) as u8,
            minute: ((seconds / 60) % 60) as u8,
            second: (seconds % 60) as u8,
        }
    }
}

impl std::fmt::Display for Rfc822Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}, {:02} {} {:04} {:02}:{:02}:{:02} +0000",
            DAYS[self.weekday],
            self.day,
            MONTHS[self.month - 1],
            self.year,
            self.hour,
            self.minute,
            self.second,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue_model::{Expiry, QueueRecipient};
    use irixmail_store::BlobHash;

    const ORIGINAL: &[u8] =
        b"From: sender@sender.example\r\nTo: who@dest.example\r\nSubject: hi\r\n\r\nbody text\r\n";

    fn message(return_path: &str, recipients: Vec<QueueRecipient>) -> QueuedMessage {
        let hash = BlobHash::from_bytes(vec![7, 7, 7, 7]);
        QueuedMessage::new(1_700_000_000, &hash, 64, return_path, recipients)
    }

    fn bounced(address: &str, reason: &str) -> QueueRecipient {
        let mut rcpt = QueueRecipient::new(address, 0, Expiry::Attempts(5));
        rcpt.status = RecipientStatus::Bounced(reason.to_string());
        rcpt
    }

    #[test]
    fn a_bounce_is_addressed_back_to_the_return_path_from_the_daemon() {
        let msg = message(
            "sender@sender.example",
            vec![bounced("who@dest.example", "550 no such user")],
        );
        let raw = build_dsn(&msg, "mail.irix.example", ORIGINAL, 1_700_000_100).expect("report");
        let text = String::from_utf8(raw).unwrap();

        assert!(text.contains("To: <sender@sender.example>\r\n"));
        assert!(text.contains("From: Mail Delivery Subsystem <MAILER-DAEMON@mail.irix.example>"));
        assert!(text.contains("Auto-Submitted: auto-replied"));
    }

    #[test]
    fn a_null_return_path_raises_no_bounce_so_a_bounce_cannot_loop() {
        let msg = message("", vec![bounced("who@dest.example", "550 no such user")]);
        assert!(build_dsn(&msg, "mail.irix.example", ORIGINAL, 0).is_none());
    }

    #[test]
    fn a_message_with_no_failed_recipient_yields_no_report() {
        let mut rcpt = QueueRecipient::new("who@dest.example", 0, Expiry::Attempts(5));
        rcpt.status = RecipientStatus::Delivered;
        let msg = message("sender@sender.example", vec![rcpt]);
        assert!(build_dsn(&msg, "mail.irix.example", ORIGINAL, 0).is_none());
    }

    #[test]
    fn the_report_is_a_multipart_delivery_status_with_three_parts() {
        let msg = message(
            "sender@sender.example",
            vec![bounced("who@dest.example", "550 no such user")],
        );
        let raw = build_dsn(&msg, "mail.irix.example", ORIGINAL, 1_700_000_100).expect("report");
        let text = String::from_utf8(raw).unwrap();

        assert!(text.contains("Content-Type: multipart/report; report-type=delivery-status;"));
        assert!(text.contains("Content-Type: message/delivery-status"));
        assert!(text.contains("Content-Type: text/plain; charset=utf-8"));
        assert!(text.contains("Content-Type: text/rfc822-headers"));

        let boundary = boundary_for(&msg);
        assert_eq!(text.matches(&format!("--{boundary}\r\n")).count(), 3);
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[test]
    fn the_status_part_marks_each_recipient_failed_with_a_permanent_status() {
        let msg = message(
            "sender@sender.example",
            vec![
                bounced("one@dest.example", "550 user unknown"),
                bounced("two@dest.example", "554 transaction failed"),
            ],
        );
        let raw = build_dsn(&msg, "mail.irix.example", ORIGINAL, 1_700_000_100).expect("report");
        let text = String::from_utf8(raw).unwrap();

        assert!(text.contains("Reporting-MTA: dns;mail.irix.example"));
        assert!(text.contains("Final-Recipient: rfc822;one@dest.example"));
        assert!(text.contains("Final-Recipient: rfc822;two@dest.example"));
        assert_eq!(text.matches("Action: failed").count(), 2);
        assert!(text.contains("Status: 5.1.1"));
        assert!(text.contains("Diagnostic-Code: smtp;550 user unknown"));
    }

    #[test]
    fn a_reason_without_a_code_still_reports_a_well_formed_permanent_status() {
        let msg = message(
            "sender@sender.example",
            vec![bounced("who@dest.example", "message expired in the queue")],
        );
        let raw = build_dsn(&msg, "mail.irix.example", ORIGINAL, 0).expect("report");
        let text = String::from_utf8(raw).unwrap();
        assert!(text.contains("Status: 5.0.0"));
        assert!(text.contains("Diagnostic-Code: smtp;message expired in the queue"));
    }

    #[test]
    fn a_multiline_diagnostic_is_folded_onto_one_header_line() {
        let folded = one_line("550 first line\r\nsecond line");
        assert!(!folded.contains('\r'));
        assert!(!folded.contains('\n'));
        assert_eq!(folded, "550 first line second line");
    }

    #[test]
    fn the_returned_headers_keep_only_the_header_block_of_the_original() {
        let kept = returned_headers(ORIGINAL);
        assert!(kept.contains("From: sender@sender.example"));
        assert!(kept.contains("Subject: hi"));
        assert!(!kept.contains("body text"));
    }

    #[test]
    fn an_oversized_header_block_is_cut_at_a_whole_line() {
        let mut original = Vec::new();
        for index in 0..1_000 {
            original
                .extend_from_slice(format!("X-Filler-{index}: padding value here\r\n").as_bytes());
        }
        let kept = returned_headers(&original);
        assert!(kept.len() <= MAX_RETURNED_HEADERS);
        assert!(kept.ends_with('\n'));
    }

    #[test]
    fn the_status_code_maps_known_remote_codes_and_defaults_the_rest() {
        assert_eq!(status_code("550 mailbox unavailable"), "5.1.1");
        assert_eq!(status_code("552 over quota"), "5.2.2");
        assert_eq!(status_code("421 try later"), "5.0.0");
        assert_eq!(status_code("no code at all"), "5.0.0");
    }

    #[test]
    fn the_boundary_is_stable_for_a_message_and_differs_between_messages() {
        let one = message("s@a.example", vec![bounced("r@b.example", "550 x")]);
        let two = {
            let hash = BlobHash::from_bytes(vec![1, 2, 3]);
            QueuedMessage::new(1_700_000_001, &hash, 64, "s@a.example", Vec::new())
        };
        assert_eq!(boundary_for(&one), boundary_for(&one));
        assert_ne!(boundary_for(&one), boundary_for(&two));
    }

    #[test]
    fn the_date_renders_a_known_instant_in_rfc_822_form() {
        let rendered = Rfc822Date::from_timestamp(1_700_000_000).to_string();
        assert_eq!(rendered, "Tue, 14 Nov 2023 22:13:20 +0000");
    }

    #[test]
    fn the_epoch_renders_as_the_start_of_1970() {
        let rendered = Rfc822Date::from_timestamp(0).to_string();
        assert_eq!(rendered, "Thu, 01 Jan 1970 00:00:00 +0000");
    }
}
