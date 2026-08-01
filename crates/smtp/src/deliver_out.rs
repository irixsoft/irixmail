use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mail_send::smtp::message::Parameters;
use mail_send::{Error as SendError, SmtpClientBuilder};
use smtp_proto::Response;

use irixmail_core::{RelayConfig, Result};
use irixmail_store::{Collection, Key, Store, Subspace};

use crate::deliver_hook::day_number;
use crate::mx_resolve::MxTarget;

const METRICS_ACCOUNT: u32 = 0;

const OUTBOUND_SUFFIX: u8 = b'o';

const SMTP_PORT: u16 = 25;

const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryAttempt {
    Delivered,
    Deferred(String),
    Bounced(String),
}

impl DeliveryAttempt {
    pub fn is_delivered(&self) -> bool {
        matches!(self, DeliveryAttempt::Delivered)
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, DeliveryAttempt::Bounced(_))
    }
}

pub async fn deliver(
    targets: &[MxTarget],
    local_host: Option<&str>,
    return_path: &str,
    recipient: &str,
    raw: &[u8],
) -> DeliveryAttempt {
    if targets.is_empty() {
        return DeliveryAttempt::Deferred("no reachable mail exchange".to_string());
    }

    let mut last = DeliveryAttempt::Deferred("no mail exchange was reached".to_string());
    for target in targets {
        match deliver_to_host(&target.host, local_host, return_path, recipient, raw).await {
            DeliveryAttempt::Delivered => return DeliveryAttempt::Delivered,
            bounced @ DeliveryAttempt::Bounced(_) => return bounced,
            deferred => last = deferred,
        }
    }
    last
}

async fn deliver_to_host(
    host: &str,
    local_host: Option<&str>,
    return_path: &str,
    recipient: &str,
    raw: &[u8],
) -> DeliveryAttempt {
    let mut builder = match SmtpClientBuilder::new(host, SMTP_PORT) {
        Ok(builder) => builder.implicit_tls(false).timeout(ATTEMPT_TIMEOUT),
        Err(reason) => return DeliveryAttempt::Deferred(reason),
    };
    if let Some(local_host) = local_host {
        builder = builder.helo_host(local_host);
    }

    match builder.connect().await {
        Ok(client) => send_envelope(client, return_path, recipient, raw).await,
        Err(SendError::MissingStartTls) => match builder.connect_plain().await {
            Ok(client) => send_envelope(client, return_path, recipient, raw).await,
            Err(err) => classify(err),
        },
        Err(err) => classify(err),
    }
}

pub async fn deliver_via_relay(
    relay: &RelayConfig,
    local_host: Option<&str>,
    return_path: &str,
    recipient: &str,
    raw: &[u8],
) -> DeliveryAttempt {
    let mut builder = match SmtpClientBuilder::new(relay.host.as_str(), relay.port) {
        Ok(builder) => builder
            .implicit_tls(relay.implicit_tls)
            .timeout(ATTEMPT_TIMEOUT),
        Err(reason) => return DeliveryAttempt::Deferred(reason),
    };
    if let Some(local_host) = local_host {
        builder = builder.helo_host(local_host);
    }
    if relay.accept_invalid_certs {
        builder = builder.allow_invalid_certs();
    }
    let has_credentials = relay.username.is_some() || relay.password.is_some();
    if let (Some(user), Some(pass)) = (relay.username.as_deref(), relay.password.as_deref()) {
        builder = builder.credentials((user, pass));
    }

    match builder.connect().await {
        Ok(client) => send_envelope(client, return_path, recipient, raw).await,
        Err(SendError::MissingStartTls) => {
            if relay.require_tls {
                DeliveryAttempt::Deferred(
                    "relay does not advertise STARTTLS (require-tls is set)".to_string(),
                )
            } else if has_credentials {
                DeliveryAttempt::Deferred(
                    "relay does not advertise STARTTLS; refusing to present credentials in cleartext"
                        .to_string(),
                )
            } else {
                match builder.connect_plain().await {
                    Ok(client) => send_envelope(client, return_path, recipient, raw).await,
                    Err(err) => classify(err),
                }
            }
        }
        Err(err) => classify(err),
    }
}

async fn send_envelope<T>(
    mut client: mail_send::SmtpClient<T>,
    return_path: &str,
    recipient: &str,
    raw: &[u8],
) -> DeliveryAttempt
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let params = Parameters::default();
    if let Err(err) = client.mail_from(return_path, &params).await {
        return classify(err);
    }
    if let Err(err) = client.rcpt_to(recipient, &params).await {
        return classify(err);
    }
    if let Err(err) = client.data(raw).await {
        return classify(err);
    }
    let _ = client.quit().await;
    DeliveryAttempt::Delivered
}

