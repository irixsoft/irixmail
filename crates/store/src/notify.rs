use std::collections::HashMap;

use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::key::Collection;

const CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeNotice {
    pub account_id: u32,
    pub collection: Collection,
    pub change_id: u64,
}

impl ChangeNotice {
    pub fn new(account_id: u32, collection: Collection, change_id: u64) -> Self {
        Self {
            account_id,
            collection,
            change_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMailNotice {
    pub account_id: u32,
    pub document_id: u32,
    pub mailbox_id: u32,
    pub sender: String,
    pub subject: String,
}

pub type Subscription = broadcast::Receiver<ChangeNotice>;
pub type MailSubscription = broadcast::Receiver<NewMailNotice>;

pub struct ChangeNotifier {
    channels: Mutex<HashMap<u32, broadcast::Sender<ChangeNotice>>>,
    firehose: broadcast::Sender<ChangeNotice>,
    mail_firehose: broadcast::Sender<NewMailNotice>,
}

impl Default for ChangeNotifier {
    fn default() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
            firehose: broadcast::Sender::new(CHANNEL_CAPACITY),
            mail_firehose: broadcast::Sender::new(CHANNEL_CAPACITY),
        }
    }
}

impl ChangeNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe_all(&self) -> Subscription {
        self.firehose.subscribe()
    }

    pub fn subscribe_new_mail(&self) -> MailSubscription {
        self.mail_firehose.subscribe()
    }

    pub fn notify_new_mail(&self, notice: NewMailNotice) {
        let _ = self.mail_firehose.send(notice);
    }

    pub fn subscribe(&self, account_id: u32) -> Subscription {
        let mut channels = self.channels.lock();
        let sender = channels
            .entry(account_id)
            .or_insert_with(|| broadcast::Sender::new(CHANNEL_CAPACITY));
        sender.subscribe()
    }

    pub fn notify(&self, change: ChangeNotice) -> usize {
        let _ = self.firehose.send(change);
        let mut channels = self.channels.lock();
        match channels.get(&change.account_id) {
            Some(sender) => match sender.send(change) {
                Ok(reached) => reached,
                Err(_) => {
                    channels.remove(&change.account_id);
                    0
                }
            },
            None => 0,
        }
    }

    pub fn notify_change(&self, account_id: u32, collection: Collection, change_id: u64) -> usize {
        self.notify(ChangeNotice::new(account_id, collection, change_id))
    }

