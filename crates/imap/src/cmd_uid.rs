use crate::parser::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UidCommand {
    Fetch,
    Search,
    Store,
    Copy,
    Move,
    Expunge,
    Sort,
    Thread,
    Unknown,
}

impl UidCommand {
    pub fn parse(name: &str) -> Self {
        match name.to_ascii_uppercase().as_str() {
            "FETCH" => UidCommand::Fetch,
            "SEARCH" => UidCommand::Search,
            "STORE" => UidCommand::Store,
            "COPY" => UidCommand::Copy,
            "MOVE" => UidCommand::Move,
            "EXPUNGE" => UidCommand::Expunge,
            "SORT" => UidCommand::Sort,
            "THREAD" => UidCommand::Thread,
            _ => UidCommand::Unknown,
        }
    }
}

pub fn uid_subcommand(args: &[Token]) -> (UidCommand, &[Token]) {
    match args.split_first() {
        Some((first, rest)) => (
            first
                .as_str()
                .map(UidCommand::parse)
                .unwrap_or(UidCommand::Unknown),
            rest,
        ),
        None => (UidCommand::Unknown, &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subcommand_is_parsed_and_the_rest_returned() {
        let args = vec![
            Token::Atom("fetch".into()),
            Token::Atom("1:*".into()),
            Token::Atom("FLAGS".into()),
        ];
        let (command, rest) = uid_subcommand(&args);
        assert_eq!(command, UidCommand::Fetch);
        assert_eq!(rest.len(), 2);
    }

    #[test]
    fn every_known_subcommand_is_recognized() {
        assert_eq!(UidCommand::parse("SEARCH"), UidCommand::Search);
        assert_eq!(UidCommand::parse("store"), UidCommand::Store);
        assert_eq!(UidCommand::parse("COPY"), UidCommand::Copy);
        assert_eq!(UidCommand::parse("move"), UidCommand::Move);
        assert_eq!(UidCommand::parse("EXPUNGE"), UidCommand::Expunge);
    }

    #[test]
    fn an_unknown_subcommand_is_flagged() {
        assert_eq!(UidCommand::parse("BOGUS"), UidCommand::Unknown);
        let (command, rest) = uid_subcommand(&[]);
        assert_eq!(command, UidCommand::Unknown);
        assert!(rest.is_empty());
    }
}
