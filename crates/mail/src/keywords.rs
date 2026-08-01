use crate::message_data::Keyword;

impl Keyword {
    pub fn to_imap(&self) -> &str {
        match self {
            Keyword::Seen => "\\Seen",
            Keyword::Draft => "\\Draft",
            Keyword::Flagged => "\\Flagged",
            Keyword::Answered => "\\Answered",
            Keyword::Deleted => "\\Deleted",
            Keyword::Recent => "\\Recent",
            Keyword::Junk => "$Junk",
            Keyword::NotJunk => "$NotJunk",
            Keyword::Forwarded => "$Forwarded",
            Keyword::Custom(name) => name,
        }
    }

    pub fn from_imap(atom: &str) -> Keyword {
        if let Some(system) = parse_system_flag(atom) {
            system
        } else {
            Keyword::Custom(atom.to_string())
        }
    }

    pub fn to_jmap(&self) -> Option<&str> {
        match self {
            Keyword::Seen => Some("$seen"),
            Keyword::Draft => Some("$draft"),
            Keyword::Flagged => Some("$flagged"),
            Keyword::Answered => Some("$answered"),
            Keyword::Junk => Some("$junk"),
            Keyword::NotJunk => Some("$notjunk"),
            Keyword::Forwarded => Some("$forwarded"),
            Keyword::Recent | Keyword::Deleted => None,
            Keyword::Custom(name) => Some(name),
        }
    }

    pub fn from_jmap(name: &str) -> Keyword {
        match parse_system_flag(name) {
            Some(Keyword::Recent) | Some(Keyword::Deleted) | None => {
                Keyword::Custom(name.to_string())
            }
            Some(system) => system,
        }
    }

    pub fn is_system(&self) -> bool {
        !matches!(self, Keyword::Custom(_))
    }
}

