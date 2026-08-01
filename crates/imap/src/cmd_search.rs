use crate::cmd_fetch::{compress_sequence, parse_sequence_set, SeqRange};
use crate::internaldate::parse_imap_date;
use crate::parser::Token;
use irixmail_store::Field;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchError {
    BadCharset,
    Invalid,
}

pub const SUPPORTED_CHARSETS: &str = "US-ASCII UTF-8";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchKey {
    All,
    Nothing,
    Flag(String, bool),
    Larger(u32),
    Smaller(u32),
    Uid(Vec<SeqRange>),
    Sequence(Vec<SeqRange>),
    Text(String),
    FieldText(Field, String),
    Header(String, String),
    Before(i64),
    On(i64),
    Since(i64),
    SentBefore(i64),
    SentOn(i64),
    SentSince(i64),
    Not(Box<SearchKey>),
    Or(Box<SearchKey>, Box<SearchKey>),
    And(Vec<SearchKey>),
    ModSeq(u64),
    Saved,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchReturn {
    pub min: bool,
    pub max: bool,
    pub all: bool,
    pub count: bool,
    pub save: bool,
}

impl SearchReturn {
    pub fn wants_untagged(&self) -> bool {
        self.min || self.max || self.all || self.count
    }
}

pub fn split_return_options(tokens: &[Token]) -> (&[Token], Option<SearchReturn>) {
    match tokens {
        [first, Token::List(options), rest @ ..]
            if first
                .as_str()
                .is_some_and(|word| word.eq_ignore_ascii_case("RETURN")) =>
        {
            let mut ret = SearchReturn::default();
            for word in options.iter().filter_map(Token::as_str) {
                match word.to_ascii_uppercase().as_str() {
                    "MIN" => ret.min = true,
                    "MAX" => ret.max = true,
                    "ALL" => ret.all = true,
                    "COUNT" => ret.count = true,
                    "SAVE" => ret.save = true,
                    _ => {}
                }
            }
            if ret == SearchReturn::default() {
                ret.all = true;
            }
            (rest, Some(ret))
        }
        _ => (tokens, None),
    }
}

pub fn esearch_response(
    tag: &str,
    uid_mode: bool,
    matches: &[u32],
    ret: &SearchReturn,
    modseq: Option<u64>,
) -> String {
    let mut line = format!("* ESEARCH (TAG \"{tag}\")");
    if uid_mode {
        line.push_str(" UID");
    }
    if ret.count {
        line.push_str(&format!(" COUNT {}", matches.len()));
    }
    if ret.min {
        if let Some(min) = matches.first() {
            line.push_str(&format!(" MIN {min}"));
        }
    }
    if ret.max {
        if let Some(max) = matches.last() {
            line.push_str(&format!(" MAX {max}"));
        }
    }
    if ret.all && !matches.is_empty() {
        line.push_str(&format!(" ALL {}", compress_sequence(matches)));
    }
    if let Some(modseq) = modseq {
        line.push_str(&format!(" MODSEQ {modseq}"));
    }
    line.push_str("\r\n");
    line
}

impl SearchKey {
    pub fn uses_modseq(&self) -> bool {
        match self {
            SearchKey::ModSeq(_) => true,
            SearchKey::Not(inner) => inner.uses_modseq(),
            SearchKey::Or(first, second) => first.uses_modseq() || second.uses_modseq(),
            SearchKey::And(keys) => keys.iter().any(SearchKey::uses_modseq),
            _ => false,
        }
    }
}

pub fn search_response(matches: &[u32], modseq: Option<u64>) -> String {
    let mut line = String::from("* SEARCH");
    for value in matches {
        line.push(' ');
        line.push_str(&value.to_string());
    }
    if let Some(modseq) = modseq {
        line.push_str(&format!(" (MODSEQ {modseq})"));
    }
    line.push_str("\r\n");
    line
}

pub fn parse_search(tokens: &[Token]) -> Result<SearchKey, SearchError> {
    let mut keys = parse_group(check_charset(tokens)?).ok_or(SearchError::Invalid)?;
    match keys.len() {
        0 => Err(SearchError::Invalid),
        1 => Ok(keys.remove(0)),
        _ => Ok(SearchKey::And(keys)),
    }
}

fn check_charset(tokens: &[Token]) -> Result<&[Token], SearchError> {
    match tokens.first().and_then(Token::as_str) {
        Some(word) if word.eq_ignore_ascii_case("CHARSET") => {
            let name = tokens
                .get(1)
                .and_then(Token::as_str)
                .ok_or(SearchError::Invalid)?;
            if SUPPORTED_CHARSETS
                .split(' ')
                .any(|charset| charset.eq_ignore_ascii_case(name))
            {
                Ok(&tokens[2..])
            } else {
                Err(SearchError::BadCharset)
            }
        }
        _ => Ok(tokens),
    }
}

fn parse_group(tokens: &[Token]) -> Option<Vec<SearchKey>> {
    let mut cursor = Cursor { tokens, pos: 0 };
    let mut keys = Vec::new();
    while cursor.peek().is_some() {
        keys.push(parse_key(&mut cursor)?);
    }
    Some(keys)
}

struct Cursor<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&'a Token> {
        let token = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(token)
    }

    fn next_str(&mut self) -> Option<&'a str> {
        self.advance().and_then(Token::as_str)
    }
}