fn classify(err: SendError) -> DeliveryAttempt {
    match err {
        SendError::UnexpectedReply(response) => classify_response(&response),
        SendError::AuthenticationFailed(response) => {
            DeliveryAttempt::Deferred(format!("{} {}", response.code, response.message))
        }
        SendError::MissingStartTls => {
            DeliveryAttempt::Deferred("exchange withdrew STARTTLS".to_string())
        }
        other => DeliveryAttempt::Deferred(other.to_string()),
    }
}

fn classify_response(response: &Response<String>) -> DeliveryAttempt {
    let text = format!("{} {}", response.code, response.message);
    if (500..600).contains(&response.code) {
        DeliveryAttempt::Bounced(text)
    } else {
        DeliveryAttempt::Deferred(text)
    }
}

pub fn record_outbound(store: &dyn Store, seconds_since_epoch: u64) -> Result<i64> {
    let day = day_number(seconds_since_epoch);
    store.add_and_get(&daily_outbound_key(day), 1)
}

pub fn outbound_total(store: &dyn Store, seconds_since_epoch: u64) -> Result<i64> {
    let day = day_number(seconds_since_epoch);
    store.counter(&daily_outbound_key(day))
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn daily_outbound_key(day: u32) -> Vec<u8> {
    Key::new(Subspace::Counter, METRICS_ACCOUNT, Collection::Email, day)
        .with_suffix(vec![OUTBOUND_SUFFIX])
        .encode()
}

#[cfg(test)]
pub(crate) mod test_sink {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    pub(crate) async fn rcpt_verdict_sink(verdict: fn(&str) -> &'static str) -> u16 {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut stream = BufReader::new(stream);
                    let _ = stream.get_mut().write_all(b"220 sink ESMTP\r\n").await;
                    let mut line = String::new();
                    let mut in_data = false;
                    loop {
                        line.clear();
                        if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        let trimmed = line.trim_end().to_string();
                        if in_data {
                            if trimmed == "." {
                                in_data = false;
                                let _ = stream.get_mut().write_all(b"250 queued\r\n").await;
                            }
                            continue;
                        }
                        let upper = trimmed.to_ascii_uppercase();
                        let reply: Vec<u8> =
                            if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                                b"250-sink\r\n250 SIZE 1048576\r\n".to_vec()
                            } else if upper.starts_with("RCPT") {
                                format!("{}\r\n", verdict(&trimmed)).into_bytes()
                            } else if upper.starts_with("DATA") {
                                in_data = true;
                                b"354 go ahead\r\n".to_vec()
                            } else if upper.starts_with("QUIT") {
                                let _ = stream.get_mut().write_all(b"221 bye\r\n").await;
                                return;
                            } else {
                                b"250 ok\r\n".to_vec()
                            };
                        let _ = stream.get_mut().write_all(&reply).await;
                    }
                });
            }
        });
        port
    }

    async fn run_relay_session<S>(
        stream: S,
        seen: Arc<Mutex<Vec<String>>>,
        advertise_auth: bool,
        auth_reply: &'static str,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let mut stream = BufReader::new(stream);
        let _ = stream.get_mut().write_all(b"220 sink ESMTP\r\n").await;
        let mut line = String::new();
        let mut in_data = false;
        loop {
            line.clear();
            if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }
            let trimmed = line.trim_end().to_string();
            if in_data {
                if trimmed == "." {
                    in_data = false;
                    let _ = stream.get_mut().write_all(b"250 queued\r\n").await;
                } else {
                    seen.lock().unwrap().push(format!("DATA:{trimmed}"));
                }
                continue;
            }
            seen.lock().unwrap().push(trimmed.clone());
            let upper = trimmed.to_ascii_uppercase();
            let reply: Vec<u8> = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                if advertise_auth {
                    b"250-sink\r\n250-AUTH PLAIN\r\n250 SIZE 1048576\r\n".to_vec()
                } else {
                    b"250-sink\r\n250 SIZE 1048576\r\n".to_vec()
                }
            } else if upper.starts_with("AUTH") {
                format!("{auth_reply}\r\n").into_bytes()
            } else if upper.starts_with("MAIL") || upper.starts_with("RCPT") {
                b"250 ok\r\n".to_vec()
            } else if upper.starts_with("DATA") {
                in_data = true;
                b"354 go ahead\r\n".to_vec()
            } else if upper.starts_with("QUIT") {
                let _ = stream.get_mut().write_all(b"221 bye\r\n").await;
                return;
            } else {
                b"250 ok\r\n".to_vec()
            };
            let _ = stream.get_mut().write_all(&reply).await;
        }
    }

    pub(crate) async fn relay_sink(advertise_auth: bool) -> (u16, Arc<Mutex<Vec<String>>>) {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let capture = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&capture);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let seen = Arc::clone(&seen);
                tokio::spawn(run_relay_session(
                    stream,
                    seen,
                    advertise_auth,
                    "235 2.7.0 ok",
                ));
            }
        });
        (port, capture)
    }

    pub(crate) async fn tls_relay_sink(auth_reply: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let key =
            rustls::pki_types::PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into());
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert.der().clone()], key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let capture = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&capture);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let seen = Arc::clone(&seen);
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    run_relay_session(tls, seen, true, auth_reply).await;
                });
            }
        });
        (port, capture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irixmail_store::{Flow, KeyPrefix, WriteOp};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

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

    const SECONDS_PER_DAY: u64 = 86_400;

    fn response(code: u16, message: &str) -> Response<String> {
        Response {
            code,
            esc: [0, 0, 0],
            message: message.to_string(),
        }
    }

    #[test]
    fn a_five_hundred_class_response_bounces_the_recipient() {
        let verdict = classify_response(&response(550, "no such user"));
        assert!(verdict.is_permanent());
        match verdict {
            DeliveryAttempt::Bounced(text) => assert!(text.contains("550")),
            other => panic!("expected a bounce, got {other:?}"),
        }
    }

    #[test]
    fn a_four_hundred_class_response_defers_the_recipient() {
        let verdict = classify_response(&response(451, "try again later"));
        assert!(!verdict.is_permanent());
        assert!(!verdict.is_delivered());
        match verdict {
            DeliveryAttempt::Deferred(text) => assert!(text.contains("451")),
            other => panic!("expected a deferral, got {other:?}"),
        }
    }

    #[test]
    fn an_unexpected_reply_is_classified_by_its_code() {
        let bounced = classify(SendError::UnexpectedReply(response(553, "bad mailbox")));
        assert!(bounced.is_permanent());
        let deferred = classify(SendError::UnexpectedReply(response(421, "shutting down")));
        assert!(!deferred.is_permanent());
        assert!(!deferred.is_delivered());
    }

    #[test]
    fn a_connection_fault_with_no_reply_defers() {
        let verdict = classify(SendError::Timeout);
        assert!(matches!(verdict, DeliveryAttempt::Deferred(_)));
        let verdict = classify(SendError::MissingStartTls);
        assert!(matches!(verdict, DeliveryAttempt::Deferred(_)));
    }

    #[tokio::test]
    async fn an_empty_target_set_defers_rather_than_failing() {
        let verdict = deliver(&[], None, "s@example.com", "r@example.org", b"body").await;
        assert!(matches!(verdict, DeliveryAttempt::Deferred(_)));
    }

    use super::test_sink::relay_sink;

    fn relay_config(port: u16) -> irixmail_core::RelayConfig {
        irixmail_core::RelayConfig {
            host: "127.0.0.1".to_string(),
            port,
            ..irixmail_core::RelayConfig::default()
        }
    }

    #[tokio::test]
    async fn a_configured_relay_receives_the_message_over_the_socket() {
        let (port, capture) = relay_sink(false).await;
        let verdict = deliver_via_relay(
            &relay_config(port),
            None,
            "alice@d.example",
            "bob@remote.example",
            b"Subject: via relay\r\n\r\nrelayed body\r\n",
        )
        .await;
        assert!(verdict.is_delivered());
        let seen = capture.lock().unwrap().clone();
        assert!(seen
            .iter()
            .any(|line| line == "MAIL FROM:<alice@d.example>"));
        assert!(seen
            .iter()
            .any(|line| line == "RCPT TO:<bob@remote.example>"));
        assert!(seen.iter().any(|line| line == "DATA:relayed body"));
    }

    use super::test_sink::tls_relay_sink;

    #[test]
    fn a_relay_authentication_rejection_is_classified_as_temporary() {
        let verdict = classify(SendError::AuthenticationFailed(response(
            535,
            "authentication credentials invalid",
        )));
        assert!(
            !verdict.is_permanent(),
            "a relay auth failure must defer, not bounce: {verdict:?}"
        );
        match verdict {
            DeliveryAttempt::Deferred(reason) => assert!(reason.contains("535"), "{reason}"),
            other => panic!("expected a deferral, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_relay_that_rejects_the_credentials_defers_the_recipient() {
        let (port, capture) = tls_relay_sink("535 5.7.8 authentication credentials invalid").await;
        let mut relay = relay_config(port);
        relay.username = Some("mailer".to_string());
        relay.password = Some("wrong".to_string());
        relay.implicit_tls = true;
        relay.accept_invalid_certs = true;
        let verdict = deliver_via_relay(
            &relay,
            None,
            "alice@d.example",
            "bob@remote.example",
            b"Subject: authed\r\n\r\nbody\r\n",
        )
        .await;
        let seen = capture.lock().unwrap().clone();
        assert!(
            seen.iter().any(|line| line.starts_with("AUTH")),
            "the relay never saw an AUTH attempt: {seen:?}"
        );
        match verdict {
            DeliveryAttempt::Deferred(reason) => assert!(reason.contains("535"), "{reason}"),
            other => panic!("a relay auth rejection must defer every recipient, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn credentials_are_never_presented_when_the_relay_offers_no_starttls() {
        let (port, capture) = relay_sink(true).await;
        let mut relay = relay_config(port);
        relay.username = Some("mailer".to_string());
        relay.password = Some("hunter2".to_string());
        let verdict = deliver_via_relay(
            &relay,
            None,
            "alice@d.example",
            "bob@remote.example",
            b"Subject: authed\r\n\r\nbody\r\n",
        )
        .await;
        let seen = capture.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|line| line.starts_with("AUTH")),
            "credentials must not travel over a cleartext connection: {seen:?}"
        );
        assert!(
            matches!(verdict, DeliveryAttempt::Deferred(_)),
            "delivery must defer rather than leak credentials, got {verdict:?}"
        );
    }

    #[tokio::test]
    async fn require_tls_refuses_a_relay_without_starttls() {
        let (port, capture) = relay_sink(false).await;
        let mut relay = relay_config(port);
        relay.require_tls = true;
        let verdict = deliver_via_relay(
            &relay,
            None,
            "alice@d.example",
            "bob@remote.example",
            b"Subject: plain\r\n\r\nbody\r\n",
        )
        .await;
        let seen = capture.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|line| line.starts_with("MAIL FROM")),
            "no mail may flow over cleartext when require-tls is set: {seen:?}"
        );
        assert!(
            matches!(verdict, DeliveryAttempt::Deferred(_)),
            "a require-tls relay without STARTTLS must defer, got {verdict:?}"
        );
    }

    #[tokio::test]
    async fn configured_credentials_are_presented_to_the_relay_over_tls() {
        let (port, capture) = tls_relay_sink("235 2.7.0 ok").await;
        let mut relay = relay_config(port);
        relay.username = Some("mailer".to_string());
        relay.password = Some("hunter2".to_string());
        relay.implicit_tls = true;
        relay.accept_invalid_certs = true;
        let verdict = deliver_via_relay(
            &relay,
            None,
            "alice@d.example",
            "bob@remote.example",
            b"Subject: authed\r\n\r\nbody\r\n",
        )
        .await;
        assert!(verdict.is_delivered());
        let seen = capture.lock().unwrap().clone();
        assert!(
            seen.iter()
                .any(|line| line == "AUTH PLAIN AG1haWxlcgBodW50ZXIy"),
            "AUTH PLAIN with the configured credentials was not presented: {seen:?}"
        );
    }

    #[test]
    fn recording_a_relay_returns_the_running_total_for_the_day() {
        let store = MemStore::default();
        assert_eq!(record_outbound(&store, 0).unwrap(), 1);
        assert_eq!(record_outbound(&store, 100).unwrap(), 2);
        assert_eq!(record_outbound(&store, 200).unwrap(), 3);
    }

    #[test]
    fn one_day_keeps_its_own_outbound_run_apart_from_the_next() {
        let store = MemStore::default();
        record_outbound(&store, SECONDS_PER_DAY * 2).unwrap();
        record_outbound(&store, SECONDS_PER_DAY * 2 + 5).unwrap();
        record_outbound(&store, SECONDS_PER_DAY * 3).unwrap();

        assert_eq!(outbound_total(&store, SECONDS_PER_DAY * 2).unwrap(), 2);
        assert_eq!(outbound_total(&store, SECONDS_PER_DAY * 3).unwrap(), 1);
        assert_eq!(outbound_total(&store, SECONDS_PER_DAY * 4).unwrap(), 0);
    }

    #[test]
    fn the_outbound_key_is_distinct_from_the_inbound_key_for_a_day() {
        let outbound = daily_outbound_key(11);
        let inbound = Key::new(Subspace::Counter, METRICS_ACCOUNT, Collection::Email, 11)
            .with_suffix(vec![b'i'])
            .encode();
        assert_ne!(outbound, inbound);
        assert_eq!(outbound[0], Subspace::Counter.as_byte());
    }
}
