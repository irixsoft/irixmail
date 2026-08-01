use std::collections::HashSet;
use std::net::SocketAddr;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use irixmail_core::{Error, Result};
use irixmail_directory::{attempt_login_blocking, Account, Directory, LoginAttempt, LoginPurpose};
use irixmail_mail::{provision_mailboxes, DeliveryRequest, DeliveryTarget, Resolution, SpecialUse};
use irixmail_store::{Collection, Key, Subspace};

use crate::cmd_auth::{
    credentials_invalid_reply, success_reply, too_many_attempts_reply, Credentials, SaslExchange,
    SaslStart, SaslStep,
};
use crate::cmd_bdat::{
    bdat_reply, chunk_disposal, chunk_ok_reply, BdatOutcome, ChunkDisposal, ChunkReceiver,
    ChunkStep,
};
use crate::cmd_data::{
    accepted_reply, data_reply, mailbox_full_reply, too_large_reply, BodyReceiver, BodyStep,
    DataOutcome,
};
use crate::cmd_ehlo::{ehlo_response, helo_response, EhloContext};
use crate::cmd_mail::{mail_reply, MailOutcome};
use crate::cmd_noop::noop_reply;
use crate::cmd_quit::quit_reply;
use crate::cmd_rcpt::{rcpt_reply, RcptOutcome, Recipient, DEFAULT_MAX_RECIPIENTS};
use crate::cmd_rset::rset_reply;
use crate::cmd_starttls::starttls_reply;
use crate::dnsbl::{self, DnsblDecision};
use crate::greylist::GreylistDecision;
use crate::inbound::{self, GauntletOutcome};
use crate::parser::{parse_command, Command};
use crate::ratelimit_in::RateDecision;
use crate::session_services::{InboundServices, SubmissionServices};
use crate::spam_decision::{Disposition, SpamDecision};
use crate::sub_auth::{guard_submission, SubmissionGate};
use crate::sub_enqueue::{enqueue_submission, Submission};
use crate::sub_from::{guard_from, OwnershipGate};

const MAX_LINE_LENGTH: usize = 1024;
const GREETING: &[u8] = b"220 IRIXMAIL ESMTP ready\r\n";
const UNROUTABLE: &[u8] = b"550 5.1.1 Mailbox unavailable\r\n";
const HOSTNAME: &str = "irixmail";
const MAX_MESSAGE_SIZE: usize = 25 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verb {
    Ehlo,
    Helo,
    Mail,
    Rcpt,
    Data,
    Bdat,
    StartTls,
    Auth,
    Rset,
    Noop,
    Quit,
    Unknown,
}

