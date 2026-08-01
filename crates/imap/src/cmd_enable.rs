use crate::parser::Token;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Enabled {
    pub condstore: bool,
    pub qresync: bool,
}

pub fn parse_enable(args: &[Token]) -> Enabled {
    let mut enabled = Enabled::default();
    for word in args.iter().filter_map(Token::as_str) {
        if word.eq_ignore_ascii_case("CONDSTORE") {
            enabled.condstore = true;
        } else if word.eq_ignore_ascii_case("QRESYNC") {
            enabled.condstore = true;
            enabled.qresync = true;
        }
    }
    enabled
}

pub fn enabled_line(args: &[Token]) -> String {
    let mut names = Vec::new();
    for word in args.iter().filter_map(Token::as_str) {
        if word.eq_ignore_ascii_case("CONDSTORE") {
            names.push("CONDSTORE");
        } else if word.eq_ignore_ascii_case("QRESYNC") {
            names.push("QRESYNC");
        }
    }
    format!("* ENABLED {}\r\n", names.join(" "))
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
    fn qresync_implies_condstore() {
        let enabled = parse_enable(&atoms(&["qresync"]));
        assert!(enabled.qresync);
        assert!(enabled.condstore);
    }

    #[test]
    fn unknown_extensions_are_not_echoed() {
        assert_eq!(
            enabled_line(&atoms(&["X-BOGUS", "CONDSTORE"])),
            "* ENABLED CONDSTORE\r\n"
        );
    }

    #[test]
    fn nothing_recognized_yields_an_empty_enabled_line() {
        assert_eq!(enabled_line(&atoms(&["X-BOGUS"])), "* ENABLED \r\n");
        assert_eq!(parse_enable(&atoms(&["X-BOGUS"])), Enabled::default());
    }
}
