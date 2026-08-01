use irixmail_mail::{Mailbox, SpecialUse};

use crate::parser::Token;

pub const DELIMITER: char = '/';

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListCommand {
    pub reference: String,
    pub patterns: Vec<String>,
    pub subscribed_only: bool,
    pub special_use_only: bool,
    pub recursive_match: bool,
    pub ret_subscribed: bool,
    pub ret_status: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListParse {
    Command(ListCommand),
    Bad(&'static str),
}

pub fn parse_list(args: &[Token]) -> ListParse {
    let mut command = ListCommand::default();
    let mut index = 0;
    if let Some(Token::List(options)) = args.first() {
        for word in options.iter().filter_map(Token::as_str) {
            match word.to_ascii_uppercase().as_str() {
                "SUBSCRIBED" => command.subscribed_only = true,
                "RECURSIVEMATCH" => command.recursive_match = true,
                "SPECIAL-USE" => command.special_use_only = true,
                _ => {}
            }
        }
        index = 1;
    }
    if command.recursive_match && !command.subscribed_only {
        return ListParse::Bad("RECURSIVEMATCH requires SUBSCRIBED");
    }
    command.reference = args
        .get(index)
        .and_then(Token::as_str)
        .unwrap_or("")
        .to_string();
    match args.get(index + 1) {
        Some(Token::List(patterns)) => {
            command.patterns = patterns
                .iter()
                .filter_map(Token::as_str)
                .map(str::to_string)
                .collect();
        }
        Some(other) => {
            command.patterns = vec![other.as_str().unwrap_or("").to_string()];
        }
        None => command.patterns = vec![String::new()],
    }
    if args
        .get(index + 2)
        .and_then(Token::as_str)
        .is_some_and(|word| word.eq_ignore_ascii_case("RETURN"))
    {
        if let Some(Token::List(options)) = args.get(index + 3) {
            let mut cursor = options.iter().peekable();
            while let Some(token) = cursor.next() {
                let Some(word) = token.as_str() else {
                    continue;
                };
                match word.to_ascii_uppercase().as_str() {
                    "SUBSCRIBED" => command.ret_subscribed = true,
                    "STATUS" => {
                        if let Some(Token::List(items)) = cursor.next() {
                            command.ret_status = Some(
                                items
                                    .iter()
                                    .filter_map(Token::as_str)
                                    .map(|item| item.to_ascii_uppercase())
                                    .collect(),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    ListParse::Command(command)
}

pub fn pattern_matched(pattern: &str, name: &str, role: SpecialUse) -> bool {
    matched(pattern, name, role)
}

pub fn extended_line(mailbox: &Mailbox, mailboxes: &[Mailbox], subscribed: bool) -> String {
    let mut attrs = attributes(mailbox, mailboxes);
    if subscribed {
        attrs.push_str(" \\Subscribed");
    }
    format!(
        "* LIST ({attrs}) \"{DELIMITER}\" {}\r\n",
        quoted(display_name(mailbox))
    )
}

pub fn childinfo_line(name: &str, exists: bool) -> String {
    let attrs = if exists { "" } else { "\\NonExistent" };
    format!(
        "* LIST ({attrs}) \"{DELIMITER}\" {} (\"CHILDINFO\" (\"SUBSCRIBED\"))\r\n",
        quoted(name)
    )
}

pub fn display_name(mailbox: &Mailbox) -> &str {
    if mailbox.role == SpecialUse::Inbox {
        "INBOX"
    } else {
        &mailbox.name
    }
}

pub fn list_responses(
    label: &str,
    mailboxes: &[Mailbox],
    reference: &str,
    pattern: &str,
) -> Vec<String> {
    let combined = format!("{reference}{pattern}");
    if combined.is_empty() {
        return vec![format!("* {label} (\\Noselect) \"{DELIMITER}\" \"\"\r\n")];
    }

    mailboxes
        .iter()
        .filter(|mailbox| matched(&combined, display_name(mailbox), mailbox.role))
        .map(|mailbox| {
            format!(
                "* {label} ({}) \"{DELIMITER}\" {}\r\n",
                attributes(mailbox, mailboxes),
                quoted(display_name(mailbox))
            )
        })
        .collect()
}

fn matched(pattern: &str, name: &str, role: SpecialUse) -> bool {
    if role == SpecialUse::Inbox {
        let pattern = pattern.to_ascii_uppercase();
        let name = name.to_ascii_uppercase();
        matches_pattern(pattern.as_bytes(), name.as_bytes(), DELIMITER as u8)
    } else {
        matches_pattern(pattern.as_bytes(), name.as_bytes(), DELIMITER as u8)
    }
}

fn matches_pattern(pattern: &[u8], name: &[u8], delim: u8) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some(b'*') => {
            matches_pattern(&pattern[1..], name, delim)
                || (!name.is_empty() && matches_pattern(pattern, &name[1..], delim))
        }
        Some(b'%') => {
            matches_pattern(&pattern[1..], name, delim)
                || (!name.is_empty()
                    && name[0] != delim
                    && matches_pattern(pattern, &name[1..], delim))
        }
        Some(&first) => {
            !name.is_empty()
                && name[0] == first
                && matches_pattern(&pattern[1..], &name[1..], delim)
        }
    }
}

fn attributes(mailbox: &Mailbox, mailboxes: &[Mailbox]) -> String {
    let mut attrs = Vec::new();
    if let Some(special) = mailbox.role.attribute() {
        attrs.push(special.to_string());
    }
    let prefix = format!("{}{DELIMITER}", display_name(mailbox));
    if mailboxes
        .iter()
        .any(|other| display_name(other).starts_with(&prefix))
    {
        attrs.push("\\HasChildren".to_string());
    } else {
        attrs.push("\\HasNoChildren".to_string());
    }
    attrs.join(" ")
}

pub fn quoted(name: &str) -> String {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_mail::provision::provision_mailboxes;

    fn folders() -> Vec<Mailbox> {
        provision_mailboxes(1_700_000_000_000)
    }

    #[test]
    fn a_star_pattern_lists_every_folder() {
        let lines = list_responses("LIST", &folders(), "", "*");
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().any(|line| line.contains("\"INBOX\"")));
        assert!(lines.iter().any(|line| line.contains("\"Sent\"")));
    }

    #[test]
    fn the_inbox_is_reported_in_uppercase() {
        let lines = list_responses("LIST", &folders(), "", "INBOX");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"INBOX\""));
    }

    #[test]
    fn the_inbox_matches_case_insensitively() {
        let lines = list_responses("LIST", &folders(), "", "inbox");
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn special_use_attributes_are_advertised() {
        let lines = list_responses("LIST", &folders(), "", "*");
        let sent = lines.iter().find(|line| line.contains("\"Sent\"")).unwrap();
        assert!(sent.contains("\\Sent"));
        assert!(sent.contains("\\HasNoChildren"));
        let spam = lines.iter().find(|line| line.contains("\"Spam\"")).unwrap();
        assert!(spam.contains("\\Junk"));
    }

    #[test]
    fn the_hierarchy_delimiter_is_reported_for_an_empty_pattern() {
        let lines = list_responses("LIST", &folders(), "", "");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\\Noselect"));
        assert!(lines[0].contains("\"/\""));
    }

    #[test]
    fn a_literal_name_matches_only_itself() {
        let lines = list_responses("LIST", &folders(), "", "Sent");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"Sent\""));
    }

    #[test]
    fn a_parent_folder_advertises_its_children() {
        let mut folders = folders();
        folders.push(Mailbox::new(6, "Archive", SpecialUse::None, 1));
        folders.push(Mailbox::new(7, "Archive/2020", SpecialUse::None, 1));
        let lines = list_responses("LIST", &folders, "", "*");
        let parent = lines
            .iter()
            .find(|line| line.contains("\"Archive\""))
            .unwrap();
        assert!(parent.contains("\\HasChildren"), "{parent}");
        assert!(!parent.contains("\\HasNoChildren"), "{parent}");
        let child = lines
            .iter()
            .find(|line| line.contains("\"Archive/2020\""))
            .unwrap();
        assert!(child.contains("\\HasNoChildren"), "{child}");
    }

    #[test]
    fn percent_matches_within_a_single_level() {
        assert!(matches_pattern(b"%", b"Sent", b'/'));
        assert!(!matches_pattern(b"%", b"a/b", b'/'));
        assert!(matches_pattern(b"*", b"a/b", b'/'));
    }
}
