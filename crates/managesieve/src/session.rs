use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use irixmail_core::{Error, Result};
use irixmail_directory::{attempt_login_blocking, Account, Directory, LoginAttempt, LoginPurpose};

use crate::parser::{tokenize_line, Token};
use crate::sasl::{decode_base64, decode_plain, encode_base64, Mechanism};

const MAX_LINE_LENGTH: usize = 8192;
pub const MAX_SCRIPT_SIZE: usize = 128 * 1024;
pub const MAX_SCRIPTS: usize = 32;

fn next_sid() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Close,
    Upgrade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NotAuthenticated,
    Authenticated,
}

pub struct Session<S> {
    stream: BufReader<S>,
    peer: SocketAddr,
    sid: u64,
    state: State,
    is_tls: bool,
    resumed: bool,
    directory: Option<Directory>,
    account: Option<Account>,
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
            state: State::NotAuthenticated,
            is_tls: false,
            resumed: false,
            directory: None,
            account: None,
        }
    }

    pub fn with_session_id(mut self, sid: u64) -> Self {
        self.sid = sid;
        self.resumed = true;
        self
    }

    pub fn with_tls(mut self) -> Self {
        self.is_tls = true;
        self
    }

    pub fn with_directory(mut self, directory: Directory) -> Self {
        self.directory = Some(directory);
        self
    }

    pub fn session_id(&self) -> u64 {
        self.sid
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn into_inner(self) -> S {
        self.stream.into_inner()
    }

    pub async fn run(&mut self) -> Result<Flow> {
        if self.resumed {
            tracing::info!(target: "irixmail::managesieve", sid = self.sid, peer = %self.peer, "starttls upgraded");
        } else {
            tracing::info!(target: "irixmail::managesieve", sid = self.sid, peer = %self.peer, tls = self.is_tls, "connection accepted");
        }
        self.send_capabilities().await?;
        self.respond("OK", None, "IRIXMAIL ManageSieve ready")
            .await?;

        let mut line = Vec::new();
        loop {
            line.clear();
            if !self.read_line(&mut line).await? {
                return Ok(Flow::Close);
            }
            let tokens = match tokenize_line(strip_cr(&line)) {
                Ok(tokens) => tokens,
                Err(error) => {
                    self.respond("NO", None, &error.to_string()).await?;
                    continue;
                }
            };
            let Some(tokens) = self.resolve_literals(tokens).await? else {
                continue;
            };
            if tokens.is_empty() {
                continue;
            }
            if let Some(flow) = self.dispatch(&tokens).await? {
                return Ok(flow);
            }
        }
    }

    async fn resolve_literals(&mut self, mut tokens: Vec<Token>) -> Result<Option<Vec<Token>>> {
        while let Some(&Token::Literal(length)) = tokens.last() {
            if length > MAX_SCRIPT_SIZE {
                self.respond("BYE", None, "literal too large").await?;
                return Err(Error::protocol("oversized ManageSieve literal"));
            }
            let mut content = vec![0u8; length];
            self.stream
                .read_exact(&mut content)
                .await
                .map_err(|err| Error::protocol(format!("literal read failed: {err}")))?;
            let mut rest = Vec::new();
            if !self.read_line(&mut rest).await? {
                return Ok(None);
            }
            let continuation = match tokenize_line(strip_cr(&rest)) {
                Ok(tokens) => tokens,
                Err(error) => {
                    self.respond("NO", None, &error.to_string()).await?;
                    return Ok(None);
                }
            };
            let Ok(content) = String::from_utf8(content) else {
                self.respond("NO", None, "script is not valid utf-8")
                    .await?;
                return Ok(None);
            };
            tokens.pop();
            tokens.push(Token::Str(content));
            tokens.extend(continuation);
        }
        Ok(Some(tokens))
    }

    async fn dispatch(&mut self, tokens: &[Token]) -> Result<Option<Flow>> {
        let verb = tokens[0].as_str().unwrap_or_default().to_ascii_uppercase();
        let needs_auth = matches!(
            verb.as_str(),
            "HAVESPACE"
                | "PUTSCRIPT"
                | "LISTSCRIPTS"
                | "SETACTIVE"
                | "GETSCRIPT"
                | "DELETESCRIPT"
                | "RENAMESCRIPT"
                | "CHECKSCRIPT"
        );
        if needs_auth && self.state != State::Authenticated {
            self.respond("NO", None, "Authenticate first").await?;
            return Ok(None);
        }
        match verb.as_str() {
            "CAPABILITY" => {
                self.send_capabilities().await?;
                self.respond("OK", None, "Capability completed").await?;
            }
            "STARTTLS" => {
                if self.is_tls {
                    self.respond("NO", None, "TLS already active").await?;
                } else {
                    self.respond("OK", None, "Begin TLS negotiation now")
                        .await?;
                    return Ok(Some(Flow::Upgrade));
                }
            }
            "AUTHENTICATE" => self.run_authenticate(tokens).await?,
            "LOGOUT" => {
                self.respond("OK", None, "Bye").await?;
                return Ok(Some(Flow::Close));
            }
            "NOOP" => match tokens.get(1).and_then(Token::as_str) {
                Some(tag) => {
                    let code = format!("TAG {}", quoted(tag));
                    self.respond("OK", Some(&code), "Done").await?;
                }
                None => self.respond("OK", None, "NOOP completed").await?,
            },
            "UNAUTHENTICATE" => {
                if self.state == State::Authenticated {
                    self.account = None;
                    self.state = State::NotAuthenticated;
                    self.respond("OK", None, "Unauthenticate completed").await?;
                } else {
                    self.respond("NO", None, "Not authenticated").await?;
                }
            }
            "HAVESPACE" => self.run_havespace(tokens).await?,
            "PUTSCRIPT" => self.run_putscript(tokens).await?,
            "LISTSCRIPTS" => self.run_listscripts().await?,
            "SETACTIVE" => self.run_setactive(tokens).await?,
            "GETSCRIPT" => self.run_getscript(tokens).await?,
            "DELETESCRIPT" => self.run_deletescript(tokens).await?,
            "RENAMESCRIPT" => self.run_renamescript(tokens).await?,
            "CHECKSCRIPT" => self.run_checkscript(tokens).await?,
            _ => self.respond("NO", None, "unknown command").await?,
        }
        Ok(None)
    }

    async fn run_authenticate(&mut self, tokens: &[Token]) -> Result<()> {
        if !self.is_tls {
            return self
                .respond("NO", Some("ENCRYPT-NEEDED"), "AUTHENTICATE requires TLS")
                .await;
        }
        if self.state == State::Authenticated {
            return self.respond("NO", None, "Already authenticated").await;
        }
        let Some(mechanism) = tokens.get(1).and_then(Token::as_str) else {
            return self
                .respond("NO", None, "AUTHENTICATE requires a mechanism")
                .await;
        };
        match Mechanism::parse(mechanism) {
            Mechanism::Unsupported => self.respond("NO", None, "unsupported mechanism").await,
            Mechanism::Plain => {
                let response = match tokens.get(2).and_then(Token::as_str) {
                    Some(initial) => initial.to_string(),
                    None => {
                        self.write_challenge("").await?;
                        match self.read_auth_response().await? {
                            Some(response) => response,
                            None => {
                                return self.respond("NO", None, "authentication cancelled").await;
                            }
                        }
                    }
                };
                match decode_plain(&response) {
                    Some((user, password)) => self.attempt(&user, &password).await,
                    None => self.respond("NO", None, "invalid SASL response").await,
                }
            }
            Mechanism::Login => {
                self.write_challenge(&encode_base64("Username:")).await?;
                let Some(user) = self.read_auth_response().await? else {
                    return self.respond("NO", None, "authentication cancelled").await;
                };
                self.write_challenge(&encode_base64("Password:")).await?;
                let Some(password) = self.read_auth_response().await? else {
                    return self.respond("NO", None, "authentication cancelled").await;
                };
                match (decode_base64(&user), decode_base64(&password)) {
                    (Some(user), Some(password)) => self.attempt(&user, &password).await,
                    _ => self.respond("NO", None, "invalid SASL response").await,
                }
            }
        }
    }

    async fn attempt(&mut self, user: &str, password: &str) -> Result<()> {
        let Some(directory) = self.directory.clone() else {
            return self.respond("NO", None, "authentication failed").await;
        };
        let ip = self.peer.ip().to_canonical().to_string();
        let attempt =
            attempt_login_blocking(&directory, Some(&ip), user, password, LoginPurpose::Mail)
                .await?;
        self.log_login(user, &attempt);
        match attempt {
            LoginAttempt::Granted(account, _) => {
                self.account = Some(*account);
                self.state = State::Authenticated;
                self.respond("OK", None, "Authenticated").await
            }
            LoginAttempt::Denied => self.respond("NO", None, "authentication failed").await,
            LoginAttempt::Throttled => {
                self.respond("NO", Some("TRYLATER"), "too many attempts, try again later")
                    .await
            }
        }
    }

    fn log_login(&self, user: &str, attempt: &LoginAttempt) {
        let outcome = match attempt {
            LoginAttempt::Granted(..) => "login succeeded",
            LoginAttempt::Denied => "login refused",
            LoginAttempt::Throttled => "login throttled",
        };
        tracing::info!(target: "irixmail::managesieve", sid = self.sid, user = %user, "{outcome}");
    }

    async fn run_havespace(&mut self, tokens: &[Token]) -> Result<()> {
        let (Some(name), Some(size)) = (
            tokens.get(1).and_then(Token::as_str).map(str::to_string),
            tokens
                .get(2)
                .and_then(Token::as_str)
                .and_then(|value| value.parse::<u64>().ok()),
        ) else {
            return self
                .respond("NO", None, "HAVESPACE requires a name and a size")
                .await;
        };
        if size > MAX_SCRIPT_SIZE as u64 {
            return self
                .respond("NO", Some("QUOTA/MAXSIZE"), "script is too large")
                .await;
        }
        let scripts = self.scripts()?;
        let exists = scripts.iter().any(|script| script.name == name);
        if !exists && scripts.len() >= MAX_SCRIPTS {
            return self.respond("NO", Some("QUOTA"), "too many scripts").await;
        }
        self.respond("OK", None, "Havespace completed").await
    }

    async fn run_putscript(&mut self, tokens: &[Token]) -> Result<()> {
        let (Some(name), Some(content)) = (
            tokens.get(1).and_then(Token::as_str).map(str::to_string),
            tokens.get(2).and_then(Token::as_str).map(str::to_string),
        ) else {
            return self
                .respond("NO", None, "PUTSCRIPT requires a name and a script")
                .await;
        };
        if name.is_empty() {
            return self.respond("NO", None, "a script needs a name").await;
        }
        if let Err(error) = irixmail_sieve::Compiler::new().compile(&content) {
            return self.respond("NO", None, &error.to_string()).await;
        }
        let account = self.account_id();
        let sieve = self.registry()?;
        match sieve.get_by_name(account, &name)? {
            Some(existing) => {
                let keeps_rules =
                    existing.rules.is_some() && irixmail_mail::script_source(&existing) == content;
                let rules_change = if keeps_rules { None } else { Some(None) };
                sieve.update(account, &existing.id, None, Some(&content), rules_change)?;
            }
            None => {
                if self.scripts()?.len() >= MAX_SCRIPTS {
                    return self.respond("NO", Some("QUOTA"), "too many scripts").await;
                }
                sieve.create(account, &name, &content, None)?;
            }
        }
        tracing::info!(target: "irixmail::managesieve", sid = self.sid, script = %name, "script stored");
        self.respond("OK", None, "Putscript completed").await
    }

    async fn run_listscripts(&mut self) -> Result<()> {
        let mut listing = String::new();
        for script in self.scripts()? {
            listing.push_str(&quoted(&script.name));
            if script.active {
                listing.push_str(" ACTIVE");
            }
            listing.push_str("\r\n");
        }
        self.write(listing.as_bytes()).await?;
        self.respond("OK", None, "Listscripts completed").await
    }

    async fn run_setactive(&mut self, tokens: &[Token]) -> Result<()> {
        let Some(name) = tokens.get(1).and_then(Token::as_str).map(str::to_string) else {
            return self.respond("NO", None, "SETACTIVE requires a name").await;
        };
        let account = self.account_id();
        let sieve = self.registry()?;
        if name.is_empty() {
            sieve.set_active(account, None)?;
        } else {
            let Some(script) = sieve.get_by_name(account, &name)? else {
                return self
                    .respond("NO", Some("NONEXISTENT"), "no script by that name")
                    .await;
            };
            sieve.set_active(account, Some(&script.id))?;
        }
        tracing::info!(target: "irixmail::managesieve", sid = self.sid, script = %name, "script activated");
        self.respond("OK", None, "Setactive completed").await
    }

    async fn run_getscript(&mut self, tokens: &[Token]) -> Result<()> {
        let Some(name) = tokens.get(1).and_then(Token::as_str).map(str::to_string) else {
            return self.respond("NO", None, "GETSCRIPT requires a name").await;
        };
        let account = self.account_id();
        let Some(script) = self.registry()?.get_by_name(account, &name)? else {
            return self
                .respond("NO", Some("NONEXISTENT"), "no script by that name")
                .await;
        };
        let source = irixmail_mail::script_source(&script);
        let reply = format!("{{{}}}\r\n{source}\r\n", source.len());
        self.write(reply.as_bytes()).await?;
        self.respond("OK", None, "Getscript completed").await
    }

    async fn run_deletescript(&mut self, tokens: &[Token]) -> Result<()> {
        let Some(name) = tokens.get(1).and_then(Token::as_str).map(str::to_string) else {
            return self
                .respond("NO", None, "DELETESCRIPT requires a name")
                .await;
        };
        let account = self.account_id();
        let sieve = self.registry()?;
        let Some(script) = sieve.get_by_name(account, &name)? else {
            return self
                .respond("NO", Some("NONEXISTENT"), "no script by that name")
                .await;
        };
        if script.active {
            return self
                .respond("NO", Some("ACTIVE"), "you may not delete the active script")
                .await;
        }
        sieve.destroy(account, &script.id)?;
        tracing::info!(target: "irixmail::managesieve", sid = self.sid, script = %name, "script deleted");
        self.respond("OK", None, "Deletescript completed").await
    }

    async fn run_renamescript(&mut self, tokens: &[Token]) -> Result<()> {
        let (Some(from), Some(to)) = (
            tokens.get(1).and_then(Token::as_str).map(str::to_string),
            tokens.get(2).and_then(Token::as_str).map(str::to_string),
        ) else {
            return self
                .respond("NO", None, "RENAMESCRIPT requires two names")
                .await;
        };
        let account = self.account_id();
        let sieve = self.registry()?;
        let Some(script) = sieve.get_by_name(account, &from)? else {
            return self
                .respond("NO", Some("NONEXISTENT"), "no script by that name")
                .await;
        };
        if sieve.get_by_name(account, &to)?.is_some() {
            return self
                .respond("NO", Some("ALREADYEXISTS"), "a script by that name exists")
                .await;
        }
        sieve.update(account, &script.id, Some(&to), None, None)?;
        tracing::info!(target: "irixmail::managesieve", sid = self.sid, from = %from, to = %to, "script renamed");
        self.respond("OK", None, "Renamescript completed").await
    }

    async fn run_checkscript(&mut self, tokens: &[Token]) -> Result<()> {
        let Some(content) = tokens.get(1).and_then(Token::as_str) else {
            return self
                .respond("NO", None, "CHECKSCRIPT requires a script")
                .await;
        };
        match irixmail_sieve::Compiler::new().compile(content) {
            Ok(_) => self.respond("OK", None, "Checkscript completed").await,
            Err(error) => self.respond("NO", None, &error.to_string()).await,
        }
    }

    fn registry(&self) -> Result<&irixmail_directory::SieveScriptRegistry> {
        self.directory
            .as_ref()
            .map(|directory| directory.sieve())
            .ok_or_else(|| Error::internal("the session has no directory"))
    }

    fn scripts(&self) -> Result<Vec<irixmail_directory::StoredScript>> {
        self.registry()?.list(self.account_id())
    }

    fn account_id(&self) -> u64 {
        self.account.as_ref().map(|account| account.id).unwrap_or(0)
    }

    async fn send_capabilities(&mut self) -> Result<()> {
        let mut out = String::new();
        out.push_str("\"IMPLEMENTATION\" \"IRIXMAIL\"\r\n");
        out.push_str(&format!(
            "\"SIEVE\" \"{}\"\r\n",
            irixmail_sieve::CAPABILITIES.join(" ")
        ));
        let sasl = if self.is_tls { "PLAIN LOGIN" } else { "" };
        out.push_str(&format!("\"SASL\" \"{sasl}\"\r\n"));
        if !self.is_tls {
            out.push_str("\"STARTTLS\"\r\n");
        }
        out.push_str("\"MAXREDIRECTS\" \"4\"\r\n");
        out.push_str("\"VERSION\" \"1.0\"\r\n");
        self.write(out.as_bytes()).await
    }

    async fn write_challenge(&mut self, challenge: &str) -> Result<()> {
        let line = format!("{}\r\n", quoted(challenge));
        self.write(line.as_bytes()).await
    }

    async fn read_auth_response(&mut self) -> Result<Option<String>> {
        let mut line = Vec::new();
        if !self.read_line(&mut line).await? {
            return Ok(None);
        }
        let line = strip_cr(&line);
        if line == b"*" {
            return Ok(None);
        }
        match tokenize_line(line) {
            Ok(tokens) => match tokens.first().and_then(Token::as_str) {
                Some("*") | None => Ok(None),
                Some(value) => Ok(Some(value.to_string())),
            },
            Err(_) => Ok(None),
        }
    }

    async fn respond(&mut self, status: &str, code: Option<&str>, text: &str) -> Result<()> {
        let line = match code {
            Some(code) => format!("{status} ({code}) {}\r\n", quoted(text)),
            None => format!("{status} {}\r\n", quoted(text)),
        };
        self.write(line.as_bytes()).await
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let stream = self.stream.get_mut();
        stream
            .write_all(bytes)
            .await
            .map_err(|err| Error::internal(format!("write failed: {err}")))?;
        stream
            .flush()
            .await
            .map_err(|err| Error::internal(format!("flush failed: {err}")))
    }

    async fn read_line(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        loop {
            match self.stream.read_u8().await {
                Ok(b'\n') => return Ok(true),
                Ok(byte) => {
                    buf.push(byte);
                    if buf.len() > MAX_LINE_LENGTH {
                        return Err(Error::protocol("line too long"));
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(!buf.is_empty());
                }
                Err(err) => return Err(Error::internal(format!("read failed: {err}"))),
            }
        }
    }
}

fn strip_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\r' | '\n' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use irixmail_directory::{password as pw, Role};

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
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.input).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for Pipe {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.output.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn peer() -> SocketAddr {
        "127.0.0.1:41900".parse().unwrap()
    }

    fn account_directory(password: &str) -> (Directory, Account, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        use irixmail_core::IdGenerator;
        use irixmail_store::{RocksdbStore, Store};

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "irixmail-managesieve-{}-{unique}",
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
        (directory, account, path)
    }

    async fn run_collect(mut session: Session<Pipe>) -> (Flow, String, Session<Pipe>) {
        let flow = session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        (flow, out, session)
    }

    async fn drive(script: &[u8]) -> (Flow, String) {
        let (flow, out, _) = run_collect(Session::new(Pipe::new(script), peer())).await;
        (flow, out)
    }

    fn authed_session(script: &[u8], directory: &Directory, account: &Account) -> Session<Pipe> {
        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory.clone());
        session.state = State::Authenticated;
        session.account = Some(account.clone());
        session
    }

    fn plain_response(user: &str, password: &str) -> String {
        encode_base64(&format!("\0{user}\0{password}"))
    }

    #[tokio::test]
    async fn the_greeting_advertises_capabilities_before_ok() {
        let (flow, out) = drive(b"LOGOUT\r\n").await;
        assert_eq!(flow, Flow::Close);
        assert!(out.contains("\"IMPLEMENTATION\" \"IRIXMAIL\""));
        assert!(out.contains("\"SIEVE\" \"fileinto envelope imap4flags"));
        assert!(out.contains("\"SASL\" \"\""));
        assert!(out.contains("\"STARTTLS\""));
        assert!(out.contains("\"VERSION\" \"1.0\""));
        assert!(out.contains("OK \"IRIXMAIL ManageSieve ready\""));
    }

    #[tokio::test]
    async fn a_tls_session_advertises_sasl_and_hides_starttls() {
        let (_, out, _) =
            run_collect(Session::new(Pipe::new(b"CAPABILITY\r\nLOGOUT\r\n"), peer()).with_tls())
                .await;
        assert!(out.contains("\"SASL\" \"PLAIN LOGIN\""));
        assert!(!out.contains("\"STARTTLS\""));
        assert!(out.contains("OK \"Capability completed\""));
    }

    #[tokio::test]
    async fn starttls_hands_back_the_upgrade_flow() {
        let (flow, out) = drive(b"STARTTLS\r\n").await;
        assert_eq!(flow, Flow::Upgrade);
        assert!(out.contains("OK \"Begin TLS negotiation now\""));
        let (_, out, _) =
            run_collect(Session::new(Pipe::new(b"STARTTLS\r\nLOGOUT\r\n"), peer()).with_tls())
                .await;
        assert!(out.contains("NO \"TLS already active\""));
    }

    #[tokio::test]
    async fn authenticate_without_tls_is_refused() {
        let (_, out) = drive(b"AUTHENTICATE \"PLAIN\" \"x\"\r\nLOGOUT\r\n").await;
        assert!(out.contains("NO (ENCRYPT-NEEDED) \"AUTHENTICATE requires TLS\""));
    }

    #[tokio::test]
    async fn commands_require_authentication() {
        let (_, out, _) =
            run_collect(Session::new(Pipe::new(b"LISTSCRIPTS\r\nLOGOUT\r\n"), peer()).with_tls())
                .await;
        assert!(out.contains("NO \"Authenticate first\""));
    }

    #[tokio::test]
    async fn a_plain_authentication_with_valid_credentials_succeeds() {
        let (directory, _, path) = account_directory("secret");
        let script = format!(
            "AUTHENTICATE \"PLAIN\" \"{}\"\r\nLOGOUT\r\n",
            plain_response("alice@example.com", "secret")
        );
        let (_, out, session) = run_collect(
            Session::new(Pipe::new(script.as_bytes()), peer())
                .with_tls()
                .with_directory(directory),
        )
        .await;
        assert!(out.contains("OK \"Authenticated\""));
        assert_eq!(session.state(), State::Authenticated);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_wrong_password_is_refused() {
        let (directory, _, path) = account_directory("secret");
        let script = format!(
            "AUTHENTICATE \"PLAIN\" \"{}\"\r\nLOGOUT\r\n",
            plain_response("alice@example.com", "wrong")
        );
        let (_, out, session) = run_collect(
            Session::new(Pipe::new(script.as_bytes()), peer())
                .with_tls()
                .with_directory(directory),
        )
        .await;
        assert!(out.contains("NO \"authentication failed\""));
        assert_eq!(session.state(), State::NotAuthenticated);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_cancelled_authentication_reports_it() {
        let (_, out, _) = run_collect(
            Session::new(
                Pipe::new(b"AUTHENTICATE \"PLAIN\"\r\n*\r\nLOGOUT\r\n"),
                peer(),
            )
            .with_tls(),
        )
        .await;
        assert!(out.contains("\"\"\r\n"));
        assert!(out.contains("NO \"authentication cancelled\""));
    }

    #[tokio::test]
    async fn the_login_mechanism_challenges_for_username_and_password() {
        let (directory, _, path) = account_directory("secret");
        let script = format!(
            "AUTHENTICATE \"LOGIN\"\r\n\"{}\"\r\n\"{}\"\r\nLOGOUT\r\n",
            encode_base64("alice@example.com"),
            encode_base64("secret")
        );
        let (_, out, session) = run_collect(
            Session::new(Pipe::new(script.as_bytes()), peer())
                .with_tls()
                .with_directory(directory),
        )
        .await;
        assert!(out.contains(&quoted(&encode_base64("Username:"))));
        assert!(out.contains(&quoted(&encode_base64("Password:"))));
        assert!(out.contains("OK \"Authenticated\""));
        assert_eq!(session.state(), State::Authenticated);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn putscript_stores_a_script_and_listscripts_marks_it_active() {
        let (directory, account, path) = account_directory("secret");
        let content = b"require \"fileinto\";\r\nfileinto \"Receipts\";\r\n";
        let script = [
            format!("PUTSCRIPT \"test\" {{{}+}}\r\n", content.len()).into_bytes(),
            content.to_vec(),
            b"\r\nLISTSCRIPTS\r\nLOGOUT\r\n".to_vec(),
        ]
        .concat();
        let (_, out, _) = run_collect(authed_session(&script, &directory, &account)).await;
        assert!(out.contains("OK \"Putscript completed\""));
        assert!(out.contains("\"test\" ACTIVE\r\n"));
        let stored = directory.sieve().list(account.id).unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].rules.is_none());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn putscript_rejects_an_invalid_script_with_its_line() {
        let (directory, account, path) = account_directory("secret");
        let (_, out, _) = run_collect(authed_session(
            b"PUTSCRIPT \"bad\" {9+}\r\nfrobnate;\r\nLOGOUT\r\n",
            &directory,
            &account,
        ))
        .await;
        assert!(out.contains("NO \"line 1:"));
        assert!(directory.sieve().list(account.id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn putscript_keeps_the_rules_sidecar_only_when_the_source_round_trips() {
        let (directory, account, path) = account_directory("secret");
        let rules = serde_json::json!([{"id": "r1", "name": "receipts", "field": "subject",
            "operator": "contains", "value": "receipt", "action": "fileinto",
            "target": "Receipts"}]);
        let source = irixmail_mail::emit_script(&irixmail_mail::stored_rule_set(&rules));
        directory
            .sieve()
            .create(account.id, "filters", &source, Some(rules))
            .unwrap();

        let same = [
            format!("PUTSCRIPT \"filters\" {{{}+}}\r\n", source.len()).into_bytes(),
            source.clone().into_bytes(),
            b"\r\nLOGOUT\r\n".to_vec(),
        ]
        .concat();
        run_collect(authed_session(&same, &directory, &account)).await;
        assert!(directory.sieve().list(account.id).unwrap()[0]
            .rules
            .is_some());

        let edited = b"PUTSCRIPT \"filters\" {8+}\r\ndiscard;\r\nLOGOUT\r\n";
        run_collect(authed_session(edited, &directory, &account)).await;
        let stored = directory.sieve().list(account.id).unwrap();
        assert!(stored[0].rules.is_none());
        assert_eq!(stored[0].source, "discard;");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn getscript_returns_the_source_as_a_literal() {
        let (directory, account, path) = account_directory("secret");
        directory
            .sieve()
            .create(account.id, "custom", "keep;", None)
            .unwrap();
        let (_, out, _) = run_collect(authed_session(
            b"GETSCRIPT \"custom\"\r\nGETSCRIPT \"missing\"\r\nLOGOUT\r\n",
            &directory,
            &account,
        ))
        .await;
        assert!(out.contains("{5}\r\nkeep;\r\nOK \"Getscript completed\""));
        assert!(out.contains("NO (NONEXISTENT) \"no script by that name\""));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn setactive_switches_and_empty_deactivates_all() {
        let (directory, account, path) = account_directory("secret");
        directory
            .sieve()
            .create(account.id, "one", "keep;", None)
            .unwrap();
        directory
            .sieve()
            .create(account.id, "two", "keep;", None)
            .unwrap();
        let (_, out, _) = run_collect(authed_session(
            b"SETACTIVE \"two\"\r\nSETACTIVE \"missing\"\r\nSETACTIVE \"\"\r\nLOGOUT\r\n",
            &directory,
            &account,
        ))
        .await;
        assert!(out.contains("OK \"Setactive completed\""));
        assert!(out.contains("NO (NONEXISTENT)"));
        assert!(directory
            .sieve()
            .active_script(account.id)
            .unwrap()
            .is_none());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn the_active_script_cannot_be_deleted() {
        let (directory, account, path) = account_directory("secret");
        directory
            .sieve()
            .create(account.id, "one", "keep;", None)
            .unwrap();
        let (_, out, _) = run_collect(authed_session(
            b"DELETESCRIPT \"one\"\r\nSETACTIVE \"\"\r\nDELETESCRIPT \"one\"\r\nLOGOUT\r\n",
            &directory,
            &account,
        ))
        .await;
        assert!(out.contains("NO (ACTIVE) \"you may not delete the active script\""));
        assert!(out.contains("OK \"Deletescript completed\""));
        assert!(directory.sieve().list(account.id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn renamescript_moves_a_name_and_guards_collisions() {
        let (directory, account, path) = account_directory("secret");
        directory
            .sieve()
            .create(account.id, "one", "keep;", None)
            .unwrap();
        directory
            .sieve()
            .create(account.id, "two", "keep;", None)
            .unwrap();
        let (_, out, _) = run_collect(authed_session(
            b"RENAMESCRIPT \"missing\" \"x\"\r\nRENAMESCRIPT \"one\" \"two\"\r\nRENAMESCRIPT \"one\" \"three\"\r\nLOGOUT\r\n",
            &directory,
            &account,
        ))
        .await;
        assert!(out.contains("NO (NONEXISTENT)"));
        assert!(out.contains("NO (ALREADYEXISTS)"));
        assert!(out.contains("OK \"Renamescript completed\""));
        assert!(directory
            .sieve()
            .get_by_name(account.id, "three")
            .unwrap()
            .is_some());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn checkscript_validates_without_storing() {
        let (directory, account, path) = account_directory("secret");
        let (_, out, _) = run_collect(authed_session(
            b"CHECKSCRIPT {5+}\r\nkeep;\r\nCHECKSCRIPT {9+}\r\nfrobnate;\r\nLOGOUT\r\n",
            &directory,
            &account,
        ))
        .await;
        assert!(out.contains("OK \"Checkscript completed\""));
        assert!(out.contains("NO \"line 1:"));
        assert!(directory.sieve().list(account.id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn havespace_enforces_the_size_and_count_limits() {
        let (directory, account, path) = account_directory("secret");
        let over = MAX_SCRIPT_SIZE + 1;
        let script = format!("HAVESPACE \"x\" {over}\r\nHAVESPACE \"x\" 1024\r\nLOGOUT\r\n");
        let (_, out, _) =
            run_collect(authed_session(script.as_bytes(), &directory, &account)).await;
        assert!(out.contains("NO (QUOTA/MAXSIZE) \"script is too large\""));
        assert!(out.contains("OK \"Havespace completed\""));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn an_oversized_literal_ends_the_connection_with_bye() {
        let (directory, account, path) = account_directory("secret");
        let script = format!("PUTSCRIPT \"x\" {{{}+}}\r\n", MAX_SCRIPT_SIZE + 1);
        let mut session = authed_session(script.as_bytes(), &directory, &account);
        let error = session.run().await.unwrap_err();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("BYE \"literal too large\""));
        assert!(error.to_string().contains("oversized"));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn noop_echoes_its_tag_and_unauthenticate_resets_the_state() {
        let (directory, account, path) = account_directory("secret");
        let (_, out, session) = run_collect(authed_session(
            b"NOOP \"sync-1\"\r\nNOOP\r\nUNAUTHENTICATE\r\nLISTSCRIPTS\r\nLOGOUT\r\n",
            &directory,
            &account,
        ))
        .await;
        assert!(out.contains("OK (TAG \"sync-1\") \"Done\""));
        assert!(out.contains("OK \"NOOP completed\""));
        assert!(out.contains("OK \"Unauthenticate completed\""));
        assert!(out.contains("NO \"Authenticate first\""));
        assert_eq!(session.state(), State::NotAuthenticated);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn an_unknown_command_is_refused_without_closing() {
        let (_, out) = drive(b"FROBNICATE\r\nLOGOUT\r\n").await;
        assert!(out.contains("NO \"unknown command\""));
        assert!(out.contains("OK \"Bye\""));
    }
}
