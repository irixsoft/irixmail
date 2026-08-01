use crate::cmd_select::{select_completion, select_responses, SelectView};

pub fn examine_responses(view: &SelectView) -> Vec<String> {
    select_responses(view, true)
}

pub fn examine_completion(tag: &str) -> String {
    select_completion(tag, "EXAMINE", true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examine_opens_read_only() {
        let view = SelectView {
            uidnext: 1,
            uidvalidity: 9,
            ..SelectView::default()
        };
        assert!(examine_responses(&view)
            .concat()
            .contains("[PERMANENTFLAGS ()]"));
        assert_eq!(
            examine_completion("a"),
            "a OK [READ-ONLY] EXAMINE completed\r\n"
        );
    }
}
