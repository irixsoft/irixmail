use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use irixmail_core::{Error, Result};
use irixmail_directory::{attempt_login_blocking, Account, Directory, LoginAttempt, LoginPurpose};
use irixmail_mail::{
    delete_message, load_metadata, provision_mailboxes, MessageStoreCache, INBOX_ID,
};
use irixmail_store::{BlobStore, ChangeNotifier, Store};

use crate::cmd_auth::{auth_list, Mechanism, SaslExchange, SaslStart, SaslStep};
use crate::cmd_capa::capa_response;
use crate::cmd_dele::dele;
use crate::cmd_list::{list_all, list_one};
use crate::cmd_noop::noop_response;
use crate::cmd_pass::{pass_response, PassOutcome};
use crate::cmd_quit::quit_response;
use crate::cmd_retr::{no_such_message, retr_response};
use crate::cmd_rset::rset;
use crate::cmd_stat::stat_response;
use crate::cmd_stls::stls_reply;
use crate::cmd_top::top_response;
use crate::cmd_uidl::{uidl_all, uidl_one};
use crate::cmd_user::{user_response, UserOutcome};
use crate::parser::{parse_command, ParsedCommand};

const MAX_LINE_LENGTH: usize = 1024;
const GREETING: &[u8] = b"+OK IRIXMAIL POP3 ready\r\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    User,
    Pass,
    Apop,
    Capa,
    Stls,
    Auth,
    Stat,
    List,
    Uidl,
    Retr,
    Top,
    Dele,
    Rset,
    Noop,
    Utf8,
    Quit,
    Unknown,
}