fn parse_key(cursor: &mut Cursor<'_>) -> Option<SearchKey> {
    let token = cursor.advance()?;
    if let Token::List(inner) = token {
        let mut keys = parse_group(inner)?;
        return Some(if keys.len() == 1 {
            keys.remove(0)
        } else {
            SearchKey::And(keys)
        });
    }
    let word = token.as_str()?.to_string();
    let key = match word.to_ascii_uppercase().as_str() {
        "ALL" => SearchKey::All,
        "SEEN" => flag("\\Seen", true),
        "UNSEEN" => flag("\\Seen", false),
        "ANSWERED" => flag("\\Answered", true),
        "UNANSWERED" => flag("\\Answered", false),
        "FLAGGED" => flag("\\Flagged", true),
        "UNFLAGGED" => flag("\\Flagged", false),
        "DELETED" => flag("\\Deleted", true),
        "UNDELETED" => flag("\\Deleted", false),
        "DRAFT" => flag("\\Draft", true),
        "UNDRAFT" => flag("\\Draft", false),
        "RECENT" | "NEW" => SearchKey::Nothing,
        "OLD" => SearchKey::All,
        "KEYWORD" => flag(cursor.next_str()?, true),
        "UNKEYWORD" => flag(cursor.next_str()?, false),
        "LARGER" => SearchKey::Larger(cursor.next_str()?.parse().ok()?),
        "SMALLER" => SearchKey::Smaller(cursor.next_str()?.parse().ok()?),
        "UID" => SearchKey::Uid(parse_sequence_set(cursor.next_str()?)?),
        "TEXT" => SearchKey::Text(cursor.next_str()?.to_string()),
        "SUBJECT" => field_text(Field::Subject, cursor)?,
        "BODY" => field_text(Field::Body, cursor)?,
        "FROM" => field_text(Field::From, cursor)?,
        "TO" => field_text(Field::To, cursor)?,
        "CC" => field_text(Field::Cc, cursor)?,
        "BCC" => field_text(Field::Bcc, cursor)?,
        "HEADER" => SearchKey::Header(
            cursor.next_str()?.to_string(),
            cursor.next_str()?.to_string(),
        ),
        "BEFORE" => SearchKey::Before(parse_imap_date(cursor.next_str()?)?),
        "ON" => SearchKey::On(parse_imap_date(cursor.next_str()?)?),
        "SINCE" => SearchKey::Since(parse_imap_date(cursor.next_str()?)?),
        "SENTBEFORE" => SearchKey::SentBefore(parse_imap_date(cursor.next_str()?)?),
        "SENTON" => SearchKey::SentOn(parse_imap_date(cursor.next_str()?)?),
        "SENTSINCE" => SearchKey::SentSince(parse_imap_date(cursor.next_str()?)?),
        "MODSEQ" => {
            let mut value = cursor.next_str()?;
            if value.starts_with('/') {
                cursor.next_str()?;
                value = cursor.next_str()?;
            }
            SearchKey::ModSeq(value.parse().ok()?)
        }
        "$" => SearchKey::Saved,
        "NOT" => SearchKey::Not(Box::new(parse_key(cursor)?)),
        "OR" => {
            let first = parse_key(cursor)?;
            let second = parse_key(cursor)?;
            SearchKey::Or(Box::new(first), Box::new(second))
        }
        _ => SearchKey::Sequence(parse_sequence_set(&word)?),
    };
    Some(key)
}

