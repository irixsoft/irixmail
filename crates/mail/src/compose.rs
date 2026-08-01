use mail_builder::headers::address::Address;
use mail_builder::MessageBuilder;

use irixmail_core::{Error, Result};

pub struct Mailbox {
    pub name: String,
    pub email: String,
}

pub struct Attachment {
    pub content_type: String,
    pub name: String,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct Compose {
    pub from: Option<Mailbox>,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub bcc: Vec<Mailbox>,
    pub subject: String,
    pub text_body: String,
    pub html_body: Option<String>,
    pub attachments: Vec<Attachment>,
}

pub fn build_message(message: &Compose) -> Result<Vec<u8>> {
    let mut builder = MessageBuilder::new()
        .subject(message.subject.clone())
        .text_body(message.text_body.clone());

    if let Some(html) = &message.html_body {
        builder = builder.html_body(html.clone());
    }
    if let Some(from) = &message.from {
        builder = builder.from(address(from));
    }
    if !message.to.is_empty() {
        builder = builder.to(addresses(&message.to));
    }
    if !message.cc.is_empty() {
        builder = builder.cc(addresses(&message.cc));
    }
    if !message.bcc.is_empty() {
        builder = builder.bcc(addresses(&message.bcc));
    }
    for attachment in &message.attachments {
        builder = builder.attachment(
            attachment.content_type.clone(),
            attachment.name.clone(),
            attachment.data.clone(),
        );
    }

    builder
        .write_to_vec()
        .map_err(|err| Error::internal(format!("could not build the message: {err}")))
}

fn address(mailbox: &Mailbox) -> Address<'static> {
    let name = mailbox.name.trim();
    Address::new_address(
        (!name.is_empty()).then(|| name.to_string()),
        mailbox.email.clone(),
    )
}

fn addresses(mailboxes: &[Mailbox]) -> Vec<Address<'static>> {
    mailboxes.iter().map(address).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_string(message: &Compose) -> String {
        String::from_utf8(build_message(message).unwrap()).unwrap()
    }

    fn mailbox(name: &str, email: &str) -> Mailbox {
        Mailbox {
            name: name.into(),
            email: email.into(),
        }
    }

    #[test]
    fn a_message_with_html_and_text_becomes_a_multipart_alternative() {
        let raw = raw_string(&Compose {
            text_body: "plain words".into(),
            html_body: Some("<p>rich <b>words</b></p>".into()),
            ..Compose::default()
        });
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("text/plain"));
        assert!(raw.contains("text/html"));
        assert!(raw.contains("plain words"));
        assert!(raw.contains("rich <b>words</b>"));
    }

    #[test]
    fn an_html_message_with_an_attachment_nests_the_alternative_inside_mixed() {
        let raw = raw_string(&Compose {
            text_body: "plain".into(),
            html_body: Some("<p>rich</p>".into()),
            attachments: vec![Attachment {
                content_type: "text/plain".into(),
                name: "note.txt".into(),
                data: b"hello".to_vec(),
            }],
            ..Compose::default()
        });
        assert!(raw.contains("multipart/mixed"));
        assert!(raw.contains("multipart/alternative"));
    }

    #[test]
    fn a_text_only_message_stays_a_single_part() {
        let raw = raw_string(&Compose {
            text_body: "just text".into(),
            ..Compose::default()
        });
        assert!(!raw.contains("multipart"));
    }

    #[test]
    fn a_sender_without_a_display_name_is_written_as_a_bare_address() {
        let raw = raw_string(&Compose {
            from: Some(mailbox("", "alice@example.com")),
            ..Compose::default()
        });
        assert!(raw.contains("From: <alice@example.com>"));
        assert!(!raw.contains("\"\""));
    }

    #[test]
    fn a_sender_with_a_display_name_keeps_it() {
        let raw = raw_string(&Compose {
            from: Some(mailbox("Alice", "alice@example.com")),
            ..Compose::default()
        });
        assert!(raw.contains("Alice"));
        assert!(raw.contains("<alice@example.com>"));
    }

    #[test]
    fn recipients_without_display_names_are_written_as_bare_addresses() {
        let raw = raw_string(&Compose {
            to: vec![mailbox("", "bob@example.net")],
            ..Compose::default()
        });
        assert!(raw.contains("To: <bob@example.net>"));
        assert!(!raw.contains("\"\""));
    }

    #[test]
    fn a_whitespace_only_display_name_is_dropped() {
        let raw = raw_string(&Compose {
            from: Some(mailbox("   ", "alice@example.com")),
            ..Compose::default()
        });
        assert!(raw.contains("From: <alice@example.com>"));
    }

    #[test]
    fn a_mixed_recipient_list_keeps_only_the_named_display_names() {
        let raw = raw_string(&Compose {
            to: vec![
                mailbox("Bob", "bob@example.net"),
                mailbox("", "carol@example.org"),
            ],
            ..Compose::default()
        });
        assert!(raw.contains("Bob"));
        assert!(raw.contains("<carol@example.org>"));
        assert!(!raw.contains("\"\""));
    }

    #[test]
    fn a_simple_message_round_trips_through_the_parser() {
        let message = Compose {
            from: Some(Mailbox {
                name: "Alice".into(),
                email: "alice@example.com".into(),
            }),
            to: vec![Mailbox {
                name: String::new(),
                email: "bob@example.net".into(),
            }],
            subject: "Hello".into(),
            text_body: "Hi Bob".into(),
            ..Compose::default()
        };
        let raw = build_message(&message).unwrap();
        let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(parsed.subject(), Some("Hello"));
        assert!(parsed.body_text(0).unwrap().contains("Hi Bob"));
    }
}
