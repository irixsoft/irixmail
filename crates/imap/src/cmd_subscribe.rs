#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscribeOutcome {
    Subscribed,
    AlreadySubscribed,
    Missing,
    Failed,
}

pub fn subscribe_response(tag: &str, outcome: SubscribeOutcome) -> String {
    match outcome {
        SubscribeOutcome::Subscribed => format!("{tag} OK SUBSCRIBE completed\r\n"),
        SubscribeOutcome::AlreadySubscribed => {
            format!("{tag} NO mailbox is already subscribed\r\n")
        }
        SubscribeOutcome::Missing => {
            format!("{tag} NO [NONEXISTENT] mailbox does not exist\r\n")
        }
        SubscribeOutcome::Failed => format!("{tag} NO SUBSCRIBE failed\r\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_mailbox_is_subscribed() {
        assert_eq!(
            subscribe_response("a", SubscribeOutcome::Subscribed),
            "a OK SUBSCRIBE completed\r\n"
        );
    }

    #[test]
    fn an_unknown_mailbox_is_refused_with_nonexistent() {
        let reply = subscribe_response("a", SubscribeOutcome::Missing);
        assert!(reply.starts_with("a NO"));
        assert!(reply.contains("[NONEXISTENT]"));
    }

    #[test]
    fn a_redundant_subscribe_is_refused() {
        assert!(subscribe_response("a", SubscribeOutcome::AlreadySubscribed).starts_with("a NO"));
        assert!(subscribe_response("a", SubscribeOutcome::Failed).starts_with("a NO"));
    }
}