fn parse_system_flag(atom: &str) -> Option<Keyword> {
    let mut chars = atom.chars();
    let rest = match chars.next() {
        Some('\\') | Some('$') => chars.as_str(),
        _ => return None,
    };
    match rest.to_ascii_lowercase().as_str() {
        "seen" => Some(Keyword::Seen),
        "draft" => Some(Keyword::Draft),
        "flagged" => Some(Keyword::Flagged),
        "answered" => Some(Keyword::Answered),
        "deleted" => Some(Keyword::Deleted),
        "recent" => Some(Keyword::Recent),
        "junk" => Some(Keyword::Junk),
        "notjunk" => Some(Keyword::NotJunk),
        "forwarded" => Some(Keyword::Forwarded),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_flags_render_with_their_imap_prefixes() {
        assert_eq!(Keyword::Seen.to_imap(), "\\Seen");
        assert_eq!(Keyword::Draft.to_imap(), "\\Draft");
        assert_eq!(Keyword::Flagged.to_imap(), "\\Flagged");
        assert_eq!(Keyword::Answered.to_imap(), "\\Answered");
        assert_eq!(Keyword::Deleted.to_imap(), "\\Deleted");
        assert_eq!(Keyword::Recent.to_imap(), "\\Recent");
        assert_eq!(Keyword::Junk.to_imap(), "$Junk");
        assert_eq!(Keyword::NotJunk.to_imap(), "$NotJunk");
        assert_eq!(Keyword::Forwarded.to_imap(), "$Forwarded");
    }

    #[test]
    fn a_custom_keyword_renders_as_its_bare_name_for_imap() {
        let keyword = Keyword::Custom("project-x".to_string());
        assert_eq!(keyword.to_imap(), "project-x");
    }

    #[test]
    fn imap_flag_atoms_parse_back_to_their_system_keywords() {
        assert_eq!(Keyword::from_imap("\\Seen"), Keyword::Seen);
        assert_eq!(Keyword::from_imap("\\Deleted"), Keyword::Deleted);
        assert_eq!(Keyword::from_imap("\\Recent"), Keyword::Recent);
        assert_eq!(Keyword::from_imap("$Junk"), Keyword::Junk);
        assert_eq!(Keyword::from_imap("$Forwarded"), Keyword::Forwarded);
    }

    #[test]
    fn imap_flag_atoms_are_matched_without_regard_to_case() {
        assert_eq!(Keyword::from_imap("\\seen"), Keyword::Seen);
        assert_eq!(Keyword::from_imap("\\SEEN"), Keyword::Seen);
        assert_eq!(Keyword::from_imap("$junk"), Keyword::Junk);
        assert_eq!(Keyword::from_imap("$NOTJUNK"), Keyword::NotJunk);
    }

    #[test]
    fn an_unknown_imap_atom_becomes_a_custom_keyword() {
        assert_eq!(
            Keyword::from_imap("$label1"),
            Keyword::Custom("$label1".to_string())
        );
        assert_eq!(
            Keyword::from_imap("urgent"),
            Keyword::Custom("urgent".to_string())
        );
        assert_eq!(
            Keyword::from_imap("\\Nonsense"),
            Keyword::Custom("\\Nonsense".to_string())
        );
    }

    #[test]
    fn imap_round_trips_every_system_flag() {
        for keyword in [
            Keyword::Seen,
            Keyword::Draft,
            Keyword::Flagged,
            Keyword::Answered,
            Keyword::Deleted,
            Keyword::Recent,
            Keyword::Junk,
            Keyword::NotJunk,
            Keyword::Forwarded,
        ] {
            assert_eq!(Keyword::from_imap(keyword.to_imap()), keyword);
        }
    }

    #[test]
    fn jmap_keywords_render_with_their_lowercase_dollar_names() {
        assert_eq!(Keyword::Seen.to_jmap(), Some("$seen"));
        assert_eq!(Keyword::Draft.to_jmap(), Some("$draft"));
        assert_eq!(Keyword::Flagged.to_jmap(), Some("$flagged"));
        assert_eq!(Keyword::Answered.to_jmap(), Some("$answered"));
        assert_eq!(Keyword::Junk.to_jmap(), Some("$junk"));
        assert_eq!(Keyword::NotJunk.to_jmap(), Some("$notjunk"));
        assert_eq!(Keyword::Forwarded.to_jmap(), Some("$forwarded"));
    }

    #[test]
    fn the_imap_only_flags_have_no_jmap_name() {
        assert_eq!(Keyword::Recent.to_jmap(), None);
        assert_eq!(Keyword::Deleted.to_jmap(), None);
    }

    #[test]
    fn a_custom_keyword_renders_as_its_bare_name_for_jmap() {
        let keyword = Keyword::Custom("invoices".to_string());
        assert_eq!(keyword.to_jmap(), Some("invoices"));
    }

    #[test]
    fn jmap_keyword_names_parse_back_to_their_system_keywords() {
        assert_eq!(Keyword::from_jmap("$seen"), Keyword::Seen);
        assert_eq!(Keyword::from_jmap("$junk"), Keyword::Junk);
        assert_eq!(Keyword::from_jmap("$forwarded"), Keyword::Forwarded);
    }

    #[test]
    fn an_unknown_jmap_name_becomes_a_custom_keyword() {
        assert_eq!(
            Keyword::from_jmap("$myown"),
            Keyword::Custom("$myown".to_string())
        );
        assert_eq!(
            Keyword::from_jmap("label"),
            Keyword::Custom("label".to_string())
        );
    }

    #[test]
    fn a_jmap_client_cannot_set_the_imap_only_flags() {
        assert_eq!(
            Keyword::from_jmap("\\Recent"),
            Keyword::Custom("\\Recent".to_string())
        );
        assert_eq!(
            Keyword::from_jmap("$deleted"),
            Keyword::Custom("$deleted".to_string())
        );
    }

    #[test]
    fn jmap_round_trips_every_keyword_it_exposes() {
        for keyword in [
            Keyword::Seen,
            Keyword::Draft,
            Keyword::Flagged,
            Keyword::Answered,
            Keyword::Junk,
            Keyword::NotJunk,
            Keyword::Forwarded,
        ] {
            let name = keyword.to_jmap().expect("exposed keyword has a name");
            assert_eq!(Keyword::from_jmap(name), keyword);
        }
    }

    #[test]
    fn system_keywords_are_distinguished_from_custom_ones() {
        assert!(Keyword::Seen.is_system());
        assert!(Keyword::Junk.is_system());
        assert!(Keyword::Recent.is_system());
        assert!(!Keyword::Custom("anything".to_string()).is_system());
    }
}
