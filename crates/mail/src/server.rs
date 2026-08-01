use std::sync::Arc;

use irixmail_core::{Result, Server};
use irixmail_directory::{AccountRegistry, AddressIndex, DomainRegistry};
use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore, RocksdbStore, Store};

use crate::deliver::{deliver, DeliveryOutcome, DeliveryRequest};
use crate::resolve::{resolve, Resolution};

#[derive(Clone)]
pub struct MailServices {
    store: Arc<dyn Store>,
    blobs: Arc<dyn BlobStore>,
    notifier: Arc<ChangeNotifier>,
    hostname: Option<String>,
}

impl MailServices {
    pub fn new(
        store: Arc<dyn Store>,
        blobs: Arc<dyn BlobStore>,
        notifier: Arc<ChangeNotifier>,
    ) -> Self {
        Self {
            store,
            blobs,
            notifier,
            hostname: None,
        }
    }

    pub fn with_hostname(mut self, hostname: impl Into<String>) -> Self {
        self.hostname = Some(hostname.into());
        self
    }

    pub fn hostname(&self) -> Option<&str> {
        self.hostname.as_deref()
    }

    pub fn from_server(server: &Server) -> Result<Self> {
        let store: Arc<dyn Store> = server.storage().store::<RocksdbStore>()?;
        let blobs: Arc<dyn BlobStore> = server.storage().blob_store::<FsBlobStore>()?;
        Ok(Self::new(store, blobs, Arc::new(ChangeNotifier::new())))
    }

    pub fn deliver(&self, request: &DeliveryRequest<'_>) -> Result<DeliveryOutcome> {
        deliver(
            self.store.as_ref(),
            self.blobs.as_ref(),
            &self.notifier,
            request,
        )
    }

    pub fn resolve(
        &self,
        index: &AddressIndex,
        domains: &DomainRegistry,
        accounts: &AccountRegistry,
        recipient: &str,
    ) -> Result<Resolution> {
        resolve(index, domains, accounts, recipient)
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    pub fn blobs(&self) -> &Arc<dyn BlobStore> {
        &self.blobs
    }

    pub fn notifier(&self) -> &Arc<ChangeNotifier> {
        &self.notifier
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ops::Range;
    use std::sync::Mutex;

    use irixmail_directory::AddressEntry;
    use irixmail_store::{BlobHash, Flow, KeyPrefix, WriteOp};

    use crate::mailbox::{Mailbox, SpecialUse};
    use irixmail_directory::{Account, Forwarding, Role, VacationResponder};

    #[derive(Default)]
    struct MemStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemStore {
        fn read_counter(map: &BTreeMap<Vec<u8>, Vec<u8>>, key: &[u8]) -> i64 {
            map.get(key)
                .map(|bytes| {
                    let mut array = [0u8; 8];
                    array.copy_from_slice(bytes);
                    i64::from_le_bytes(array)
                })
                .unwrap_or(0)
        }
    }

    impl Store for MemStore {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            Ok(self.map.lock().unwrap().get(key).cloned())
        }

        fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
            self.map
                .lock()
                .unwrap()
                .insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &[u8]) -> Result<()> {
            self.map.lock().unwrap().remove(key);
            Ok(())
        }

        fn iterate(
            &self,
            prefix: &KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            let bound = prefix.encode();
            let map = self.map.lock().unwrap();
            for (key, value) in map.iter() {
                if !key.starts_with(&bound) {
                    continue;
                }
                if visit(key, value)? == Flow::Stop {
                    break;
                }
            }
            Ok(())
        }

        fn batch(&self, ops: &[WriteOp]) -> Result<()> {
            let mut map = self.map.lock().unwrap();
            for op in ops {
                match op {
                    WriteOp::Set { key, value } => {
                        map.insert(key.clone(), value.clone());
                    }
                    WriteOp::Delete { key } => {
                        map.remove(key);
                    }
                    WriteOp::Add { key, by } => {
                        let next = Self::read_counter(&map, key) + by;
                        map.insert(key.clone(), next.to_le_bytes().to_vec());
                    }
                }
            }
            Ok(())
        }

        fn add_and_get(&self, key: &[u8], by: i64) -> Result<i64> {
            let mut map = self.map.lock().unwrap();
            let next = Self::read_counter(&map, key) + by;
            map.insert(key.to_vec(), next.to_le_bytes().to_vec());
            Ok(next)
        }

        fn counter(&self, key: &[u8]) -> Result<i64> {
            let map = self.map.lock().unwrap();
            Ok(Self::read_counter(&map, key))
        }
    }

    #[derive(Default)]
    struct MemBlobStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemBlobStore {
        fn digest(bytes: &[u8]) -> BlobHash {
            let sum = bytes
                .iter()
                .fold(0u32, |acc, b| acc.wrapping_add(*b as u32));
            let mut raw = (bytes.len() as u32).to_be_bytes().to_vec();
            raw.extend_from_slice(&sum.to_be_bytes());
            BlobHash::from_bytes(raw)
        }
    }

