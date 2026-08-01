use std::collections::HashSet;

use irixmail_core::Result;
use irixmail_directory::Directory;
use irixmail_mail::{DeliveryRequest, MailServices, Resolution};
use irixmail_store::{Collection, Key, Subspace};

use crate::deliver_out::DeliveryAttempt;

#[derive(Clone)]
pub struct LocalDelivery {
    directory: Directory,
    mail: MailServices,
}

impl LocalDelivery {
    pub fn new(directory: Directory, mail: MailServices) -> Self {
        Self { directory, mail }
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalRoute {
    Deliver { account_id: u64 },
    Redirect { destination: String },
    Unknown,
    Remote,
}

pub fn hosted_domains(directory: &Directory) -> HashSet<String> {
    let domains = match directory.domains().list() {
        Ok(domains) => domains,
        Err(err) => {
            tracing::warn!(error = %err, "could not list the hosted domains");
            return HashSet::new();
        }
    };
    let mut hosted = HashSet::new();
    for domain in domains {
        hosted.insert(domain.name.to_ascii_lowercase());
        for alias in domain.aliases {
            hosted.insert(alias.to_ascii_lowercase());
        }
    }
    hosted
}

pub fn is_hosted(hosted: &HashSet<String>, address: &str) -> bool {
    match address.rsplit_once('@') {
        Some((_, domain)) if !domain.is_empty() => hosted.contains(&domain.to_ascii_lowercase()),
        _ => false,
    }
}

pub fn route_of(resolution: &Resolution) -> LocalRoute {
    match resolution {
        Resolution::Local { account_id, .. } => LocalRoute::Deliver {
            account_id: *account_id,
        },
        Resolution::Forward { destination } => LocalRoute::Redirect {
            destination: destination.clone(),
        },
        Resolution::Rejected | Resolution::Unknown => LocalRoute::Unknown,
    }
}

pub fn route(local: &LocalDelivery, hosted: &HashSet<String>, address: &str) -> Result<LocalRoute> {
    if !is_hosted(hosted, address) {
        return Ok(LocalRoute::Remote);
    }
    let resolution = local.mail.resolve(
        local.directory.addresses(),
        local.directory.domains(),
        local.directory.accounts(),
        address,
    )?;
    Ok(route_of(&resolution))
}

pub fn deliver_local(
    local: &LocalDelivery,
    account_id: u64,
    mail_from: &str,
    recipient: &str,
    raw: &[u8],
    now: u64,
) -> DeliveryAttempt {
    match file_locally(local, account_id, mail_from, recipient, raw, now) {
        Ok(attempt) => attempt,
        Err(err) => {
            tracing::warn!(recipient = %recipient, error = %err, "local delivery failed");
            DeliveryAttempt::Deferred(format!("local delivery failed: {err}"))
        }
    }
}

fn file_locally(
    local: &LocalDelivery,
    account_id: u64,
    mail_from: &str,
    recipient: &str,
    raw: &[u8],
    now: u64,
) -> Result<DeliveryAttempt> {
    let directory = &local.directory;
    let account = directory.accounts().get(account_id)?;
    let mut mailboxes =
        irixmail_mail::load_mailboxes(local.mail.store().as_ref(), account.id as u32)?;
    if mailboxes.is_empty() {
        mailboxes = irixmail_mail::provision_mailboxes(account.created_at);
    }

    let key = Key::new(Subspace::Counter, account.id as u32, Collection::Email, 0).encode();
    let document_id = local.mail.store().add_and_get(&key, 1)? as u32;

    let request = DeliveryRequest {
        account: &account,
        mailboxes: &mailboxes,
        mail_from,
        recipient,
        document_id,
        raw,
        target_override: None,
        received_at: now,
    };

    let outcome = crate::deliver_hook::deliver_inbound(&local.mail, &[request])?;
    if outcome
        .deliveries
        .iter()
        .any(|delivery| delivery.is_over_quota())
    {
        return Ok(DeliveryAttempt::Bounced("552 5.2.2 Mailbox full".into()));
    }
    crate::deliver_hook::enqueue_relays(&local.mail, &outcome);
    crate::deliver_hook::respond_vacations(
        &local.mail,
        &[(&account, recipient)],
        &outcome,
        mail_from,
        raw,
        now,
    );
    Ok(DeliveryAttempt::Delivered)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use irixmail_core::IdGenerator;
    use irixmail_store::{Flow, KeyPrefix, Store, WriteOp};

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
                if key.starts_with(&bound) && visit(key, value)? == Flow::Stop {
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
            Ok(Self::read_counter(&self.map.lock().unwrap(), key))
        }
    }

    fn directory() -> Directory {
        Directory::new(
            Arc::new(MemStore::default()) as Arc<dyn Store>,
            Arc::new(IdGenerator::new(1)),
            None,
        )
    }

    fn hosted_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[derive(Default)]
    struct MemBlobStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemBlobStore {
        fn digest(bytes: &[u8]) -> irixmail_store::BlobHash {
            let sum = bytes
                .iter()
                .fold(0u32, |acc, b| acc.wrapping_add(*b as u32));
            let mut raw = (bytes.len() as u32).to_be_bytes().to_vec();
            raw.extend_from_slice(&sum.to_be_bytes());
            irixmail_store::BlobHash::from_bytes(raw)
        }
    }

    impl irixmail_store::BlobStore for MemBlobStore {
        fn get(
            &self,
            hash: &irixmail_store::BlobHash,
            range: std::ops::Range<usize>,
        ) -> Result<Option<Vec<u8>>> {
            let map = self.map.lock().unwrap();
            let Some(data) = map.get(hash.as_bytes()) else {
                return Ok(None);
            };
            let start = range.start.min(data.len());
            let end = range.end.min(data.len()).max(start);
            Ok(Some(data[start..end].to_vec()))
        }

        fn put(&self, bytes: &[u8]) -> Result<irixmail_store::BlobHash> {
            let hash = Self::digest(bytes);
            self.map
                .lock()
                .unwrap()
                .insert(hash.as_bytes().to_vec(), bytes.to_vec());
            Ok(hash)
        }

        fn delete(&self, hash: &irixmail_store::BlobHash) -> Result<()> {
            self.map.lock().unwrap().remove(hash.as_bytes());
            Ok(())
        }
    }

    #[test]
    fn a_locally_routed_message_triggers_the_vacation_reply() {
        use irixmail_directory::Role;
        use irixmail_store::{BlobStore, ChangeNotifier};

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(1)), None);
        let domain = directory.domains().create("d.example", Vec::new()).unwrap();
        let mut account = directory
            .accounts()
            .create("c", domain.id, "", Role::User)
            .unwrap();
        account.vacation.enabled = true;
        account.vacation.body = "away until monday".to_string();
        directory.accounts().update(account.clone()).unwrap();

