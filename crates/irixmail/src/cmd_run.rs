use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use irixmail_core::{
    init_logging, BootstrapConfig, IdGenerator, LogBuffer, RuntimeConfig, Server, Shutdown, Storage,
};
use irixmail_directory::{Directory, RecoveryAdmin, SecretCipher};
use irixmail_dns::Resolver;
use irixmail_http::{
    register_http, register_http_redirect, register_https, AppState, TlsHandles as AppTlsHandles,
};
use irixmail_imap::register_imap;
use irixmail_mail::{purge_orphans, MailServices, PURGE_GRACE_SECS};
use irixmail_pop3::register_pop3;
use irixmail_smtp::{
    enqueue, register_inbound, register_inbound_tls, register_outbound, register_submission,
    wakeup_channel, Enqueue, Expiry, InboundListener, InboundServices, InboundTlsListener,
    LocalDelivery, OutboundDelivery, SubmissionServices,
};
use irixmail_store::{
    prune_change_logs, BlobStore, ChangeNotifier, FsBlobStore, RocksdbStore, Store,
};
use irixmail_tls::acme_account::production_directory;
use irixmail_tls::{
    inspect, issue_with_retry, needs_issuance, register_renewal, self_signed, AcmeAccount,
    AcmePersist, CertSource, CertStore, Http01Challenges, IssueRequest, RenewalSchedule,
    RetryPolicy, SniResolver, TlsServices,
};
use tokio::net::TcpListener;

const CHANGELOG_RETAIN: u64 = 10_000;

pub fn config_path() -> PathBuf {
    std::env::var_os("IRIXMAIL_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/irixmail/config.toml"))
}

fn socket_addr(bind: &str, port: u16) -> anyhow::Result<SocketAddr> {
    format!("{bind}:{port}")
        .parse()
        .with_context(|| format!("parsing the listener address {bind}:{port}"))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub fn run() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the tokio runtime")?;
    runtime.block_on(serve())
}

async fn serve() -> anyhow::Result<()> {
    let path = config_path();
    let config = BootstrapConfig::load(&path)
        .with_context(|| format!("loading configuration from {}", path.display()))?;
    config.validate().context("validating the configuration")?;

    let logs = LogBuffer::new();
    let _telemetry =
        init_logging(&config.log, &config.paths.logs, &logs).context("initializing logging")?;

    let shutdown = Shutdown::new();
    let (server, ready, store) = boot(&config, logs, &shutdown).await?;
    let tasks = server.registry().start_all();
    ready.store(true, std::sync::atomic::Ordering::Relaxed);
    tracing::info!("irixmail is running");

    let cause = Shutdown::wait_for_signal().await;
    wind_down(
        tasks,
        &shutdown,
        cause,
        store.as_ref(),
        &ready,
        std::time::Duration::from_secs(15),
    )
    .await;
    Ok(())
}

