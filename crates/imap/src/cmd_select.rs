use crate::cmd_fetch::{parse_sequence_set, SeqRange};
use crate::parser::Token;

pub const SYSTEM_FLAGS: &str = "\\Answered \\Flagged \\Deleted \\Seen \\Draft";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectView {
    pub exists: u32,
    pub recent: u32,
    pub unseen: Option<u32>,
    pub uidnext: u32,
    pub uidvalidity: u32,
    pub highest_modseq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectParams {
    pub condstore: bool,
    pub qresync: Option<QresyncParam>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QresyncParam {
    pub uidvalidity: u32,
    pub modseq: u64,
    pub known_uids: Option<Vec<SeqRange>>,
}

pub fn parse_select_params(arg: Option<&Token>) -> SelectParams {
    let mut params = SelectParams::default();
    let Some(Token::List(items)) = arg else {
        return params;
    };
    let mut cursor = items.iter().peekable();
    while let Some(token) = cursor.next() {
        let Some(word) = token.as_str() else {
            continue;
        };
        if word.eq_ignore_ascii_case("CONDSTORE") {
            params.condstore = true;
        } else if word.eq_ignore_ascii_case("QRESYNC") {
            let Some(Token::List(inner)) = cursor.next() else {
                continue;
            };
            let uidvalidity = inner
                .first()
                .and_then(Token::as_str)
                .and_then(|v| v.parse().ok());
            let modseq = inner
                .get(1)
                .and_then(Token::as_str)
                .and_then(|v| v.parse().ok());
            if let (Some(uidvalidity), Some(modseq)) = (uidvalidity, modseq) {
                let known_uids = inner
                    .get(2)
                    .and_then(Token::as_str)
                    .and_then(parse_sequence_set);
                params.qresync = Some(QresyncParam {
                    uidvalidity,
                    modseq,
                    known_uids,
                });
            }
        }
    }
    params
}

pub fn select_responses(view: &SelectView, read_only: bool) -> Vec<String> {
    let mut lines = vec![
        format!("* {} EXISTS\r\n", view.exists),
        format!("* {} RECENT\r\n", view.recent),
    ];
    if let Some(unseen) = view.unseen {
        lines.push(format!("* OK [UNSEEN {unseen}] first unseen message\r\n"));
    }
    lines.push(format!(
        "* OK [UIDVALIDITY {}] uid validity\r\n",
        view.uidvalidity
    ));
    lines.push(format!(
        "* OK [UIDNEXT {}] predicted next uid\r\n",
        view.uidnext
    ));
    lines.push(format!(
        "* OK [HIGHESTMODSEQ {}] highest mod-sequence\r\n",
        view.highest_modseq.max(1)
    ));
    lines.push(format!("* FLAGS ({SYSTEM_FLAGS})\r\n"));
    if read_only {
        lines.push("* OK [PERMANENTFLAGS ()] no permanent flags\r\n".to_string());
    } else {
        lines.push(format!(
            "* OK [PERMANENTFLAGS ({SYSTEM_FLAGS} \\*)] limited\r\n"
        ));
    }
    lines
}

pub fn select_completion(tag: &str, command: &str, read_only: bool) -> String {
    let access = if read_only { "READ-ONLY" } else { "READ-WRITE" };
    format!("{tag} OK [{access}] {command} completed\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SelectView {
        SelectView {
            exists: 0,
            recent: 0,
            unseen: None,
            uidnext: 1,
            uidvalidity: 1_700,
            highest_modseq: 1,
        }
    }

    #[test]
    fn a_read_write_select_reports_the_required_untagged_lines() {
        let lines = select_responses(&view(), false);
        let text = lines.concat();
        assert!(text.contains("* 0 EXISTS"));
        assert!(text.contains("* 0 RECENT"));
        assert!(text.contains("[UIDVALIDITY 1700]"));
        assert!(text.contains("[UIDNEXT 1]"));
        assert!(text.contains("* FLAGS (\\Answered"));
        assert!(
            text.contains("[PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)]")
        );
    }

    #[test]
    fn a_read_only_examine_has_no_permanent_flags() {
        let lines = select_responses(&view(), true);
        let text = lines.concat();
        assert!(text.contains("[PERMANENTFLAGS ()]"));
    }

    #[test]
    fn the_unseen_hint_is_optional() {
        let mut value = view();
        assert!(!select_responses(&value, false).concat().contains("UNSEEN"));
        value.unseen = Some(2);
        assert!(select_responses(&value, false)
            .concat()
            .contains("[UNSEEN 2]"));
    }

    #[test]
    fn the_completion_marks_the_access_mode() {
        assert_eq!(
            select_completion("a", "SELECT", false),
            "a OK [READ-WRITE] SELECT completed\r\n"
        );
        assert_eq!(
            select_completion("a", "EXAMINE", true),
            "a OK [READ-ONLY] EXAMINE completed\r\n"
        );
    }
}
