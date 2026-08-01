use irixmail_mail::Mailbox;

use crate::cmd_list::list_responses;

pub fn lsub_responses(mailboxes: &[Mailbox], reference: &str, pattern: &str) -> Vec<String> {
    list_responses("LSUB", mailboxes, reference, pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_mail::provision::provision_mailboxes;

    #[test]
    fn lsub_uses_the_lsub_label() {
        let mailboxes = provision_mailboxes(1_700_000_000_000);
        let lines = lsub_responses(&mailboxes, "", "*");
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|line| line.starts_with("* LSUB")));
    }

    #[test]
    fn a_literal_pattern_matches_one_subscribed_folder() {
        let mailboxes = provision_mailboxes(1_700_000_000_000);
        let lines = lsub_responses(&mailboxes, "", "Drafts");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"Drafts\""));
    }
}