impl Verb {
    fn from_line(line: &[u8]) -> Self {
        let end = line
            .iter()
            .position(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
            .unwrap_or(line.len());
        let mut keyword = [0u8; 8];
        let word = &line[..end];
        if word.is_empty() || word.len() > keyword.len() {
            return Verb::Unknown;
        }
        for (slot, byte) in keyword.iter_mut().zip(word) {
            *slot = byte.to_ascii_uppercase();
        }
        match &keyword[..word.len()] {
            b"EHLO" => Verb::Ehlo,
            b"HELO" => Verb::Helo,
            b"MAIL" => Verb::Mail,
            b"RCPT" => Verb::Rcpt,
            b"DATA" => Verb::Data,
            b"BDAT" => Verb::Bdat,
            b"STARTTLS" => Verb::StartTls,
            b"AUTH" => Verb::Auth,
            b"RSET" => Verb::Rset,
            b"NOOP" => Verb::Noop,
            b"QUIT" => Verb::Quit,
            _ => Verb::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Connected,
    Greeted,
    Mail,
    Rcpt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmtpMode {
    Inbound,
    Submission,
}

#[derive(Clone)]
pub enum SessionServices {
    Inbound(Box<InboundServices>),
    Submission(Box<SubmissionServices>),
}

impl SessionServices {
    pub fn mode(&self) -> SmtpMode {
        match self {
            SessionServices::Inbound(_) => SmtpMode::Inbound,
            SessionServices::Submission(_) => SmtpMode::Submission,
        }
    }
}

#[derive(Default)]
pub struct SessionData {
    pub helo_domain: String,
    pub mail_from: Option<String>,
    pub smtputf8: bool,
    pub rcpt_to: Vec<String>,
    pub is_tls: bool,
    pub authenticated: bool,
}

impl SessionData {
    fn reset_transaction(&mut self) {
        self.mail_from = None;
        self.smtputf8 = false;
        self.rcpt_to.clear();
    }
}

#[derive(Default)]
pub struct AcceptedMessage {
    pub mail_from: Option<String>,
    pub rcpt_to: Vec<String>,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Close,
    Upgrade,
}

pub struct Session<S> {
    stream: BufReader<S>,
    peer: SocketAddr,
    sid: u64,
    stage: Stage,
    data: SessionData,
    accepted: Option<AcceptedMessage>,
    local_domains: HashSet<String>,
    chunks: Option<ChunkReceiver>,
    mode: SmtpMode,
    services: Option<SessionServices>,
    dnsbl: DnsblDecision,
    account: Option<Account>,
    starttls_upgrade: bool,
}

fn next_sid() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn reply_code(reply: &[u8]) -> &str {
    reply
        .get(..3)
        .and_then(|code| std::str::from_utf8(code).ok())
        .unwrap_or("---")
}

impl<S> Session<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: S, peer: SocketAddr) -> Self {
        Self {
            stream: BufReader::new(stream),
            peer,
            sid: next_sid(),
            stage: Stage::Connected,
            data: SessionData::default(),
            accepted: None,
            local_domains: HashSet::new(),
            chunks: None,
            mode: SmtpMode::Inbound,
            services: None,
            dnsbl: DnsblDecision::Allow,
            account: None,
            starttls_upgrade: false,
        }
    }

    pub fn with_local_domains(mut self, domains: HashSet<String>) -> Self {
        self.local_domains = domains;
        self
    }

    pub fn with_session_id(mut self, sid: u64) -> Self {
        self.sid = sid;
        self
    }

    pub fn session_id(&self) -> u64 {
        self.sid
    }

    pub fn with_tls(mut self) -> Self {
        self.data.is_tls = true;
        self
    }

    pub fn with_starttls_upgrade(mut self) -> Self {
        self.data.is_tls = true;
        self.starttls_upgrade = true;
        self
    }

    pub fn with_mode(mut self, mode: SmtpMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_inbound_services(self, services: InboundServices) -> Self {
        self.with_services(SessionServices::Inbound(Box::new(services)))
    }

    pub fn with_submission_services(self, services: SubmissionServices) -> Self {
        self.with_services(SessionServices::Submission(Box::new(services)))
    }

    fn with_services(mut self, services: SessionServices) -> Self {
        self.mode = services.mode();
        self.services = Some(services);
        self
    }

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    pub fn mode(&self) -> SmtpMode {
        self.mode
    }

    pub fn services(&self) -> Option<&SessionServices> {
        self.services.as_ref()
    }

    pub fn inbound_services(&self) -> Option<&InboundServices> {
        match &self.services {
            Some(SessionServices::Inbound(services)) => Some(services.as_ref()),
            _ => None,
        }
    }

    pub fn submission_services(&self) -> Option<&SubmissionServices> {
        match &self.services {
            Some(SessionServices::Submission(services)) => Some(services.as_ref()),
            _ => None,
        }
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn data(&self) -> &SessionData {
        &self.data
    }

    pub fn last_accepted(&self) -> Option<&AcceptedMessage> {
        self.accepted.as_ref()
    }

    pub fn into_inner(self) -> S {
        self.stream.into_inner()
    }

    pub async fn run(&mut self) -> Result<Flow> {
        if let Some(reply) = self.rate_limit_connection() {
            self.write(reply).await?;
            return Ok(Flow::Close);
        }
        self.check_blocklist().await;
        if self.mode == SmtpMode::Inbound {
            if self.starttls_upgrade {
                tracing::info!(
                    target: "irixmail::smtp::inbound",
                    sid = self.sid,
                    peer = %self.peer,
                    "starttls upgraded"
                );
            } else {
                tracing::info!(
                    target: "irixmail::smtp::inbound",
                    sid = self.sid,
                    peer = %self.peer,
                    tls = self.data.is_tls,
                    "connection accepted"
                );
            }
        }
        if !self.starttls_upgrade {
            self.write(GREETING).await?;
        }

        let mut line = Vec::with_capacity(MAX_LINE_LENGTH);
        loop {
            line.clear();
            if !self.read_line(&mut line).await? {
                return Ok(Flow::Close);
            }
            match self.dispatch(&line).await? {
                Some(flow) => return Ok(flow),
                None => continue,
            }
        }
    }

    async fn dispatch(&mut self, line: &[u8]) -> Result<Option<Flow>> {
        match Verb::from_line(line) {
            Verb::Ehlo => {
                self.data.reset_transaction();
                self.chunks = None;
                self.data.helo_domain = helo_argument(line);
                self.stage = Stage::Greeted;
                let mut ctx = EhloContext::new(HOSTNAME);
                ctx.is_tls = self.data.is_tls;
                ctx.authenticated = self.data.authenticated;
                let reply = ehlo_response(&ctx);
                self.write(&reply).await?;
                Ok(None)
            }
            Verb::Helo => {
                self.data.reset_transaction();
                self.chunks = None;
                self.data.helo_domain = helo_argument(line);
                self.stage = Stage::Greeted;
                let reply = helo_response(HOSTNAME);
                self.write(&reply).await?;
                Ok(None)
            }
            Verb::Mail => {
                if let Some(reply) = self.submission_gate() {
                    self.write(reply).await?;
                    return Ok(None);
                }
                let params = match parse_command(&terminated(line)) {
                    Ok(Command::Mail { from }) => from,
                    _ => {
                        self.write(b"501 5.5.4 Syntax error in MAIL command\r\n")
                            .await?;
                        return Ok(None);
                    }
                };
                if let OwnershipGate::Reject(reply) = self.check_sender(&params.address) {
                    self.write(reply).await?;
                    return Ok(None);
                }
                let greeted = self.stage != Stage::Connected;
                let sender_pending = self.data.mail_from.is_some();
                match mail_reply(&params, greeted, sender_pending, MAX_MESSAGE_SIZE) {
                    MailOutcome::Reject(reply) => {
                        self.write(reply).await?;
                    }
                    MailOutcome::Accept { path, reply } => {
                        self.data.reset_transaction();
                        self.chunks = None;
                        if self.mode == SmtpMode::Inbound {
                            tracing::info!(
                                target: "irixmail::smtp::inbound",
                                sid = self.sid,
                                sender = %path.address,
                                "sender accepted"
                            );
                        }
                        self.data.mail_from = Some(path.address);
                        self.data.smtputf8 = path.smtputf8;
                        self.stage = Stage::Mail;
                        self.write(reply).await?;
                    }
                }
                Ok(None)
            }
            Verb::Rcpt => {
                if let Some(reply) = self.submission_gate() {
                    self.write(reply).await?;
                    return Ok(None);
                }
                let mut params = match parse_command(&terminated(line)) {
                    Ok(Command::Rcpt { to }) => to,
                    _ => {
                        self.write(b"501 5.5.4 Syntax error in RCPT command\r\n")
                            .await?;
                        return Ok(None);
                    }
                };
                // the wire parser collapses <postmaster> to an empty path
                if params.address.is_empty() {
                    params.address = "postmaster".to_string();
                }
                let sender_pending = self.data.mail_from.is_some();
                let recipient = self.classify_recipient(&params.address);
                match rcpt_reply(
                    &params,
                    sender_pending,
                    recipient,
                    self.data.authenticated,
                    self.data.rcpt_to.len(),
                    DEFAULT_MAX_RECIPIENTS,
                ) {
                    RcptOutcome::Reject(reply) => {
                        if self.mode == SmtpMode::Inbound {
                            tracing::info!(
                                target: "irixmail::smtp::inbound",
                                sid = self.sid,
                                recipient = %params.address,
                                code = reply_code(reply),
                                "recipient refused"
                            );
                        }
                        self.write(reply).await?;
                    }
                    RcptOutcome::Accept { path, reply } => {
                        match self.greylist_defers(&path.address) {
                            Some(defer) => {
                                tracing::info!(
                                    target: "irixmail::smtp::inbound",
                                    sid = self.sid,
                                    recipient = %path.address,
                                    code = reply_code(defer),
                                    "recipient greylisted"
                                );
                                self.write(defer).await?;
                            }
                            None => {
                                if self.mode == SmtpMode::Inbound {
                                    tracing::info!(
                                        target: "irixmail::smtp::inbound",
                                        sid = self.sid,
                                        recipient = %path.address,
                                        "recipient accepted"
                                    );
                                }
                                self.data.rcpt_to.push(path.address);
                                self.stage = Stage::Rcpt;
                                self.write(reply).await?;
                            }
                        }
                    }
                }
                Ok(None)
            }
            Verb::Data => {
                if let Some(reply) = self.submission_gate() {
                    self.write(reply).await?;
                    return Ok(None);
                }
                let prompt = match data_reply(self.stage == Stage::Rcpt) {
                    DataOutcome::Reject(reply) => {
                        self.write(reply).await?;
                        return Ok(None);
                    }
                    DataOutcome::Ready(reply) => reply,
                };
                self.write(prompt).await?;
                match self.read_data().await? {
                    Some(body) => {
                        let mail_from = self.data.mail_from.take();
                        let rcpt_to = std::mem::take(&mut self.data.rcpt_to);
                        let reply = self.accept_body(mail_from, rcpt_to, body).await?;
                        self.data.reset_transaction();
                        self.stage = Stage::Greeted;
                        self.write(&reply).await?;
                    }
                    None => {
                        self.data.reset_transaction();
                        self.stage = Stage::Greeted;
                        self.write(too_large_reply()).await?;
                    }
                }
                Ok(None)
            }
            Verb::Bdat => {
                if let Some(reply) = self.submission_gate() {
                    self.write(reply).await?;
                    return Ok(None);
                }
                let (chunk_size, is_last) = match parse_command(&terminated(line)) {
                    Ok(Command::Bdat {
                        chunk_size,
                        is_last,
                    }) => (chunk_size, is_last),
                    _ => {
                        self.write(b"501 5.5.4 Syntax error in BDAT command\r\n")
                            .await?;
                        return Ok(None);
                    }
                };
                let accumulated = self.chunks.as_ref().map_or(0, ChunkReceiver::len);
                match bdat_reply(
                    self.stage == Stage::Rcpt,
                    chunk_size,
                    accumulated,
                    MAX_MESSAGE_SIZE,
                    is_last,
                ) {
                    BdatOutcome::Reject(reply) => {
                        self.chunks = None;
                        match chunk_disposal(chunk_size, MAX_MESSAGE_SIZE) {
                            ChunkDisposal::Drain(count) => {
                                self.drain_chunk(count).await?;
                                self.write(reply).await?;
                            }
                            ChunkDisposal::Close => {
                                self.write(reply).await?;
                                return Ok(Some(Flow::Close));
                            }
                        }
                    }
                    BdatOutcome::TooLarge(reply) => {
                        self.chunks = None;
                        self.data.reset_transaction();
                        self.stage = Stage::Greeted;
                        match chunk_disposal(chunk_size, MAX_MESSAGE_SIZE) {
                            ChunkDisposal::Drain(count) => {
                                self.drain_chunk(count).await?;
                                self.write(reply).await?;
                            }
                            ChunkDisposal::Close => {
                                self.write(reply).await?;
                                return Ok(Some(Flow::Close));
                            }
                        }
                    }
                    BdatOutcome::Receive {
                        chunk_size,
                        is_last,
                    } => {
                        let chunk = self.read_chunk(chunk_size).await?;
                        let receiver = self
                            .chunks
                            .get_or_insert_with(|| ChunkReceiver::new(MAX_MESSAGE_SIZE));
                        match receiver.push_chunk(&chunk, is_last) {
                            ChunkStep::Accepted => {
                                self.write(chunk_ok_reply()).await?;
                            }
                            ChunkStep::Complete => {
                                let body = self.chunks.take().unwrap().into_message();
                                let mail_from = self.data.mail_from.take();
                                let rcpt_to = std::mem::take(&mut self.data.rcpt_to);
                                let reply = self.accept_body(mail_from, rcpt_to, body).await?;
                                self.data.reset_transaction();
                                self.stage = Stage::Greeted;
                                self.write(&reply).await?;
                            }
                            ChunkStep::TooLarge => {
                                self.chunks = None;
                                self.data.reset_transaction();
                                self.stage = Stage::Greeted;
                                self.write(crate::cmd_bdat::too_large_reply()).await?;
                            }
                        }
                    }
                }
                Ok(None)
            }
            Verb::StartTls => {
                let reply = starttls_reply(self.data.is_tls, true);
                self.write(reply.bytes()).await?;
                if reply.upgrades() {
                    Ok(Some(Flow::Upgrade))
                } else {
                    Ok(None)
                }
            }
            Verb::Auth => {
                let (mechanism, initial_response) = match parse_command(&terminated(line)) {
                    Ok(Command::Auth {
                        mechanism,
                        initial_response,
                    }) => (mechanism, initial_response),
                    _ => {
                        self.write(b"501 5.5.4 Syntax error in AUTH command\r\n")
                            .await?;
                        return Ok(None);
                    }
                };
                let start = SaslExchange::begin(
                    mechanism,
                    self.data.is_tls,
                    self.data.authenticated,
                    &initial_response,
                );
                self.run_auth(start).await?;
                Ok(None)
            }
            Verb::Rset => {
                self.data.reset_transaction();
                self.chunks = None;
                if self.stage != Stage::Connected {
                    self.stage = Stage::Greeted;
                }
                self.write(rset_reply()).await?;
                Ok(None)
            }
            Verb::Noop => {
                self.write(noop_reply()).await?;
                Ok(None)
            }
            Verb::Quit => {
                self.write(quit_reply()).await?;
                Ok(Some(Flow::Close))
            }
            Verb::Unknown => {
                self.write(b"500 5.5.2 Command unrecognized\r\n").await?;
                Ok(None)
            }
        }
    }

    async fn run_auth(&mut self, start: SaslStart) -> Result<()> {
        let (mut exchange, mut step) = match start {
            SaslStart::Reply { bytes, .. } => {
                self.write(bytes).await?;
                return Ok(());
            }
            SaslStart::Continue { exchange, step } => (exchange, step),
        };
        loop {
            match step {
                SaslStep::Challenge(bytes) => {
                    self.write(bytes).await?;
                    let Some(response) = self.read_auth_response().await? else {
                        self.write(b"501 5.5.2 Authentication cancelled\r\n")
                            .await?;
                        return Ok(());
                    };
                    step = exchange.advance(&response);
                }
                SaslStep::Reply { bytes, .. } => {
                    self.write(bytes).await?;
                    return Ok(());
                }
                SaslStep::Resolved(credentials) => {
                    match self.verify_credentials(&credentials).await? {
                        LoginAttempt::Granted(account, _) => {
                            self.data.authenticated = true;
                            self.account = Some(*account);
                            self.write(success_reply()).await?;
                        }
                        LoginAttempt::Denied => {
                            self.write(credentials_invalid_reply()).await?;
                        }
                        LoginAttempt::Throttled => {
                            self.write(too_many_attempts_reply()).await?;
                        }
                    }
                    return Ok(());
                }
            }
        }
    }

    async fn read_auth_response(&mut self) -> Result<Option<String>> {
        let mut line = Vec::with_capacity(MAX_LINE_LENGTH);
        if !self.read_line(&mut line).await? {
            return Err(Error::protocol("connection closed during AUTH"));
        }
        let response = String::from_utf8_lossy(strip_crlf(&line))
            .trim()
            .to_string();
        if response == "*" {
            return Ok(None);
        }
        Ok(Some(response))
    }

    async fn verify_credentials(&self, credentials: &Credentials) -> Result<LoginAttempt> {
        let Some(directory) = self.directory() else {
            return Ok(LoginAttempt::Denied);
        };
        let ip = self.peer.ip().to_canonical().to_string();
        attempt_login_blocking(
            directory,
            Some(&ip),
            &credentials.authcid,
            &credentials.password,
            LoginPurpose::Mail,
        )
        .await
    }

    fn directory(&self) -> Option<&Directory> {
        match &self.services {
            Some(SessionServices::Inbound(services)) => Some(services.directory()),
            Some(SessionServices::Submission(services)) => Some(services.directory()),
            None => None,
        }
    }

    fn classify_recipient(&self, address: &str) -> Recipient {
        match domain_of(address) {
            Some(domain) if self.local_domains.contains(&domain) => {}
            None if address.trim().eq_ignore_ascii_case("postmaster") => {}
            _ => return Recipient::Remote,
        }
        let Some(directory) = self.directory() else {
            return Recipient::Local;
        };
        match irixmail_mail::resolve(
            directory.addresses(),
            directory.domains(),
            directory.accounts(),
            address,
        ) {
            Ok(Resolution::Local { .. } | Resolution::Forward { .. }) => Recipient::Local,
            Ok(Resolution::Rejected | Resolution::Unknown) | Err(_) => Recipient::LocalUnknown,
        }
    }

    fn submission_gate(&self) -> Option<&'static [u8]> {
        if self.mode != SmtpMode::Submission {
            return None;
        }
        match guard_submission(self.data.authenticated) {
            SubmissionGate::Proceed => None,
            SubmissionGate::Reject(reply) => Some(reply),
        }
    }

    fn check_sender(&self, declared: &str) -> OwnershipGate {
        if self.mode != SmtpMode::Submission {
            return OwnershipGate::Proceed;
        }
        let owned = self.owned_addresses();
        guard_from(declared, owned.iter().map(String::as_str))
    }

    fn owned_addresses(&self) -> Vec<String> {
        let Some(account) = &self.account else {
            return Vec::new();
        };
        let mut owned = Vec::with_capacity(account.aliases.len() + 1);
        if let Some(directory) = self.directory() {
            if let Ok(domain) = directory.domains().get(account.domain_id) {
                owned.push(format!("{}@{}", account.local_part, domain.name));
            }
        }
        owned.extend(account.aliases.iter().cloned());
        owned
    }

    async fn check_blocklist(&mut self) {
        let Some(services) = self.inbound_services() else {
            return;
        };
        if services.dnsbl().is_empty() {
            return;
        }
        self.dnsbl = dnsbl::check(services.dnsbl(), services.dns(), self.peer.ip()).await;
        match &self.dnsbl {
            DnsblDecision::Allow => tracing::info!(
                target: "irixmail::smtp::inbound",
                sid = self.sid,
                verdict = "clean",
                "dnsbl verdict"
            ),
            DnsblDecision::Reject { zone, .. } => tracing::info!(
                target: "irixmail::smtp::inbound",
                sid = self.sid,
                verdict = "listed",
                zone = %zone,
                "dnsbl verdict"
            ),
        }
    }

    fn rate_limit_connection(&self) -> Option<&'static [u8]> {
        if self.starttls_upgrade {
            return None;
        }
        match self
            .inbound_services()?
            .rate_limiter()
            .on_connect(self.peer.ip())
        {
            RateDecision::Deny(reply) => Some(reply),
            _ => None,
        }
    }

    fn rate_limit_message(&self) -> Option<&'static [u8]> {
        match self
            .inbound_services()?
            .rate_limiter()
            .on_message(self.peer.ip())
        {
            RateDecision::Deny(reply) => Some(reply),
            _ => None,
        }
    }

    fn greylist_defers(&self, recipient: &str) -> Option<&'static [u8]> {
        let services = self.inbound_services()?;
        let from = self.data.mail_from.as_deref().unwrap_or("");
        match services
            .greylist()
            .check_or_allow(from, recipient, self.data.authenticated)
        {
            GreylistDecision::Defer(reply) => Some(reply),
            GreylistDecision::Allow => None,
        }
    }