    impl BlobStore for MemBlobStore {
        fn get(&self, hash: &BlobHash, range: Range<usize>) -> Result<Option<Vec<u8>>> {
            let map = self.map.lock().unwrap();
            let Some(data) = map.get(hash.as_bytes()) else {
                return Ok(None);
            };
            let start = range.start.min(data.len());
            let end = range.end.min(data.len()).max(start);
            Ok(Some(data[start..end].to_vec()))
        }

        fn put(&self, bytes: &[u8]) -> Result<BlobHash> {
            let hash = Self::digest(bytes);
            self.map
                .lock()
                .unwrap()
                .insert(hash.as_bytes().to_vec(), bytes.to_vec());
            Ok(hash)
        }

        fn delete(&self, hash: &BlobHash) -> Result<()> {
            self.map.lock().unwrap().remove(hash.as_bytes());
            Ok(())
        }
    }

    const INBOX_ID: u32 = 1;

    const MESSAGE: &[u8] = concat!(
        "From: someone@example.com\r\n",
        "To: me@example.org\r\n",
        "Subject: Hello\r\n",
        "\r\n",
        "Body text.\r\n",
    )
    .as_bytes();

    fn services() -> MailServices {
        MailServices::new(
            Arc::new(MemStore::default()),
            Arc::new(MemBlobStore::default()),
            Arc::new(ChangeNotifier::new()),
        )
    }

    fn account() -> Account {
        Account {
            id: 7,
            local_part: "me".to_string(),
            domain_id: 1,
            display_name: String::new(),
            enabled: true,
            role: Role::User,
            aliases: Vec::new(),
            forwarding: Forwarding::default(),
            quota_bytes: 0,
            quota_messages: 0,
            locale: String::new(),
            timezone: String::new(),
            signature: String::new(),
            vacation: VacationResponder::default(),
            created_at: 0,
        }
    }

    fn mailboxes() -> Vec<Mailbox> {
        vec![Mailbox::new(INBOX_ID, "Inbox", SpecialUse::Inbox, 1)]
    }

    #[test]
    fn the_bundle_delivers_a_message_against_its_shared_backends() {
        let services = services();
        let account = account();
        let mailboxes = mailboxes();
        let request = DeliveryRequest {
            account: &account,
            mailboxes: &mailboxes,
            mail_from: "someone@example.com",
            recipient: "me@example.org",
            document_id: 10,
            raw: MESSAGE,
            target_override: None,
            received_at: 1_700_000_000,
        };

        let outcome = services.deliver(&request).expect("deliver");
        assert_eq!(outcome.filed_into, vec![INBOX_ID]);
        assert!(outcome.was_filed());
    }

    fn domains() -> DomainRegistry {
        DomainRegistry::new(
            Arc::new(MemStore::default()),
            Arc::new(irixmail_core::IdGenerator::new(0)),
        )
    }

    fn registry() -> AccountRegistry {
        AccountRegistry::new(
            Arc::new(MemStore::default()),
            Arc::new(irixmail_core::IdGenerator::new(0)),
        )
    }

    #[test]
    fn the_bundle_resolves_through_a_supplied_address_index() {
        let services = services();
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let index = AddressIndex::new(store);
        index
            .set(AddressEntry::account("me@example.org", 7))
            .expect("index the address");

        let resolution = services
            .resolve(&index, &domains(), &registry(), "me@example.org")
            .expect("resolve");
        assert_eq!(resolution.account_id(), Some(7));
    }

    #[test]
    fn an_unknown_recipient_resolves_to_unknown() {
        let services = services();
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let index = AddressIndex::new(store);

        let resolution = services
            .resolve(&index, &domains(), &registry(), "nobody@example.org")
            .expect("resolve");
        assert!(matches!(resolution, Resolution::Unknown));
    }

    #[test]
    fn a_delivery_wakes_a_watcher_on_the_bundle_change_hub() {
        let services = services();
        let mut watcher = services.notifier().subscribe(7);
        let account = account();
        let mailboxes = mailboxes();
        let request = DeliveryRequest {
            account: &account,
            mailboxes: &mailboxes,
            mail_from: "someone@example.com",
            recipient: "me@example.org",
            document_id: 11,
            raw: MESSAGE,
            target_override: None,
            received_at: 1_700_000_000,
        };

        services.deliver(&request).expect("deliver");
        let notice = watcher.try_recv().expect("a change notice was published");
        assert_eq!(notice.account_id, 7);
    }

    #[test]
    fn cloning_the_bundle_shares_its_backends_and_change_hub() {
        let services = services();
        let clone = services.clone();

        assert!(Arc::ptr_eq(services.store(), clone.store()));
        assert!(Arc::ptr_eq(services.blobs(), clone.blobs()));
        assert!(Arc::ptr_eq(services.notifier(), clone.notifier()));
    }
}