        let mail = MailServices::new(Arc::clone(&store), blobs, Arc::new(ChangeNotifier::new()));
        let local = LocalDelivery::new(directory, mail);

        let raw = b"From: sender@remote.example\r\nTo: c@d.example\r\nSubject: hi\r\n\r\nbody\r\n";
        let attempt = deliver_local(
            &local,
            account.id,
            "sender@remote.example",
            "c@d.example",
            raw,
            1_700_000_000,
        );
        assert_eq!(attempt, DeliveryAttempt::Delivered);

        let queued = crate::queue_enqueue::load(store.as_ref(), 1)
            .unwrap()
            .expect("the vacation reply is queued for a queue-routed delivery");
        assert_eq!(queued.recipients.len(), 1);
        assert_eq!(queued.recipients[0].address, "sender@remote.example");
    }

    #[test]
    fn a_hosted_domain_is_recognised_case_insensitively() {
        let hosted = hosted_set(&["hosted.example"]);
        assert!(is_hosted(&hosted, "bob@hosted.example"));
        assert!(is_hosted(&hosted, "Bob@HOSTED.Example"));
        assert!(!is_hosted(&hosted, "bob@remote.example"));
    }

    #[test]
    fn an_address_without_a_domain_is_not_hosted() {
        let hosted = hosted_set(&["hosted.example"]);
        assert!(!is_hosted(&hosted, "bob"));
        assert!(!is_hosted(&hosted, ""));
    }

    #[test]
    fn a_resolved_account_routes_to_local_delivery() {
        let resolution = Resolution::Local {
            account_id: 7,
            via_catch_all: false,
        };
        assert_eq!(route_of(&resolution), LocalRoute::Deliver { account_id: 7 });
    }

    #[test]
    fn a_forward_entry_routes_to_its_destination() {
        let resolution = Resolution::Forward {
            destination: "elsewhere@remote.example".to_string(),
        };
        assert_eq!(
            route_of(&resolution),
            LocalRoute::Redirect {
                destination: "elsewhere@remote.example".to_string()
            }
        );
    }

    #[test]
    fn a_rejected_or_unknown_address_routes_to_a_bounce() {
        assert_eq!(route_of(&Resolution::Rejected), LocalRoute::Unknown);
        assert_eq!(route_of(&Resolution::Unknown), LocalRoute::Unknown);
    }

    #[test]
    fn hosted_domains_include_aliases() {
        let directory = directory();
        directory
            .domains()
            .create("hosted.example", vec!["alt.example".to_string()])
            .unwrap();

        let hosted = hosted_domains(&directory);
        assert!(hosted.contains("hosted.example"));
        assert!(hosted.contains("alt.example"));
        assert!(!hosted.contains("remote.example"));
    }
}
