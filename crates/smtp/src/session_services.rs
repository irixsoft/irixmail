use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use mail_auth::MessageAuthenticator;
use serde_json::Value;

use irixmail_core::{Error, Result};
use irixmail_directory::Directory;
use irixmail_dns::Resolver;
use irixmail_mail::MailServices;
use irixmail_store::{settings_key, BlobStore, ExpiringStore, Store, TtlStore};

use crate::arc::ArcVerifier;
use crate::dkim_sign::DomainSigner;
use crate::dkim_verify::DkimVerifier;
use crate::dmarc::DmarcVerifier;
use crate::dnsbl::DnsblConfig;
use crate::greylist::{Greylist, GreylistConfig};
use crate::ratelimit_in::{RateLimiter, RateLimits};
use crate::spf::{SpfConfig, SpfVerifier};

#[derive(Clone)]
pub struct InboundServices {
    directory: Directory,
    resolver: MessageAuthenticator,
    dns: Resolver,
    spf: Arc<SpfVerifier>,
    dkim: Arc<DkimVerifier>,
    dmarc: Arc<DmarcVerifier>,
    arc: Arc<ArcVerifier>,
    dnsbl: DnsblConfig,
    greylist: Arc<Greylist>,
    rate_limiter: Arc<RateLimiter>,
    mail: MailServices,
}

impl InboundServices {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        directory: Directory,
        resolver: MessageAuthenticator,
        dns: Resolver,
        spf: Arc<SpfVerifier>,
        dkim: Arc<DkimVerifier>,
        dmarc: Arc<DmarcVerifier>,
        arc: Arc<ArcVerifier>,
        dnsbl: DnsblConfig,
        greylist: Arc<Greylist>,
        rate_limiter: Arc<RateLimiter>,
        mail: MailServices,
    ) -> Self {
        Self {
            directory,
            resolver,
            dns,
            spf,
            dkim,
            dmarc,
            arc,
            dnsbl,
            greylist,
            rate_limiter,
            mail,
        }
    }

    pub fn with_defaults(
        directory: Directory,
        mail: MailServices,
        ehlo_domain: impl Into<String>,
        dnsbl_zones: Vec<String>,
    ) -> Result<Self> {
        let ttl = Arc::new(TtlStore::new());
        let expiring = Arc::new(ExpiringStore::new(Arc::clone(mail.store())));
        Ok(Self::new(
            directory,
            authenticator()?,
            Resolver::from_system()?,
            Arc::new(SpfVerifier::new(
                authenticator()?,
                SpfConfig::new(ehlo_domain),
            )),
            Arc::new(DkimVerifier::new(authenticator()?)),
            Arc::new(DmarcVerifier::new(authenticator()?)),
            Arc::new(ArcVerifier::new(authenticator()?)),
            DnsblConfig { zones: dnsbl_zones },
            Arc::new(Greylist::new(expiring, GreylistConfig::default())),
            Arc::new(RateLimiter::new(ttl, RateLimits::default())),
            mail,
        ))
    }

    pub fn for_connection(&self) -> Self {
        let value = match self.mail.store().get(&settings_key()) {
            Ok(Some(bytes)) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) => value,
                Err(_) => return self.clone(),
            },
            _ => return self.clone(),
        };

        let mut tuned = self.clone();
        if let Some(zones) = value["antiSpam"]["dnsblZones"].as_array() {
            tuned.dnsbl = DnsblConfig {
                zones: zones
                    .iter()
                    .filter_map(|zone| zone.as_str())
                    .map(str::to_string)
                    .collect(),
            };
        }
        if let Some(window) = value["antiSpam"]["greylistWindowSeconds"].as_u64() {
            tuned.greylist = Arc::new(self.greylist.reconfigured(GreylistConfig {
                window: Duration::from_secs(window),
            }));
        }
        let limits = self.rate_limiter.limits();
        let tuned_limits = RateLimits {
            max_connections: value["rateLimits"]["maxConnectionsPerIp"]
                .as_u64()
                .map(|max| max as u32)
                .unwrap_or(limits.max_connections),
            max_messages: value["rateLimits"]["maxMessagesPerConnection"]
                .as_u64()
                .map(|max| max as u32)
                .unwrap_or(limits.max_messages),
            window: limits.window,
        };
        if tuned_limits != limits {
            tuned.rate_limiter = Arc::new(self.rate_limiter.reconfigured(tuned_limits));
        }
        tuned
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    pub fn resolver(&self) -> &MessageAuthenticator {
        &self.resolver
    }

    pub fn dns(&self) -> &Resolver {
        &self.dns
    }

    pub fn spf(&self) -> &SpfVerifier {
        &self.spf
    }

    pub fn dkim(&self) -> &DkimVerifier {
        &self.dkim
    }

    pub fn dmarc(&self) -> &DmarcVerifier {
        &self.dmarc
    }

    pub fn arc(&self) -> &ArcVerifier {
        &self.arc
    }

    pub fn dnsbl(&self) -> &DnsblConfig {
        &self.dnsbl
    }

    pub fn greylist(&self) -> &Greylist {
        &self.greylist
    }

    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.rate_limiter
    }

    pub fn mail(&self) -> &MailServices {
        &self.mail
    }
}

fn authenticator() -> Result<MessageAuthenticator> {
    MessageAuthenticator::new_system_conf()
        .map_err(|err| Error::internal(format!("could not initialize the DNS resolver: {err}")))
}