    async fn accept_body(
        &mut self,
        mail_from: Option<String>,
        rcpt_to: Vec<String>,
        body: Vec<u8>,
    ) -> Result<Vec<u8>> {
        match self.mode {
            SmtpMode::Submission => self.accept_submission(mail_from, rcpt_to, body).await,
            SmtpMode::Inbound => self.accept_inbound(mail_from, rcpt_to, body).await,
        }
    }

    async fn accept_submission(
        &mut self,
        mail_from: Option<String>,
        rcpt_to: Vec<String>,
        body: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let Some(services) = self.submission_services().cloned() else {
            self.accepted = Some(AcceptedMessage {
                mail_from,
                rcpt_to,
                body,
            });
            return Ok(accepted_reply().to_vec());
        };

        let return_path = mail_from.clone().unwrap_or_default();
        let signing_domain =
            crate::sub_headers::from_domain(&body).or_else(|| domain_of(&return_path));
        let host = signing_domain.as_deref().unwrap_or(HOSTNAME);
        let now = crate::deliver_out::now_seconds();
        let completed = crate::sub_headers::complete_headers(&body, host, now);
        let signed = match signing_domain.as_ref().and_then(|domain| {
            services
                .signer(domain)
                .map(|signer| signer.sign_message(&completed))
        }) {
            Some(Ok(signed)) => signed,
            Some(Err(err)) => {
                tracing::warn!(error = %err, "submission signing failed");
                return Ok(submission_tempfail_reply().to_vec());
            }
            None => completed,
        };

        let submission = Submission {
            return_path: &return_path,
            recipients: &rcpt_to,
        };
        if let Err(err) = enqueue_submission(
            services.store().as_ref(),
            services.blobs().as_ref(),
            &signed,
            &submission,
        ) {
            tracing::warn!(error = %err, "submission enqueue failed");
            return Ok(submission_tempfail_reply().to_vec());
        }

        self.accepted = Some(AcceptedMessage {
            mail_from,
            rcpt_to,
            body: signed,
        });
        Ok(accepted_reply().to_vec())
    }

    async fn accept_inbound(
        &mut self,
        mail_from: Option<String>,
        rcpt_to: Vec<String>,
        body: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let Some(services) = self.inbound_services().cloned() else {
            self.accepted = Some(AcceptedMessage {
                mail_from,
                rcpt_to,
                body,
            });
            return Ok(accepted_reply().to_vec());
        };

        if let Some(reply) = self.rate_limit_message() {
            return Ok(reply.to_vec());
        }

        let from = mail_from.clone().unwrap_or_default();
        let GauntletOutcome {
            verdict,
            auth_results,
        } = inbound::run_gauntlet(
            &services,
            self.sid,
            self.peer.ip(),
            &self.data.helo_domain,
            &from,
            &body,
            &self.dnsbl,
        )
        .await;

        self.dnsbl = DnsblDecision::Allow;

        let disposition = match verdict {
            SpamDecision::Accept(disposition) => {
                let target = match disposition {
                    Disposition::Spam => "junk",
                    Disposition::Inbox => "inbox",
                };
                tracing::info!(
                    target: "irixmail::smtp::inbound",
                    sid = self.sid,
                    verdict = "accept",
                    disposition = target,
                    "spam verdict"
                );
                disposition
            }
            SpamDecision::Defer(reply) => {
                tracing::info!(
                    target: "irixmail::smtp::inbound",
                    sid = self.sid,
                    verdict = "defer",
                    code = reply_code(reply),
                    "spam verdict"
                );
                return Ok(reply.to_vec());
            }
            SpamDecision::Reject(reply) => {
                tracing::info!(
                    target: "irixmail::smtp::inbound",
                    sid = self.sid,
                    verdict = "reject",
                    code = reply_code(reply),
                    "spam verdict"
                );
                return Ok(reply.to_vec());
            }
        };

        let authed = inbound::prepend_header("Authentication-Results", &auth_results, &body);
        let now = crate::deliver_out::now_seconds();
        let mut stamped = inbound::build_received(
            &self.data.helo_domain,
            self.peer.ip(),
            services.spf().host_domain(),
            self.data.is_tls,
            now,
            now,
        );
        stamped.extend_from_slice(&authed);
        if let Some(reply) =
            self.deliver_inbound(&services, &from, &rcpt_to, &stamped, disposition)?
        {
            return Ok(reply);
        }

        tracing::info!(
            target: "irixmail::smtp::inbound",
            sid = self.sid,
            recipients = rcpt_to.len(),
            "message ingested"
        );
        self.accepted = Some(AcceptedMessage {
            mail_from,
            rcpt_to,
            body: stamped,
        });
        Ok(accepted_reply().to_vec())
    }

    fn deliver_inbound(
        &self,
        services: &InboundServices,
        mail_from: &str,
        rcpt_to: &[String],
        raw: &[u8],
        disposition: Disposition,
    ) -> Result<Option<Vec<u8>>> {
        let directory = services.directory();
        let mail = services.mail();

        // A spam disposition files into Junk instead of the inbox.
        let target_override = match disposition {
            Disposition::Spam => Some(DeliveryTarget::Role(SpecialUse::Junk)),
            _ => None,
        };

        let received_at = crate::deliver_out::now_seconds();
        let mut recipients = Vec::new();
        let mut forwarded = 0usize;
        for recipient in rcpt_to {
            match mail.resolve(
                directory.addresses(),
                directory.domains(),
                directory.accounts(),
                recipient,
            )? {
                Resolution::Local { account_id, .. } => {
                    match directory.accounts().get(account_id) {
                        Ok(account) => {
                            let mut mailboxes = irixmail_mail::load_mailboxes(
                                mail.store().as_ref(),
                                account.id as u32,
                            )?;
                            if mailboxes.is_empty() {
                                mailboxes = provision_mailboxes(account.created_at);
                            }
                            let document_id = self.next_document_id(services, account.id)?;
                            recipients.push((account, recipient.clone(), mailboxes, document_id));
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "irixmail::smtp::inbound",
                                sid = self.sid,
                                recipient = %recipient,
                                error = %err,
                                "account lookup failed"
                            );
                        }
                    }
                }
                Resolution::Forward { destination } => {
                    match crate::deliver_hook::enqueue_forward(
                        mail,
                        mail_from,
                        &destination,
                        raw,
                        received_at,
                    ) {
                        Ok(()) => {
                            forwarded += 1;
                            tracing::info!(
                                target: "irixmail::smtp::inbound",
                                sid = self.sid,
                                recipient = %recipient,
                                destination = %destination,
                                "forwarding"
                            );
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "irixmail::smtp::inbound",
                                sid = self.sid,
                                recipient = %recipient,
                                destination = %destination,
                                error = %err,
                                "the forwarded copy could not be queued"
                            );
                        }
                    }
                }
                Resolution::Rejected | Resolution::Unknown => {
                    tracing::warn!(
                        target: "irixmail::smtp::inbound",
                        sid = self.sid,
                        recipient = %recipient,
                        "recipient no longer resolves at delivery"
                    );
                }
            }
        }

        if recipients.is_empty() {
            if forwarded > 0 {
                return Ok(None);
            }
            tracing::warn!(
                target: "irixmail::smtp::inbound",
                sid = self.sid,
                "no recipient could be delivered, refusing"
            );
            return Ok(Some(UNROUTABLE.to_vec()));
        }

        for (_, recipient, ..) in &recipients {
            tracing::info!(
                target: "irixmail::smtp::inbound",
                sid = self.sid,
                recipient = %recipient,
                mailbox = match target_override {
                    Some(_) => "junk",
                    None => "inbox",
                },
                "delivering"
            );
        }
        let requests: Vec<DeliveryRequest> = recipients
            .iter()
            .map(
                |(account, recipient, mailboxes, document_id)| DeliveryRequest {
                    account,
                    mailboxes,
                    mail_from,
                    recipient,
                    document_id: *document_id,
                    raw,
                    target_override,
                    received_at,
                },
            )
            .collect();

        let outcome = crate::deliver_hook::deliver_inbound(mail, &requests)?;
        let refused_everywhere = outcome
            .deliveries
            .iter()
            .all(|delivery| delivery.is_over_quota() && delivery.relays.is_empty());
        if refused_everywhere && forwarded == 0 {
            tracing::warn!(
                target: "irixmail::smtp::inbound",
                sid = self.sid,
                "every recipient is over quota, refusing"
            );
            return Ok(Some(mailbox_full_reply().to_vec()));
        }
        crate::deliver_hook::enqueue_relays(mail, &outcome);
        let addresses: Vec<&str> = recipients
            .iter()
            .map(|(_, recipient, ..)| recipient.as_str())
            .collect();
        crate::deliver_hook::bounce_over_quota(
            mail,
            &addresses,
            &outcome,
            mail_from,
            raw,
            received_at,
        );
        if !matches!(disposition, Disposition::Spam) {
            let pairs: Vec<(&Account, &str)> = recipients
                .iter()
                .map(|(account, recipient, ..)| (account, recipient.as_str()))
                .collect();
            crate::deliver_hook::respond_vacations(
                mail,
                &pairs,
                &outcome,
                mail_from,
                raw,
                received_at,
            );
        }
        Ok(None)
    }

    fn next_document_id(&self, services: &InboundServices, account_id: u64) -> Result<u32> {
        let key = Key::new(Subspace::Counter, account_id as u32, Collection::Email, 0).encode();
        let next = services.mail().store().add_and_get(&key, 1)?;
        Ok(next as u32)
    }

    async fn read_data(&mut self) -> Result<Option<Vec<u8>>> {
        let mut receiver = BodyReceiver::new(MAX_MESSAGE_SIZE);
        let mut line = Vec::with_capacity(MAX_LINE_LENGTH);
        let mut overflowed = false;
        loop {
            line.clear();
            if !self.read_data_line(&mut line).await? {
                return Err(Error::protocol("connection closed during DATA"));
            }
            let body = strip_crlf(&line);
            if overflowed {
                if body == b"." {
                    return Ok(None);
                }
                continue;
            }
            match receiver.push_line(body) {
                BodyStep::Continue => {}
                BodyStep::Complete => return Ok(Some(receiver.into_body())),
                BodyStep::TooLarge => overflowed = true,
            }
        }
    }

    // DATA content has no per-line cap (unlike command lines); only the total message size
    // bounds it, so an over-long line stops accumulating and degrades to the drain-then-552 path.
    async fn read_data_line(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        loop {
            let byte = match self.stream.read_u8().await {
                Ok(byte) => byte,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(!buf.is_empty());
                }
                Err(err) => return Err(Error::from(err)),
            };
            if byte == b'\n' {
                return Ok(true);
            }
            if buf.len() <= MAX_MESSAGE_SIZE {
                buf.push(byte);
            }
        }
    }

    async fn read_chunk(&mut self, count: usize) -> Result<Vec<u8>> {
        let mut chunk = vec![0u8; count];
        self.stream
            .read_exact(&mut chunk)
            .await
            .map_err(|_| Error::protocol("connection closed during BDAT chunk"))?;
        Ok(chunk)
    }

    async fn drain_chunk(&mut self, count: usize) -> Result<()> {
        let mut buffer = [0u8; 4096];
        let mut remaining = count;
        while remaining > 0 {
            let take = remaining.min(buffer.len());
            self.stream
                .read_exact(&mut buffer[..take])
                .await
                .map_err(|_| Error::protocol("connection closed during BDAT chunk"))?;
            remaining -= take;
        }
        Ok(())
    }

    async fn read_line(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        loop {
            let byte = match self.stream.read_u8().await {
                Ok(byte) => byte,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(!buf.is_empty());
                }
                Err(err) => return Err(Error::from(err)),
            };
            if byte == b'\n' {
                return Ok(true);
            }
            if buf.len() >= MAX_LINE_LENGTH {
                return Err(Error::protocol("command line exceeds the maximum length"));
            }
            buf.push(byte);
        }
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

fn helo_argument(line: &[u8]) -> String {
    let trimmed = strip_crlf(line);
    let rest = match trimmed.iter().position(|b| matches!(b, b' ' | b'\t')) {
        Some(split) => &trimmed[split + 1..],
        None => return String::new(),
    };
    let start = rest
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .unwrap_or(rest.len());
    let end = rest[start..]
        .iter()
        .position(|b| matches!(b, b' ' | b'\t'))
        .map(|offset| start + offset)
        .unwrap_or(rest.len());
    String::from_utf8_lossy(&rest[start..end]).into_owned()
}