async fn wind_down(
    mut tasks: tokio::task::JoinSet<()>,
    shutdown: &Shutdown,
    cause: irixmail_core::ShutdownCause,
    store: &dyn Store,
    ready: &std::sync::atomic::AtomicBool,
    grace: std::time::Duration,
) {
    tracing::info!(cause = cause.label(), "graceful shutdown requested");
    ready.store(false, std::sync::atomic::Ordering::Relaxed);
    shutdown.trigger(cause);
    let deadline = tokio::time::Instant::now() + grace;
    loop {
        tokio::select! {
            joined = tasks.join_next() => {
                if joined.is_none() {
                    break;
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                tracing::warn!("the drain grace period elapsed; aborting remaining services");
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                break;
            }
        }
    }
    if let Err(err) = store.flush() {
        tracing::warn!(error = %err, "the store flush on shutdown failed");
    }
}

async fn boot(
    config: &BootstrapConfig,
    logs: LogBuffer,
    shutdown: &Shutdown,
) -> anyhow::Result<(Server, Arc<std::sync::atomic::AtomicBool>, Arc<dyn Store>)> {
    let store =
        Arc::new(RocksdbStore::open(&config.paths.db).context("opening the RocksDB store")?);
    let blobs = Arc::new(FsBlobStore::open(&config.paths.blobs).context("opening the blob store")?);

    let server = Server::new(
        RuntimeConfig::new(config.clone()),
        IdGenerator::new(config.server.node_id),
        logs.clone(),
        Storage::new(Arc::clone(&store), Arc::clone(&blobs)),
    );

    let recovery_admin =
        RecoveryAdmin::from_env().context("reading the recovery admin credential")?;
    let directory = Directory::new(
        Arc::clone(&store) as Arc<dyn Store>,
        server.ids_handle(),
        recovery_admin,
    );
    let notifier = Arc::new(ChangeNotifier::new());
    let backfill_directory = directory.clone();
    let secrets = SecretCipher::load_or_create(&config.paths.secret_key)
        .context("loading the credential encryption key")?;
    let mut state = AppState::new(
        directory,
        logs,
        Arc::clone(&store) as Arc<dyn Store>,
        Arc::clone(&blobs) as Arc<dyn BlobStore>,
        Arc::clone(&notifier),
        config.server.hostname.clone(),
        Resolver::from_system().context("initializing the DNS resolver for the API")?,
        secrets,
    );
    state.listeners = config.listeners.clone();
    let detected = irixmail_dns::public_ip::detect_all().await;
    state.public_ipv4 = irixmail_dns::public_ip::first_v4(&detected);
    state.public_ipv6 = irixmail_dns::public_ip::first_v6(&detected);

    let mail = MailServices::new(
        Arc::clone(&store) as Arc<dyn Store>,
        Arc::clone(&blobs) as Arc<dyn BlobStore>,
        Arc::clone(&notifier),
    )
    .with_hostname(config.server.hostname.clone());

    let (queue_tx, queue_rx) = wakeup_channel();
    state.queue_wakeups = Some(queue_tx.clone());

    let submit_store = Arc::clone(&store) as Arc<dyn Store>;
    let submit_blobs = Arc::clone(&blobs) as Arc<dyn BlobStore>;
    state.submitter = Some(Arc::new(
        move |raw: &[u8], return_path: &str, recipients: &[String]| {
            let now = unix_now();
            let recipients: Vec<(String, Expiry)> = recipients
                .iter()
                .map(|address| (address.clone(), Expiry::Attempts(25)))
                .collect();
            let request = Enqueue {
                created: now,
                return_path,
                recipients: &recipients,
                first_due: now,
            };
            enqueue(submit_store.as_ref(), submit_blobs.as_ref(), raw, &request)?;
            let _ = queue_tx.try_send(());
            Ok(())
        },
    ));

    let tls = TlsServices::from_server(&server).context("initializing TLS services")?;
    match tls
        .cert_store()
        .load(&config.server.hostname)
        .context("loading the stored certificate")?
    {
        Some(material) => tls
            .sni_resolver()
            .set(&material)
            .context("installing the stored certificate")?,
        None => {
            let material = self_signed::generate(vec![config.server.hostname.clone()])
                .context("generating a self-signed certificate")?;
            tls.cert_store()
                .save(&config.server.hostname, &material, CertSource::SelfSigned)
                .context("persisting the self-signed certificate")?;
            tls.sni_resolver()
                .set(&material)
                .context("installing the self-signed certificate")?;
        }
    }
    let (reissue_tx, mut reissue_rx) = tokio::sync::mpsc::channel::<()>(1);
    state.tls = Some(AppTlsHandles {
        http01: tls.http01_challenges().clone(),
        cert_store: Some(tls.cert_store_handle()),
        provider: Some(tls.provider().clone()),
        sni_resolver: Some(tls.sni_resolver_handle()),
        reissue: Some(reissue_tx),
    });
    let issuance = TlsIssuance {
        hostname: config.server.hostname.clone(),
        http01: tls.http01_challenges().clone(),
        cert_store: tls.cert_store_handle(),
        sni_resolver: tls.sni_resolver_handle(),
        persist: AcmePersist::new(crate::setup_cert::certs_dir(config)),
    };
    let renewal_issuance = issuance.clone();
    server
        .registry()
        .register_background("tls-reissue", move || async move {
            while reissue_rx.recv().await.is_some() {
                issuance.run().await;
            }
        });
    let acceptor = tls.acceptor().context("building the TLS acceptor")?;

    if let Some(https_port) = config.listeners.http.tls {
        let https_addr = format!("{}:{https_port}", config.listeners.bind);
        let https = TcpListener::bind(&https_addr)
            .await
            .with_context(|| format!("binding the HTTPS listener on {https_addr}"))?;
        register_https(
            server.registry(),
            https,
            acceptor.clone(),
            state.clone(),
            shutdown.subscribe(),
        );
    }

    let bind = config.listeners.bind.clone();
    let imap_starttls = match config.listeners.imap.plain {
        Some(port) => Some(socket_addr(&bind, port)?),
        None => None,
    };
    let imap_implicit = match config.listeners.imap.tls {
        Some(port) => Some(socket_addr(&bind, port)?),
        None => None,
    };
    register_imap(
        server.registry(),
        imap_starttls,
        imap_implicit,
        acceptor.clone(),
        state.directory.clone(),
        Arc::clone(&blobs) as Arc<dyn BlobStore>,
        Arc::clone(&notifier),
    )
    .await
    .context("registering the IMAP listeners")?;

    let pop3_starttls = match config.listeners.pop3.plain {
        Some(port) => Some(socket_addr(&bind, port)?),
        None => None,
    };
    let pop3_implicit = match config.listeners.pop3.tls {
        Some(port) => Some(socket_addr(&bind, port)?),
        None => None,
    };
    register_pop3(
        server.registry(),
        pop3_starttls,
        pop3_implicit,
        acceptor.clone(),
        state.directory.clone(),
        Arc::clone(&blobs) as Arc<dyn BlobStore>,
        Arc::clone(&notifier),
    )
    .await
    .context("registering the POP3 listeners")?;

    if let Some(port) = config.listeners.smtp.plain {
        let inbound = InboundServices::with_defaults(
            state.directory.clone(),
            mail.clone(),
            config.server.hostname.clone(),
            Vec::new(),
        )
        .context("assembling the inbound SMTP services")?;
        let listener = InboundListener::bind(socket_addr(&bind, port)?)
            .await
            .context("binding the inbound SMTP listener")?;
        register_inbound(server.registry(), listener, acceptor.clone(), inbound);
    }

    if let Some(port) = config.listeners.smtp.tls {
        let inbound = InboundServices::with_defaults(
            state.directory.clone(),
            mail.clone(),
            config.server.hostname.clone(),
            Vec::new(),
        )
        .context("assembling the implicit TLS inbound SMTP services")?;
        let listener = InboundTlsListener::bind(socket_addr(&bind, port)?)
            .await
            .context("binding the implicit TLS inbound SMTP listener")?;
        register_inbound_tls(server.registry(), listener, acceptor.clone(), inbound);
    }

    let submission = SubmissionServices::new(
        state.directory.clone(),
        Arc::clone(&store) as Arc<dyn Store>,
        Arc::clone(&blobs) as Arc<dyn BlobStore>,
    );
    let sub_starttls = match config.listeners.submission.plain {
        Some(port) => Some(socket_addr(&bind, port)?),
        None => None,
    };
    let sub_implicit = match config.listeners.submission.tls {
        Some(port) => Some(socket_addr(&bind, port)?),
        None => None,
    };
    register_submission(
        server.registry(),
        sub_starttls,
        sub_implicit,
        acceptor.clone(),
        submission,
    )
    .await
    .context("registering the submission listeners")?;

    let outbound = OutboundDelivery::new(
        Arc::clone(&store) as Arc<dyn Store>,
        Arc::clone(&blobs) as Arc<dyn BlobStore>,
        Resolver::from_system().context("initializing the outbound DNS resolver")?,
    )
    .with_relay(config.relay.clone())
    .with_hostname(config.server.hostname.clone())
    .with_local_delivery(LocalDelivery::new(state.directory.clone(), mail.clone()));
    register_outbound(
        server.registry(),
        Arc::clone(&store) as Arc<dyn Store>,
        unix_now,
        queue_rx,
        shutdown.subscribe(),
        move |batch| {
            let outbound = outbound.clone();
            async move { outbound.process(batch, unix_now()).await }
        },
    );

    let renewal_store = tls.cert_store_handle();
    let renewal_resolver = tls.sni_resolver_handle();
    let renewal_hostname = config.server.hostname.clone();
    let schedule = RenewalSchedule::default();
    let renew_before = schedule.renew_before;
    register_renewal(server.registry(), schedule, move || {
        let store = renewal_store.clone();
        let resolver = renewal_resolver.clone();
        let hostname = renewal_hostname.clone();
        let issuance = renewal_issuance.clone();
        async move {
            match store.load(&hostname) {
                Ok(Some(material)) => {
                    if let Err(err) = resolver.set(&material) {
                        tracing::warn!(error = %err, "could not reload the TLS certificate");
                    }
                    let summary = inspect(&material);
                    if needs_issuance(summary.as_ref(), unix_now(), renew_before) {
                        issuance.run().await;
                    }
                }
                Ok(None) => tracing::warn!("no TLS certificate is present to reload"),
                Err(err) => tracing::warn!(error = %err, "could not read the TLS certificate"),
            }
        }
    });

    let backfill_store = Arc::clone(&store) as Arc<dyn Store>;
    let backfill_notifier = Arc::clone(&notifier);
    server
        .registry()
        .register_background("maintenance:thread-backfill", move || async move {
            let task = tokio::task::spawn_blocking(move || {
                let accounts = backfill_directory.accounts().list()?;
                let ids: Vec<u32> = accounts.iter().map(|a| a.id as u32).collect();
                irixmail_mail::backfill_threads(
                    backfill_store.as_ref(),
                    backfill_notifier.as_ref(),
                    &ids,
                )
            });
            match task.await {
                Ok(Ok(0)) => {}
                Ok(Ok(updated)) => tracing::info!(updated, "backfilled message threads"),
                Ok(Err(err)) => tracing::warn!(error = %err, "the thread backfill failed"),
                Err(err) => tracing::warn!(error = %err, "the thread backfill task panicked"),
            }
        });

    let push_store = Arc::clone(&store) as Arc<dyn Store>;
    let push_notifier = Arc::clone(&notifier);
    let push_contact = format!("mailto:postmaster@{}", config.server.hostname);
    let push_navigate = format!("https://{}/webmail/", config.server.hostname);
    server
        .registry()
        .register_background("push:webpush", move || async move {
            irixmail_http::push_worker::run_push_worker(
                push_store,
                push_notifier,
                push_contact,
                push_navigate,
            )
            .await;
        });

    let purge_store = Arc::clone(&store) as Arc<dyn Store>;
    let purge_blobs = Arc::clone(&blobs) as Arc<dyn BlobStore>;
    server
        .registry()
        .register_background("maintenance:purge", move || async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
            loop {
                ticker.tick().await;
                match purge_orphans(
                    purge_store.as_ref(),
                    purge_blobs.as_ref(),
                    unix_now(),
                    PURGE_GRACE_SECS,
                ) {
                    Ok(0) => {}
                    Ok(purged) => tracing::info!(purged, "purged orphaned blobs"),
                    Err(err) => tracing::warn!(error = %err, "the blob purge pass failed"),
                }
                match prune_change_logs(purge_store.as_ref(), CHANGELOG_RETAIN) {
                    Ok(0) => {}
                    Ok(pruned) => tracing::info!(pruned, "pruned old change-log entries"),
                    Err(err) => tracing::warn!(error = %err, "the change-log prune pass failed"),
                }
            }
        });

    let sweep_store = irixmail_store::ExpiringStore::new(Arc::clone(&store) as Arc<dyn Store>);
    server
        .registry()
        .register_background("maintenance:expiring-sweep", move || async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
            loop {
                ticker.tick().await;
                match sweep_store.sweep_expired() {
                    Ok(0) => {}
                    Ok(swept) => tracing::info!(swept, "reclaimed expired entries"),
                    Err(err) => tracing::warn!(error = %err, "the expiring-entry sweep failed"),
                }
            }
        });

    let update_slot = Arc::clone(&state.update_available);
    server
        .registry()
        .register_background("update-check", move || async move {
            let mut ticker = tokio::time::interval_at(
                tokio::time::Instant::now() + std::time::Duration::from_secs(2 * 60),
                std::time::Duration::from_secs(6 * 60 * 60),
            );
            loop {
                ticker.tick().await;
                let found = tokio::task::spawn_blocking(crate::cmd_update::newer_release)
                    .await
                    .unwrap_or(None);
                if let Some(tag) = found {
                    tracing::info!(release = %tag, "a newer irixmail release is available");
                    if let Ok(mut slot) = update_slot.write() {
                        *slot = Some(tag);
                    }
                }
            }
        });

    let recheck_directory = state.directory.clone();
    let recheck_resolver = state.resolver.clone();
    let recheck_hostname = state.hostname.clone();
    let recheck_ipv4 = state.public_ipv4;
    let recheck_ipv6 = state.public_ipv6;
    server
        .registry()
        .register_background("dns-recheck", move || async move {
            let mut ticker = tokio::time::interval_at(
                tokio::time::Instant::now() + std::time::Duration::from_secs(5 * 60),
                std::time::Duration::from_secs(6 * 60 * 60),
            );
            loop {
                ticker.tick().await;
                let input = irixmail_http::RecheckInput {
                    directory: &recheck_directory,
                    resolver: &recheck_resolver,
                    hostname: &recheck_hostname,
                    ipv4: recheck_ipv4,
                    ipv6: recheck_ipv6,
                };
                let updated = irixmail_http::recheck_all(&input).await;
                if updated > 0 {
                    tracing::info!(updated, "dns verification status refreshed");
                }
            }
        });

    let http_port = config.listeners.http.plain.unwrap_or(80);
    let address = format!("{}:{http_port}", config.listeners.bind);
    let listener = TcpListener::bind(&address)
        .await
        .with_context(|| format!("binding the HTTP listener on {address}"))?;
    let services = Arc::clone(&state.services);
    let ready = Arc::clone(&state.ready);
    match config.listeners.http.tls {
        Some(https_port) => register_http_redirect(
            server.registry(),
            listener,
            state,
            https_port,
            shutdown.subscribe(),
        ),
        None => register_http(server.registry(), listener, state, shutdown.subscribe()),
    }
    let _ = services.set(
        server
            .registry()
            .registered()
            .into_iter()
            .map(|(name, _kind)| name)
            .collect(),
    );

    Ok((server, ready, Arc::clone(&store) as Arc<dyn Store>))
}