pub fn local_domains(directory: &Directory) -> HashSet<String> {
    let domains = match directory.domains().list() {
        Ok(domains) => domains,
        Err(err) => {
            tracing::warn!(error = %err, "could not list the hosted domains; treating recipients as remote");
            return HashSet::new();
        }
    };
    let mut hosted = HashSet::new();
    for domain in domains.into_iter().filter(|domain| domain.accepts_mail()) {
        hosted.insert(domain.name.to_ascii_lowercase());
        for alias in domain.aliases {
            hosted.insert(alias.to_ascii_lowercase());
        }
    }
    hosted
}

#[derive(Clone)]
pub struct SubmissionServices {
    directory: Directory,
    store: Arc<dyn Store>,
    blobs: Arc<dyn BlobStore>,
}

impl SubmissionServices {
    pub fn new(directory: Directory, store: Arc<dyn Store>, blobs: Arc<dyn BlobStore>) -> Self {
        Self {
            directory,
            store,
            blobs,
        }
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    pub fn signer(&self, domain: &str) -> Option<DomainSigner> {
        let record = self
            .directory
            .domains()
            .get_by_name(domain)
            .ok()
            .flatten()?;
        let key = self
            .directory
            .dkim()
            .get_or_create(record.id, "default")
            .ok()?;
        DomainSigner::from_key(&record.name, &key).ok()
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    pub fn blobs(&self) -> &Arc<dyn BlobStore> {
        &self.blobs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::ops::Range;
    use std::sync::Mutex;

    use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};

    use irixmail_core::{IdGenerator, Result};
    use irixmail_store::{BlobHash, ChangeNotifier, Flow, KeyPrefix, TtlStore, WriteOp};

    use crate::greylist::GreylistConfig;
    use crate::ratelimit_in::RateLimits;
    use crate::spf::SpfConfig;

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

    fn directory() -> Directory {
        Directory::new(
            Arc::new(MemStore::default()),
            Arc::new(IdGenerator::new(0)),
            None,
        )
    }

    fn resolver() -> MessageAuthenticator {
        MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default()).unwrap()
    }

    fn dns() -> Resolver {
        use hickory_resolver::config::{ResolverConfig as DnsConfig, ResolverOpts as DnsOpts};
        Resolver::from_config(DnsConfig::default(), DnsOpts::default())
    }

    fn inbound() -> InboundServices {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let notifier = Arc::new(ChangeNotifier::new());
        let ttl = Arc::new(TtlStore::new());
        let expiring = Arc::new(irixmail_store::ExpiringStore::new(Arc::clone(&store)));
        InboundServices::new(
            directory(),
            resolver(),
            dns(),
            Arc::new(SpfVerifier::new(
                resolver(),
                SpfConfig::new("mx.irixsoft.com"),
            )),
            Arc::new(DkimVerifier::new(resolver())),
            Arc::new(DmarcVerifier::new(resolver())),
            Arc::new(ArcVerifier::new(resolver())),
            DnsblConfig::default(),
            Arc::new(Greylist::new(expiring, GreylistConfig::default())),
            Arc::new(RateLimiter::new(ttl, RateLimits::default())),
            MailServices::new(store, blobs, notifier),
        )
    }

    fn submission() -> SubmissionServices {
        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        SubmissionServices::new(directory(), store, blobs)
    }

    #[test]
    fn the_inbound_bundle_hands_out_its_services() {
        let services = inbound();
        assert!(services.directory().recovery_admin().is_none());
        assert!(!services.dnsbl().is_empty());
        let _ = services.resolver();
        let _ = services.spf();
        let _ = services.dkim();
        let _ = services.dmarc();
        let _ = services.arc();
        let _ = services.greylist();
        let _ = services.rate_limiter();
        let _ = services.mail();
    }

    #[test]
    fn cloning_the_inbound_bundle_shares_its_handles() {
        let services = inbound();
        let cloned = services.clone();
        assert!(Arc::ptr_eq(&services.spf, &cloned.spf));
        assert!(Arc::ptr_eq(&services.greylist, &cloned.greylist));
    }

    #[test]
    fn local_domains_include_aliases_and_skip_disabled_domains() {
        let directory = directory();
        directory
            .domains()
            .create("hosted.example", vec!["alt.example".to_string()])
            .unwrap();
        let mut dark = directory
            .domains()
            .create("dark.example", Vec::new())
            .unwrap();
        dark.enabled = false;
        directory.domains().update(dark).unwrap();

        let hosted = local_domains(&directory);
        assert!(hosted.contains("hosted.example"));
        assert!(hosted.contains("alt.example"));
        assert!(!hosted.contains("dark.example"));
    }

    #[test]
    fn the_submission_bundle_resolves_a_signer_for_an_alias_domain() {
        let services = submission();
        services
            .directory()
            .domains()
            .create("irixsoft.com", vec!["irix.example".to_string()])
            .unwrap();

        let signer = services.signer("irix.example");
        assert!(signer.is_some(), "an alias domain resolves its signer");
    }

    #[test]
    fn the_submission_bundle_resolves_a_live_signer_ignoring_case() {
        let services = submission();
        assert!(services.signer("irixsoft.com").is_none());

        services
            .directory()
            .domains()
            .create("irixsoft.com", Vec::new())
            .unwrap();

        let signer = services.signer("IriXSoft.COM");
        assert!(
            signer.is_some(),
            "a hosted domain mints its signer on demand"
        );
        let _ = services.store();
        let _ = services.blobs();
    }
}
