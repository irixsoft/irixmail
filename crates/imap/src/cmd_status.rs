use crate::cmd_list::quoted;
use crate::parser::Token;

pub const DEFAULT_ITEMS: [&str; 4] = ["MESSAGES", "RECENT", "UIDNEXT", "UIDVALIDITY"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusValues {
    pub messages: u32,
    pub recent: u32,
    pub uidnext: u32,
    pub uidvalidity: u32,
    pub unseen: u32,
    pub highest_modseq: u64,
}

pub fn requested_items(arg: Option<&Token>) -> Vec<String> {
    match arg.and_then(Token::as_list) {
        Some(items) if !items.is_empty() => items
            .iter()
            .filter_map(Token::as_str)
            .map(|item| item.to_ascii_uppercase())
            .collect(),
        _ => DEFAULT_ITEMS.iter().map(|item| item.to_string()).collect(),
    }
}

pub fn status_line(name: &str, requested: &[String], values: &StatusValues) -> String {
    let mut parts = Vec::new();
    for item in requested {
        let value = match item.as_str() {
            "MESSAGES" => Some(values.messages as u64),
            "RECENT" => Some(values.recent as u64),
            "UIDNEXT" => Some(values.uidnext as u64),
            "UIDVALIDITY" => Some(values.uidvalidity as u64),
            "UNSEEN" => Some(values.unseen as u64),
            "HIGHESTMODSEQ" => Some(values.highest_modseq.max(1)),
            _ => None,
        };
        if let Some(value) = value {
            parts.push(format!("{item} {value}"));
        }
    }
    format!("* STATUS {} ({})\r\n", quoted(name), parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> StatusValues {
        StatusValues {
            messages: 3,
            recent: 0,
            uidnext: 4,
            uidvalidity: 1_700,
            unseen: 1,
            highest_modseq: 1,
        }
    }

    #[test]
    fn the_requested_items_are_emitted_in_order() {
        let requested = vec!["MESSAGES".into(), "UIDNEXT".into()];
        let line = status_line("INBOX", &requested, &values());
        assert_eq!(line, "* STATUS \"INBOX\" (MESSAGES 3 UIDNEXT 4)\r\n");
    }

    #[test]
    fn unknown_items_are_dropped() {
        let requested = vec!["BOGUS".into(), "RECENT".into()];
        let line = status_line("INBOX", &requested, &values());
        assert_eq!(line, "* STATUS \"INBOX\" (RECENT 0)\r\n");
    }

    #[test]
    fn a_name_with_quotes_or_backslashes_is_escaped() {
        let requested = vec!["MESSAGES".into()];
        let line = status_line(r#"Quo"te\d"#, &requested, &values());
        assert_eq!(line, "* STATUS \"Quo\\\"te\\\\d\" (MESSAGES 3)\r\n");
    }

    #[test]
    fn a_missing_list_falls_back_to_the_defaults() {
        assert_eq!(requested_items(None), DEFAULT_ITEMS);
    }

    #[test]
    fn a_provided_list_is_uppercased() {
        let arg = Token::List(vec![
            Token::Atom("messages".into()),
            Token::Atom("unseen".into()),
        ]);
        assert_eq!(requested_items(Some(&arg)), vec!["MESSAGES", "UNSEEN"]);
    }

    #[test]
    fn an_empty_list_falls_back_to_the_defaults() {
        let arg = Token::List(Vec::new());
        assert_eq!(requested_items(Some(&arg)), DEFAULT_ITEMS);
    }
}
