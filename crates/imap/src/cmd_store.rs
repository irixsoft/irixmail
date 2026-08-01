use crate::parser::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreMode {
    Replace,
    Add,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreOp {
    pub mode: StoreMode,
    pub silent: bool,
    pub flags: Vec<String>,
}

pub fn parse_unchanged_since(token: Option<&Token>) -> Option<u64> {
    let Some(Token::List(items)) = token else {
        return None;
    };
    let mut words = items.iter().filter_map(Token::as_str);
    words
        .next()
        .filter(|word| word.eq_ignore_ascii_case("UNCHANGEDSINCE"))?;
    words.next().and_then(|value| value.parse().ok())
}

pub fn parse_store(item: Option<&str>, flags_arg: Option<&Token>) -> Option<StoreOp> {
    let item = item?.to_ascii_uppercase();
    let (mode, rest) = if let Some(rest) = item.strip_prefix('+') {
        (StoreMode::Add, rest)
    } else if let Some(rest) = item.strip_prefix('-') {
        (StoreMode::Remove, rest)
    } else {
        (StoreMode::Replace, item.as_str())
    };
    let silent = rest.ends_with(".SILENT");
    let base = rest.strip_suffix(".SILENT").unwrap_or(rest);
    if base != "FLAGS" {
        return None;
    }
    let flags = match flags_arg {
        Some(Token::List(items)) => items
            .iter()
            .filter_map(Token::as_str)
            .map(str::to_string)
            .collect(),
        Some(other) => other.as_str().map(|flag| vec![flag.to_string()])?,
        None => return None,
    };
    Some(StoreOp {
        mode,
        silent,
        flags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replace_carries_the_flag_list() {
        let flags = Token::List(vec![Token::Atom("\\Seen".into())]);
        let op = parse_store(Some("FLAGS"), Some(&flags)).unwrap();
        assert_eq!(op.mode, StoreMode::Replace);
        assert!(!op.silent);
        assert_eq!(op.flags, vec!["\\Seen"]);
    }

    #[test]
    fn the_add_and_silent_variants_are_recognized() {
        let flags = Token::List(vec![Token::Atom("\\Deleted".into())]);
        let op = parse_store(Some("+FLAGS.SILENT"), Some(&flags)).unwrap();
        assert_eq!(op.mode, StoreMode::Add);
        assert!(op.silent);
    }

    #[test]
    fn the_remove_variant_is_recognized() {
        let flags = Token::Atom("\\Flagged".into());
        let op = parse_store(Some("-flags"), Some(&flags)).unwrap();
        assert_eq!(op.mode, StoreMode::Remove);
        assert_eq!(op.flags, vec!["\\Flagged"]);
    }

    #[test]
    fn an_unknown_item_is_rejected() {
        let flags = Token::List(vec![Token::Atom("\\Seen".into())]);
        assert_eq!(parse_store(Some("BOGUS"), Some(&flags)), None);
    }

    #[test]
    fn missing_flags_are_rejected() {
        assert_eq!(parse_store(Some("FLAGS"), None), None);
    }
}
