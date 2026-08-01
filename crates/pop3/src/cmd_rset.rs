use crate::session::MessageEntry;

pub fn rset(messages: &mut [MessageEntry]) -> String {
    for message in messages.iter_mut() {
        message.deleted = false;
    }
    let total: u64 = messages.iter().map(|message| message.size).sum();
    format!(
        "+OK maildrop has {} messages ({total} octets)\r\n",
        messages.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(number: u32, deleted: bool) -> MessageEntry {
        MessageEntry {
            number,
            size: 10,
            uid: format!("uid{number}"),
            document_id: number,
            deleted,
        }
    }

    #[test]
    fn deletions_are_undone() {
        let mut drop = vec![message(1, true), message(2, true)];
        let response = rset(&mut drop);
        assert!(drop.iter().all(|message| !message.deleted));
        assert!(response.contains("2 messages (20 octets)"));
    }

    #[test]
    fn an_empty_maildrop_resets_cleanly() {
        let mut drop: Vec<MessageEntry> = Vec::new();
        assert!(rset(&mut drop).starts_with("+OK"));
    }
}