    pub fn watched_account_count(&self) -> usize {
        self.channels.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::TryRecvError;

    #[tokio::test]
    async fn new_mail_notices_reach_their_subscribers() {
        let notifier = ChangeNotifier::new();
        let mut feed = notifier.subscribe_new_mail();

        notifier.notify_new_mail(NewMailNotice {
            account_id: 1,
            document_id: 42,
            mailbox_id: 3,
            sender: "Ana Lang".to_string(),
            subject: "Hello".to_string(),
        });

        let notice = feed.recv().await.unwrap();
        assert_eq!(notice.account_id, 1);
        assert_eq!(notice.document_id, 42);
        assert_eq!(notice.mailbox_id, 3);
        assert_eq!(notice.sender, "Ana Lang");
        assert_eq!(notice.subject, "Hello");
    }

    #[test]
    fn notify_with_no_subscribers_reaches_nobody() {
        let notifier = ChangeNotifier::new();
        let reached = notifier.notify_change(1, Collection::Email, 7);
        assert_eq!(reached, 0);
        assert_eq!(notifier.watched_account_count(), 0);
    }

    #[tokio::test]
    async fn a_firehose_subscriber_sees_notices_for_every_account() {
        let notifier = ChangeNotifier::new();
        let mut all = notifier.subscribe_all();

        notifier.notify_change(1, Collection::Email, 5);
        notifier.notify_change(2, Collection::Mailbox, 9);

        assert_eq!(
            all.recv().await.unwrap(),
            ChangeNotice::new(1, Collection::Email, 5)
        );
        assert_eq!(
            all.recv().await.unwrap(),
            ChangeNotice::new(2, Collection::Mailbox, 9)
        );
    }

    #[tokio::test]
    async fn the_firehose_works_without_per_account_watchers() {
        let notifier = ChangeNotifier::new();
        let mut all = notifier.subscribe_all();

        let reached = notifier.notify_change(7, Collection::Email, 1);
        assert_eq!(reached, 0, "no per-account watcher exists");
        assert_eq!(
            all.recv().await.unwrap(),
            ChangeNotice::new(7, Collection::Email, 1)
        );
    }

    #[tokio::test]
    async fn a_subscriber_receives_a_later_notice() {
        let notifier = ChangeNotifier::new();
        let mut sub = notifier.subscribe(1);

        let reached = notifier.notify_change(1, Collection::Mailbox, 42);
        assert_eq!(reached, 1);

        let change = sub.recv().await.unwrap();
        assert_eq!(
            change,
            ChangeNotice::new(1, Collection::Mailbox, 42),
            "the notice carries the coordinates it was published with"
        );
    }

    #[tokio::test]
    async fn every_watcher_of_an_account_shares_one_stream() {
        let notifier = ChangeNotifier::new();
        let mut first = notifier.subscribe(1);
        let mut second = notifier.subscribe(1);

        let reached = notifier.notify_change(1, Collection::Email, 3);
        assert_eq!(reached, 2);
        assert_eq!(notifier.watched_account_count(), 1);

        assert_eq!(
            first.recv().await.unwrap(),
            ChangeNotice::new(1, Collection::Email, 3)
        );
        assert_eq!(
            second.recv().await.unwrap(),
            ChangeNotice::new(1, Collection::Email, 3)
        );
    }

    #[tokio::test]
    async fn notices_are_isolated_per_account() {
        let notifier = ChangeNotifier::new();
        let mut watcher_one = notifier.subscribe(1);
        let mut watcher_two = notifier.subscribe(2);

        notifier.notify_change(1, Collection::Email, 5);

        assert_eq!(
            watcher_one.recv().await.unwrap(),
            ChangeNotice::new(1, Collection::Email, 5)
        );
        assert_eq!(watcher_two.try_recv(), Err(TryRecvError::Empty));
    }

    #[tokio::test]
    async fn a_subscriber_only_sees_notices_published_after_it_attached() {
        let notifier = ChangeNotifier::new();

        notifier.notify_change(1, Collection::Email, 1);

        let mut sub = notifier.subscribe(1);

        notifier.notify_change(1, Collection::Email, 2);

        let change = sub.recv().await.unwrap();
        assert_eq!(
            change.change_id, 2,
            "only the post-subscription notice arrives"
        );
        assert_eq!(sub.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn an_account_channel_is_pruned_once_its_last_watcher_drops() {
        let notifier = ChangeNotifier::new();
        let sub = notifier.subscribe(1);
        assert_eq!(notifier.watched_account_count(), 1);

        drop(sub);

        assert_eq!(notifier.watched_account_count(), 1);

        let reached = notifier.notify_change(1, Collection::Email, 9);
        assert_eq!(reached, 0);
        assert_eq!(notifier.watched_account_count(), 0);
    }

    #[tokio::test]
    async fn different_collections_in_one_account_share_the_channel() {
        let notifier = ChangeNotifier::new();
        let mut sub = notifier.subscribe(1);

        notifier.notify_change(1, Collection::Mailbox, 1);
        notifier.notify_change(1, Collection::Email, 2);

        assert_eq!(
            sub.recv().await.unwrap(),
            ChangeNotice::new(1, Collection::Mailbox, 1)
        );
        assert_eq!(
            sub.recv().await.unwrap(),
            ChangeNotice::new(1, Collection::Email, 2)
        );
    }
}