fn terminated(line: &[u8]) -> Vec<u8> {
    let body = strip_crlf(line);
    let mut buf = Vec::with_capacity(body.len() + 2);
    buf.extend_from_slice(body);
    buf.extend_from_slice(b"\r\n");
    buf
}

fn domain_of(address: &str) -> Option<String> {
    let domain = address.rsplit_once('@')?.1;
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_ascii_lowercase())
    }
}

fn submission_tempfail_reply() -> &'static [u8] {
    b"451 4.3.0 Unable to accept the message right now, try again later\r\n"
}

fn strip_crlf(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    &line[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Arc;

    struct Pipe {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Pipe {
        fn new(input: &[u8]) -> Self {
            Self {
                input: Cursor::new(input.to_vec()),
                output: Vec::new(),
            }
        }
    }

    impl AsyncRead for Pipe {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.input).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for Pipe {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.output.extend_from_slice(buf);
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    fn peer() -> SocketAddr {
        "127.0.0.1:2525".parse().unwrap()
    }

    fn hosting(domain: &str) -> HashSet<String> {
        HashSet::from([domain.to_string()])
    }

    async fn drive(script: &[u8]) -> (Flow, Vec<u8>, Stage) {
        let mut session =
            Session::new(Pipe::new(script), peer()).with_local_domains(hosting("d.example"));
        let flow = session.run().await.unwrap();
        let stage = session.stage();
        let mut pipe_out = Vec::new();
        std::mem::swap(&mut pipe_out, &mut session.stream.get_mut().output);
        (flow, pipe_out, stage)
    }

    #[test]
    fn verbs_are_parsed_case_insensitively() {
        assert_eq!(Verb::from_line(b"ehlo host\r\n"), Verb::Ehlo);
        assert_eq!(Verb::from_line(b"MAIL FROM:<a@b>\r\n"), Verb::Mail);
        assert_eq!(Verb::from_line(b"QuIt\r\n"), Verb::Quit);
        assert_eq!(Verb::from_line(b"WHAT\r\n"), Verb::Unknown);
        assert_eq!(Verb::from_line(b"\r\n"), Verb::Unknown);
    }

    #[tokio::test]
    async fn greeting_is_sent_before_any_command() {
        let (flow, out, _) = drive(b"QUIT\r\n").await;
        assert_eq!(flow, Flow::Close);
        assert!(out.starts_with(GREETING));
    }

    #[tokio::test]
    async fn a_full_transaction_advances_the_state_machine() {
        let (flow, out, stage) = drive(
            b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nhello\r\n.\r\nQUIT\r\n",
        )
        .await;
        assert_eq!(flow, Flow::Close);
        assert_eq!(stage, Stage::Greeted);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("354 "));
        assert!(text.contains("221 2.0.0 Bye"));
    }

    #[tokio::test]
    async fn an_unknown_verb_gets_a_five_hundred() {
        let (_, out, _) = drive(b"FROBNICATE\r\nQUIT\r\n").await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("500 5.5.2 Command unrecognized"));
    }

    #[tokio::test]
    async fn mail_before_greeting_is_rejected() {
        let (_, out, _) = drive(b"MAIL FROM:<a@b.example>\r\nQUIT\r\n").await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("503 5.5.1 Send EHLO/HELO first"));
    }

    #[tokio::test]
    async fn rcpt_without_mail_is_rejected() {
        let (_, out, _) = drive(b"EHLO client\r\nRCPT TO:<c@d.example>\r\nQUIT\r\n").await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("503 5.5.1 Send MAIL before RCPT"));
    }

    #[tokio::test]
    async fn a_local_recipient_is_accepted_and_recorded() {
        let mut session = Session::new(
            Pipe::new(b"EHLO c\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nQUIT\r\n"),
            peer(),
        )
        .with_local_domains(hosting("d.example"));
        session.run().await.unwrap();
        assert_eq!(session.stage(), Stage::Rcpt);
        assert_eq!(session.data().rcpt_to, vec!["c@d.example".to_string()]);
        let out = std::mem::take(&mut session.stream.get_mut().output);
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("250 2.1.5 Recipient OK"));
    }

    #[tokio::test]
    async fn an_unauthenticated_remote_recipient_is_refused_as_relaying() {
        let (_, out, _) = drive(
            b"EHLO c\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<user@remote.example>\r\nQUIT\r\n",
        )
        .await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("550 5.7.1 Relaying not allowed"));
    }

    #[tokio::test]
    async fn data_before_rcpt_is_rejected() {
        let (_, out, _) =
            drive(b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nDATA\r\nQUIT\r\n").await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("503 5.5.1 Need RCPT before DATA"));
    }

    #[tokio::test]
    async fn starttls_requests_an_upgrade() {
        let (flow, out, _) = drive(b"EHLO client\r\nSTARTTLS\r\n").await;
        assert_eq!(flow, Flow::Upgrade);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("220 2.0.0 Ready to start TLS"));
    }

    #[tokio::test]
    async fn data_body_is_collected_and_dot_unstuffed() {
        let mut session = Session::new(
            Pipe::new(b"EHLO c\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\n..dotted\r\nbody\r\n.\r\nQUIT\r\n"),
            peer(),
        )
        .with_local_domains(hosting("d.example"));
        session.run().await.unwrap();
        let accepted = session.last_accepted().unwrap();
        assert_eq!(accepted.body, b".dotted\r\nbody\r\n");
        assert_eq!(accepted.mail_from.as_deref(), Some("a@b.example"));
        assert_eq!(accepted.rcpt_to, vec!["c@d.example".to_string()]);
    }

    #[tokio::test]
    async fn a_long_physical_data_line_is_accepted_not_dropped() {
        let long_line = "x".repeat(2000);
        let script = format!(
            "EHLO c\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nSubject: hi\r\n\r\n{long_line}\r\n.\r\nQUIT\r\n"
        );
        let mut session = Session::new(Pipe::new(script.as_bytes()), peer())
            .with_local_domains(hosting("d.example"));
        session.run().await.unwrap();
        let accepted = session.last_accepted().unwrap();
        assert!(
            accepted
                .body
                .windows(long_line.len())
                .any(|window| window == long_line.as_bytes()),
            "the long line survives intact"
        );
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("250 2.0.0 Message accepted"), "{out}");
        assert!(
            out.contains("221 2.0.0 Bye"),
            "connection survived to QUIT: {out}"
        );
    }

    #[tokio::test]
    async fn a_single_bdat_last_chunk_is_accepted_verbatim() {
        let mut session = Session::new(
            Pipe::new(
                b"EHLO c\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nBDAT 11 LAST\r\nhello world\r\nQUIT\r\n",
            ),
            peer(),
        )
        .with_local_domains(hosting("d.example"));
        session.run().await.unwrap();
        let accepted = session.last_accepted().unwrap();
        assert_eq!(accepted.body, b"hello world");
        assert_eq!(accepted.mail_from.as_deref(), Some("a@b.example"));
        assert_eq!(accepted.rcpt_to, vec!["c@d.example".to_string()]);
        assert_eq!(session.stage(), Stage::Greeted);
    }

    #[tokio::test]
    async fn multiple_bdat_chunks_are_concatenated() {
        let mut session = Session::new(
            Pipe::new(
                b"EHLO c\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nBDAT 5\r\nhelloBDAT 5 LAST\r\nworldQUIT\r\n",
            ),
            peer(),
        )
        .with_local_domains(hosting("d.example"));
        session.run().await.unwrap();
        let accepted = session.last_accepted().unwrap();
        assert_eq!(accepted.body, b"helloworld");
        let out = std::mem::take(&mut session.stream.get_mut().output);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250 2.6.0 Chunk accepted"));
        assert!(text.contains("250 2.0.0 Message accepted"));
    }

    #[tokio::test]
    async fn bdat_before_rcpt_is_rejected() {
        let mut session = Session::new(
            Pipe::new(b"EHLO c\r\nMAIL FROM:<a@b.example>\r\nBDAT 4 LAST\r\nbodyQUIT\r\n"),
            peer(),
        )
        .with_local_domains(hosting("d.example"));
        session.run().await.unwrap();
        assert!(session.last_accepted().is_none());
        let out = std::mem::take(&mut session.stream.get_mut().output);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("503 5.5.1 Need RCPT before BDAT"));
    }

    #[tokio::test]
    async fn a_rejected_bdat_chunk_is_drained_not_parsed_as_commands() {
        let (_, out, _) = drive(b"EHLO c\r\nBDAT 7 LAST\r\nXFROG\r\nNOOP\r\nQUIT\r\n").await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("503 5.5.1 Need RCPT before BDAT"));
        assert!(
            !text.contains("500 "),
            "chunk payload leaked into the command stream: {text}"
        );
        assert!(text.contains("221 "));
    }

    #[tokio::test]
    async fn an_oversized_bdat_declaration_replies_552_and_closes_the_connection() {
        let (flow, out, _) = drive(
            b"EHLO c\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nBDAT 30000000 LAST\r\nXFROG\r\nQUIT\r\n",
        )
        .await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("552 5.3.4 "));
        assert_eq!(flow, Flow::Close);
        assert!(
            !text.contains("500 ") && !text.contains("221 "),
            "commands were processed after the oversized declaration: {text}"
        );
    }

    #[tokio::test]
    async fn ehlo_advertises_capabilities_and_records_the_domain() {
        let mut session = Session::new(Pipe::new(b"EHLO client.example\r\nQUIT\r\n"), peer());
        session.run().await.unwrap();
        assert_eq!(session.data().helo_domain, "client.example");
        let out = std::mem::take(&mut session.stream.get_mut().output);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250-irixmail"));
        assert!(text.contains("STARTTLS\r\n"));
        assert!(text.contains("SIZE "));
        assert!(text.contains("PIPELINING\r\n"));
    }

    #[tokio::test]
    async fn helo_returns_a_single_greeting_line() {
        let mut session = Session::new(Pipe::new(b"HELO client.example\r\nQUIT\r\n"), peer());
        session.run().await.unwrap();
        assert_eq!(session.data().helo_domain, "client.example");
        let out = std::mem::take(&mut session.stream.get_mut().output);
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("250-"));
        assert!(text.contains("250 irixmail at your service\r\n"));
    }

    #[tokio::test]
    async fn rset_clears_an_in_progress_transaction() {
        let mut session = Session::new(
            Pipe::new(b"EHLO c\r\nMAIL FROM:<a@b.example>\r\nRSET\r\nQUIT\r\n"),
            peer(),
        );
        session.run().await.unwrap();
        assert_eq!(session.stage(), Stage::Greeted);
        assert!(session.data().mail_from.is_none());
        assert!(session.data().rcpt_to.is_empty());
    }

    #[tokio::test]
    async fn an_oversized_mail_is_refused() {
        let script = format!(
            "EHLO c\r\nMAIL FROM:<a@b.example> SIZE={}\r\nQUIT\r\n",
            MAX_MESSAGE_SIZE + 1
        );
        let (_, out, stage) = drive(script.as_bytes()).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("552 5.3.4 "));
        assert_eq!(stage, Stage::Greeted);
    }

    #[tokio::test]
    async fn smtputf8_is_recorded_on_the_session() {
        let mut session = Session::new(
            Pipe::new(b"EHLO c\r\nMAIL FROM:<a@b.example> SMTPUTF8\r\nQUIT\r\n"),
            peer(),
        );
        session.run().await.unwrap();
        assert_eq!(session.data().mail_from.as_deref(), Some("a@b.example"));
        assert!(session.data().smtputf8);
    }

    #[test]
    fn a_session_defaults_to_inbound_with_no_services() {
        let session = Session::new(Pipe::new(b""), peer());
        assert_eq!(session.mode(), SmtpMode::Inbound);
        assert!(session.services().is_none());
        assert!(session.inbound_services().is_none());
        assert!(session.submission_services().is_none());
    }

    #[test]
    fn with_mode_switches_the_session_face() {
        let session = Session::new(Pipe::new(b""), peer()).with_mode(SmtpMode::Submission);
        assert_eq!(session.mode(), SmtpMode::Submission);
        assert!(session.services().is_none());
    }

    #[tokio::test]
    async fn a_submission_mode_session_still_drives_the_state_machine() {
        let mut session =
            Session::new(Pipe::new(b"EHLO c\r\nQUIT\r\n"), peer()).with_mode(SmtpMode::Submission);
        let flow = session.run().await.unwrap();
        assert_eq!(flow, Flow::Close);
        assert_eq!(session.mode(), SmtpMode::Submission);
    }

    fn b64(bytes: &[u8]) -> String {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine as _;
        STANDARD.encode(bytes)
    }

    async fn drive_tls_with_directory(script: &[u8]) -> (Vec<u8>, bool) {
        use std::collections::BTreeMap;
        use std::ops::Range;
        use std::sync::{Arc, Mutex};

        use irixmail_core::{Error, IdGenerator, Result};
        use irixmail_directory::Directory;
        use irixmail_store::{BlobHash, BlobStore, Flow as StoreFlow, KeyPrefix, Store, WriteOp};

        #[derive(Default)]
        struct MemStore {
            map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
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
                visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<StoreFlow>,
            ) -> Result<()> {
                let bound = prefix.encode();
                let map = self.map.lock().unwrap();
                for (key, value) in map.iter() {
                    if key.starts_with(&bound) && visit(key, value)? == StoreFlow::Stop {
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
                        WriteOp::Add { .. } => return Err(Error::internal("unexpected add")),
                    }
                }
                Ok(())
            }
            fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
                Err(Error::internal("unexpected add"))
            }
            fn counter(&self, _key: &[u8]) -> Result<i64> {
                Ok(0)
            }
        }

        struct MemBlobStore;

        impl BlobStore for MemBlobStore {
            fn get(&self, _hash: &BlobHash, _range: Range<usize>) -> Result<Option<Vec<u8>>> {
                Ok(None)
            }
            fn put(&self, bytes: &[u8]) -> Result<BlobHash> {
                Ok(BlobHash::from_bytes(
                    (bytes.len() as u32).to_be_bytes().to_vec(),
                ))
            }
            fn delete(&self, _hash: &BlobHash) -> Result<()> {
                Ok(())
            }
        }

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore);
        let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
        {
            use irixmail_directory::{password as pw, Role};
            let domain = directory.domains().create("d.example", Vec::new()).unwrap();
            let account = directory
                .accounts()
                .create("alice", domain.id, "Alice", Role::User)
                .unwrap();
            directory
                .credentials()
                .set_primary_password(account.id, pw::hash("secret").unwrap())
                .unwrap();
        }
        let services = SubmissionServices::new(directory, store, blobs);
        let mut session =
            Session::new(Pipe::new(script), peer()).with_submission_services(services);
        session.data.is_tls = true;
        session.run().await.unwrap();
        let authenticated = session.data().authenticated;
        let out = std::mem::take(&mut session.stream.get_mut().output);
        (out, authenticated)
    }

    #[tokio::test]
    async fn auth_on_a_plaintext_channel_is_refused() {
        let payload = b64(b"\0alice@d.example\0secret");
        let script = format!("EHLO c\r\nAUTH PLAIN {payload}\r\nQUIT\r\n");
        let (_, out, _) = drive(script.as_bytes()).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("530 5.7.0 Must issue a STARTTLS command first"));
    }

    #[tokio::test]
    async fn auth_with_an_unknown_account_is_rejected_and_leaves_the_session_unauthenticated() {
        let payload = b64(b"\0ghost@absent.example\0secret");
        let script = format!("EHLO c\r\nAUTH PLAIN {payload}\r\nQUIT\r\n");
        let (out, authenticated) = drive_tls_with_directory(script.as_bytes()).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("535 5.7.8 Authentication credentials invalid"));
        assert!(!authenticated);
    }

    #[tokio::test]
    async fn repeated_failed_auth_attempts_lock_the_source() {
        let wrong = b64(b"\0alice@d.example\0wrong");
        let right = b64(b"\0alice@d.example\0secret");
        let mut script = String::from("EHLO c\r\n");
        for _ in 0..5 {
            script.push_str(&format!("AUTH PLAIN {wrong}\r\n"));
        }
        script.push_str(&format!("AUTH PLAIN {right}\r\nQUIT\r\n"));
        let (out, authenticated) = drive_tls_with_directory(script.as_bytes()).await;
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("454 4.7.0"),
            "the locked attempt should be deferred, got: {text}"
        );
        assert!(!authenticated);
    }

    #[tokio::test]
    async fn login_walks_its_challenges_before_the_verification_decides() {
        let script = format!(
            "EHLO c\r\nAUTH LOGIN\r\n{}\r\n{}\r\nQUIT\r\n",
            b64(b"ghost@absent.example"),
            b64(b"secret"),
        );
        let (out, _) = drive_tls_with_directory(script.as_bytes()).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("334 VXNlcm5hbWU6"));
        assert!(text.contains("334 UGFzc3dvcmQ6"));
        assert!(text.contains("535 5.7.8"));
    }

    #[tokio::test]
    async fn a_cancelled_auth_exchange_is_answered_and_does_not_authenticate() {
        let script = "EHLO c\r\nAUTH LOGIN\r\n*\r\nQUIT\r\n";
        let (out, authenticated) = drive_tls_with_directory(script.as_bytes()).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("334 VXNlcm5hbWU6"));
        assert!(text.contains("501 5.5.2 Authentication cancelled"));
        assert!(!authenticated);
    }

    fn session_over_directory(
        configure: impl FnOnce(&irixmail_directory::AddressIndex),
    ) -> Session<Pipe> {
        use std::collections::BTreeMap;
        use std::ops::Range;
        use std::sync::{Arc, Mutex};

        use irixmail_core::{Error, IdGenerator, Result};
        use irixmail_directory::Directory;
        use irixmail_store::{BlobHash, BlobStore, Flow as StoreFlow, KeyPrefix, Store, WriteOp};

        #[derive(Default)]
        struct MemStore {
            map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
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
                visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<StoreFlow>,
            ) -> Result<()> {
                let bound = prefix.encode();
                let map = self.map.lock().unwrap();
                for (key, value) in map.iter() {
                    if key.starts_with(&bound) && visit(key, value)? == StoreFlow::Stop {
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
                        WriteOp::Add { .. } => return Err(Error::internal("unexpected add")),
                    }
                }
                Ok(())
            }
            fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
                Err(Error::internal("unexpected add"))
            }
            fn counter(&self, _key: &[u8]) -> Result<i64> {
                Ok(0)
            }
        }

        struct MemBlobStore;

        impl BlobStore for MemBlobStore {
            fn get(&self, _hash: &BlobHash, _range: Range<usize>) -> Result<Option<Vec<u8>>> {
                Ok(None)
            }
            fn put(&self, bytes: &[u8]) -> Result<BlobHash> {
                Ok(BlobHash::from_bytes(
                    (bytes.len() as u32).to_be_bytes().to_vec(),
                ))
            }
            fn delete(&self, _hash: &BlobHash) -> Result<()> {
                Ok(())
            }
        }

        let store: Arc<dyn Store> = Arc::new(MemStore::default());
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore);
        let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
        configure(directory.addresses());
        let services = SubmissionServices::new(directory, store, blobs);
        Session::new(Pipe::new(b""), peer())
            .with_local_domains(hosting("d.example"))
            .with_submission_services(services)
    }

    #[test]
    fn a_hosted_address_in_the_index_is_a_known_local_recipient() {
        use irixmail_directory::AddressEntry;
        let session = session_over_directory(|index| {
            index.set(AddressEntry::account("c@d.example", 7)).unwrap();
        });
        assert_eq!(session.classify_recipient("c@d.example"), Recipient::Local);
    }

    #[test]
    fn a_hosted_address_absent_from_the_index_is_locally_unknown() {
        let session = session_over_directory(|_| {});
        assert_eq!(
            session.classify_recipient("ghost@d.example"),
            Recipient::LocalUnknown
        );
    }

    #[test]
    fn a_hosted_domain_catch_all_makes_any_local_recipient_known() {
        let session = session_over_directory(|index| {
            index.set_catch_all("d.example", 9).unwrap();
        });
        assert_eq!(
            session.classify_recipient("anyone@d.example"),
            Recipient::Local
        );
    }

    #[test]
    fn a_hosted_address_the_index_rejects_is_locally_unknown() {
        use irixmail_directory::AddressEntry;
        let session = session_over_directory(|index| {
            index
                .set(AddressEntry::reject("blocked@d.example"))
                .unwrap();
        });
        assert_eq!(
            session.classify_recipient("blocked@d.example"),
            Recipient::LocalUnknown
        );
    }

    #[test]
    fn a_remote_recipient_is_classified_without_consulting_the_index() {
        let session = session_over_directory(|_| {});
        assert_eq!(
            session.classify_recipient("user@remote.example"),
            Recipient::Remote
        );
    }

    #[test]
    fn a_hosted_recipient_is_local_when_no_directory_is_attached() {
        let session = Session::new(Pipe::new(b""), peer()).with_local_domains(hosting("d.example"));
        assert_eq!(session.classify_recipient("c@d.example"), Recipient::Local);
    }

    fn inbound_session(
        script: &[u8],
        greylist_window: std::time::Duration,
        rate_limits: crate::ratelimit_in::RateLimits,
        configure: impl FnOnce(&Directory),
    ) -> Session<Pipe> {
        use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};
        inbound_session_dns(script, greylist_window, rate_limits, configure, || {
            mail_auth::MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default())
                .unwrap()
        })
    }

    fn inbound_session_dns(
        script: &[u8],
        greylist_window: std::time::Duration,
        rate_limits: crate::ratelimit_in::RateLimits,
        configure: impl FnOnce(&Directory),
        authenticator: impl Fn() -> mail_auth::MessageAuthenticator,
    ) -> Session<Pipe> {
        inbound_session_shared(
            script,
            greylist_window,
            rate_limits,
            configure,
            authenticator,
            None,
            peer(),
        )
        .0
    }

    #[allow(clippy::too_many_arguments)]
    fn inbound_session_shared(
        script: &[u8],
        greylist_window: std::time::Duration,
        rate_limits: crate::ratelimit_in::RateLimits,
        configure: impl FnOnce(&Directory),
        authenticator: impl Fn() -> mail_auth::MessageAuthenticator,
        shared_store: Option<std::sync::Arc<dyn irixmail_store::Store>>,
        peer_addr: SocketAddr,
    ) -> (Session<Pipe>, std::sync::Arc<dyn irixmail_store::Store>) {
        use std::collections::BTreeMap;
        use std::ops::Range;
        use std::sync::{Arc, Mutex};

        use hickory_resolver::config::{ResolverConfig as DnsConfig, ResolverOpts as DnsOpts};

        use irixmail_core::{IdGenerator, Result};
        use irixmail_dns::Resolver as DnsResolver;
        use irixmail_mail::MailServices;
        use irixmail_store::{
            BlobHash, BlobStore, ChangeNotifier, Flow as StoreFlow, KeyPrefix, Store, TtlStore,
            WriteOp,
        };

        use crate::arc::ArcVerifier;
        use crate::dkim_verify::DkimVerifier;
        use crate::dmarc::DmarcVerifier;
        use crate::dnsbl::DnsblConfig;
        use crate::greylist::{Greylist, GreylistConfig};
        use crate::ratelimit_in::RateLimiter;
        use crate::spf::{SpfConfig, SpfVerifier};

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
                visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<StoreFlow>,
            ) -> Result<()> {
                let bound = prefix.encode();
                let map = self.map.lock().unwrap();
                for (key, value) in map.iter() {
                    if key.starts_with(&bound) && visit(key, value)? == StoreFlow::Stop {
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

        let store: Arc<dyn Store> = shared_store.unwrap_or_else(|| Arc::new(MemStore::default()));
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let notifier = Arc::new(ChangeNotifier::new());
        let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
        configure(&directory);
        let mut hosted = crate::session_services::local_domains(&directory);
        hosted.extend(hosting("d.example"));

        let ttl = Arc::new(TtlStore::new());
        let expiring = Arc::new(irixmail_store::ExpiringStore::new(Arc::clone(&store)));
        let services = InboundServices::new(
            directory,
            authenticator(),
            DnsResolver::from_config(DnsConfig::default(), DnsOpts::default()),
            Arc::new(SpfVerifier::new(
                authenticator(),
                SpfConfig::new("mx.d.example"),
            )),
            Arc::new(DkimVerifier::new(authenticator())),
            Arc::new(DmarcVerifier::new(authenticator())),
            Arc::new(ArcVerifier::new(authenticator())),
            DnsblConfig { zones: Vec::new() },
            Arc::new(Greylist::new(
                expiring,
                GreylistConfig {
                    window: greylist_window,
                },
            )),
            Arc::new(RateLimiter::new(ttl, rate_limits)),
            MailServices::new(Arc::clone(&store), blobs, notifier),
        );
        let session = Session::new(Pipe::new(script), peer_addr)
            .with_local_domains(hosted)
            .with_inbound_services(services);
        (session, store)
    }

    fn seed_local_account(directory: &Directory) {
        use irixmail_directory::{AddressEntry, Role};
        let domain = directory.domains().create("d.example", Vec::new()).unwrap();
        let account = directory
            .accounts()
            .create("c", domain.id, "", Role::User)
            .unwrap();
        directory
            .addresses()
            .set(AddressEntry::account("c@d.example", account.id))
            .unwrap();
    }

    const INBOUND_MESSAGE: &[u8] =
        b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: hi\r\n\r\nbody text\r\n.\r\nQUIT\r\n";

    async fn drive_inbound(mut session: Session<Pipe>) -> (Vec<u8>, Option<AcceptedMessage>) {
        session.run().await.unwrap();
        let accepted = session.accepted.take();
        let out = std::mem::take(&mut session.stream.get_mut().output);
        (out, accepted)
    }

    async fn counting_dns_sink() -> (
        u16,
        Arc<std::sync::Mutex<std::collections::HashMap<String, u32>>>,
    ) {
        use std::collections::HashMap;
        use std::sync::Mutex;
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        let counts = Arc::new(Mutex::new(HashMap::new()));
        let seen = Arc::clone(&counts);
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                let Ok((len, peer)) = socket.recv_from(&mut buf).await else {
                    return;
                };
                if len < 12 {
                    continue;
                }
                let query = &buf[..len];
                let mut idx = 12;
                let mut name = String::new();
                while idx < len {
                    let label_len = query[idx] as usize;
                    idx += 1;
                    if label_len == 0 {
                        break;
                    }
                    if !name.is_empty() {
                        name.push('.');
                    }
                    name.push_str(&String::from_utf8_lossy(&query[idx..idx + label_len]));
                    idx += label_len;
                }
                if idx + 4 > len {
                    continue;
                }
                let qtype = u16::from_be_bytes([query[idx], query[idx + 1]]);
                let name = name.to_ascii_lowercase();
                if qtype == 16 {
                    *seen.lock().unwrap().entry(name.clone()).or_insert(0) += 1;
                }
                let question = &query[12..idx + 4];
                let mut reply = Vec::with_capacity(96);
                reply.extend_from_slice(&query[0..2]);
                if qtype == 16 && name == "b.example" {
                    reply.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]);
                    reply.extend_from_slice(question);
                    let txt = b"v=spf1 +all";
                    reply.extend_from_slice(&[0xC0, 0x0C, 0, 16, 0, 1, 0, 0, 1, 0x2C]);
                    reply.extend_from_slice(&((txt.len() + 1) as u16).to_be_bytes());
                    reply.push(txt.len() as u8);
                    reply.extend_from_slice(txt);
                } else {
                    reply.extend_from_slice(&[0x81, 0x83, 0, 1, 0, 0, 0, 0, 0, 0]);
                    reply.extend_from_slice(question);
                }
                let _ = socket.send_to(&reply, peer).await;
            }
        });
        (port, counts)
    }

    const SIGNED_INBOUND: &[u8] =
        b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nDKIM-Signature: v=1; a=rsa-sha256; d=b.example; s=x; c=simple/simple; h=From:To:Subject; bh=LYejN+2S0SuUUIERUVJ1BFP3qkDbhSFB1GfCb/JmeHM=; b=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: hi\r\n\r\nbody text\r\n.\r\nQUIT\r\n";

    #[tokio::test]
    async fn spf_and_dkim_are_each_evaluated_once_per_inbound_message() {
        use mail_auth::hickory_resolver::config::{
            NameServerConfigGroup, ResolverConfig, ResolverOpts,
        };
        let (port, counts) = counting_dns_sink().await;
        let session = inbound_session_dns(
            SIGNED_INBOUND,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            seed_local_account,
            move || {
                let config = ResolverConfig::from_parts(
                    None,
                    Vec::new(),
                    NameServerConfigGroup::from_ips_clear(
                        &["127.0.0.1".parse().unwrap()],
                        port,
                        true,
                    ),
                );
                let mut opts = ResolverOpts::default();
                opts.attempts = 1;
                opts.timeout = std::time::Duration::from_secs(3);
                mail_auth::MessageAuthenticator::new(config, opts).unwrap()
            },
        );
        let (out, _) = drive_inbound(session).await;
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("250 2.0.0 Message accepted"));

        let seen = counts.lock().unwrap().clone();
        assert_eq!(
            seen.get("b.example").copied().unwrap_or(0),
            1,
            "SPF must be evaluated exactly once per message: {seen:?}"
        );
        assert_eq!(
            seen.get("x._domainkey.b.example").copied().unwrap_or(0),
            1,
            "DKIM must be evaluated exactly once per message: {seen:?}"
        );
    }

    #[tokio::test]
    async fn an_inbound_message_is_accepted_signed_and_delivered_when_no_signal_turns_it_away() {
        let session = inbound_session(
            INBOUND_MESSAGE,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            seed_local_account,
        );
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250 2.0.0 Message accepted"));
        let accepted = accepted.expect("the message was recorded");
        let stamped = String::from_utf8(accepted.body).unwrap();
        assert!(stamped.contains("Authentication-Results: mx.d.example"));
    }

    fn seed_quota_account(directory: &Directory, local: &str, quota_bytes: u64) -> u64 {
        use irixmail_directory::{AddressEntry, Role};
        let domain = match directory.domains().get_by_name("d.example").unwrap() {
            Some(domain) => domain,
            None => directory.domains().create("d.example", Vec::new()).unwrap(),
        };
        let mut account = directory
            .accounts()
            .create(local, domain.id, "", Role::User)
            .unwrap();
        directory
            .addresses()
            .set(AddressEntry::account(
                format!("{local}@d.example"),
                account.id,
            ))
            .unwrap();
        let id = account.id;
        if quota_bytes > 0 {
            account.quota_bytes = quota_bytes;
            directory.accounts().update(account).unwrap();
        }
        id
    }

    #[tokio::test]
    async fn an_over_quota_recipient_is_deferred_with_452_not_silently_dropped() {
        let mut account_id = 0u64;
        let session = inbound_session(
            INBOUND_MESSAGE,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                account_id = seed_quota_account(directory, "c", 1);
            },
        );
        let store = Arc::clone(session.inbound_services().unwrap().mail().store());
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("452 4.2.2"),
            "expected a mailbox-full deferral: {text}"
        );
        assert!(!text.contains("250 2.0.0 Message accepted"), "{text}");
        assert!(
            accepted.is_none(),
            "an undeliverable message must not be recorded as accepted"
        );
        let data = irixmail_mail::load_data(store.as_ref(), account_id as u32, 1).unwrap();
        assert!(data.is_none(), "an over-quota message must not be stored");
    }

    const FANOUT_MESSAGE: &[u8] =
        b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nRCPT TO:<full@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: hi\r\n\r\nbody text\r\n.\r\nQUIT\r\n";

    #[tokio::test]
    async fn a_full_mailbox_in_a_fanout_bounces_that_recipient_and_accepts_the_rest() {
        let mut ok_id = 0u64;
        let session = inbound_session(
            FANOUT_MESSAGE,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                ok_id = seed_quota_account(directory, "c", 0);
                seed_quota_account(directory, "full", 1);
            },
        );
        let services = session.inbound_services().unwrap().clone();
        let store = Arc::clone(services.mail().store());
        let (out, _) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250 2.0.0 Message accepted"), "{text}");

        let data = irixmail_mail::load_data(store.as_ref(), ok_id as u32, 1)
            .unwrap()
            .expect("the healthy recipient keeps the message");
        assert!(!data.mailboxes.is_empty());

        let queued = crate::queue_enqueue::load(store.as_ref(), 1)
            .unwrap()
            .expect("a bounce to the sender was queued");
        assert_eq!(queued.return_path, "");
        assert_eq!(queued.recipients.len(), 1);
        assert_eq!(queued.recipients[0].address, "a@b.example");
        let body = services
            .mail()
            .blobs()
            .get_all(&queued.blob_hash())
            .unwrap()
            .unwrap();
        let dsn = String::from_utf8_lossy(&body);
        assert!(dsn.contains("full@d.example"), "{dsn}");
        assert!(dsn.contains("Status: 5.2.2"), "{dsn}");
    }

    #[tokio::test]
    async fn an_over_quota_message_from_a_null_sender_is_dropped_without_a_bounce_loop() {
        const NULL_SENDER: &[u8] =
            b"EHLO client\r\nMAIL FROM:<>\r\nRCPT TO:<c@d.example>\r\nRCPT TO:<full@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: hi\r\n\r\nbody text\r\n.\r\nQUIT\r\n";
        let session = inbound_session(
            NULL_SENDER,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                seed_quota_account(directory, "c", 0);
                seed_quota_account(directory, "full", 1);
            },
        );
        let store = Arc::clone(session.inbound_services().unwrap().mail().store());
        let (out, _) = drive_inbound(session).await;
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("250 2.0.0 Message accepted"));
        assert!(
            crate::queue_enqueue::load(store.as_ref(), 1)
                .unwrap()
                .is_none(),
            "a null-sender failure must never generate a bounce"
        );
    }

    const RECEIPT_MESSAGE: &[u8] =
        b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: your receipt\r\n\r\nbody text\r\n.\r\nQUIT\r\n";

    fn seed_vacationing_account(
        directory: &Directory,
        vacation: irixmail_directory::VacationResponder,
    ) -> u64 {
        use irixmail_directory::{AddressEntry, Role};
        let domain = directory.domains().create("d.example", Vec::new()).unwrap();
        let mut account = directory
            .accounts()
            .create("c", domain.id, "", Role::User)
            .unwrap();
        directory
            .addresses()
            .set(AddressEntry::account("c@d.example", account.id))
            .unwrap();
        account.vacation = vacation;
        directory.accounts().update(account.clone()).unwrap();
        account.id
    }

    const TWO_INBOUND_MESSAGES: &[u8] =
        b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: first\r\n\r\nbody one\r\n.\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: second\r\n\r\nbody two\r\n.\r\nQUIT\r\n";

    #[tokio::test]
    async fn a_vacationing_account_auto_replies_once_per_sender() {
        use irixmail_store::BlobHash;
        let vacation = irixmail_directory::VacationResponder {
            enabled: true,
            subject: "Away".to_string(),
            body: "Back soon".to_string(),
            active_from: None,
            active_to: None,
        };
        let mut account_id = 0u64;
        let session = inbound_session(
            TWO_INBOUND_MESSAGES,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                account_id = seed_vacationing_account(directory, vacation);
            },
        );
        let mail = session.inbound_services().unwrap().mail();
        let store = Arc::clone(mail.store());
        let blobs = Arc::clone(mail.blobs());
        store
            .batch(&irixmail_mail::provision_ops(account_id as u32, 0))
            .unwrap();

        let (out, _) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text.matches("250 2.0.0 Message accepted").count(),
            2,
            "unexpected replies: {text}"
        );

        let reply = crate::queue_enqueue::load(store.as_ref(), 1)
            .unwrap()
            .expect("the auto-reply was queued");
        assert_eq!(reply.recipients.len(), 1);
        assert_eq!(reply.recipients[0].address, "a@b.example");
        assert_eq!(reply.return_path, "", "auto-replies use a null return path");
        let raw = blobs
            .get_all(&BlobHash::from_bytes(reply.blob_hash.clone()))
            .unwrap()
            .expect("the reply body is stored");
        let text = String::from_utf8(raw).unwrap();
        assert!(text.contains("Auto-Submitted: auto-replied"));
        assert!(text.contains("Away"));

        assert!(
            crate::queue_enqueue::load(store.as_ref(), 2)
                .unwrap()
                .is_none(),
            "the second message within the period must not produce another reply"
        );

        let cache =
            irixmail_mail::MessageStoreCache::build(store.as_ref(), account_id as u32).unwrap();
        assert_eq!(
            cache.entries().count(),
            2,
            "both inbound messages are still delivered"
        );
    }

    #[tokio::test]
    async fn a_vacation_reply_is_not_sent_outside_the_stored_window() {
        let now = crate::deliver_out::now_seconds();
        let vacation = irixmail_directory::VacationResponder {
            enabled: true,
            subject: "Away".to_string(),
            body: "Back soon".to_string(),
            active_from: None,
            active_to: Some(now - 1_000),
        };
        let mut account_id = 0u64;
        let session = inbound_session(
            RECEIPT_MESSAGE,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                account_id = seed_vacationing_account(directory, vacation);
            },
        );
        let store = Arc::clone(session.inbound_services().unwrap().mail().store());
        store
            .batch(&irixmail_mail::provision_ops(account_id as u32, 0))
            .unwrap();

        drive_inbound(session).await;

        assert!(
            crate::queue_enqueue::load(store.as_ref(), 1)
                .unwrap()
                .is_none(),
            "an expired window must suppress the auto-reply"
        );
        let stored = irixmail_mail::load_data(store.as_ref(), account_id as u32, 1).unwrap();
        assert!(stored.is_some(), "the message itself is still delivered");
    }

    #[tokio::test]
    async fn an_inbound_message_is_stamped_with_a_received_header_above_authentication_results() {
        let session = inbound_session(
            INBOUND_MESSAGE,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            seed_local_account,
        );
        let (_out, accepted) = drive_inbound(session).await;
        let body = String::from_utf8(accepted.expect("the message was recorded").body).unwrap();
        assert!(
            body.starts_with("Received: from client ["),
            "unexpected head: {}",
            &body[..body.len().min(80)]
        );
        assert!(body.contains("by mx.d.example (IRIXMAIL) with ESMTP"));
        let received = body.find("Received:").unwrap();
        let auth = body.find("Authentication-Results:").unwrap();
        assert!(
            received < auth,
            "Received must precede Authentication-Results"
        );
    }

    #[tokio::test]
    async fn a_first_greylist_sighting_defers_the_message_instead_of_accepting_it() {
        let session = inbound_session(
            INBOUND_MESSAGE,
            std::time::Duration::from_secs(3600),
            crate::ratelimit_in::RateLimits::default(),
            seed_local_account,
        );
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("452 4.2.2 Greylisted"));
        assert!(!text.contains("250 2.0.0 Message accepted"));
        assert!(accepted.is_none());
    }

    fn test_authenticator() -> mail_auth::MessageAuthenticator {
        use mail_auth::hickory_resolver::config::{ResolverConfig, ResolverOpts};
        mail_auth::MessageAuthenticator::new(ResolverConfig::default(), ResolverOpts::default())
            .unwrap()
    }

    fn capture_logs() -> (irixmail_core::LogBuffer, tracing::subscriber::DefaultGuard) {
        use tracing_subscriber::layer::SubscriberExt;
        let buffer = irixmail_core::LogBuffer::new();
        let guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(buffer.layer()));
        (buffer, guard)
    }

    fn inbound_log_text(logs: &irixmail_core::LogBuffer) -> String {
        logs.snapshot()
            .into_iter()
            .filter(|record| record.source == "irixmail::smtp::inbound")
            .map(|record| record.message)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn an_inbound_transaction_logs_each_decision_under_one_session_id() {
        let (logs, _guard) = capture_logs();
        let session = inbound_session(
            INBOUND_MESSAGE,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            seed_local_account,
        );
        drive_inbound(session).await;

        let text = inbound_log_text(&logs);
        for needle in [
            "connection accepted",
            "sender accepted",
            "recipient accepted",
            "spf verdict",
            "dkim verdict",
            "dmarc verdict",
            "spam verdict",
            "delivering",
            "message ingested",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }

        let first = text.lines().next().unwrap();
        let sid = first
            .split("sid=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("the first line carries a session id");
        let tagged = format!("sid={sid}");
        for line in text.lines() {
            assert!(line.contains(&tagged), "line missing {tagged}: {line}");
        }
    }

    #[tokio::test]
    async fn mail_to_an_alias_domain_is_accepted_and_delivered() {
        use irixmail_directory::{AddressEntry, Role};

        let script = b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@alt.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@alt.example\r\nSubject: hi\r\n\r\nbody text\r\n.\r\nQUIT\r\n";
        let session = inbound_session(
            script,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                let domain = directory
                    .domains()
                    .create("d.example", vec!["alt.example".to_string()])
                    .unwrap();
                let account = directory
                    .accounts()
                    .create("c", domain.id, "", Role::User)
                    .unwrap();
                directory
                    .addresses()
                    .set(AddressEntry::account("c@d.example", account.id))
                    .unwrap();
            },
        );
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250 2.1.5 Recipient OK"), "got: {text}");
        assert!(text.contains("250 2.0.0 Message accepted"), "got: {text}");
        assert!(accepted.is_some(), "the message must be filed");
    }

    #[tokio::test]
    async fn mail_to_a_disabled_domain_is_refused() {
        use irixmail_directory::{AddressEntry, Role};

        let script =
            b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<c@dark.example>\r\nQUIT\r\n";
        let session = inbound_session(
            script,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                let mut domain = directory
                    .domains()
                    .create("dark.example", Vec::new())
                    .unwrap();
                let account = directory
                    .accounts()
                    .create("c", domain.id, "", Role::User)
                    .unwrap();
                directory
                    .addresses()
                    .set(AddressEntry::account("c@dark.example", account.id))
                    .unwrap();
                domain.enabled = false;
                directory.domains().update(domain).unwrap();
            },
        );
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("250 2.1.5 Recipient OK"),
            "a disabled domain must refuse mail: {text}"
        );
        assert!(text.contains("550"), "got: {text}");
        assert!(accepted.is_none());
    }

    #[tokio::test]
    async fn a_forward_only_address_relays_instead_of_silently_accepting() {
        use irixmail_directory::AddressEntry;

        let (session, store) = inbound_session_shared(
            INBOUND_MESSAGE,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                directory.domains().create("d.example", Vec::new()).unwrap();
                directory
                    .addresses()
                    .set(AddressEntry::forward("c@d.example", "far@remote.example"))
                    .unwrap();
            },
            test_authenticator,
            None,
            peer(),
        );
        let (out, _accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250 2.0.0 Message accepted"), "got: {text}");

        let queued = crate::queue_enqueue::load(store.as_ref(), 1)
            .unwrap()
            .expect("the forwarded copy must be queued, not dropped");
        assert_eq!(queued.recipients.len(), 1);
        assert_eq!(queued.recipients[0].address, "far@remote.example");
        assert_eq!(queued.return_path, "a@b.example");
    }

    #[tokio::test]
    async fn a_dangling_address_entry_is_refused_and_logged() {
        use irixmail_directory::AddressEntry;

        let (logs, _guard) = capture_logs();
        let session = inbound_session(
            INBOUND_MESSAGE,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                directory.domains().create("d.example", Vec::new()).unwrap();
                directory
                    .addresses()
                    .set(AddressEntry::account("c@d.example", 9999))
                    .unwrap();
            },
        );
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("250 2.0.0 Message accepted"),
            "a message that reaches nobody must not be acknowledged: {text}"
        );
        assert!(text.contains("550"), "got: {text}");
        assert!(accepted.is_none());

        let logged = inbound_log_text(&logs);
        assert!(
            logged.contains("account lookup failed"),
            "the loss must be logged: {logged}"
        );
    }

    #[tokio::test]
    async fn refusals_and_deferrals_are_logged_with_their_codes() {
        let (logs, _guard) = capture_logs();
        let script = b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<ghost@d.example>\r\nRCPT TO:<c@d.example>\r\nQUIT\r\n";
        let session = inbound_session(
            script,
            std::time::Duration::from_secs(3600),
            crate::ratelimit_in::RateLimits::default(),
            seed_local_account,
        );
        drive_inbound(session).await;

        let text = inbound_log_text(&logs);
        assert!(text.contains("recipient refused"), "got:\n{text}");
        assert!(
            text.contains("550"),
            "the refusal carries its code:\n{text}"
        );
        assert!(text.contains("greylisted"), "got:\n{text}");
    }

    #[tokio::test]
    async fn a_postmaster_recipient_is_delivered_to_the_admin() {
        use irixmail_directory::Role;
        let script = b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<postmaster@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: postmaster@d.example\r\nSubject: report\r\n\r\nreport body\r\n.\r\nQUIT\r\n";
        let (session, _) = inbound_session_shared(
            script,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                let domain = directory.domains().create("d.example", Vec::new()).unwrap();
                directory
                    .accounts()
                    .create("boss", domain.id, "", Role::Admin)
                    .unwrap();
            },
            test_authenticator,
            None,
            peer(),
        );
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250 2.1.5 Recipient OK"), "got: {text}");
        assert!(accepted.is_some());
    }

    #[tokio::test]
    async fn a_bare_postmaster_recipient_is_accepted() {
        use irixmail_directory::Role;
        let script = b"EHLO client\r\nMAIL FROM:<a@b.example>\r\nRCPT TO:<postmaster>\r\nQUIT\r\n";
        let session = inbound_session(
            script,
            std::time::Duration::ZERO,
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                let domain = directory.domains().create("d.example", Vec::new()).unwrap();
                directory
                    .accounts()
                    .create("boss", domain.id, "", Role::Admin)
                    .unwrap();
            },
        );
        let (out, _) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250 2.1.5 Recipient OK"), "got: {text}");
    }

    #[tokio::test]
    async fn a_greylisted_sender_retrying_from_a_different_ip_is_admitted() {
        let window = std::time::Duration::from_secs(3600);
        let limits = crate::ratelimit_in::RateLimits::default();

        let (first, store) = inbound_session_shared(
            INBOUND_MESSAGE,
            window,
            limits,
            seed_local_account,
            test_authenticator,
            None,
            "198.51.100.7:40001".parse().unwrap(),
        );
        let (out, accepted) = drive_inbound(first).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("452 4.2.2 Greylisted"), "got: {text}");
        assert!(accepted.is_none());

        let (second, _) = inbound_session_shared(
            INBOUND_MESSAGE,
            window,
            limits,
            |_| {},
            test_authenticator,
            Some(store),
            "203.0.113.9:40002".parse().unwrap(),
        );
        let (out, accepted) = drive_inbound(second).await;
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("250 2.0.0 Message accepted"),
            "a retry from a completely different address must pass: {text}"
        );
        assert!(accepted.is_some());
    }

    #[tokio::test]
    async fn greylisting_defers_at_rcpt_before_any_body_transfers() {
        let session = inbound_session(
            INBOUND_MESSAGE,
            std::time::Duration::from_secs(3600),
            crate::ratelimit_in::RateLimits::default(),
            seed_local_account,
        );
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("452 4.2.2 Greylisted"), "got: {text}");
        assert!(!text.contains("250 2.1.5 Recipient OK"), "got: {text}");
        assert!(
            !text.contains("354"),
            "the body must never transfer: {text}"
        );
        assert!(accepted.is_none());
    }

    #[tokio::test]
    async fn a_second_recipient_gets_its_own_greylist_verdict() {
        use irixmail_directory::{AddressEntry, Role};

        let script = b"EHLO client\r\n\
MAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nRSET\r\n\
MAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nRCPT TO:<c2@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: hi\r\n\r\nbody\r\n.\r\nQUIT\r\n";
        let session = inbound_session(
            script,
            std::time::Duration::from_secs(3600),
            crate::ratelimit_in::RateLimits::default(),
            |directory| {
                seed_local_account(directory);
                let domain = directory
                    .domains()
                    .get_by_name("d.example")
                    .unwrap()
                    .unwrap();
                let account = directory
                    .accounts()
                    .create("c2", domain.id, "", Role::User)
                    .unwrap();
                directory
                    .addresses()
                    .set(AddressEntry::account("c2@d.example", account.id))
                    .unwrap();
            },
        );
        let (out, accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        let deferrals = text.matches("452 4.2.2 Greylisted").count();
        assert_eq!(
            deferrals, 2,
            "the fresh pair in each transaction defers independently: {text}"
        );
        assert!(text.contains("250 2.1.5 Recipient OK"), "got: {text}");
        assert!(
            text.contains("250 2.0.0 Message accepted"),
            "the known pair must still be delivered: {text}"
        );
        let accepted = accepted.expect("the message reaches the known recipient");
        assert_eq!(accepted.rcpt_to, vec!["c@d.example".to_string()]);
    }

    #[tokio::test]
    async fn a_second_message_past_the_per_ip_allowance_is_deferred() {
        let two = b"EHLO client\r\n\
MAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: one\r\n\r\nbody\r\n.\r\n\
MAIL FROM:<a@b.example>\r\nRCPT TO:<c@d.example>\r\nDATA\r\nFrom: a@b.example\r\nTo: c@d.example\r\nSubject: two\r\n\r\nbody\r\n.\r\nQUIT\r\n";
        let limits = crate::ratelimit_in::RateLimits {
            max_connections: 0,
            max_messages: 1,
            window: std::time::Duration::from_secs(3600),
        };
        let session = inbound_session(two, std::time::Duration::ZERO, limits, seed_local_account);
        let (out, _accepted) = drive_inbound(session).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("250 2.0.0 Message accepted"));
        assert!(text.contains("452 4.7.0 Too many messages, try again later"));
    }

    #[tokio::test]
    async fn submission_refuses_mail_until_the_session_is_authenticated() {
        let script = "EHLO c\r\nMAIL FROM:<alice@d.example>\r\nQUIT\r\n";
        let (out, authenticated) = drive_tls_with_directory(script.as_bytes()).await;
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("530 5.7.0 Authentication required"));
        assert!(!authenticated);
    }

    fn authenticated_submission() -> Session<Pipe> {
        authenticated_submission_opts(false)
    }

    fn authenticated_submission_opts(fail_queue_writes: bool) -> Session<Pipe> {
        use std::collections::BTreeMap;
        use std::ops::Range;
        use std::sync::{Arc, Mutex};

        use irixmail_core::{IdGenerator, Result};
        use irixmail_directory::{AddressEntry, Directory, Role};
        use irixmail_store::{BlobHash, BlobStore, Flow as StoreFlow, KeyPrefix, Store, WriteOp};

        #[derive(Default)]
        struct MemStore {
            map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
            fail_queue_puts: bool,
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
                if self.fail_queue_puts && key.first() == Some(&Subspace::Queue.as_byte()) {
                    return Err(irixmail_core::Error::store("injected queue write failure"));
                }
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
                visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<StoreFlow>,
            ) -> Result<()> {
                let bound = prefix.encode();
                let map = self.map.lock().unwrap();
                for (key, value) in map.iter() {
                    if key.starts_with(&bound) && visit(key, value)? == StoreFlow::Stop {
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

        #[derive(Default)]
        struct MemBlobStore {
            map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
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
                let mut raw = (bytes.len() as u32).to_be_bytes().to_vec();
                let sum = bytes
                    .iter()
                    .fold(0u32, |acc, b| acc.wrapping_add(*b as u32));
                raw.extend_from_slice(&sum.to_be_bytes());
                let hash = BlobHash::from_bytes(raw);
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

        let store: Arc<dyn Store> = Arc::new(MemStore {
            fail_queue_puts: fail_queue_writes,
            ..MemStore::default()
        });
        let blobs: Arc<dyn BlobStore> = Arc::new(MemBlobStore::default());
        let directory = Directory::new(Arc::clone(&store), Arc::new(IdGenerator::new(0)), None);
        let domain = directory.domains().create("d.example", Vec::new()).unwrap();
        let mut account = directory
            .accounts()
            .create("alice", domain.id, "", Role::User)
            .unwrap();
        account.aliases = vec!["sales@d.example".to_string()];
        directory.accounts().update(account.clone()).unwrap();
        directory
            .addresses()
            .set(AddressEntry::account("alice@d.example", account.id))
            .unwrap();

        let services = SubmissionServices::new(directory, Arc::clone(&store), Arc::clone(&blobs));
        let mut session = Session::new(Pipe::new(b""), peer())
            .with_local_domains(hosting("d.example"))
            .with_submission_services(services);
        session.data.is_tls = true;
        session.data.authenticated = true;
        session.account = Some(account);
        session
    }

    #[test]
    fn an_authenticated_account_owns_its_primary_address_and_aliases() {
        let session = authenticated_submission();
        let owned = session.owned_addresses();
        assert!(owned.contains(&"alice@d.example".to_string()));
        assert!(owned.contains(&"sales@d.example".to_string()));
    }

    #[test]
    fn a_sender_the_account_owns_passes_the_ownership_check() {
        let session = authenticated_submission();
        assert!(session.check_sender("alice@d.example").is_allowed());
        assert!(session.check_sender("sales@d.example").is_allowed());
        assert!(session.check_sender("").is_allowed());
    }

    #[test]
    fn a_sender_the_account_does_not_own_is_refused_at_mail() {
        let session = authenticated_submission();
        let OwnershipGate::Reject(reply) = session.check_sender("mallory@d.example") else {
            panic!("a foreign sender should be refused");
        };
        assert!(reply.starts_with(b"550"));
    }

    #[tokio::test]
    async fn a_submission_is_signed_with_a_key_from_the_live_directory() {
        let mut session = authenticated_submission();
        let recipients = vec!["bob@remote.example".to_string()];
        let body =
            b"From: alice@d.example\r\nTo: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n"
                .to_vec();
        let reply = session
            .accept_submission(Some("alice@d.example".to_string()), recipients, body)
            .await
            .unwrap();
        assert!(reply.starts_with(b"250 2.0.0 Message accepted"));

        let stamped = String::from_utf8(session.last_accepted().unwrap().body.clone()).unwrap();
        assert!(
            stamped.starts_with("DKIM-Signature: "),
            "the signer must resolve from the live directory, not a boot-time map"
        );
        assert!(stamped.contains("d=d.example"));
    }

    #[tokio::test]
    async fn an_owned_submission_is_signed_and_queued() {
        let mut session = authenticated_submission();
        let recipients = vec!["bob@remote.example".to_string()];
        let body =
            b"From: alice@d.example\r\nTo: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n"
                .to_vec();
        let reply = session
            .accept_submission(Some("alice@d.example".to_string()), recipients, body)
            .await
            .unwrap();
        assert!(reply.starts_with(b"250 2.0.0 Message accepted"));

        let accepted = session.last_accepted().expect("the message was recorded");
        let stamped = String::from_utf8(accepted.body.clone()).unwrap();
        assert!(stamped.starts_with("DKIM-Signature: "));
        assert!(stamped.contains("d=d.example"));

        let services = session.submission_services().unwrap();
        let mut found = 0;
        services
            .store()
            .iterate(
                &irixmail_store::KeyPrefix::subspace(Subspace::Queue),
                &mut |_key, _value| {
                    found += 1;
                    Ok(irixmail_store::Flow::Continue)
                },
            )
            .unwrap();
        assert_eq!(found, 1);
    }

    #[tokio::test]
    async fn the_dkim_signature_aligns_with_the_from_header_not_the_envelope() {
        let mut session = authenticated_submission();
        session
            .submission_services()
            .unwrap()
            .directory()
            .domains()
            .create("b.example", Vec::new())
            .unwrap();
        let recipients = vec!["bob@remote.example".to_string()];
        let body =
            b"From: alice@d.example\r\nTo: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n"
                .to_vec();
        let reply = session
            .accept_submission(Some("alice@b.example".to_string()), recipients, body)
            .await
            .unwrap();
        assert!(reply.starts_with(b"250 2.0.0 Message accepted"));

        let stamped = String::from_utf8(session.last_accepted().unwrap().body.clone()).unwrap();
        assert!(stamped.starts_with("DKIM-Signature: "));
        assert!(
            stamped.contains("d=d.example"),
            "d= must follow the From header domain so DMARC aligns: {stamped}"
        );
        assert!(!stamped.contains("d=b.example"), "{stamped}");
        assert!(
            stamped.contains("@d.example>"),
            "the Message-ID host must follow the From header domain: {stamped}"
        );
    }

    #[tokio::test]
    async fn a_submission_without_a_from_header_signs_with_the_envelope_domain() {
        let mut session = authenticated_submission();
        let recipients = vec!["bob@remote.example".to_string()];
        let body = b"To: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n".to_vec();
        let reply = session
            .accept_submission(Some("alice@d.example".to_string()), recipients, body)
            .await
            .unwrap();
        assert!(reply.starts_with(b"250 2.0.0 Message accepted"));

        let stamped = String::from_utf8(session.last_accepted().unwrap().body.clone()).unwrap();
        assert!(stamped.starts_with("DKIM-Signature: "));
        assert!(stamped.contains("d=d.example"), "{stamped}");
    }

    #[tokio::test]
    async fn a_store_failure_during_submission_replies_451_instead_of_dropping() {
        let mut session = authenticated_submission_opts(true);
        let recipients = vec!["bob@remote.example".to_string()];
        let body =
            b"From: alice@d.example\r\nTo: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n"
                .to_vec();
        let reply = session
            .accept_submission(Some("alice@d.example".to_string()), recipients, body)
            .await
            .expect("a store failure must produce a reply, not drop the connection");
        assert!(
            reply.starts_with(b"451"),
            "got: {}",
            String::from_utf8_lossy(&reply)
        );
    }

    #[tokio::test]
    async fn a_submission_missing_originator_headers_gets_them_completed_before_signing() {
        let mut session = authenticated_submission();
        let recipients = vec!["bob@remote.example".to_string()];
        let body =
            b"From: alice@d.example\r\nTo: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n"
                .to_vec();
        session
            .accept_submission(Some("alice@d.example".to_string()), recipients, body)
            .await
            .unwrap();

        let queued = String::from_utf8(session.last_accepted().unwrap().body.clone()).unwrap();
        assert!(queued.starts_with("DKIM-Signature: "));
        assert!(queued.contains("Date: "));
        assert!(queued.contains("Message-ID: <"));
        assert!(queued.contains("@d.example>"));
        assert!(queued.contains("MIME-Version: 1.0"));
    }

    #[tokio::test]
    async fn a_submission_that_already_carries_originator_headers_is_left_unchanged() {
        let mut session = authenticated_submission();
        let recipients = vec!["bob@remote.example".to_string()];
        let body = b"Date: Tue, 14 Nov 2023 22:13:20 +0000\r\nMessage-ID: <keep@d.example>\r\nMIME-Version: 1.0\r\nFrom: alice@d.example\r\nTo: bob@remote.example\r\nSubject: hi\r\n\r\nbody\r\n".to_vec();
        session
            .accept_submission(
                Some("alice@d.example".to_string()),
                recipients,
                body.clone(),
            )
            .await
            .unwrap();

        let queued = session.last_accepted().unwrap().body.clone();
        assert!(
            queued.ends_with(&body),
            "only the DKIM signature may be prepended"
        );
        let text = String::from_utf8(queued).unwrap();
        assert!(text.starts_with("DKIM-Signature: "));
        assert_eq!(text.matches("\nMessage-ID:").count(), 1);
        assert_eq!(text.matches("\nDate:").count(), 1);
    }
}
