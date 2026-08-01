#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsubscribeOutcome {
    Unsubscribed,
    NotSubscribed,
    Failed,
}

pub fn unsubscribe_response(tag: &str, outcome: UnsubscribeOutcome) -> String {
    match outcome {
        UnsubscribeOutcome::Unsubscribed => format!("{tag} OK UNSUBSCRIBE completed\r\n"),
        UnsubscribeOutcome::NotSubscribed => {
            format!("{tag} NO mailbox is not subscribed\r\n")
        }
        UnsubscribeOutcome::Failed => format!("{tag} NO UNSUBSCRIBE failed\r\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subscribed_mailbox_is_unsubscribed() {
        assert_eq!(
            unsubscribe_response("a", UnsubscribeOutcome::Unsubscribed),
            "a OK UNSUBSCRIBE completed\r\n"
        );
    }

    #[test]
    fn a_mailbox_that_was_never_subscribed_is_refused() {
        assert!(unsubscribe_response("a", UnsubscribeOutcome::NotSubscribed).starts_with("a NO"));
        assert!(unsubscribe_response("a", UnsubscribeOutcome::Failed).starts_with("a NO"));
    }
}