impl Verb {
    fn from_word(word: &[u8]) -> Self {
        let mut upper = [0u8; 4];
        if word.is_empty() || word.len() > upper.len() {
            return Verb::Unknown;
        }
        for (slot, byte) in upper.iter_mut().zip(word) {
            *slot = byte.to_ascii_uppercase();
        }
        match &upper[..word.len()] {
            b"USER" => Verb::User,
            b"PASS" => Verb::Pass,
            b"APOP" => Verb::Apop,
            b"CAPA" => Verb::Capa,
            b"STLS" => Verb::Stls,
            b"AUTH" => Verb::Auth,
            b"STAT" => Verb::Stat,
            b"LIST" => Verb::List,
            b"UIDL" => Verb::Uidl,
            b"RETR" => Verb::Retr,
            b"TOP" => Verb::Top,
            b"DELE" => Verb::Dele,
            b"RSET" => Verb::Rset,
            b"NOOP" => Verb::Noop,
            b"UTF8" => Verb::Utf8,
            b"QUIT" => Verb::Quit,
            _ => Verb::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Authorization,
    Transaction,
}

#[derive(Default)]
pub struct SessionData {
    pub user: Option<String>,
    pub is_tls: bool,
    pub authenticated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageEntry {
    pub number: u32,
    pub size: u64,
    pub uid: String,
    pub document_id: u32,
    pub deleted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Close,
    Upgrade,
}

fn next_sid() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Session<S> {
    stream: BufReader<S>,
    peer: SocketAddr,
    sid: u64,
    state: State,
    data: SessionData,
    greet: bool,
    directory: Option<Directory>,
    blobs: Option<Arc<dyn BlobStore>>,
    notifier: Option<Arc<ChangeNotifier>>,
    account: Option<Account>,
    messages: Vec<MessageEntry>,
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
            state: State::Authorization,
            data: SessionData::default(),
            greet: true,
            directory: None,
            blobs: None,
            notifier: None,
            account: None,
            messages: Vec::new(),
        }
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

    pub fn without_greeting(mut self) -> Self {
        self.greet = false;
        self
    }

    pub fn with_directory(mut self, directory: Directory) -> Self {
        self.directory = Some(directory);
        self
    }

    pub fn with_blobs(mut self, blobs: Arc<dyn BlobStore>) -> Self {
        self.blobs = Some(blobs);
        self
    }

    pub fn with_notifier(mut self, notifier: Arc<ChangeNotifier>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn data(&self) -> &SessionData {
        &self.data
    }

    pub fn into_inner(self) -> S {
        self.stream.into_inner()
    }

    pub async fn run(&mut self) -> Result<Flow> {
        if self.greet {
            tracing::info!(
                target: "irixmail::pop3",
                sid = self.sid,
                peer = %self.peer,
                tls = self.data.is_tls,
                "connection accepted"
            );
            self.write(GREETING).await?;
        } else {
            tracing::info!(
                target: "irixmail::pop3",
                sid = self.sid,
                peer = %self.peer,
                "stls upgraded"
            );
        }

        let mut line = Vec::with_capacity(MAX_LINE_LENGTH);
        loop {
            line.clear();
            if !self.read_line(&mut line).await? {
                return Ok(Flow::Close);
            }
            match self.dispatch(parse_command(&line)).await? {
                Some(flow) => return Ok(flow),
                None => continue,
            }
        }
    }

    async fn dispatch(&mut self, command: ParsedCommand) -> Result<Option<Flow>> {
        match Verb::from_word(command.verb.as_bytes()) {
            Verb::User => {
                if self.state != State::Authorization {
                    self.write(user_response(UserOutcome::WrongState)).await?;
                } else if command.rest.trim().is_empty() {
                    self.write(user_response(UserOutcome::Empty)).await?;
                } else {
                    self.data.user = Some(command.rest.clone());
                    self.write(user_response(UserOutcome::Accepted)).await?;
                }
                Ok(None)
            }
            Verb::Pass => {
                let outcome = self.run_pass(&command.rest).await?;
                if outcome == PassOutcome::Authenticated {
                    self.state = State::Transaction;
                    self.data.authenticated = true;
                    self.load_maildrop()?;
                }
                self.write(pass_response(outcome)).await?;
                Ok(None)
            }
            Verb::Apop => {
                self.err("APOP is not supported").await?;
                Ok(None)
            }
            Verb::Capa => {
                self.write(&capa_response(self.data.is_tls)).await?;
                Ok(None)
            }
            Verb::Stls => {
                if !self.data.is_tls && self.state != State::Authorization {
                    self.err("STLS not allowed now").await?;
                    return Ok(None);
                }
                let reply = stls_reply(self.data.is_tls, true);
                self.write(reply.line()).await?;
                Ok(reply.upgrades().then_some(Flow::Upgrade))
            }
            Verb::Auth => self.run_auth(&command).await,
            Verb::Stat => {
                if self.state == State::Transaction {
                    self.write(stat_response(&self.messages).as_bytes()).await?;
                } else {
                    self.err("authenticate first").await?;
                }
                Ok(None)
            }
            Verb::List => {
                if self.state != State::Transaction {
                    self.err("authenticate first").await?;
                } else {
                    let response = match command.args.first() {
                        Some(arg) => match arg.parse::<u32>() {
                            Ok(number) => list_one(&self.messages, number),
                            Err(_) => "-ERR invalid message number\r\n".to_string(),
                        },
                        None => list_all(&self.messages),
                    };
                    self.write(response.as_bytes()).await?;
                }
                Ok(None)
            }
            Verb::Uidl => {
                if self.state != State::Transaction {
                    self.err("authenticate first").await?;
                } else {
                    let response = match command.args.first() {
                        Some(arg) => match arg.parse::<u32>() {
                            Ok(number) => uidl_one(&self.messages, number),
                            Err(_) => "-ERR invalid message number\r\n".to_string(),
                        },
                        None => uidl_all(&self.messages),
                    };
                    self.write(response.as_bytes()).await?;
                }
                Ok(None)
            }
            Verb::Retr => {
                if self.state != State::Transaction {
                    self.err("authenticate first").await?;
                } else {
                    let number = command.args.first().and_then(|arg| arg.parse::<u32>().ok());
                    let found = number.and_then(|number| {
                        self.messages
                            .iter()
                            .find(|message| message.number == number && !message.deleted)
                            .cloned()
                    });
                    match found {
                        Some(message) => match self.load_body(message.document_id)? {
                            Some((raw, _, _)) => {
                                self.write(&retr_response(message.size, &raw)).await?
                            }
                            None => self.write(no_such_message()).await?,
                        },
                        None => self.write(no_such_message()).await?,
                    }
                }
                Ok(None)
            }
            Verb::Top => {
                if self.state != State::Transaction {
                    self.err("authenticate first").await?;
                    return Ok(None);
                }
                let number = command.args.first().and_then(|arg| arg.parse::<u32>().ok());
                let lines = command
                    .args
                    .get(1)
                    .and_then(|arg| arg.parse::<usize>().ok());
                let (Some(number), Some(lines)) = (number, lines) else {
                    self.err("TOP expects a message number and line count")
                        .await?;
                    return Ok(None);
                };
                let document_id = self
                    .messages
                    .iter()
                    .find(|message| message.number == number && !message.deleted)
                    .map(|message| message.document_id);
                match document_id {
                    Some(document_id) => match self.load_body(document_id)? {
                        Some((raw, header, body)) => {
                            let response = top_response(&raw[header], &raw[body], lines);
                            self.write(&response).await?;
                        }
                        None => self.write(no_such_message()).await?,
                    },
                    None => self.write(no_such_message()).await?,
                }
                Ok(None)
            }
            Verb::Dele => {
                if self.state != State::Transaction {
                    self.err("authenticate first").await?;
                } else {
                    match command.args.first().and_then(|arg| arg.parse::<u32>().ok()) {
                        Some(number) => {
                            let response = dele(&mut self.messages, number);
                            self.write(response.as_bytes()).await?;
                        }
                        None => self.err("DELE expects a message number").await?,
                    }
                }
                Ok(None)
            }
            Verb::Rset => {
                if self.state != State::Transaction {
                    self.err("authenticate first").await?;
                } else {
                    let response = rset(&mut self.messages);
                    self.write(response.as_bytes()).await?;
                }
                Ok(None)
            }
            Verb::Noop => {
                if self.state == State::Transaction {
                    self.write(noop_response()).await?;
                } else {
                    self.err("authenticate first").await?;
                }
                Ok(None)
            }
            Verb::Utf8 => {
                self.ok("UTF8 enabled").await?;
                Ok(None)
            }
            Verb::Quit => {
                if self.state == State::Transaction {
                    self.commit_deletions()?;
                }
                self.write(quit_response()).await?;
                Ok(Some(Flow::Close))
            }
            Verb::Unknown => {
                self.err("unknown command").await?;
                Ok(None)
            }
        }
    }

    async fn run_auth(&mut self, command: &ParsedCommand) -> Result<Option<Flow>> {
        if self.state != State::Authorization {
            self.err("already in the transaction state").await?;
            return Ok(None);
        }
        let Some(mechanism) = command.args.first().map(|name| Mechanism::parse(name)) else {
            self.write(auth_list()).await?;
            return Ok(None);
        };
        let initial = command.args.get(1).map(String::as_str);
        let (mut exchange, mut step) =
            match SaslExchange::begin(mechanism, self.data.is_tls, initial) {
                SaslStart::Reply(line) => {
                    self.write(line).await?;
                    return Ok(None);
                }
                SaslStart::Continue { exchange, step } => (exchange, step),
            };
        loop {
            match step {
                SaslStep::Challenge(challenge) => {
                    let line = format!("+ {challenge}\r\n");
                    self.write(line.as_bytes()).await?;
                    let Some(response) = self.read_auth_response().await? else {
                        self.err("authentication cancelled").await?;
                        return Ok(None);
                    };
                    step = exchange.advance(&response);
                }
                SaslStep::Failed(reason) => {
                    self.err(reason).await?;
                    return Ok(None);
                }
                SaslStep::Resolved(credentials) => {
                    let attempt = self
                        .resolve_login(&credentials.username, &credentials.password)
                        .await?;
                    self.log_login(&credentials.username, &attempt);
                    match attempt {
                        LoginAttempt::Granted(account, _) => {
                            self.account = Some(*account);
                            self.data.authenticated = true;
                            self.state = State::Transaction;
                            self.load_maildrop()?;
                            self.ok("mailbox ready").await?;
                        }
                        LoginAttempt::Denied => self.err("authentication failed").await?,
                        LoginAttempt::Throttled => {
                            self.err("[AUTH] too many failed authentication attempts")
                                .await?
                        }
                    }
                    return Ok(None);
                }
            }
        }
    }

    async fn read_auth_response(&mut self) -> Result<Option<String>> {
        let mut line = Vec::with_capacity(MAX_LINE_LENGTH);
        if !self.read_line(&mut line).await? {
            return Err(Error::protocol("connection closed during AUTH"));
        }
        let response = String::from_utf8_lossy(&line).trim().to_string();
        if response == "*" {
            return Ok(None);
        }
        Ok(Some(response))
    }

    async fn run_pass(&mut self, password: &str) -> Result<PassOutcome> {
        if self.state != State::Authorization {
            return Ok(PassOutcome::WrongState);
        }
        let Some(username) = self.data.user.clone() else {
            return Ok(PassOutcome::NeedUser);
        };
        if !self.data.is_tls {
            return Ok(PassOutcome::NeedTls);
        }
        let attempt = self.resolve_login(&username, password).await?;
        self.log_login(&username, &attempt);
        match attempt {
            LoginAttempt::Granted(account, _) => {
                self.account = Some(*account);
                Ok(PassOutcome::Authenticated)
            }
            LoginAttempt::Denied => Ok(PassOutcome::Failed),
            LoginAttempt::Throttled => Ok(PassOutcome::Throttled),
        }
    }

    fn log_login(&self, user: &str, attempt: &LoginAttempt) {
        let outcome = match attempt {
            LoginAttempt::Granted(..) => "login succeeded",
            LoginAttempt::Denied => "login refused",
            LoginAttempt::Throttled => "login throttled",
        };
        tracing::info!(
            target: "irixmail::pop3",
            sid = self.sid,
            user = %user,
            "{outcome}"
        );
    }

    async fn resolve_login(&self, username: &str, password: &str) -> Result<LoginAttempt> {
        let Some(directory) = self.directory.as_ref() else {
            return Ok(LoginAttempt::Denied);
        };
        let ip = self.peer.ip().to_canonical().to_string();
        attempt_login_blocking(directory, Some(&ip), username, password, LoginPurpose::Mail).await
    }

    fn message_context(&self) -> Option<(Arc<dyn Store>, u32)> {
        let store = self.directory.as_ref()?.store();
        let account_id = self.account.as_ref()?.id as u32;
        Some((store, account_id))
    }

    fn load_maildrop(&mut self) -> Result<()> {
        let Some((store, account_id)) = self.message_context() else {
            return Ok(());
        };
        let created_at = self
            .account
            .as_ref()
            .map(|account| account.created_at)
            .unwrap_or(0);
        let uid_validity = provision_mailboxes(created_at)
            .into_iter()
            .find(|mailbox| mailbox.id == INBOX_ID)
            .map(|mailbox| mailbox.uid_validity)
            .unwrap_or(0);
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut entries: Vec<(u32, u32, u32)> = cache
            .in_mailbox(INBOX_ID)
            .filter_map(|entry| {
                entry
                    .uid_in(INBOX_ID)
                    .map(|uid| (uid, entry.document_id, entry.size))
            })
            .collect();
        entries.sort_by_key(|(uid, _, _)| *uid);
        self.messages = entries
            .into_iter()
            .enumerate()
            .map(|(index, (uid, document_id, size))| MessageEntry {
                number: index as u32 + 1,
                size: size as u64,
                uid: format!("{uid_validity}{uid}"),
                document_id,
                deleted: false,
            })
            .collect();
        tracing::info!(
            target: "irixmail::pop3",
            sid = self.sid,
            messages = self.messages.len(),
            bytes = self.messages.iter().map(|message| message.size).sum::<u64>(),
            "maildrop loaded"
        );
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    fn load_body(
        &self,
        document_id: u32,
    ) -> Result<Option<(Vec<u8>, std::ops::Range<usize>, std::ops::Range<usize>)>> {
        let Some((store, account_id)) = self.message_context() else {
            return Ok(None);
        };
        let Some(blobs) = self.blobs.as_ref() else {
            return Ok(None);
        };
        let Some(metadata) = load_metadata(store.as_ref(), account_id, document_id)? else {
            return Ok(None);
        };
        let Some(raw) = blobs.get_all(&metadata.blob_hash())? else {
            return Ok(None);
        };
        let (header, body) = match metadata.root() {
            Some(root) => (root.header.as_range(), root.body.as_range()),
            None => (0..0, 0..raw.len()),
        };
        Ok(Some((raw, header, body)))
    }

    fn commit_deletions(&mut self) -> Result<()> {
        let Some((store, account_id)) = self.message_context() else {
            return Ok(());
        };
        let (Some(blobs), Some(notifier)) = (self.blobs.clone(), self.notifier.clone()) else {
            return Ok(());
        };
        let doomed: Vec<u32> = self
            .messages
            .iter()
            .filter(|message| message.deleted)
            .map(|message| message.document_id)
            .collect();
        let deleted = doomed.len();
        for document_id in doomed {
            delete_message(
                store.as_ref(),
                blobs.as_ref(),
                notifier.as_ref(),
                account_id,
                document_id,
            )?;
        }
        self.messages.retain(|message| !message.deleted);
        if deleted > 0 {
            tracing::info!(
                target: "irixmail::pop3",
                sid = self.sid,
                deleted = deleted,
                "deletions committed"
            );
        }
        Ok(())
    }

    async fn ok(&mut self, text: &str) -> Result<()> {
        let line = format!("+OK {text}\r\n");
        self.write(line.as_bytes()).await
    }

    async fn err(&mut self, text: &str) -> Result<()> {
        let line = format!("-ERR {text}\r\n");
        self.write(line.as_bytes()).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
        "127.0.0.1:1100".parse().unwrap()
    }

    async fn drive(script: &[u8]) -> (Flow, String, State) {
        let mut session = Session::new(Pipe::new(script), peer());
        let flow = session.run().await.unwrap();
        let state = session.state();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        (flow, out, state)
    }

    fn transaction(script: &[u8]) -> Session<Pipe> {
        let mut session = Session::new(Pipe::new(script), peer()).with_tls();
        session.state = State::Transaction;
        session.data.authenticated = true;
        session
    }

    fn global_logs() -> irixmail_core::LogBuffer {
        use std::sync::OnceLock;
        use tracing_subscriber::layer::SubscriberExt;
        static LOGS: OnceLock<irixmail_core::LogBuffer> = OnceLock::new();
        LOGS.get_or_init(|| {
            let buffer = irixmail_core::LogBuffer::new();
            let _ = tracing::subscriber::set_global_default(
                tracing_subscriber::registry().with(buffer.layer()),
            );
            buffer
        })
        .clone()
    }

    fn pop3_log_text(logs: &irixmail_core::LogBuffer, sid: u64) -> String {
        let tagged = format!("sid={sid} ");
        logs.snapshot()
            .into_iter()
            .filter(|record| record.source == "irixmail::pop3")
            .map(|record| record.message)
            .filter(|message| message.contains(&tagged))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn a_pop3_session_logs_each_decision_under_one_session_id() {
        let logs = global_logs();
        let (directory, path) = account_directory("secret");
        let mut session = Session::new(
            Pipe::new(b"USER alice@example.com\r\nPASS secret\r\nSTAT\r\nQUIT\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory);
        let sid = session.session_id();
        session.run().await.unwrap();

        let text = pop3_log_text(&logs, sid);
        for needle in ["connection accepted", "login succeeded", "maildrop loaded"] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_failed_pop3_login_is_logged() {
        let logs = global_logs();
        let (directory, path) = account_directory("secret");
        let mut session = Session::new(
            Pipe::new(b"USER alice@example.com\r\nPASS wrong\r\nQUIT\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory);
        let sid = session.session_id();
        session.run().await.unwrap();
        let text = pop3_log_text(&logs, sid);
        assert!(text.contains("login refused"), "got:\n{text}");
        assert!(text.contains("alice@example.com"), "got:\n{text}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn verbs_parse_case_insensitively() {
        assert_eq!(Verb::from_word(b"user"), Verb::User);
        assert_eq!(Verb::from_word(b"RETR"), Verb::Retr);
        assert_eq!(Verb::from_word(b"WHAT"), Verb::Unknown);
        assert_eq!(Verb::from_word(b""), Verb::Unknown);
    }

    #[tokio::test]
    async fn the_greeting_is_sent_first() {
        let (flow, out, _) = drive(b"QUIT\r\n").await;
        assert_eq!(flow, Flow::Close);
        assert!(out.starts_with("+OK IRIXMAIL POP3 ready"));
    }

    #[tokio::test]
    async fn pass_over_plaintext_requires_stls() {
        let (_, out, state) = drive(b"USER alice@example.com\r\nPASS secret\r\n").await;
        assert_eq!(state, State::Authorization);
        assert!(out.contains("+OK send PASS"));
        assert!(out.contains("-ERR [AUTH] STLS required before PASS"));
    }

    #[tokio::test]
    async fn pass_over_tls_without_a_directory_fails() {
        let mut session = Session::new(
            Pipe::new(b"USER alice@example.com\r\nPASS secret\r\n"),
            peer(),
        )
        .with_tls();
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::Authorization);
        assert!(out.contains("-ERR authentication failed"));
    }

    fn account_directory(password: &str) -> (irixmail_directory::Directory, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        use irixmail_core::IdGenerator;
        use irixmail_directory::{password as pw, Directory, Role};
        use irixmail_store::{RocksdbStore, Store};

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "irixmail-pop3-auth-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();

        let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(path.join("db")).unwrap());
        let directory = Directory::new(store, Arc::new(IdGenerator::new(0)), None);
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        directory
            .credentials()
            .set_primary_password(account.id, pw::hash(password).unwrap())
            .unwrap();
        (directory, path)
    }

    #[tokio::test]
    async fn user_and_pass_with_valid_credentials_enter_the_transaction_state() {
        let (directory, path) = account_directory("secret");
        let mut session = Session::new(
            Pipe::new(b"USER alice@example.com\r\nPASS secret\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory);
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::Transaction);
        assert!(out.contains("+OK mailbox ready"));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn repeated_failed_logins_lock_the_source() {
        let (directory, path) = account_directory("secret");
        let mut script = String::from("USER alice@example.com\r\n");
        for _ in 0..5 {
            script.push_str("PASS wrong\r\n");
        }
        script.push_str("PASS secret\r\nQUIT\r\n");
        let mut session = Session::new(Pipe::new(script.as_bytes()), peer())
            .with_tls()
            .with_directory(directory);
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::Authorization);
        assert!(
            out.contains("too many failed authentication attempts"),
            "the locked attempt should be refused, got: {out}"
        );
        assert!(!out.contains("+OK mailbox ready"));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn auth_plain_with_valid_credentials_authenticates() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine as _;

        let (directory, path) = account_directory("secret");
        let payload = STANDARD.encode(b"\0alice@example.com\0secret");
        let script = format!("AUTH PLAIN {payload}\r\n");
        let mut session = Session::new(Pipe::new(script.as_bytes()), peer())
            .with_tls()
            .with_directory(directory);
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::Transaction);
        assert!(out.contains("+OK mailbox ready"));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[allow(clippy::type_complexity)]
    fn seed_account(
        messages: &[&[u8]],
    ) -> (
        Directory,
        Arc<dyn Store>,
        Account,
        Arc<dyn BlobStore>,
        Arc<ChangeNotifier>,
        std::path::PathBuf,
    ) {
        use std::sync::atomic::{AtomicU32, Ordering};

        use irixmail_core::IdGenerator;
        use irixmail_directory::{password as pw, Role};
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{FsBlobStore, RocksdbStore};

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "irixmail-pop3-drop-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(path.join("blobs")).unwrap();

        let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(path.join("db")).unwrap());
        let directory = Directory::new(store.clone(), Arc::new(IdGenerator::new(0)), None);
        let domain = directory
            .domains()
            .create("example.com", Vec::new())
            .unwrap();
        let account = directory
            .accounts()
            .create("alice", domain.id, "Alice", Role::User)
            .unwrap();
        directory
            .credentials()
            .set_primary_password(account.id, pw::hash("secret").unwrap())
            .unwrap();

        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let notifier = Arc::new(ChangeNotifier::new());
        let mailboxes = provision_mailboxes(account.created_at);
        for (index, raw) in messages.iter().enumerate() {
            let document_id = index as u32 + 1;
            deliver(
                store.as_ref(),
                blobs.as_ref(),
                &notifier,
                &DeliveryRequest {
                    account: &account,
                    mailboxes: &mailboxes,
                    sieve: None,
                    mail_from: "sender@example.net",
                    recipient: "alice@example.com",
                    document_id,
                    raw,
                    target_override: None,
                    received_at: 1_700_000_000,
                },
            )
            .unwrap();
        }
        (directory, store, account, blobs, notifier, path)
    }

    async fn maildrop_session(messages: &[&[u8]], script: &[u8]) -> String {
        let (directory, _store, _account, blobs, notifier, path) = seed_account(messages);
        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(notifier);
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        let _ = std::fs::remove_dir_all(&path);
        out
    }

    #[tokio::test]
    async fn stat_list_uidl_reflect_the_delivered_inbox() {
        let messages: [&[u8]; 2] = [
            b"Subject: One\r\nFrom: a@example.net\r\n\r\nfirst body\r\n",
            b"Subject: Two\r\nFrom: b@example.net\r\n\r\nsecond body\r\n",
        ];
        let size1 = messages[0].len();
        let size2 = messages[1].len();
        let total = size1 + size2;
        let out = maildrop_session(
            &messages,
            b"USER alice@example.com\r\nPASS secret\r\nSTAT\r\nLIST\r\nUIDL\r\nQUIT\r\n",
        )
        .await;
        assert!(out.contains(&format!("+OK 2 {total}\r\n")), "STAT: {out}");
        assert!(out.contains(&format!("1 {size1}\r\n")), "LIST msg1: {out}");
        assert!(out.contains(&format!("2 {size2}\r\n")), "LIST msg2: {out}");
        assert!(out.contains("+OK\r\n1 "), "UIDL first entry: {out}");
    }

    #[tokio::test]
    async fn retr_returns_the_full_message_and_top_limits_body_lines() {
        let message: &[u8] =
            b"Subject: Hello\r\nFrom: a@example.net\r\n\r\nline one\r\nline two\r\nline three\r\n";
        let messages: [&[u8]; 1] = [message];
        let out = maildrop_session(
            &messages,
            b"USER alice@example.com\r\nPASS secret\r\nRETR 1\r\nTOP 1 1\r\nTOP 1 0\r\nQUIT\r\n",
        )
        .await;
        assert!(
            out.contains(&format!(
                "+OK {} octets\r\nSubject: Hello\r\nFrom: a@example.net\r\n\r\nline one\r\nline two\r\nline three\r\n.\r\n",
                message.len()
            )),
            "RETR full message: {out}"
        );
        assert!(
            out.contains("+OK\r\nSubject: Hello\r\nFrom: a@example.net\r\n\r\nline one\r\n.\r\n"),
            "TOP 1 1 headers + one body line: {out}"
        );
        assert!(
            out.contains("+OK\r\nSubject: Hello\r\nFrom: a@example.net\r\n\r\n.\r\n"),
            "TOP 1 0 headers only: {out}"
        );
    }

    #[tokio::test]
    async fn utf8_is_acknowledged_and_noop_answers_in_transaction() {
        let (_, out, _) = drive(b"CAPA\r\nUTF8\r\nQUIT\r\n").await;
        assert!(out.contains("UTF8\r\n"), "CAPA advertises UTF8: {out}");
        assert!(
            out.contains("+OK UTF8 enabled"),
            "UTF8 command acknowledged: {out}"
        );

        let mut session = transaction(b"NOOP\r\n");
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("+OK"), "NOOP in transaction: {out}");
    }

    #[tokio::test]
    async fn retr_and_top_normalize_bare_lf_to_crlf() {
        let messages: [&[u8]; 1] = [b"Subject: Bare\nFrom: a@example.net\n\nline one\nline two\n"];
        let out = maildrop_session(
            &messages,
            b"USER alice@example.com\r\nPASS secret\r\nRETR 1\r\nTOP 1 1\r\nQUIT\r\n",
        )
        .await;
        assert!(
            out.contains(
                "Subject: Bare\r\nFrom: a@example.net\r\n\r\nline one\r\nline two\r\n.\r\n"
            ),
            "RETR normalizes bare LF: {out}"
        );
        assert!(
            out.contains("Subject: Bare\r\nFrom: a@example.net\r\n\r\nline one\r\n.\r\n"),
            "TOP normalizes bare LF: {out}"
        );
    }

    #[tokio::test]
    async fn quit_expunges_messages_marked_for_deletion() {
        let messages: [&[u8]; 2] = [
            b"Subject: One\r\nFrom: a@example.net\r\n\r\nfirst\r\n",
            b"Subject: Two\r\nFrom: b@example.net\r\n\r\nsecond\r\n",
        ];
        let (directory, store, account, blobs, notifier, path) = seed_account(&messages);
        let account_id = account.id as u32;
        let mut session = Session::new(
            Pipe::new(b"USER alice@example.com\r\nPASS secret\r\nDELE 1\r\nQUIT\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(blobs)
        .with_notifier(notifier);
        session.run().await.unwrap();

        let cache = MessageStoreCache::build(store.as_ref(), account_id).unwrap();
        let mut inbox: Vec<u32> = cache
            .in_mailbox(INBOX_ID)
            .map(|entry| entry.document_id)
            .collect();
        inbox.sort_unstable();
        assert_eq!(
            inbox,
            vec![2],
            "message 1 expunged from the store, message 2 survives"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn pass_before_user_is_rejected() {
        let (_, out, _) = drive(b"PASS secret\r\n").await;
        assert!(out.contains("-ERR send USER first"));
    }

    #[tokio::test]
    async fn transaction_commands_need_authentication() {
        let (_, out, _) = drive(b"STAT\r\n").await;
        assert!(out.contains("-ERR authenticate first"));
    }

    #[tokio::test]
    async fn stls_requests_an_upgrade() {
        let (flow, out, _) = drive(b"STLS\r\n").await;
        assert_eq!(flow, Flow::Upgrade);
        assert!(out.contains("+OK begin TLS negotiation"));
    }

    #[tokio::test]
    async fn quit_closes_the_session() {
        let (flow, out, _) = drive(b"QUIT\r\n").await;
        assert_eq!(flow, Flow::Close);
        assert!(out.contains("+OK IRIXMAIL POP3 signing off"));
    }

    #[tokio::test]
    async fn auth_lists_mechanisms_when_called_bare() {
        let (_, out, _) = drive(b"AUTH\r\n").await;
        assert!(out.contains("PLAIN"));
        assert!(out.contains("LOGIN"));
    }

    #[tokio::test]
    async fn auth_plain_on_plaintext_requires_stls() {
        let (_, out, _) = drive(b"AUTH PLAIN\r\n").await;
        assert!(out.contains("-ERR [AUTH] STLS required"));
    }

    #[tokio::test]
    async fn auth_plain_over_tls_without_a_directory_fails() {
        use base64::Engine as _;
        let payload =
            base64::engine::general_purpose::STANDARD.encode(b"\0alice@example.com\0secret");
        let script = format!("AUTH PLAIN {payload}\r\n");
        let mut session = Session::new(Pipe::new(script.as_bytes()), peer()).with_tls();
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::Authorization);
        assert!(out.contains("-ERR authentication failed"));
    }

    #[tokio::test]
    async fn an_unknown_command_is_an_error() {
        let (_, out, _) = drive(b"FROB\r\n").await;
        assert!(out.contains("-ERR unknown command"));
    }

    #[tokio::test]
    async fn stat_reports_the_empty_maildrop_once_authenticated() {
        let mut session = transaction(b"STAT\r\n");
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("+OK 0 0"));
    }

    #[tokio::test]
    async fn noop_succeeds_once_authenticated() {
        let mut session = transaction(b"NOOP\r\n");
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("+OK"));
    }
}
