use irixmail_core::Result;
use irixmail_store::{
    ChangeKind, ChangeLog, ChangeNotifier, Collection, Flow, KeyPrefix, Store, Subspace,
};
use serde::{Deserialize, Serialize};

const TAG_PUSH_SUBSCRIPTION: u8 = 0x34;

pub const MAX_SUBSCRIPTIONS: usize = 15;
pub const MAX_EXPIRES_SECS: u64 = 7 * 24 * 3600;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PushSubscriptionRecord {
    pub id: u64,
    pub device_client_id: String,
    pub url: String,
    pub keys: Option<PushKeys>,
    pub verification_code: String,
    pub verified: bool,
    pub expires: u64,
    pub types: Vec<String>,
    #[serde(default)]
    pub verification_sends: u32,
}

impl PushSubscriptionRecord {
    pub fn wants(&self, type_name: &str) -> bool {
        self.types.is_empty() || self.types.iter().any(|wanted| wanted == type_name)
    }
}

pub fn load_subscriptions(
    store: &dyn Store,
    account_id: u32,
    now: u64,
) -> Result<Vec<PushSubscriptionRecord>> {
    let Some(bytes) = store.get(&record_key(account_id))? else {
        return Ok(Vec::new());
    };
    let subscriptions: Vec<PushSubscriptionRecord> = serde_json::from_slice(&bytes)
        .map_err(|err| irixmail_core::Error::store(format!("push subscriptions: {err}")))?;
    Ok(subscriptions
        .into_iter()
        .filter(|subscription| subscription.expires > now)
        .collect())
}

pub fn save_subscriptions(
    store: &dyn Store,
    notifier: &ChangeNotifier,
    account_id: u32,
    subscriptions: &[PushSubscriptionRecord],
) -> Result<()> {
    write_subscriptions(store, account_id, subscriptions)?;
    let change_id = ChangeLog::new(store).record(
        account_id,
        Collection::PushSubscription,
        0,
        ChangeKind::Update,
    )?;
    notifier.notify_change(account_id, Collection::PushSubscription, change_id);
    Ok(())
}

pub fn save_subscriptions_quiet(
    store: &dyn Store,
    account_id: u32,
    subscriptions: &[PushSubscriptionRecord],
) -> Result<()> {
    write_subscriptions(store, account_id, subscriptions)
}

fn write_subscriptions(
    store: &dyn Store,
    account_id: u32,
    subscriptions: &[PushSubscriptionRecord],
) -> Result<()> {
    let key = record_key(account_id);
    if subscriptions.is_empty() {
        store.delete(&key)?;
    } else {
        let bytes = serde_json::to_vec(subscriptions)
            .map_err(|err| irixmail_core::Error::store(format!("push subscriptions: {err}")))?;
        store.put(&key, &bytes)?;
    }
    Ok(())
}

pub fn accounts_with_subscriptions(store: &dyn Store) -> Result<Vec<u32>> {
    let prefix = KeyPrefix::subspace(Subspace::Registry);
    let mut accounts = Vec::new();
    store.iterate(&prefix, &mut |key, _value| {
        if key.len() == 6 && key[1] == TAG_PUSH_SUBSCRIPTION {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&key[2..]);
            accounts.push(u32::from_be_bytes(bytes));
        }
        Ok(Flow::Continue)
    })?;
    Ok(accounts)
}

fn record_key(account_id: u32) -> Vec<u8> {
    let mut key = vec![Subspace::Registry.as_byte(), TAG_PUSH_SUBSCRIPTION];
    key.extend_from_slice(&account_id.to_be_bytes());
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_context;

    fn record(id: u64, expires: u64) -> PushSubscriptionRecord {
        PushSubscriptionRecord {
            id,
            device_client_id: format!("device-{id}"),
            url: "https://push.example.com/sub".to_string(),
            keys: None,
            verification_code: "code".to_string(),
            verified: false,
            expires,
            types: Vec::new(),
            verification_sends: 0,
        }
    }

    #[test]
    fn subscriptions_round_trip_per_account() {
        let ctx = test_context();
        let store = ctx.store.as_ref();
        save_subscriptions(store, &ctx.notifier, 1, &[record(1, u64::MAX)]).unwrap();
        save_subscriptions(
            store,
            &ctx.notifier,
            2,
            &[record(1, u64::MAX), record(2, u64::MAX)],
        )
        .unwrap();

        assert_eq!(load_subscriptions(store, 1, 0).unwrap().len(), 1);
        assert_eq!(load_subscriptions(store, 2, 0).unwrap().len(), 2);
        assert!(load_subscriptions(store, 3, 0).unwrap().is_empty());
    }

    #[test]
    fn expired_subscriptions_are_pruned_on_load() {
        let ctx = test_context();
        let store = ctx.store.as_ref();
        save_subscriptions(
            store,
            &ctx.notifier,
            1,
            &[record(1, 100), record(2, u64::MAX)],
        )
        .unwrap();

        let live = load_subscriptions(store, 1, 200).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, 2);
    }

    #[test]
    fn saving_an_empty_list_removes_the_record() {
        let ctx = test_context();
        let store = ctx.store.as_ref();
        save_subscriptions(store, &ctx.notifier, 1, &[record(1, u64::MAX)]).unwrap();
        save_subscriptions(store, &ctx.notifier, 1, &[]).unwrap();

        assert!(load_subscriptions(store, 1, 0).unwrap().is_empty());
        assert!(accounts_with_subscriptions(store).unwrap().is_empty());
    }

    #[test]
    fn accounts_with_subscriptions_lists_every_account() {
        let ctx = test_context();
        let store = ctx.store.as_ref();
        save_subscriptions(store, &ctx.notifier, 5, &[record(1, u64::MAX)]).unwrap();
        save_subscriptions(store, &ctx.notifier, 9, &[record(1, u64::MAX)]).unwrap();

        assert_eq!(accounts_with_subscriptions(store).unwrap(), vec![5, 9]);
    }

    #[test]
    fn saving_notifies_the_push_subscription_collection() {
        let ctx = test_context();
        let mut firehose = ctx.notifier.subscribe_all();
        save_subscriptions(ctx.store.as_ref(), &ctx.notifier, 1, &[record(1, u64::MAX)]).unwrap();

        let notice = firehose.try_recv().unwrap();
        assert_eq!(notice.account_id, 1);
        assert_eq!(notice.collection, Collection::PushSubscription);
    }

    #[test]
    fn quiet_saves_do_not_notify() {
        let ctx = test_context();
        let mut firehose = ctx.notifier.subscribe_all();
        save_subscriptions_quiet(ctx.store.as_ref(), 1, &[record(1, u64::MAX)]).unwrap();

        assert!(firehose.try_recv().is_err());
        assert_eq!(
            load_subscriptions(ctx.store.as_ref(), 1, 0).unwrap().len(),
            1
        );
    }

    #[test]
    fn an_empty_types_list_wants_everything() {
        let mut sub = record(1, u64::MAX);
        assert!(sub.wants("Email"));
        sub.types = vec!["Mailbox".to_string()];
        assert!(sub.wants("Mailbox"));
        assert!(!sub.wants("Email"));
    }
}