fn flag(atom: &str, present: bool) -> SearchKey {
    SearchKey::Flag(atom.to_string(), present)
}

fn field_text(field: Field, cursor: &mut Cursor<'_>) -> Option<SearchKey> {
    Some(SearchKey::FieldText(field, cursor.next_str()?.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms(words: &[&str]) -> Vec<Token> {
        words
            .iter()
            .map(|word| Token::Atom((*word).into()))
            .collect()
    }

    #[test]
    fn an_empty_result_set_has_no_numbers() {
        assert_eq!(search_response(&[], None), "* SEARCH\r\n");
    }

    #[test]
    fn matches_are_space_separated() {
        assert_eq!(search_response(&[1, 3, 5], None), "* SEARCH 1 3 5\r\n");
    }

    #[test]
    fn a_modseq_annotation_trails_the_matches() {
        assert_eq!(search_response(&[2], Some(7)), "* SEARCH 2 (MODSEQ 7)\r\n");
    }

    #[test]
    fn modseq_criteria_parse_with_and_without_entry_details() {
        assert_eq!(
            parse_search(&atoms(&["MODSEQ", "42"])),
            Ok(SearchKey::ModSeq(42))
        );
        assert_eq!(
            parse_search(&atoms(&["MODSEQ", "/flags/\\Draft", "priv", "42"])),
            Ok(SearchKey::ModSeq(42))
        );
        assert!(parse_search(&atoms(&["MODSEQ", "42"]))
            .unwrap()
            .uses_modseq());
    }

    #[test]
    fn a_lone_flag_key_parses_to_its_negation_or_presence() {
        assert_eq!(parse_search(&atoms(&["ALL"])), Ok(SearchKey::All));
        assert_eq!(parse_search(&atoms(&["unseen"])), Ok(flag("\\Seen", false)));
        assert_eq!(
            parse_search(&atoms(&["FLAGGED"])),
            Ok(flag("\\Flagged", true))
        );
    }

    #[test]
    fn recent_and_new_match_nothing_while_old_matches_everything() {
        assert_eq!(parse_search(&atoms(&["RECENT"])), Ok(SearchKey::Nothing));
        assert_eq!(parse_search(&atoms(&["NEW"])), Ok(SearchKey::Nothing));
        assert_eq!(parse_search(&atoms(&["OLD"])), Ok(SearchKey::All));
    }

    #[test]
    fn multiple_keys_form_an_implicit_and() {
        let parsed = parse_search(&atoms(&["UNSEEN", "FLAGGED"])).unwrap();
        assert_eq!(
            parsed,
            SearchKey::And(vec![flag("\\Seen", false), flag("\\Flagged", true)])
        );
        assert_eq!(parse_search(&[]), Err(SearchError::Invalid));
    }

    #[test]
    fn text_keyword_larger_and_uid_take_an_argument() {
        let parsed = parse_search(&[
            Token::Atom("TEXT".into()),
            Token::Quoted("hello world".into()),
        ])
        .unwrap();
        assert_eq!(parsed, SearchKey::Text("hello world".into()));
        assert_eq!(
            parse_search(&atoms(&["LARGER", "2048"])),
            Ok(SearchKey::Larger(2048))
        );
        assert_eq!(
            parse_search(&atoms(&["KEYWORD", "$Label1"])),
            Ok(flag("$Label1", true))
        );
        assert!(matches!(
            parse_search(&atoms(&["UID", "1:5"])),
            Ok(SearchKey::Uid(_))
        ));
    }

    #[test]
    fn not_and_or_nest_their_operands() {
        assert_eq!(
            parse_search(&atoms(&["NOT", "DELETED"])),
            Ok(SearchKey::Not(Box::new(flag("\\Deleted", true))))
        );
        assert_eq!(
            parse_search(&atoms(&["OR", "SEEN", "FLAGGED"])),
            Ok(SearchKey::Or(
                Box::new(flag("\\Seen", true)),
                Box::new(flag("\\Flagged", true))
            ))
        );
    }

    #[test]
    fn a_parenthesised_group_is_an_and_of_its_members() {
        let tokens = vec![Token::List(atoms(&["SEEN", "FLAGGED"]))];
        assert_eq!(
            parse_search(&tokens),
            Ok(SearchKey::And(vec![
                flag("\\Seen", true),
                flag("\\Flagged", true)
            ]))
        );
    }

    #[test]
    fn a_bare_number_is_a_sequence_set() {
        assert!(matches!(
            parse_search(&atoms(&["1,3:5"])),
            Ok(SearchKey::Sequence(_))
        ));
    }

    #[test]
    fn a_supported_charset_is_accepted_and_an_unknown_one_is_refused() {
        assert_eq!(
            parse_search(&atoms(&["CHARSET", "UTF-8", "ALL"])),
            Ok(SearchKey::All)
        );
        assert_eq!(
            parse_search(&atoms(&["CHARSET", "us-ascii", "ALL"])),
            Ok(SearchKey::All)
        );
        assert_eq!(
            parse_search(&atoms(&["CHARSET", "KOI8-R", "ALL"])),
            Err(SearchError::BadCharset)
        );
        assert_eq!(
            parse_search(&atoms(&["CHARSET"])),
            Err(SearchError::Invalid)
        );
    }

    #[test]
    fn date_criteria_parse_to_midnight_timestamps() {
        assert_eq!(
            parse_search(&atoms(&["SINCE", "1-Feb-2020"])),
            Ok(SearchKey::Since(1_580_515_200))
        );
        assert_eq!(
            parse_search(&atoms(&["BEFORE", "1-Feb-2020"])),
            Ok(SearchKey::Before(1_580_515_200))
        );
        assert_eq!(
            parse_search(&atoms(&["ON", "1-Feb-2020"])),
            Ok(SearchKey::On(1_580_515_200))
        );
        assert_eq!(
            parse_search(&atoms(&["SINCE", "not-a-date"])),
            Err(SearchError::Invalid)
        );
    }

    #[test]
    fn sent_date_criteria_parse_to_midnight_timestamps() {
        assert_eq!(
            parse_search(&atoms(&["SENTSINCE", "1-Feb-2021"])),
            Ok(SearchKey::SentSince(1_612_137_600))
        );
        assert_eq!(
            parse_search(&atoms(&["SENTBEFORE", "1-Feb-2021"])),
            Ok(SearchKey::SentBefore(1_612_137_600))
        );
        assert_eq!(
            parse_search(&atoms(&["SENTON", "1-Feb-2021"])),
            Ok(SearchKey::SentOn(1_612_137_600))
        );
    }

    #[test]
    fn field_scoped_text_criteria_parse_with_their_field() {
        assert_eq!(
            parse_search(&atoms(&["SUBJECT", "hello"])),
            Ok(SearchKey::FieldText(Field::Subject, "hello".into()))
        );
        assert_eq!(
            parse_search(&atoms(&["FROM", "smith"])),
            Ok(SearchKey::FieldText(Field::From, "smith".into()))
        );
        assert_eq!(
            parse_search(&atoms(&["BODY", "urgent"])),
            Ok(SearchKey::FieldText(Field::Body, "urgent".into()))
        );
        assert_eq!(
            parse_search(&atoms(&["CC", "team"])),
            Ok(SearchKey::FieldText(Field::Cc, "team".into()))
        );
    }

    #[test]
    fn header_criterion_parses_field_name_and_string() {
        assert_eq!(
            parse_search(&atoms(&["HEADER", "X-Custom", "hello"])),
            Ok(SearchKey::Header("X-Custom".into(), "hello".into()))
        );
        assert_eq!(
            parse_search(&atoms(&["HEADER", "X-Custom"])),
            Err(SearchError::Invalid)
        );
    }

    #[test]
    fn an_unsupported_criterion_is_rejected() {
        assert_eq!(
            parse_search(&atoms(&["GIBBERISH"])),
            Err(SearchError::Invalid)
        );
    }
}