#[derive(Clone)]
struct TlsIssuance {
    hostname: String,
    http01: Http01Challenges,
    cert_store: Arc<CertStore>,
    sni_resolver: Arc<SniResolver>,
    persist: AcmePersist,
}

impl TlsIssuance {
    async fn account(&self) -> Option<AcmeAccount> {
        match irixmail_tls::acme_account::load_or_create(
            &self.persist,
            production_directory(),
            None,
        )
        .await
        {
            Ok(account) => Some(account),
            Err(err) => {
                tracing::warn!(error = %err, "could not obtain an ACME account");
                None
            }
        }
    }

    async fn run(&self) {
        let Some(account) = self.account().await else {
            return;
        };
        let request = IssueRequest {
            account: &account,
            domains: vec![self.hostname.clone()],
            http01: &self.http01,
        };
        match issue_with_retry(&request, &RetryPolicy::default()).await {
            Ok(material) => {
                if let Err(err) = self
                    .cert_store
                    .save(&self.hostname, &material, CertSource::Acme)
                {
                    tracing::warn!(error = %err, "could not persist the issued certificate");
                    return;
                }
                if let Err(err) = self.sni_resolver.set(&material) {
                    tracing::warn!(error = %err, "could not install the issued certificate");
                }
            }
            Err(err) => tracing::warn!(error = %err, "certificate issuance failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_core::ProtocolListener;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn ephemeral() -> ProtocolListener {
        ProtocolListener {
            plain: Some(0),
            tls: Some(0),
        }
    }

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("irixmail-boot-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn wind_down_drains_signal_aware_tasks_flushes_the_store_and_marks_unready() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        use irixmail_core::registry::Registry;
        use irixmail_core::ShutdownCause;
        use irixmail_store::{Flow, KeyPrefix, WriteOp};

        #[derive(Default)]
        struct SpyStore {
            flushed: AtomicBool,
        }

        impl Store for SpyStore {
            fn flush(&self) -> irixmail_core::Result<()> {
                self.flushed.store(true, Ordering::Relaxed);
                Ok(())
            }
            fn get(&self, _key: &[u8]) -> irixmail_core::Result<Option<Vec<u8>>> {
                Ok(None)
            }
            fn put(&self, _key: &[u8], _value: &[u8]) -> irixmail_core::Result<()> {
                Ok(())
            }
            fn delete(&self, _key: &[u8]) -> irixmail_core::Result<()> {
                Ok(())
            }
            fn iterate(
                &self,
                _prefix: &KeyPrefix,
                _visit: &mut dyn FnMut(&[u8], &[u8]) -> irixmail_core::Result<Flow>,
            ) -> irixmail_core::Result<()> {
                Ok(())
            }
            fn batch(&self, _ops: &[WriteOp]) -> irixmail_core::Result<()> {
                Ok(())
            }
            fn add_and_get(&self, _key: &[u8], _by: i64) -> irixmail_core::Result<i64> {
                Ok(0)
            }
            fn counter(&self, _key: &[u8]) -> irixmail_core::Result<i64> {
                Ok(0)
            }
        }

        let shutdown = Shutdown::new();
        let registry = Registry::new();
        let mut signal = shutdown.subscribe();
        let drained = Arc::new(AtomicBool::new(false));
        let drained_in_task = Arc::clone(&drained);
        registry.register_background("test:drain", move || async move {
            signal.recv().await;
            drained_in_task.store(true, Ordering::Relaxed);
        });
        let tasks = registry.start_all();
        let ready = std::sync::atomic::AtomicBool::new(true);
        let spy = SpyStore::default();

        wind_down(
            tasks,
            &shutdown,
            ShutdownCause::Internal,
            &spy,
            &ready,
            std::time::Duration::from_secs(5),
        )
        .await;

        assert!(
            drained.load(Ordering::Relaxed),
            "the task must finish by observing the signal, not by being aborted"
        );
        assert!(spy.flushed.load(Ordering::Relaxed));
        assert!(!ready.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn boot_registers_every_listener_and_background_service() {
        let dir = temp_dir();
        let mut config = BootstrapConfig::default();
        config.paths.db = dir.join("db");
        config.paths.blobs = dir.join("blobs");
        config.paths.logs = dir.join("logs");
        config.paths.secret_key = dir.join("credential.key");
        config.server.hostname = "mail.test".to_string();
        config.listeners.bind = "127.0.0.1".to_string();
        config.listeners.smtp = ephemeral();
        config.listeners.submission = ephemeral();
        config.listeners.imap = ephemeral();
        config.listeners.pop3 = ephemeral();
        config.listeners.http = ephemeral();

        let shutdown = Shutdown::new();
        let (server, ready, _store) = boot(&config, LogBuffer::new(), &shutdown).await.unwrap();
        assert!(!ready.load(std::sync::atomic::Ordering::Relaxed));
        let names: Vec<String> = server
            .registry()
            .registered()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        for expected in [
            "https:443",
            "imap:143",
            "imap:993",
            "pop3:110",
            "pop3:995",
            "smtp:25",
            "smtps",
            "smtp:587",
            "smtp:465",
            "smtp:queue",
            "tls-renewal",
            "tls-reissue",
            "maintenance:purge",
            "maintenance:expiring-sweep",
            "update-check",
            "dns-recheck",
            "http:80",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing {expected} in {names:?}"
            );
        }

        let mut tasks = server.registry().start_all();
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        let _ = std::fs::remove_dir_all(&dir);
    }
}
