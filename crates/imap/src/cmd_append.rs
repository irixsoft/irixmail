use crate::internaldate::parse_internaldate;
use crate::parser::Token;

pub const CONTINUE: &str = "+ ready for literal data\r\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendCommand {
    pub mailbox: String,
    pub flags: Vec<String>,
    pub internaldate: Option<u64>,
    pub literal_len: u32,
    pub sync: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppendError {
    MissingMailbox,
    MissingLiteral,
    BadDate { literal_len: u32, sync: bool },
}

pub fn parse_append(args: &[Token]) -> Result<AppendCommand, AppendError> {
    let mailbox = args
        .first()
        .and_then(Token::as_str)
        .ok_or(AppendError::MissingMailbox)?
        .to_string();
    let (literal_len, sync) = match args.last() {
        Some(Token::Literal { length, sync }) => (*length, *sync),
        _ => return Err(AppendError::MissingLiteral),
    };
    let flags = args
        .iter()
        .find_map(Token::as_list)
        .map(|items| {
            items
                .iter()
                .filter_map(Token::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let internaldate = match args[1..args.len() - 1]
        .iter()
        .find_map(|token| match token {
            Token::Quoted(value) => Some(value.as_str()),
            _ => None,
        }) {
        Some(value) => {
            Some(parse_internaldate(value).ok_or(AppendError::BadDate { literal_len, sync })?)
        }
        None => None,
    };
    Ok(AppendCommand {
        mailbox,
        flags,
        internaldate,
        literal_len,
        sync,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendGroup {
    pub flags: Vec<String>,
    pub internaldate: Option<u64>,
    pub literal_len: u32,
    pub sync: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Continuation {
    End,
    Group(AppendGroup),
    Bad { literal_len: u32, sync: bool },
}

pub fn parse_continuation(tokens: &[Token]) -> Continuation {
    let (literal_len, sync) = match tokens.last() {
        Some(Token::Literal { length, sync }) => (*length, *sync),
        _ => return Continuation::End,
    };
    let flags = tokens
        .iter()
        .find_map(Token::as_list)
        .map(|items| {
            items
                .iter()
                .filter_map(Token::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let internaldate = match tokens[..tokens.len() - 1]
        .iter()
        .find_map(|token| match token {
            Token::Quoted(value) => Some(value.as_str()),
            _ => None,
        }) {
        Some(value) => match parse_internaldate(value) {
            Some(stamp) => Some(stamp),
            None => return Continuation::Bad { literal_len, sync },
        },
        None => None,
    };
    Continuation::Group(AppendGroup {
        flags,
        internaldate,
        literal_len,
        sync,
    })
}

pub fn append_ok(tag: &str) -> String {
    format!("{tag} OK APPEND completed\r\n")
}

pub fn try_create(tag: &str) -> String {
    format!("{tag} NO [TRYCREATE] mailbox does not exist\r\n")
}

pub fn append_bad(tag: &str) -> String {
    format!("{tag} BAD APPEND requires a mailbox and a message literal\r\n")
}

pub fn too_big(tag: &str) -> String {
    format!("{tag} NO [TOOBIG] message exceeds the fixed limit\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mailbox_and_literal_are_extracted() {
        let args = vec![
            Token::Atom("INBOX".into()),
            Token::Literal {
                length: 42,
                sync: true,
            },
        ];
        assert_eq!(
            parse_append(&args),
            Ok(AppendCommand {
                mailbox: "INBOX".into(),
                flags: Vec::new(),
                internaldate: None,
                literal_len: 42,
                sync: true,
            })
        );
    }

    #[test]
    fn a_quoted_date_time_is_parsed_as_the_internaldate() {
        let args = vec![
            Token::Atom("INBOX".into()),
            Token::List(vec![Token::Atom("\\Seen".into())]),
            Token::Quoted("15-Apr-1985 01:02:18 +0000".into()),
            Token::Literal {
                length: 5,
                sync: false,
            },
        ];
        let command = parse_append(&args).unwrap();
        assert_eq!(command.internaldate, Some(482_374_938));
        assert_eq!(command.flags, vec!["\\Seen"]);
    }

    #[test]
    fn a_malformed_date_time_reports_the_literal_for_draining() {
        let args = vec![
            Token::Atom("INBOX".into()),
            Token::Quoted("not a date".into()),
            Token::Literal {
                length: 9,
                sync: false,
            },
        ];
        assert_eq!(
            parse_append(&args),
            Err(AppendError::BadDate {
                literal_len: 9,
                sync: false
            })
        );
    }

    #[test]
    fn a_quoted_mailbox_name_is_not_mistaken_for_a_date() {
        let args = vec![
            Token::Quoted("My Folder".into()),
            Token::Literal {
                length: 3,
                sync: true,
            },
        ];
        let command = parse_append(&args).unwrap();
        assert_eq!(command.mailbox, "My Folder");
        assert_eq!(command.internaldate, None);
    }

    #[test]
    fn a_non_synchronizing_literal_keeps_its_flag() {
        let args = vec![
            Token::Atom("INBOX".into()),
            Token::Literal {
                length: 7,
                sync: false,
            },
        ];
        let command = parse_append(&args).unwrap();
        assert_eq!(command.literal_len, 7);
        assert!(!command.sync);
    }

    #[test]
    fn flags_between_the_mailbox_and_literal_are_collected() {
        let args = vec![
            Token::Atom("INBOX".into()),
            Token::List(vec![
                Token::Atom("\\Seen".into()),
                Token::Atom("\\Draft".into()),
            ]),
            Token::Literal {
                length: 10,
                sync: true,
            },
        ];
        let command = parse_append(&args).unwrap();
        assert_eq!(command.flags, vec!["\\Seen", "\\Draft"]);
        assert_eq!(command.literal_len, 10);
    }

    #[test]
    fn a_missing_literal_is_rejected() {
        let args = vec![Token::Atom("INBOX".into())];
        assert_eq!(parse_append(&args), Err(AppendError::MissingLiteral));
    }

    #[test]
    fn a_missing_mailbox_is_rejected() {
        assert_eq!(
            parse_append(&[Token::Literal {
                length: 5,
                sync: true
            }]),
            Err(AppendError::MissingMailbox)
        );
    }

    #[test]
    fn the_responses_carry_their_status() {
        assert!(append_ok("a").starts_with("a OK"));
        assert!(try_create("a").contains("[TRYCREATE]"));
        assert!(append_bad("a").starts_with("a BAD"));
        assert!(too_big("a").contains("[TOOBIG]"));
    }
}
