use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

use irixmail_core::{Error, Result};
use irixmail_directory::{attempt_login_blocking, Account, Directory, LoginAttempt, LoginPurpose};
use irixmail_mail::message_text;
use irixmail_mail::provision::provision_mailboxes;
use irixmail_mail::subscriptions;
use irixmail_mail::SpecialUse;
use irixmail_mail::{
    allocate_document_id, append_message, assign_uid_validity, create_mailbox, delete_mailbox,
    delete_message, load_mailboxes, load_metadata, rename_mailbox, update_message, update_messages,
    AppendRequest, Keyword, Mailbox, MessageCacheEntry, MessageStoreCache, FIRST_USER_MAILBOX_ID,
};
use irixmail_store::{
    tokenize as fts_tokenize, BlobStore, ChangeLog, ChangeNotifier, Collection, Field, FtsIndex,
    Query, Store,
};

use std::collections::HashMap;

use crate::bodystructure::build_bodystructure;
use crate::cmd_append::{
    append_bad, parse_append, parse_continuation, too_big, try_create, AppendError, AppendGroup,
    Continuation, CONTINUE,
};
use crate::cmd_authenticate::{Mechanism, SaslExchange, SaslStart, SaslStep};
use crate::cmd_capability::{capability_codes, capability_line, CapabilityContext};
use crate::cmd_check::check_completion;
use crate::cmd_close::close_completion;
use crate::cmd_enable::{enabled_line, parse_enable};
use crate::cmd_examine::{examine_completion, examine_responses};
use crate::cmd_fetch::{
    compress_sequence, fetch_items, fetch_line, is_body_item, is_seen_setting_item,
    is_structure_item, parse_sequence_set, sequence_contains, split_fetch_modifiers, BodyData,
    FetchExtras, FetchMods, SeqPoint, SeqRange,
};
use crate::cmd_idle::{
    idle_completion, idle_done, CONTINUE as IDLE_CONTINUE, IDLE_TIMED_OUT, IDLE_TIMEOUT,
};
use crate::cmd_list::{
    childinfo_line, display_name, extended_line, parse_list, pattern_matched, ListCommand,
    ListParse, DELIMITER,
};
use crate::cmd_login::{
    parse_login, LoginCredentials, AUTH_FAILED, COMPLETED, PRIVACY_REQUIRED, THROTTLED,
};
use crate::cmd_logout::{logout_response, BYE};
use crate::cmd_lsub::lsub_responses;
use crate::cmd_namespace::{namespace_completion, NAMESPACE_LINE};
use crate::cmd_noop::noop_response;
use crate::cmd_quota::{quota_line, quotaroot_line, QuotaLimitsView};
use crate::cmd_search::{
    esearch_response, parse_search, search_response, split_return_options, SearchError, SearchKey,
    SearchReturn, SUPPORTED_CHARSETS,
};
use crate::cmd_select::{
    parse_select_params, select_completion, select_responses, QresyncParam, SelectView,
};
use crate::cmd_sort::{
    base_subject, header_value, parse_sort_keys, sort_response, SortKey, SortSpec,
};
use crate::cmd_starttls::starttls_reply;
use crate::cmd_status::{requested_items, status_line, StatusValues};
use crate::cmd_store::{parse_store, parse_unchanged_since, StoreMode, StoreOp};
use crate::cmd_subscribe::{subscribe_response, SubscribeOutcome};
use crate::cmd_thread::{parse_algorithm, thread_response, ThreadAlgorithm};
use crate::cmd_uid::{uid_subcommand, UidCommand};
use crate::cmd_unsubscribe::{unsubscribe_response, UnsubscribeOutcome};
use crate::envelope::build_envelope;
use crate::parser::{parse_command, tokenize_args, Command as ParsedCommand, ParseError, Token};

const MAX_LINE_LENGTH: usize = 8192;
const MAX_APPEND_SIZE: usize = 25 * 1024 * 1024;

enum CommandRead {
    Ready(ParsedCommand),
    Rejected,
    Closed,
}

enum AppendResult {
    Stored(u32),
    OverQuota,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Capability,
    Noop,
    Logout,
    StartTls,
    Login,
    Authenticate,
    Select,
    Examine,
    Create,
    Delete,
    Rename,
    Subscribe,
    Unsubscribe,
    List,
    Lsub,
    Namespace,
    Status,
    Append,
    Check,
    Close,
    Expunge,
    Search,
    Fetch,
    Store,
    Copy,
    Move,
    Uid,
    Idle,
    Id,
    Unselect,
    Enable,
    Sort,
    Thread,
    GetQuota,
    GetQuotaRoot,
    Unknown,
}

impl Command {
    fn from_word(word: &[u8]) -> Self {
        let mut upper = [0u8; 12];
        if word.is_empty() || word.len() > upper.len() {
            return Command::Unknown;
        }
        for (slot, byte) in upper.iter_mut().zip(word) {
            *slot = byte.to_ascii_uppercase();
        }
        match &upper[..word.len()] {
            b"CAPABILITY" => Command::Capability,
            b"NOOP" => Command::Noop,
            b"LOGOUT" => Command::Logout,
            b"STARTTLS" => Command::StartTls,
            b"LOGIN" => Command::Login,
            b"AUTHENTICATE" => Command::Authenticate,
            b"SELECT" => Command::Select,
            b"EXAMINE" => Command::Examine,
            b"CREATE" => Command::Create,
            b"DELETE" => Command::Delete,
            b"RENAME" => Command::Rename,
            b"SUBSCRIBE" => Command::Subscribe,
            b"UNSUBSCRIBE" => Command::Unsubscribe,
            b"LIST" => Command::List,
            b"LSUB" => Command::Lsub,
            b"NAMESPACE" => Command::Namespace,
            b"STATUS" => Command::Status,
            b"APPEND" => Command::Append,
            b"CHECK" => Command::Check,
            b"CLOSE" => Command::Close,
            b"EXPUNGE" => Command::Expunge,
            b"SEARCH" => Command::Search,
            b"FETCH" => Command::Fetch,
            b"STORE" => Command::Store,
            b"COPY" => Command::Copy,
            b"MOVE" => Command::Move,
            b"UID" => Command::Uid,
            b"IDLE" => Command::Idle,
            b"ID" => Command::Id,
            b"UNSELECT" => Command::Unselect,
            b"ENABLE" => Command::Enable,
            b"SORT" => Command::Sort,
            b"THREAD" => Command::Thread,
            b"GETQUOTA" => Command::GetQuota,
            b"GETQUOTAROOT" => Command::GetQuotaRoot,
            _ => Command::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    NotAuthenticated,
    Authenticated,
    Selected,
}

#[derive(Default)]
pub struct SessionData {
    pub user: Option<String>,
    pub mailbox: Option<String>,
    pub is_tls: bool,
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
    last_exists: Option<u32>,
    read_only: bool,
    condstore: bool,
    qresync: bool,
    saved_search: Option<Vec<u32>>,
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
            data: SessionData::default(),
            greet: true,
            directory: None,
            blobs: None,
            notifier: None,
            account: None,
            last_exists: None,
            read_only: false,
            condstore: false,
            qresync: false,
            saved_search: None,
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

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn data(&self) -> &SessionData {
        &self.data
    }

    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    pub fn into_inner(self) -> S {
        self.stream.into_inner()
    }

    pub async fn run(&mut self) -> Result<Flow> {
        if self.greet {
            tracing::info!(
                target: "irixmail::imap",
                sid = self.sid,
                peer = %self.peer,
                tls = self.data.is_tls,
                "connection accepted"
            );
            let codes = capability_codes(&CapabilityContext {
                is_tls: self.data.is_tls,
                authenticated: false,
            });
            let greeting = format!("* OK [CAPABILITY {codes}] IRIXMAIL IMAP4rev1 ready\r\n");
            self.write(greeting.as_bytes()).await?;
        } else {
            tracing::info!(
                target: "irixmail::imap",
                sid = self.sid,
                peer = %self.peer,
                "starttls upgraded"
            );
        }

        let mut line = Vec::with_capacity(MAX_LINE_LENGTH);
        loop {
            line.clear();
            if !self.read_line(&mut line).await? {
                return Ok(Flow::Close);
            }
            match parse_command(&line) {
                Ok(command) => match self.resolve_literals(command).await? {
                    CommandRead::Ready(command) => match self.dispatch(command).await? {
                        Some(flow) => return Ok(flow),
                        None => continue,
                    },
                    CommandRead::Rejected => continue,
                    CommandRead::Closed => return Ok(Flow::Close),
                },
                Err(error) => self.reject(&line, error).await?,
            }
        }
    }

    async fn resolve_literals(&mut self, mut command: ParsedCommand) -> Result<CommandRead> {
        if command.name.eq_ignore_ascii_case("APPEND") {
            return Ok(CommandRead::Ready(command));
        }
        let mut resolved: Vec<Token> = Vec::new();
        loop {
            match command.args.last() {
                Some(Token::Literal { length, sync }) => {
                    let length = *length as usize;
                    let sync = *sync;
                    command.args.pop();
                    resolved.append(&mut command.args);
                    if length > MAX_LINE_LENGTH {
                        self.reply(&command.tag, "BAD", "literal too long").await?;
                        return Ok(CommandRead::Closed);
                    }
                    if sync {
                        self.write(CONTINUE.as_bytes()).await?;
                    }
                    let octets = self.read_literal(length).await?;
                    resolved.push(Token::LiteralValue(octets));
                    let mut continuation = Vec::new();
                    if !self.read_line(&mut continuation).await? {
                        return Ok(CommandRead::Closed);
                    }
                    match tokenize_args(&continuation) {
                        Ok(tokens) => command.args = tokens,
                        Err(error) => {
                            self.reject(&continuation, error).await?;
                            return Ok(CommandRead::Rejected);
                        }
                    }
                }
                _ => {
                    resolved.append(&mut command.args);
                    command.args = resolved;
                    return Ok(CommandRead::Ready(command));
                }
            }
        }
    }

    async fn dispatch(&mut self, parsed: ParsedCommand) -> Result<Option<Flow>> {
        let tag = parsed.tag.as_str();
        match Command::from_word(parsed.name.as_bytes()) {
            Command::Capability => {
                let ctx = CapabilityContext {
                    is_tls: self.data.is_tls,
                    authenticated: self.state != State::NotAuthenticated,
                };
                self.write(&capability_line(&ctx)).await?;
                self.reply(tag, "OK", "CAPABILITY completed").await?;
                Ok(None)
            }
            Command::Noop => {
                self.push_new_mail().await?;
                self.write(noop_response(tag).as_bytes()).await?;
                Ok(None)
            }
            Command::Logout => {
                self.write(BYE.as_bytes()).await?;
                self.write(logout_response(tag).as_bytes()).await?;
                Ok(Some(Flow::Close))
            }
            Command::StartTls => {
                if !self.data.is_tls && self.state != State::NotAuthenticated {
                    self.reply(tag, "BAD", "STARTTLS not allowed now").await?;
                    return Ok(None);
                }
                let reply = starttls_reply(self.data.is_tls, true);
                self.reply(tag, reply.status(), reply.text()).await?;
                Ok(reply.upgrades().then_some(Flow::Upgrade))
            }
            Command::Login => self.run_login(&parsed).await,
            Command::Authenticate => self.run_authenticate(&parsed).await,
            verb @ (Command::Select | Command::Examine) => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                    return Ok(None);
                }
                let name = parsed.args.first().and_then(Token::as_str).unwrap_or("");
                let Some(mailbox) = self.mailbox_named(name) else {
                    self.reply(tag, "NO", "mailbox does not exist").await?;
                    return Ok(None);
                };
                let params = parse_select_params(parsed.args.get(1));
                if params.qresync.is_some() && !self.qresync {
                    self.reply(tag, "BAD", "QRESYNC has not been enabled")
                        .await?;
                    return Ok(None);
                }
                if params.condstore {
                    self.condstore = true;
                }
                let view = self.select_view(&mailbox)?;
                let (lines, completion) = if verb == Command::Examine {
                    (examine_responses(&view), examine_completion(tag))
                } else {
                    (
                        select_responses(&view, false),
                        select_completion(tag, "SELECT", false),
                    )
                };
                for line in lines {
                    self.write(line.as_bytes()).await?;
                }
                if let Some(qresync) = &params.qresync {
                    for line in self.qresync_replay(&mailbox, qresync)? {
                        self.write(&line).await?;
                    }
                }
                tracing::info!(
                    target: "irixmail::imap",
                    sid = self.sid,
                    mailbox = %display_name(&mailbox),
                    messages = view.exists,
                    "mailbox selected"
                );
                self.saved_search = None;
                self.data.mailbox = Some(display_name(&mailbox).to_string());
                self.state = State::Selected;
                self.read_only = verb == Command::Examine;
                self.last_exists = Some(view.exists);
                self.write(completion.as_bytes()).await?;
                Ok(None)
            }
            Command::Enable => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    let enabled = parse_enable(&parsed.args);
                    self.condstore |= enabled.condstore;
                    self.qresync |= enabled.qresync;
                    self.write(enabled_line(&parsed.args).as_bytes()).await?;
                    self.reply(tag, "OK", "ENABLE completed").await?;
                }
                Ok(None)
            }
            Command::List => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    match parse_list(&parsed.args) {
                        ListParse::Bad(reason) => self.reply(tag, "BAD", reason).await?,
                        ListParse::Command(command) => self.run_list(tag, &command).await?,
                    }
                }
                Ok(None)
            }
            Command::Lsub => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    let reference = parsed.args.first().and_then(Token::as_str).unwrap_or("");
                    let pattern = parsed.args.get(1).and_then(Token::as_str).unwrap_or("");
                    match self.subscribed_mailboxes() {
                        Ok(mailboxes) => {
                            for line in lsub_responses(&mailboxes, reference, pattern) {
                                self.write(line.as_bytes()).await?;
                            }
                            self.reply(tag, "OK", "LSUB completed").await?;
                        }
                        Err(error) => {
                            tracing::warn!(
                                target: "irixmail::imap",
                                sid = self.sid,
                                error = %error,
                                "LSUB failed"
                            );
                            self.reply(tag, "NO", "LSUB failed").await?;
                        }
                    }
                }
                Ok(None)
            }
            Command::Subscribe => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    let name = parsed.args.first().and_then(Token::as_str).unwrap_or("");
                    let outcome = match (self.mailbox_named(name), self.message_context()) {
                        (None, _) => SubscribeOutcome::Missing,
                        (Some(_), None) => SubscribeOutcome::Failed,
                        (Some(mailbox), Some((store, account_id))) => {
                            match subscriptions::subscribe(
                                store.as_ref(),
                                account_id,
                                display_name(&mailbox),
                            ) {
                                Ok(true) => SubscribeOutcome::Subscribed,
                                Ok(false) => SubscribeOutcome::AlreadySubscribed,
                                Err(_) => SubscribeOutcome::Failed,
                            }
                        }
                    };
                    self.write(subscribe_response(tag, outcome).as_bytes())
                        .await?;
                }
                Ok(None)
            }
            Command::Unsubscribe => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    let name = parsed.args.first().and_then(Token::as_str).unwrap_or("");
                    let outcome = match self.message_context() {
                        None => UnsubscribeOutcome::Failed,
                        Some((store, account_id)) => {
                            match subscriptions::unsubscribe(store.as_ref(), account_id, name) {
                                Ok(true) => UnsubscribeOutcome::Unsubscribed,
                                Ok(false) => UnsubscribeOutcome::NotSubscribed,
                                Err(_) => UnsubscribeOutcome::Failed,
                            }
                        }
                    };
                    self.write(unsubscribe_response(tag, outcome).as_bytes())
                        .await?;
                }
                Ok(None)
            }
            Command::Namespace => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    self.write(NAMESPACE_LINE.as_bytes()).await?;
                    self.write(namespace_completion(tag).as_bytes()).await?;
                }
                Ok(None)
            }
            Command::Status => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    let name = parsed.args.first().and_then(Token::as_str).unwrap_or("");
                    match self.mailbox_named(name) {
                        Some(mailbox) => {
                            let requested = requested_items(parsed.args.get(1));
                            let values = self.status_values(&mailbox)?;
                            let line = status_line(display_name(&mailbox), &requested, &values);
                            self.write(line.as_bytes()).await?;
                            self.reply(tag, "OK", "STATUS completed").await?;
                        }
                        None => {
                            self.reply(tag, "NO", "mailbox does not exist").await?;
                        }
                    }
                }
                Ok(None)
            }
            Command::Create => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    let name = parsed
                        .args
                        .first()
                        .and_then(Token::as_str)
                        .unwrap_or("")
                        .to_string();
                    self.run_create(tag, &name).await?;
                }
                Ok(None)
            }
            Command::Delete => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    let name = parsed
                        .args
                        .first()
                        .and_then(Token::as_str)
                        .unwrap_or("")
                        .to_string();
                    self.run_delete(tag, &name).await?;
                }
                Ok(None)
            }
            Command::Rename => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                } else {
                    let source = parsed
                        .args
                        .first()
                        .and_then(Token::as_str)
                        .unwrap_or("")
                        .to_string();
                    let target = parsed
                        .args
                        .get(1)
                        .and_then(Token::as_str)
                        .unwrap_or("")
                        .to_string();
                    self.run_rename(tag, &source, &target).await?;
                }
                Ok(None)
            }
            Command::Append => self.run_append(&parsed).await,
            Command::Fetch => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else {
                    self.run_fetch(tag, &parsed.args, false).await?;
                }
                Ok(None)
            }
            Command::Search => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else {
                    self.run_search(tag, &parsed.args, false).await?;
                }
                Ok(None)
            }
            Command::Sort => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else {
                    self.run_sort(tag, &parsed.args, false).await?;
                }
                Ok(None)
            }
            Command::Thread => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else {
                    self.run_thread(tag, &parsed.args, false).await?;
                }
                Ok(None)
            }
            Command::Store => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else if self.read_only {
                    self.refuse_read_only(tag).await?;
                } else {
                    let set =
                        self.resolve_set(parsed.args.first().and_then(Token::as_str), false)?;
                    let unchanged_since = parse_unchanged_since(parsed.args.get(1));
                    let shift = usize::from(unchanged_since.is_some());
                    let op = parse_store(
                        parsed.args.get(1 + shift).and_then(Token::as_str),
                        parsed.args.get(2 + shift),
                    );
                    match (set, op) {
                        (Some(ranges), Some(op)) => {
                            self.run_store(tag, "STORE", &ranges, &op, false, unchanged_since)
                                .await?;
                        }
                        _ => {
                            self.reply(tag, "BAD", "STORE expects a sequence set, item and flags")
                                .await?;
                        }
                    }
                }
                Ok(None)
            }
            Command::Copy => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else {
                    self.run_copy(tag, &parsed.args, false, false).await?;
                }
                Ok(None)
            }
            Command::Move => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else if self.read_only {
                    self.refuse_read_only(tag).await?;
                } else {
                    self.run_copy(tag, &parsed.args, false, true).await?;
                }
                Ok(None)
            }
            Command::Expunge => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else if self.read_only {
                    self.refuse_read_only(tag).await?;
                } else {
                    let lines = self.apply_expunge(None)?;
                    tracing::info!(
                        target: "irixmail::imap",
                        sid = self.sid,
                        expunged = lines.len(),
                        "messages expunged"
                    );
                    for line in lines {
                        self.write(&line).await?;
                    }
                    self.reply(tag, "OK", "EXPUNGE completed").await?;
                }
                Ok(None)
            }
            Command::Close => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else {
                    if !self.read_only {
                        // RFC 3501 §6.4.2: expunge without untagged EXPUNGE responses.
                        self.apply_expunge(None)?;
                    }
                    self.saved_search = None;
                    self.data.mailbox = None;
                    self.state = State::Authenticated;
                    self.write(close_completion(tag).as_bytes()).await?;
                }
                Ok(None)
            }
            Command::Check => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else {
                    self.push_new_mail().await?;
                    self.write(check_completion(tag).as_bytes()).await?;
                }
                Ok(None)
            }
            Command::Uid => self.run_uid(&parsed).await,
            Command::Idle => self.run_idle(tag).await,
            Command::Id => {
                let line = format!(
                    "* ID (\"name\" \"IRIXMAIL\" \"version\" \"{}\")\r\n",
                    env!("CARGO_PKG_VERSION")
                );
                self.write(line.as_bytes()).await?;
                self.reply(tag, "OK", "ID completed").await?;
                Ok(None)
            }
            Command::Unselect => {
                if self.state != State::Selected {
                    self.reply(tag, "NO", "Select a mailbox first").await?;
                } else {
                    self.saved_search = None;
                    self.data.mailbox = None;
                    self.state = State::Authenticated;
                    self.reply(tag, "OK", "UNSELECT completed").await?;
                }
                Ok(None)
            }
            Command::GetQuota => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                    return Ok(None);
                }
                let root = parsed.args.first().and_then(Token::as_str).unwrap_or("-");
                if !root.is_empty() {
                    self.reply(tag, "NO", "no such quota root").await?;
                    return Ok(None);
                }
                let view = self.quota_view()?;
                self.write(quota_line(&view).as_bytes()).await?;
                self.reply(tag, "OK", "GETQUOTA completed").await?;
                Ok(None)
            }
            Command::GetQuotaRoot => {
                if self.state == State::NotAuthenticated {
                    self.reply(tag, "NO", "Authenticate first").await?;
                    return Ok(None);
                }
                let name = parsed.args.first().and_then(Token::as_str).unwrap_or("");
                let Some(mailbox) = self.mailbox_named(name) else {
                    self.reply(tag, "NO", "mailbox does not exist").await?;
                    return Ok(None);
                };
                let view = self.quota_view()?;
                self.write(quotaroot_line(display_name(&mailbox)).as_bytes())
                    .await?;
                self.write(quota_line(&view).as_bytes()).await?;
                self.reply(tag, "OK", "GETQUOTAROOT completed").await?;
                Ok(None)
            }
            Command::Unknown => {
                self.reply(tag, "BAD", "Command unrecognized").await?;
                Ok(None)
            }
        }
    }

    async fn run_login(&mut self, parsed: &ParsedCommand) -> Result<Option<Flow>> {
        let tag = parsed.tag.as_str();
        if !self.data.is_tls {
            self.reply(tag, "NO", PRIVACY_REQUIRED).await?;
            return Ok(None);
        }
        if self.state != State::NotAuthenticated {
            self.reply(tag, "NO", "Already authenticated").await?;
            return Ok(None);
        }
        let Ok(credentials) = parse_login(&parsed.args) else {
            self.reply(tag, "BAD", "LOGIN expects a username and password")
                .await?;
            return Ok(None);
        };
        let attempt = self.resolve_login(&credentials).await?;
        self.log_login(&credentials.username, &attempt);
        match attempt {
            LoginAttempt::Granted(account, _) => {
                self.data.user = Some(credentials.username);
                self.account = Some(*account);
                self.state = State::Authenticated;
                let text = format!("[CAPABILITY {}] {COMPLETED}", self.session_codes());
                self.reply(tag, "OK", &text).await?;
            }
            LoginAttempt::Denied => {
                self.reply(tag, "NO", AUTH_FAILED).await?;
            }
            LoginAttempt::Throttled => {
                self.reply(tag, "NO", THROTTLED).await?;
            }
        }
        Ok(None)
    }

    fn session_codes(&self) -> String {
        capability_codes(&CapabilityContext {
            is_tls: self.data.is_tls,
            authenticated: self.state != State::NotAuthenticated,
        })
    }

    fn log_login(&self, user: &str, attempt: &LoginAttempt) {
        let outcome = match attempt {
            LoginAttempt::Granted(..) => "login succeeded",
            LoginAttempt::Denied => "login refused",
            LoginAttempt::Throttled => "login throttled",
        };
        tracing::info!(
            target: "irixmail::imap",
            sid = self.sid,
            user = %user,
            "{outcome}"
        );
    }

    async fn run_authenticate(&mut self, parsed: &ParsedCommand) -> Result<Option<Flow>> {
        let tag = parsed.tag.clone();
        let mechanism = parsed
            .args
            .first()
            .and_then(Token::as_str)
            .map(Mechanism::parse)
            .unwrap_or(Mechanism::Unsupported);
        let initial = parsed.args.get(1).and_then(Token::as_str);
        let authenticated = self.state != State::NotAuthenticated;

        let (mut exchange, mut step) =
            match SaslExchange::begin(mechanism, self.data.is_tls, authenticated, initial) {
                SaslStart::Reply { status, text } => {
                    self.reply(&tag, status, text).await?;
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
                        self.reply(&tag, "BAD", "authentication cancelled").await?;
                        return Ok(None);
                    };
                    step = exchange.advance(&response);
                }
                SaslStep::Failed { status, text } => {
                    self.reply(&tag, status, text).await?;
                    return Ok(None);
                }
                SaslStep::Resolved(credentials) => {
                    let attempt = self.resolve_login(&credentials).await?;
                    self.log_login(&credentials.username, &attempt);
                    match attempt {
                        LoginAttempt::Granted(account, _) => {
                            self.data.user = Some(credentials.username);
                            self.account = Some(*account);
                            self.state = State::Authenticated;
                            let text = format!(
                                "[CAPABILITY {}] AUTHENTICATE completed",
                                self.session_codes()
                            );
                            self.reply(&tag, "OK", &text).await?;
                        }
                        LoginAttempt::Denied => {
                            self.reply(&tag, "NO", AUTH_FAILED).await?;
                        }
                        LoginAttempt::Throttled => {
                            self.reply(&tag, "NO", THROTTLED).await?;
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
            return Err(Error::protocol("connection closed during AUTHENTICATE"));
        }
        let response = String::from_utf8_lossy(strip_crlf(&line))
            .trim()
            .to_string();
        if response == "*" {
            return Ok(None);
        }
        Ok(Some(response))
    }

    async fn run_append(&mut self, parsed: &ParsedCommand) -> Result<Option<Flow>> {
        let tag = parsed.tag.clone();
        if self.state == State::NotAuthenticated {
            self.reply(&tag, "NO", "Authenticate first").await?;
            return Ok(None);
        }
        let command = match parse_append(&parsed.args) {
            Ok(command) => command,
            Err(AppendError::BadDate { literal_len, sync }) => {
                // a non-synchronizing literal is already in flight; drain it
                if !sync {
                    if literal_len as usize > MAX_APPEND_SIZE {
                        self.reply(&tag, "BAD", "cannot parse the APPEND date-time")
                            .await?;
                        return Ok(Some(Flow::Close));
                    }
                    self.read_literal(literal_len as usize).await?;
                    let mut trailing = Vec::new();
                    self.read_line(&mut trailing).await?;
                }
                self.reply(&tag, "BAD", "cannot parse the APPEND date-time")
                    .await?;
                return Ok(None);
            }
            Err(_) => {
                self.write(append_bad(&tag).as_bytes()).await?;
                return Ok(None);
            }
        };
        if command.literal_len as usize > MAX_APPEND_SIZE {
            self.write(too_big(&tag).as_bytes()).await?;
            return if command.sync {
                Ok(None)
            } else {
                Ok(Some(Flow::Close))
            };
        }
        let Some(mailbox) = self.mailbox_named(&command.mailbox) else {
            tracing::info!(
                target: "irixmail::imap",
                sid = self.sid,
                mailbox = %command.mailbox,
                reason = "no such mailbox",
                "append refused"
            );
            // a non-synchronizing literal is already in flight; drain it
            if !command.sync {
                self.read_literal(command.literal_len as usize).await?;
                let mut trailing = Vec::new();
                self.read_line(&mut trailing).await?;
            }
            self.write(try_create(&tag).as_bytes()).await?;
            return Ok(None);
        };
        let mut group = AppendGroup {
            flags: command.flags,
            internaldate: command.internaldate,
            literal_len: command.literal_len,
            sync: command.sync,
        };
        let mut uids: Vec<u32> = Vec::new();
        loop {
            if group.literal_len as usize > MAX_APPEND_SIZE {
                self.write(too_big(&tag).as_bytes()).await?;
                return if group.sync {
                    Ok(None)
                } else {
                    Ok(Some(Flow::Close))
                };
            }
            if group.sync {
                self.write(CONTINUE.as_bytes()).await?;
            }
            let message = self.read_literal(group.literal_len as usize).await?;
            match self
                .append_one(&mailbox, &group.flags, group.internaldate, &message)
                .await?
            {
                AppendResult::Stored(uid) => uids.push(uid),
                AppendResult::OverQuota => {
                    let mut trailing = Vec::new();
                    self.read_line(&mut trailing).await?;
                    self.drain_appends(&trailing).await?;
                    self.reply(&tag, "NO", "[OVERQUOTA] not enough storage")
                        .await?;
                    return Ok(None);
                }
                AppendResult::Failed => {
                    let mut trailing = Vec::new();
                    self.read_line(&mut trailing).await?;
                    self.drain_appends(&trailing).await?;
                    self.reply(&tag, "NO", "APPEND failed").await?;
                    return Ok(None);
                }
            }
            let mut trailing = Vec::new();
            self.read_line(&mut trailing).await?;
            let tokens = tokenize_args(&trailing).unwrap_or_default();
            match parse_continuation(&tokens) {
                Continuation::End => break,
                Continuation::Group(next) => group = next,
                Continuation::Bad { literal_len, sync } => {
                    if !sync {
                        if literal_len as usize > MAX_APPEND_SIZE {
                            self.reply(&tag, "BAD", "cannot parse the APPEND date-time")
                                .await?;
                            return Ok(Some(Flow::Close));
                        }
                        self.read_literal(literal_len as usize).await?;
                        let mut rest = Vec::new();
                        self.read_line(&mut rest).await?;
                        self.drain_appends(&rest).await?;
                    }
                    self.reply(&tag, "BAD", "cannot parse the APPEND date-time")
                        .await?;
                    return Ok(None);
                }
            }
        }
        let code = format!(
            "[APPENDUID {} {}] APPEND completed",
            mailbox.uid_validity,
            compress_sequence(&uids)
        );
        self.reply(&tag, "OK", &code).await?;
        Ok(None)
    }

    // Consume any further in-flight non-sync literal groups so their bytes never run as commands.
    async fn drain_appends(&mut self, first_line: &[u8]) -> Result<()> {
        let mut line = first_line.to_vec();
        loop {
            let tokens = match tokenize_args(&line) {
                Ok(tokens) => tokens,
                Err(_) => return Ok(()),
            };
            let Some(Token::Literal { length, sync }) = tokens.last() else {
                return Ok(());
            };
            if *sync || *length as usize > MAX_APPEND_SIZE {
                return Ok(());
            }
            self.read_literal(*length as usize).await?;
            line.clear();
            if !self.read_line(&mut line).await? {
                return Ok(());
            }
        }
    }

    async fn append_one(
        &mut self,
        mailbox: &Mailbox,
        flags: &[String],
        internaldate: Option<u64>,
        message: &[u8],
    ) -> Result<AppendResult> {
        let context = self.message_context().and_then(|(store, account_id)| {
            match (
                self.notifier.clone(),
                self.blobs.clone(),
                self.account.clone(),
            ) {
                (Some(notifier), Some(blobs), Some(account)) => {
                    Some((store, account_id, notifier, blobs, account))
                }
                _ => None,
            }
        });
        let Some((store, account_id, notifier, blobs, account)) = context else {
            tracing::warn!(
                target: "irixmail::imap",
                sid = self.sid,
                mailbox = %display_name(mailbox),
                "append failed: session storage context missing"
            );
            return Ok(AppendResult::Failed);
        };
        let keywords: Vec<Keyword> = flags.iter().map(|flag| Keyword::from_imap(flag)).collect();
        let document_id = allocate_document_id(store.as_ref(), account_id)?;
        let request = AppendRequest {
            account: &account,
            mailbox,
            flags: keywords,
            received_at: internaldate.unwrap_or_else(now_seconds),
            document_id,
            raw: message,
        };
        let outcome = append_message(store.as_ref(), blobs.as_ref(), notifier.as_ref(), &request)?;
        if outcome.over_quota {
            tracing::info!(
                target: "irixmail::imap",
                sid = self.sid,
                mailbox = %display_name(mailbox),
                reason = "over quota",
                "append refused"
            );
            Ok(AppendResult::OverQuota)
        } else {
            tracing::info!(
                target: "irixmail::imap",
                sid = self.sid,
                mailbox = %display_name(mailbox),
                uid = outcome.uid,
                bytes = message.len(),
                "message appended"
            );
            Ok(AppendResult::Stored(outcome.uid))
        }
    }

    async fn run_idle(&mut self, tag: &str) -> Result<Option<Flow>> {
        if self.state == State::NotAuthenticated {
            self.reply(tag, "NO", "Authenticate first").await?;
            return Ok(None);
        }
        // Subscribe before the continuation so a delivery racing the client cannot be missed.
        let mut updates = match (&self.notifier, &self.account) {
            (Some(notifier), Some(account)) => Some(notifier.subscribe(account.id as u32)),
            _ => None,
        };
        self.write(IDLE_CONTINUE.as_bytes()).await?;

        enum IdleEvent {
            Read(std::result::Result<Result<bool>, tokio::time::error::Elapsed>),
            Changed(Option<irixmail_store::ChangeNotice>),
            Unsubscribed,
        }

        let mut line = Vec::with_capacity(MAX_LINE_LENGTH);
        loop {
            let event = match updates.as_mut() {
                Some(subscription) => tokio::select! {
                    read = tokio::time::timeout(IDLE_TIMEOUT, self.read_line(&mut line)) => {
                        IdleEvent::Read(read)
                    }
                    notice = subscription.recv() => match notice {
                        Ok(notice) => IdleEvent::Changed(Some(notice)),
                        Err(broadcast::error::RecvError::Lagged(_)) => IdleEvent::Changed(None),
                        Err(broadcast::error::RecvError::Closed) => IdleEvent::Unsubscribed,
                    },
                },
                None => IdleEvent::Read(
                    tokio::time::timeout(IDLE_TIMEOUT, self.read_line(&mut line)).await,
                ),
            };
            match event {
                IdleEvent::Read(Ok(Ok(true))) => {
                    if idle_done(&line) {
                        break;
                    }
                    line.clear();
                }
                IdleEvent::Read(Ok(Ok(false))) => return Ok(Some(Flow::Close)),
                IdleEvent::Read(Ok(Err(err))) => return Err(err),
                IdleEvent::Read(Err(_)) => {
                    self.write(IDLE_TIMED_OUT.as_bytes()).await?;
                    return Ok(Some(Flow::Close));
                }
                IdleEvent::Changed(Some(notice)) => {
                    if notice.collection == Collection::Email {
                        self.push_new_mail().await?;
                    }
                }
                IdleEvent::Changed(None) => self.push_new_mail().await?,
                IdleEvent::Unsubscribed => updates = None,
            }
        }
        self.write(idle_completion(tag).as_bytes()).await?;
        Ok(None)
    }

    async fn push_new_mail(&mut self) -> Result<()> {
        if self.state != State::Selected {
            return Ok(());
        }
        let Some(name) = self.data.mailbox.clone() else {
            return Ok(());
        };
        let Some(mailbox) = self.mailbox_named(&name) else {
            return Ok(());
        };
        let view = self.select_view(&mailbox)?;
        if self.last_exists.is_none_or(|last| view.exists > last) {
            self.write(format!("* {} EXISTS\r\n", view.exists).as_bytes())
                .await?;
            self.write(format!("* {} RECENT\r\n", view.recent).as_bytes())
                .await?;
            self.last_exists = Some(view.exists);
        }
        Ok(())
    }

    async fn run_uid(&mut self, parsed: &ParsedCommand) -> Result<Option<Flow>> {
        let tag = parsed.tag.as_str();
        if self.state != State::Selected {
            self.reply(tag, "NO", "Select a mailbox first").await?;
            return Ok(None);
        }
        let (subcommand, rest) = uid_subcommand(&parsed.args);
        match subcommand {
            UidCommand::Fetch => {
                self.run_fetch(tag, rest, true).await?;
            }
            UidCommand::Search => {
                self.run_search(tag, rest, true).await?;
            }
            UidCommand::Sort => {
                self.run_sort(tag, rest, true).await?;
            }
            UidCommand::Thread => {
                self.run_thread(tag, rest, true).await?;
            }
            UidCommand::Store => {
                if self.read_only {
                    self.refuse_read_only(tag).await?;
                    return Ok(None);
                }
                let set = self.resolve_set(rest.first().and_then(Token::as_str), true)?;
                let unchanged_since = parse_unchanged_since(rest.get(1));
                let shift = usize::from(unchanged_since.is_some());
                let op = parse_store(
                    rest.get(1 + shift).and_then(Token::as_str),
                    rest.get(2 + shift),
                );
                match (set, op) {
                    (Some(ranges), Some(op)) => {
                        self.run_store(tag, "UID STORE", &ranges, &op, true, unchanged_since)
                            .await?;
                    }
                    _ => {
                        self.reply(
                            tag,
                            "BAD",
                            "UID STORE expects a sequence set, item and flags",
                        )
                        .await?;
                    }
                }
            }
            UidCommand::Copy => {
                self.run_copy(tag, rest, true, false).await?;
            }
            UidCommand::Move => {
                if self.read_only {
                    self.refuse_read_only(tag).await?;
                    return Ok(None);
                }
                self.run_copy(tag, rest, true, true).await?;
            }
            UidCommand::Expunge => {
                if self.read_only {
                    self.refuse_read_only(tag).await?;
                    return Ok(None);
                }
                let set = self.resolve_set(rest.first().and_then(Token::as_str), true)?;
                match set {
                    Some(ranges) => {
                        for line in self.apply_expunge(Some(&ranges))? {
                            self.write(&line).await?;
                        }
                        self.reply(tag, "OK", "UID EXPUNGE completed").await?;
                    }
                    None => {
                        self.reply(tag, "BAD", "UID EXPUNGE expects a sequence set")
                            .await?;
                    }
                }
            }
            UidCommand::Unknown => {
                self.reply(tag, "BAD", "Unsupported UID subcommand").await?;
            }
        }
        Ok(None)
    }

    async fn read_literal(&mut self, length: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; length];
        self.stream.read_exact(&mut buf).await?;
        Ok(buf)
    }

    async fn resolve_login(&self, credentials: &LoginCredentials) -> Result<LoginAttempt> {
        let Some(directory) = self.directory.as_ref() else {
            return Ok(LoginAttempt::Denied);
        };
        let ip = self.peer.ip().to_canonical().to_string();
        attempt_login_blocking(
            directory,
            Some(&ip),
            &credentials.username,
            &credentials.password,
            LoginPurpose::Mail,
        )
        .await
    }

    fn mailboxes(&self) -> Vec<Mailbox> {
        let created_at = self
            .account
            .as_ref()
            .map(|account| account.created_at)
            .unwrap_or(0);
        let mut mailboxes = provision_mailboxes(created_at);
        if let Some((store, account_id)) = self.message_context() {
            match load_mailboxes(store.as_ref(), account_id) {
                Ok(persisted) => mailboxes.extend(
                    persisted
                        .into_iter()
                        .filter(|mailbox| mailbox.id >= FIRST_USER_MAILBOX_ID),
                ),
                Err(error) => tracing::warn!(
                    target: "irixmail::imap",
                    sid = self.sid,
                    error = %error,
                    "mailbox list load failed"
                ),
            }
        }
        mailboxes
    }

    fn subscribed_mailboxes(&self) -> Result<Vec<Mailbox>> {
        let Some((store, account_id)) = self.message_context() else {
            return Ok(self.mailboxes());
        };
        let subscribed: HashSet<String> = subscriptions::subscriptions(store.as_ref(), account_id)?
            .into_iter()
            .collect();
        if subscribed.is_empty() {
            return Ok(self.mailboxes());
        }
        Ok(self
            .mailboxes()
            .into_iter()
            .filter(|mailbox| subscribed.contains(display_name(mailbox)))
            .collect())
    }

    fn message_context(&self) -> Option<(Arc<dyn Store>, u32)> {
        let store = self.directory.as_ref()?.store();
        let account_id = self.account.as_ref()?.id as u32;
        Some((store, account_id))
    }

    fn select_view(&self, mailbox: &Mailbox) -> Result<SelectView> {
        let Some((store, account_id)) = self.message_context() else {
            return Ok(SelectView {
                uidnext: 1,
                uidvalidity: mailbox.uid_validity,
                ..SelectView::default()
            });
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        Ok(SelectView {
            exists: cache.in_mailbox(mailbox.id).count() as u32,
            recent: 0,
            unseen: first_unseen_seqno(&cache, mailbox.id),
            uidnext: mailbox.last_uid(store.as_ref(), account_id)? + 1,
            uidvalidity: mailbox.uid_validity,
            highest_modseq: self.highest_modseq_value(),
        })
    }

    fn status_values(&self, mailbox: &Mailbox) -> Result<StatusValues> {
        let Some((store, account_id)) = self.message_context() else {
            return Ok(StatusValues {
                uidnext: 1,
                uidvalidity: mailbox.uid_validity,
                ..StatusValues::default()
            });
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        Ok(StatusValues {
            messages: cache.in_mailbox(mailbox.id).count() as u32,
            recent: 0,
            uidnext: mailbox.last_uid(store.as_ref(), account_id)? + 1,
            uidvalidity: mailbox.uid_validity,
            unseen: count_unseen(&cache, mailbox.id),
            highest_modseq: self.highest_modseq_value(),
        })
    }

    async fn run_search(&mut self, tag: &str, args: &[Token], uid_mode: bool) -> Result<()> {
        let label = if uid_mode { "UID SEARCH" } else { "SEARCH" };
        let (rest, ret) = split_return_options(args);
        match parse_search(rest) {
            Ok(key) => {
                let outcome = self.search_matches(&key, uid_mode)?;
                match ret {
                    Some(ret) => {
                        if ret.save {
                            self.saved_search = Some(saved_selection(&outcome.uids, &ret));
                        }
                        if ret.wants_untagged() {
                            let line =
                                esearch_response(tag, uid_mode, &outcome.ids, &ret, outcome.modseq);
                            self.write(line.as_bytes()).await?;
                        }
                    }
                    None => {
                        self.write(search_response(&outcome.ids, outcome.modseq).as_bytes())
                            .await?;
                    }
                }
                self.reply(tag, "OK", &format!("{label} completed")).await?;
            }
            Err(SearchError::BadCharset) => {
                self.refuse_charset(tag).await?;
            }
            Err(SearchError::Invalid) => {
                self.reply(tag, "BAD", &format!("unsupported {label} criteria"))
                    .await?;
            }
        }
        Ok(())
    }

    async fn run_sort(&mut self, tag: &str, args: &[Token], uid_mode: bool) -> Result<()> {
        let label = if uid_mode { "UID SORT" } else { "SORT" };
        let Some(specs) = parse_sort_keys(args.first()) else {
            return self
                .reply(tag, "BAD", &format!("{label} expects a sort key list"))
                .await;
        };
        let Some(charset) = args.get(1).and_then(Token::as_str) else {
            return self
                .reply(tag, "BAD", &format!("{label} expects a charset"))
                .await;
        };
        if !charset_supported(charset) {
            return self.refuse_charset(tag).await;
        }
        match parse_search(args.get(2..).unwrap_or(&[])) {
            Ok(key) => {
                let with_headers = specs.iter().any(SortSpec::needs_headers);
                let mut entries = self.sortable_matches(&key, with_headers)?;
                entries.sort_by(|a, b| compare_entries(a, b, &specs));
                let ids: Vec<u32> = entries
                    .iter()
                    .map(|entry| if uid_mode { entry.uid } else { entry.seqno })
                    .collect();
                self.write(sort_response(&ids).as_bytes()).await?;
                self.reply(tag, "OK", &format!("{label} completed")).await
            }
            Err(SearchError::BadCharset) => self.refuse_charset(tag).await,
            Err(SearchError::Invalid) => {
                self.reply(tag, "BAD", &format!("unsupported {label} criteria"))
                    .await
            }
        }
    }

    async fn run_thread(&mut self, tag: &str, args: &[Token], uid_mode: bool) -> Result<()> {
        let label = if uid_mode { "UID THREAD" } else { "THREAD" };
        let Some(algorithm) = args
            .first()
            .and_then(Token::as_str)
            .and_then(parse_algorithm)
        else {
            return self
                .reply(tag, "BAD", &format!("{label} expects an algorithm"))
                .await;
        };
        let Some(charset) = args.get(1).and_then(Token::as_str) else {
            return self
                .reply(tag, "BAD", &format!("{label} expects a charset"))
                .await;
        };
        if !charset_supported(charset) {
            return self.refuse_charset(tag).await;
        }
        match parse_search(args.get(2..).unwrap_or(&[])) {
            Ok(key) => {
                let with_headers = algorithm == ThreadAlgorithm::OrderedSubject;
                let entries = self.sortable_matches(&key, with_headers)?;
                let mut map: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
                for entry in &entries {
                    let group = match algorithm {
                        ThreadAlgorithm::References => entry.thread_id.to_string(),
                        ThreadAlgorithm::OrderedSubject => base_subject(&entry.subject),
                    };
                    let id = if uid_mode { entry.uid } else { entry.seqno };
                    map.entry(group).or_default().push((entry.uid, id));
                }
                let mut groups: Vec<Vec<(u32, u32)>> = map.into_values().collect();
                for group in &mut groups {
                    group.sort_by_key(|(uid, _)| *uid);
                }
                groups.sort_by_key(|group| group.first().map(|(uid, _)| *uid).unwrap_or(0));
                let rendered: Vec<Vec<u32>> = groups
                    .iter()
                    .map(|group| group.iter().map(|(_, id)| *id).collect())
                    .collect();
                self.write(thread_response(&rendered).as_bytes()).await?;
                self.reply(tag, "OK", &format!("{label} completed")).await
            }
            Err(SearchError::BadCharset) => self.refuse_charset(tag).await,
            Err(SearchError::Invalid) => {
                self.reply(tag, "BAD", &format!("unsupported {label} criteria"))
                    .await
            }
        }
    }

    fn sortable_matches(&self, key: &SearchKey, with_headers: bool) -> Result<Vec<SortableEntry>> {
        let Some(name) = self.data.mailbox.clone() else {
            return Ok(Vec::new());
        };
        let Some(mailbox) = self.mailbox_named(&name) else {
            return Ok(Vec::new());
        };
        let Some((store, account_id)) = self.message_context() else {
            return Ok(Vec::new());
        };
        let modseqs = if key.uses_modseq() {
            Some(self.modseq_map(store.as_ref(), account_id)?)
        } else {
            None
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut members: Vec<(u32, &MessageCacheEntry)> = cache
            .in_mailbox(mailbox.id)
            .filter_map(|entry| entry.uid_in(mailbox.id).map(|uid| (uid, entry)))
            .collect();
        members.sort_by_key(|(uid, _)| *uid);
        let universe: Vec<u32> = members.iter().map(|(_, entry)| entry.document_id).collect();
        let saved: &[u32] = self.saved_search.as_deref().unwrap_or(&[]);
        let ctx = SearchCtx {
            total: members.len() as u32,
            uid_max: members.last().map(|(uid, _)| *uid).unwrap_or(0),
            members: &members,
            universe: &universe,
            store: store.as_ref(),
            blobs: self.blobs.as_deref(),
            account_id,
            modseqs: modseqs.as_ref(),
            saved_uids: saved,
        };
        let matched = eval_search(&ctx, key)?;

        let mut entries = Vec::new();
        for (index, (uid, entry)) in members.iter().enumerate() {
            if !matched.contains(&entry.document_id) {
                continue;
            }
            let mut sortable = SortableEntry {
                seqno: index as u32 + 1,
                uid: *uid,
                size: entry.size,
                received_at: entry.received_at,
                sent_at: entry.sent_at,
                thread_id: entry.thread_id,
                subject: String::new(),
                from: String::new(),
                to: String::new(),
                cc: String::new(),
            };
            if with_headers {
                if let Some(metadata) =
                    load_metadata(store.as_ref(), account_id, entry.document_id)?
                {
                    let headers = &metadata.raw_headers;
                    sortable.subject = header_value(headers, "subject").unwrap_or_default();
                    sortable.from = header_value(headers, "from").unwrap_or_default();
                    sortable.to = header_value(headers, "to").unwrap_or_default();
                    sortable.cc = header_value(headers, "cc").unwrap_or_default();
                }
            }
            entries.push(sortable);
        }
        Ok(entries)
    }

    fn resolve_set(&self, raw: Option<&str>, uid_mode: bool) -> Result<Option<Vec<SeqRange>>> {
        let Some(raw) = raw else {
            return Ok(None);
        };
        if raw != "$" {
            return Ok(parse_sequence_set(raw));
        }
        let saved = match &self.saved_search {
            Some(uids) if !uids.is_empty() => uids.clone(),
            _ => return Ok(Some(Vec::new())),
        };
        if uid_mode {
            return Ok(Some(ranges_from(&saved)));
        }
        let Some(name) = self.data.mailbox.clone() else {
            return Ok(Some(Vec::new()));
        };
        let Some(mailbox) = self.mailbox_named(&name) else {
            return Ok(Some(Vec::new()));
        };
        let Some((store, account_id)) = self.message_context() else {
            return Ok(Some(Vec::new()));
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut uids: Vec<u32> = cache
            .in_mailbox(mailbox.id)
            .filter_map(|entry| entry.uid_in(mailbox.id))
            .collect();
        uids.sort_unstable();
        let seqnos: Vec<u32> = uids
            .iter()
            .enumerate()
            .filter(|(_, uid)| saved.contains(uid))
            .map(|(index, _)| index as u32 + 1)
            .collect();
        Ok(Some(ranges_from(&seqnos)))
    }

    async fn run_fetch(&mut self, tag: &str, args: &[Token], uid_mode: bool) -> Result<()> {
        let label = if uid_mode { "UID FETCH" } else { "FETCH" };
        let (item_args, mods) = split_fetch_modifiers(args.get(1..).unwrap_or(&[]));
        let set = self.resolve_set(args.first().and_then(Token::as_str), uid_mode)?;
        let Some(ranges) = set else {
            self.reply(tag, "BAD", &format!("{label} expects a sequence set"))
                .await?;
            return Ok(());
        };
        let mut items = fetch_items(item_args);
        if let Some(mods) = &mods {
            if mods.vanished && (!uid_mode || !self.qresync) {
                self.reply(tag, "BAD", "VANISHED requires QRESYNC and UID FETCH")
                    .await?;
                return Ok(());
            }
            self.condstore = true;
            if !items.iter().any(|item| item == "MODSEQ") {
                items.push("MODSEQ".to_string());
            }
        }
        if items.iter().any(|item| item == "MODSEQ") {
            self.condstore = true;
        }
        for line in self.fetch_lines(&ranges, &items, uid_mode, mods.as_ref())? {
            self.write(&line).await?;
        }
        self.reply(tag, "OK", &format!("{label} completed")).await
    }

    async fn run_list(&mut self, tag: &str, command: &ListCommand) -> Result<()> {
        let mailboxes = self.mailboxes();
        let combined: Vec<String> = command
            .patterns
            .iter()
            .map(|pattern| format!("{}{pattern}", command.reference))
            .collect();
        if combined.iter().all(String::is_empty) {
            self.write(format!("* LIST (\\Noselect) \"{DELIMITER}\" \"\"\r\n").as_bytes())
                .await?;
            self.reply(tag, "OK", "LIST completed").await?;
            return Ok(());
        }
        let want_subscribed = command.subscribed_only || command.ret_subscribed;
        let subscribed: HashSet<String> = if want_subscribed {
            self.subscribed_mailboxes()?
                .iter()
                .map(|mailbox| display_name(mailbox).to_string())
                .collect()
        } else {
            HashSet::new()
        };
        let status_ctx = match &command.ret_status {
            Some(_) => self
                .message_context()
                .map(|(store, account_id)| {
                    MessageStoreCache::build(store.as_ref(), account_id)
                        .map(|cache| (cache, store, account_id))
                })
                .transpose()?,
            None => None,
        };
        let matches_any = |name: &str, role: SpecialUse| {
            combined
                .iter()
                .any(|pattern| pattern_matched(pattern, name, role))
        };
        let mut emitted: HashSet<String> = HashSet::new();
        for mailbox in &mailboxes {
            let name = display_name(mailbox).to_string();
            if command.special_use_only && mailbox.role.attribute().is_none() {
                continue;
            }
            let is_subscribed = subscribed.contains(&name);
            if command.subscribed_only && !is_subscribed {
                continue;
            }
            if !matches_any(&name, mailbox.role) {
                continue;
            }
            self.write(
                extended_line(mailbox, &mailboxes, want_subscribed && is_subscribed).as_bytes(),
            )
            .await?;
            emitted.insert(name.clone());
            if let (Some(items), Some((cache, store, account_id))) =
                (&command.ret_status, &status_ctx)
            {
                let values =
                    self.status_values_from(mailbox, cache, store.as_ref(), *account_id)?;
                self.write(status_line(&name, items, &values).as_bytes())
                    .await?;
            }
        }
        if command.recursive_match {
            let mut parents: Vec<String> = Vec::new();
            for name in &subscribed {
                let mut prefix = String::new();
                for part in name.split(DELIMITER) {
                    if !prefix.is_empty() {
                        if !subscribed.contains(&prefix)
                            && !emitted.contains(&prefix)
                            && !parents.contains(&prefix)
                            && matches_any(&prefix, SpecialUse::None)
                        {
                            parents.push(prefix.clone());
                        }
                        prefix.push(DELIMITER);
                    }
                    prefix.push_str(part);
                }
            }
            for parent in parents {
                let exists = mailboxes
                    .iter()
                    .any(|mailbox| display_name(mailbox) == parent);
                self.write(childinfo_line(&parent, exists).as_bytes())
                    .await?;
            }
        }
        self.reply(tag, "OK", "LIST completed").await
    }

    fn status_values_from(
        &self,
        mailbox: &Mailbox,
        cache: &MessageStoreCache,
        store: &dyn Store,
        account_id: u32,
    ) -> Result<StatusValues> {
        Ok(StatusValues {
            messages: cache.in_mailbox(mailbox.id).count() as u32,
            recent: 0,
            uidnext: mailbox.last_uid(store, account_id)? + 1,
            uidvalidity: mailbox.uid_validity,
            unseen: count_unseen(cache, mailbox.id),
            highest_modseq: self.highest_modseq_value(),
        })
    }

    fn quota_view(&self) -> Result<QuotaLimitsView> {
        let Some((store, account_id)) = self.message_context() else {
            return Ok(QuotaLimitsView::default());
        };
        let usage = irixmail_store::Quota::new(store.as_ref()).usage(account_id)?;
        let (byte_limit, message_limit) = match self.account.as_ref() {
            Some(account) => (account.byte_quota(), account.message_quota()),
            None => (None, None),
        };
        Ok(QuotaLimitsView {
            used_bytes: usage.bytes,
            byte_limit,
            used_messages: usage.messages,
            message_limit,
        })
    }

    fn highest_modseq_value(&self) -> u64 {
        let Some((store, account_id)) = self.message_context() else {
            return 1;
        };
        ChangeLog::new(store.as_ref())
            .latest_change_id(account_id, Collection::Email)
            .unwrap_or(0)
            .max(1)
    }

    fn modseq_map(&self, store: &dyn Store, account_id: u32) -> Result<HashMap<u32, u64>> {
        let mut map = HashMap::new();
        for entry in ChangeLog::new(store).changes_since(account_id, Collection::Email, 0)? {
            map.insert(entry.document_id, entry.change_id);
        }
        Ok(map)
    }

    fn qresync_replay(&self, mailbox: &Mailbox, qresync: &QresyncParam) -> Result<Vec<Vec<u8>>> {
        let mut lines = Vec::new();
        if qresync.uidvalidity != mailbox.uid_validity {
            return Ok(lines);
        }
        let Some((store, account_id)) = self.message_context() else {
            return Ok(lines);
        };
        let log = ChangeLog::new(store.as_ref());
        let mut vanished: Vec<u32> = log
            .vanished_since(account_id, qresync.modseq)?
            .into_iter()
            .filter(|entry| entry.mailbox_id == mailbox.id)
            .map(|entry| entry.uid)
            .collect();
        if let Some(known) = &qresync.known_uids {
            let largest = vanished.iter().copied().max().unwrap_or(0);
            vanished.retain(|uid| sequence_contains(known, *uid, largest));
        }
        if !vanished.is_empty() {
            lines.push(
                format!("* VANISHED (EARLIER) {}\r\n", compress_sequence(&vanished)).into_bytes(),
            );
        }
        let map = self.modseq_map(store.as_ref(), account_id)?;
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut members: Vec<(u32, &MessageCacheEntry)> = cache
            .in_mailbox(mailbox.id)
            .filter_map(|entry| entry.uid_in(mailbox.id).map(|uid| (uid, entry)))
            .collect();
        members.sort_by_key(|(uid, _)| *uid);
        let items = ["UID".to_string(), "FLAGS".to_string(), "MODSEQ".to_string()];
        for (index, (uid, entry)) in members.iter().enumerate() {
            let modseq = map.get(&entry.document_id).copied().unwrap_or(1);
            if modseq <= qresync.modseq {
                continue;
            }
            let extras = FetchExtras {
                modseq: Some(modseq),
                ..FetchExtras::default()
            };
            lines.push(fetch_line(
                index as u32 + 1,
                *uid,
                entry,
                &items,
                false,
                &extras,
            ));
        }
        Ok(lines)
    }

    fn fetch_lines(
        &self,
        ranges: &[SeqRange],
        items: &[String],
        uid_mode: bool,
        mods: Option<&FetchMods>,
    ) -> Result<Vec<Vec<u8>>> {
        let Some(name) = self.data.mailbox.clone() else {
            return Ok(Vec::new());
        };
        let Some(mailbox) = self.mailbox_named(&name) else {
            return Ok(Vec::new());
        };
        let Some((store, account_id)) = self.message_context() else {
            return Ok(Vec::new());
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut targets: Vec<(u32, &MessageCacheEntry)> = cache
            .in_mailbox(mailbox.id)
            .filter_map(|entry| entry.uid_in(mailbox.id).map(|uid| (uid, entry)))
            .collect();
        targets.sort_by_key(|(uid, _)| *uid);
        let total = targets.len() as u32;
        let uid_max = targets.last().map(|(uid, _)| *uid).unwrap_or(0);
        let wants_body = items.iter().any(|item| is_body_item(item));
        let wants_envelope = items.iter().any(|item| item == "ENVELOPE");
        let wants_bodystructure = items.iter().any(|item| item == "BODYSTRUCTURE");
        let wants_body_brief = items.iter().any(|item| item == "BODY");
        let wants_structure = items.iter().any(|item| is_structure_item(item));
        let wants_message = wants_body || wants_envelope || wants_structure;
        let sets_seen = !self.read_only && items.iter().any(|item| is_seen_setting_item(item));
        let wants_modseq = items.iter().any(|item| item == "MODSEQ") || mods.is_some();
        let modseqs = if wants_modseq {
            Some(self.modseq_map(store.as_ref(), account_id)?)
        } else {
            None
        };
        let changed_floor = match mods {
            Some(m)
                if ChangeLog::new(store.as_ref()).can_calculate(
                    account_id,
                    Collection::Email,
                    m.changed_since,
                )? =>
            {
                Some(m.changed_since)
            }
            _ => None,
        };
        let mut lines = Vec::new();
        if let Some(m) = mods {
            if m.vanished {
                let vanished: Vec<u32> = ChangeLog::new(store.as_ref())
                    .vanished_since(account_id, m.changed_since)?
                    .into_iter()
                    .filter(|entry| entry.mailbox_id == mailbox.id)
                    .map(|entry| entry.uid)
                    .collect();
                let largest = uid_max.max(vanished.iter().copied().max().unwrap_or(0));
                let vanished: Vec<u32> = vanished
                    .into_iter()
                    .filter(|uid| sequence_contains(ranges, *uid, largest))
                    .collect();
                if !vanished.is_empty() {
                    lines.push(
                        format!("* VANISHED (EARLIER) {}\r\n", compress_sequence(&vanished))
                            .into_bytes(),
                    );
                }
            }
        }
        for (index, (uid, entry)) in targets.iter().enumerate() {
            let seqno = index as u32 + 1;
            let selected = if uid_mode {
                sequence_contains(ranges, *uid, uid_max)
            } else {
                sequence_contains(ranges, seqno, total)
            };
            if !selected {
                continue;
            }
            let modseq = modseqs
                .as_ref()
                .map(|map| map.get(&entry.document_id).copied().unwrap_or(1));
            if let Some(since) = changed_floor {
                if modseq.unwrap_or(1) <= since {
                    continue;
                }
            }
            let loaded = if wants_message {
                self.load_message_body(store.as_ref(), account_id, entry.document_id)?
            } else {
                None
            };
            let extras = match loaded.as_ref() {
                Some(body) => FetchExtras {
                    body: Some(BodyData {
                        full: &body.raw,
                        header: body.raw.get(body.header.clone()).unwrap_or(&[]),
                        text: body.raw.get(body.text.clone()).unwrap_or(&[]),
                        parts: &body.metadata.parts,
                    }),
                    envelope: wants_envelope.then(|| build_envelope(&body.raw)).flatten(),
                    structure: wants_bodystructure
                        .then(|| build_bodystructure(&body.raw, true))
                        .flatten(),
                    structure_brief: wants_body_brief
                        .then(|| build_bodystructure(&body.raw, false))
                        .flatten(),
                    modseq,
                },
                None => FetchExtras {
                    modseq,
                    ..FetchExtras::default()
                },
            };
            let implicit_seen = sets_seen && !entry.has_keyword(&Keyword::Seen);
            match (implicit_seen, self.notifier.as_ref()) {
                (true, Some(notifier)) => {
                    update_message(
                        store.as_ref(),
                        notifier.as_ref(),
                        account_id,
                        entry.document_id,
                        |data| {
                            data.add_keyword(Keyword::Seen);
                            Ok(())
                        },
                    )?;
                    let mut updated = (*entry).clone();
                    updated.keywords.push(Keyword::Seen);
                    let mut line_items = items.to_vec();
                    if !line_items.iter().any(|item| item == "FLAGS") {
                        line_items.push("FLAGS".to_string());
                    }
                    lines.push(fetch_line(
                        seqno,
                        *uid,
                        &updated,
                        &line_items,
                        uid_mode,
                        &extras,
                    ));
                }
                _ => lines.push(fetch_line(seqno, *uid, entry, items, uid_mode, &extras)),
            }
        }
        Ok(lines)
    }

    fn load_message_body(
        &self,
        store: &dyn Store,
        account_id: u32,
        document_id: u32,
    ) -> Result<Option<LoadedBody>> {
        let Some(blobs) = self.blobs.as_ref() else {
            return Ok(None);
        };
        let Some(metadata) = load_metadata(store, account_id, document_id)? else {
            return Ok(None);
        };
        let Some(raw) = blobs.get_all(&metadata.blob_hash())? else {
            return Ok(None);
        };
        let (header, text) = match metadata.root() {
            Some(root) => (root.header.as_range(), root.body.as_range()),
            None => (0..0, 0..raw.len()),
        };
        Ok(Some(LoadedBody {
            raw,
            header,
            text,
            metadata,
        }))
    }

    async fn run_store(
        &mut self,
        tag: &str,
        label: &str,
        ranges: &[SeqRange],
        op: &StoreOp,
        uid_mode: bool,
        unchanged_since: Option<u64>,
    ) -> Result<()> {
        if unchanged_since.is_some() {
            self.condstore = true;
        }
        match self.apply_store(ranges, op, uid_mode, unchanged_since) {
            Ok((lines, modified)) => {
                for line in lines {
                    self.write(&line).await?;
                }
                if modified.is_empty() {
                    self.reply(tag, "OK", &format!("{label} completed")).await
                } else {
                    let text = format!(
                        "[MODIFIED {}] {label} completed",
                        compress_sequence(&modified)
                    );
                    self.reply(tag, "OK", &text).await
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "irixmail::imap",
                    sid = self.sid,
                    error = %error,
                    "{label} failed"
                );
                self.reply(tag, "NO", &format!("{label} failed")).await
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn apply_store(
        &self,
        ranges: &[SeqRange],
        op: &StoreOp,
        uid_mode: bool,
        unchanged_since: Option<u64>,
    ) -> Result<(Vec<Vec<u8>>, Vec<u32>)> {
        let Some(name) = self.data.mailbox.clone() else {
            return Ok((Vec::new(), Vec::new()));
        };
        let Some(mailbox) = self.mailbox_named(&name) else {
            return Ok((Vec::new(), Vec::new()));
        };
        let Some((store, account_id)) = self.message_context() else {
            return Ok((Vec::new(), Vec::new()));
        };
        let Some(notifier) = self.notifier.clone() else {
            return Ok((Vec::new(), Vec::new()));
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut members: Vec<(u32, &MessageCacheEntry)> = cache
            .in_mailbox(mailbox.id)
            .filter_map(|entry| entry.uid_in(mailbox.id).map(|uid| (uid, entry)))
            .collect();
        members.sort_by_key(|(uid, _)| *uid);
        let total = members.len() as u32;
        let uid_max = members.last().map(|(uid, _)| *uid).unwrap_or(0);
        let report_modseq = unchanged_since.is_some() || self.condstore;
        let modseqs = if report_modseq || unchanged_since.is_some() {
            Some(self.modseq_map(store.as_ref(), account_id)?)
        } else {
            None
        };
        let calculable = match unchanged_since {
            Some(since) => ChangeLog::new(store.as_ref()).can_calculate(
                account_id,
                Collection::Email,
                since,
            )?,
            None => true,
        };

        let mut targets: Vec<(u32, u32, u32)> = Vec::new();
        let mut modified: Vec<u32> = Vec::new();
        for (index, (uid, entry)) in members.iter().enumerate() {
            let seqno = index as u32 + 1;
            let selected = if uid_mode {
                sequence_contains(ranges, *uid, uid_max)
            } else {
                sequence_contains(ranges, seqno, total)
            };
            if !selected {
                continue;
            }
            if let Some(since) = unchanged_since {
                let modseq = modseqs
                    .as_ref()
                    .and_then(|map| map.get(&entry.document_id).copied())
                    .unwrap_or(1);
                if !calculable || modseq > since {
                    modified.push(if uid_mode { *uid } else { seqno });
                    continue;
                }
            }
            targets.push((seqno, *uid, entry.document_id));
        }

        let ids: Vec<u32> = targets
            .iter()
            .map(|(_, _, document_id)| *document_id)
            .collect();
        let results = update_messages(
            store.as_ref(),
            notifier.as_ref(),
            account_id,
            &ids,
            |_, data| {
                data.keywords = desired_keywords(&data.keywords, op);
                Ok(())
            },
        )?;

        let mut lines = Vec::new();
        if !op.silent {
            for (seqno, uid, document_id) in targets {
                let Some(updated) = results.iter().find(|u| u.document_id == document_id) else {
                    continue;
                };
                let entry = MessageCacheEntry {
                    document_id,
                    mailboxes: Vec::new(),
                    keywords: updated.data.keywords.clone(),
                    thread_id: 0,
                    size: 0,
                    received_at: 0,
                    sent_at: 0,
                };
                let mut items = vec!["FLAGS".to_string()];
                let mut extras = FetchExtras::default();
                if report_modseq {
                    items.push("MODSEQ".to_string());
                    extras.modseq = Some(updated.change_id.unwrap_or_else(|| {
                        modseqs
                            .as_ref()
                            .and_then(|map| map.get(&document_id).copied())
                            .unwrap_or(1)
                    }));
                }
                lines.push(fetch_line(seqno, uid, &entry, &items, uid_mode, &extras));
            }
        }
        Ok((lines, modified))
    }

    async fn run_copy(
        &mut self,
        tag: &str,
        args: &[Token],
        uid_mode: bool,
        move_mode: bool,
    ) -> Result<()> {
        let label = match (uid_mode, move_mode) {
            (true, true) => "UID MOVE",
            (true, false) => "UID COPY",
            (false, true) => "MOVE",
            (false, false) => "COPY",
        };
        let Some(ranges) = self.resolve_set(args.first().and_then(Token::as_str), uid_mode)? else {
            self.reply(
                tag,
                "BAD",
                &format!("{label} expects a sequence set and mailbox"),
            )
            .await?;
            return Ok(());
        };
        let target_name = args
            .get(1)
            .and_then(Token::as_str)
            .unwrap_or("")
            .to_string();
        let Some(target) = self.mailbox_named(&target_name) else {
            self.reply(tag, "NO", "[TRYCREATE] target mailbox does not exist")
                .await?;
            return Ok(());
        };
        let selected = self
            .data
            .mailbox
            .as_deref()
            .and_then(|name| self.mailbox_named(name));
        if selected.is_some_and(|source| source.id == target.id) {
            self.reply(
                tag,
                "NO",
                "[CANNOT] source and destination mailboxes are the same",
            )
            .await?;
            return Ok(());
        }
        let result = self.copy_apply(&ranges, &target, uid_mode, move_mode)?;
        let copyuid = (!result.src_uids.is_empty()).then(|| {
            format!(
                "[COPYUID {} {} {}]",
                target.uid_validity,
                uid_list(&result.src_uids),
                uid_list(&result.dst_uids)
            )
        });
        if move_mode {
            if let Some(code) = &copyuid {
                self.write(format!("* OK {code} Copied UIDs\r\n").as_bytes())
                    .await?;
            }
            for line in &result.expunges {
                self.write(line).await?;
            }
            self.reply(tag, "OK", &format!("{label} completed")).await?;
        } else {
            let text = match &copyuid {
                Some(code) => format!("{code} {label} completed"),
                None => format!("{label} completed"),
            };
            self.reply(tag, "OK", &text).await?;
        }
        Ok(())
    }

    fn copy_apply(
        &self,
        ranges: &[SeqRange],
        target: &Mailbox,
        uid_mode: bool,
        move_mode: bool,
    ) -> Result<CopyResult> {
        let mut result = CopyResult::default();
        let Some(name) = self.data.mailbox.clone() else {
            return Ok(result);
        };
        let Some(source) = self.mailbox_named(&name) else {
            return Ok(result);
        };
        let Some((store, account_id)) = self.message_context() else {
            return Ok(result);
        };
        let Some(notifier) = self.notifier.clone() else {
            return Ok(result);
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut members: Vec<(u32, &MessageCacheEntry)> = cache
            .in_mailbox(source.id)
            .filter_map(|entry| entry.uid_in(source.id).map(|uid| (uid, entry)))
            .collect();
        members.sort_by_key(|(uid, _)| *uid);
        let total = members.len() as u32;
        let uid_max = members.last().map(|(uid, _)| *uid).unwrap_or(0);

        let mut selected: Vec<(u32, u32, u32, Option<u32>)> = Vec::new();
        for (index, (uid, entry)) in members.iter().enumerate() {
            let seqno = index as u32 + 1;
            let hit = if uid_mode {
                sequence_contains(ranges, *uid, uid_max)
            } else {
                sequence_contains(ranges, seqno, total)
            };
            if hit {
                selected.push((seqno, *uid, entry.document_id, entry.uid_in(target.id)));
            }
        }

        let mut expunged = Vec::new();
        for (seqno, src_uid, document_id, existing) in selected {
            let dst_uid = match existing {
                Some(dst_uid) => {
                    if move_mode {
                        let source_id = source.id;
                        update_message(
                            store.as_ref(),
                            notifier.as_ref(),
                            account_id,
                            document_id,
                            |data| {
                                data.remove_mailbox(source_id);
                                Ok(())
                            },
                        )?;
                    }
                    dst_uid
                }
                None => {
                    let dst_uid = target.next_uid(store.as_ref(), account_id)?;
                    let target_id = target.id;
                    let source_id = source.id;
                    update_message(
                        store.as_ref(),
                        notifier.as_ref(),
                        account_id,
                        document_id,
                        |data| {
                            data.add_mailbox(target_id, dst_uid);
                            if move_mode {
                                data.remove_mailbox(source_id);
                            }
                            Ok(())
                        },
                    )?;
                    dst_uid
                }
            };
            result.src_uids.push(src_uid);
            result.dst_uids.push(dst_uid);
            if move_mode {
                expunged.push(seqno);
            }
        }

        expunged.sort_unstable();
        for (removed, seqno) in expunged.iter().enumerate() {
            let adjusted = seqno - removed as u32;
            result
                .expunges
                .push(format!("* {adjusted} EXPUNGE\r\n").into_bytes());
        }
        Ok(result)
    }

    fn apply_expunge(&self, ranges: Option<&[SeqRange]>) -> Result<Vec<Vec<u8>>> {
        let mut lines = Vec::new();
        let Some(name) = self.data.mailbox.clone() else {
            return Ok(lines);
        };
        let Some(mailbox) = self.mailbox_named(&name) else {
            return Ok(lines);
        };
        let Some((store, account_id)) = self.message_context() else {
            return Ok(lines);
        };
        let Some(notifier) = self.notifier.clone() else {
            return Ok(lines);
        };
        let Some(blobs) = self.blobs.clone() else {
            return Ok(lines);
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut members: Vec<(u32, &MessageCacheEntry)> = cache
            .in_mailbox(mailbox.id)
            .filter_map(|entry| entry.uid_in(mailbox.id).map(|uid| (uid, entry)))
            .collect();
        members.sort_by_key(|(uid, _)| *uid);
        let uid_max = members.last().map(|(uid, _)| *uid).unwrap_or(0);

        let mut selected: Vec<(u32, u32, u32, usize)> = Vec::new();
        for (index, (uid, entry)) in members.iter().enumerate() {
            if !entry.has_keyword(&Keyword::Deleted) {
                continue;
            }
            if let Some(ranges) = ranges {
                if !sequence_contains(ranges, *uid, uid_max) {
                    continue;
                }
            }
            let seqno = index as u32 + 1;
            selected.push((seqno, *uid, entry.document_id, entry.mailboxes.len()));
        }

        let mut vanished_uids = Vec::new();
        for (removed, (seqno, uid, document_id, mailbox_count)) in selected.iter().enumerate() {
            if *mailbox_count <= 1 {
                delete_message(
                    store.as_ref(),
                    blobs.as_ref(),
                    notifier.as_ref(),
                    account_id,
                    *document_id,
                )?;
            } else {
                let mailbox_id = mailbox.id;
                update_message(
                    store.as_ref(),
                    notifier.as_ref(),
                    account_id,
                    *document_id,
                    |data| {
                        data.remove_mailbox(mailbox_id);
                        data.remove_keyword(&Keyword::Deleted);
                        Ok(())
                    },
                )?;
            }
            if self.qresync {
                vanished_uids.push(*uid);
            } else {
                let adjusted = seqno - removed as u32;
                lines.push(format!("* {adjusted} EXPUNGE\r\n").into_bytes());
            }
        }
        if !vanished_uids.is_empty() {
            lines
                .push(format!("* VANISHED {}\r\n", compress_sequence(&vanished_uids)).into_bytes());
        }
        Ok(lines)
    }

    async fn run_create(&mut self, tag: &str, name: &str) -> Result<()> {
        if name.is_empty() {
            self.reply(tag, "BAD", "CREATE expects a mailbox name")
                .await?;
            return Ok(());
        }
        if self.mailbox_named(name).is_some() {
            self.reply(tag, "NO", "mailbox already exists").await?;
            return Ok(());
        }
        let Some((store, account_id)) = self.message_context() else {
            self.reply(tag, "NO", "mailbox store unavailable").await?;
            return Ok(());
        };
        let Some(notifier) = self.notifier.clone() else {
            self.reply(tag, "NO", "mailbox store unavailable").await?;
            return Ok(());
        };
        let created_at = self
            .account
            .as_ref()
            .map(|account| account.created_at)
            .unwrap_or(0);
        create_mailbox(
            store.as_ref(),
            notifier.as_ref(),
            account_id,
            name,
            assign_uid_validity(created_at),
        )?;
        tracing::info!(target: "irixmail::imap", sid = self.sid, mailbox = %name, "mailbox created");
        self.reply(tag, "OK", "CREATE completed").await?;
        Ok(())
    }

    async fn run_delete(&mut self, tag: &str, name: &str) -> Result<()> {
        let Some(mailbox) = self.mailbox_named(name) else {
            self.reply(tag, "NO", "mailbox does not exist").await?;
            return Ok(());
        };
        if mailbox.id < FIRST_USER_MAILBOX_ID {
            self.reply(tag, "NO", "cannot delete a system mailbox")
                .await?;
            return Ok(());
        }
        let Some((store, account_id)) = self.message_context() else {
            self.reply(tag, "NO", "mailbox store unavailable").await?;
            return Ok(());
        };
        let (Some(notifier), Some(blobs)) = (self.notifier.clone(), self.blobs.clone()) else {
            self.reply(tag, "NO", "mailbox store unavailable").await?;
            return Ok(());
        };
        delete_mailbox(
            store.as_ref(),
            blobs.as_ref(),
            notifier.as_ref(),
            account_id,
            mailbox.id,
            true,
        )?;
        tracing::info!(target: "irixmail::imap", sid = self.sid, mailbox = %name, "mailbox deleted");
        self.reply(tag, "OK", "DELETE completed").await?;
        Ok(())
    }

    async fn run_rename(&mut self, tag: &str, source: &str, target: &str) -> Result<()> {
        let Some(mailbox) = self.mailbox_named(source) else {
            self.reply(tag, "NO", "mailbox does not exist").await?;
            return Ok(());
        };
        if mailbox.id < FIRST_USER_MAILBOX_ID {
            self.reply(tag, "NO", "cannot rename a system mailbox")
                .await?;
            return Ok(());
        }
        if target.is_empty() {
            self.reply(tag, "BAD", "RENAME expects a target name")
                .await?;
            return Ok(());
        }
        if self.mailbox_named(target).is_some() {
            self.reply(tag, "NO", "target mailbox already exists")
                .await?;
            return Ok(());
        }
        let Some((store, account_id)) = self.message_context() else {
            self.reply(tag, "NO", "mailbox store unavailable").await?;
            return Ok(());
        };
        let Some(notifier) = self.notifier.clone() else {
            self.reply(tag, "NO", "mailbox store unavailable").await?;
            return Ok(());
        };
        rename_mailbox(
            store.as_ref(),
            notifier.as_ref(),
            account_id,
            mailbox.id,
            target,
        )?;
        tracing::info!(
            target: "irixmail::imap",
            sid = self.sid,
            mailbox = %source,
            renamed_to = %target,
            "mailbox renamed"
        );
        self.reply(tag, "OK", "RENAME completed").await?;
        Ok(())
    }

    fn search_matches(&self, key: &SearchKey, uid_mode: bool) -> Result<SearchOutcome> {
        let Some(name) = self.data.mailbox.clone() else {
            return Ok(SearchOutcome::default());
        };
        let Some(mailbox) = self.mailbox_named(&name) else {
            return Ok(SearchOutcome::default());
        };
        let Some((store, account_id)) = self.message_context() else {
            return Ok(SearchOutcome::default());
        };
        let modseqs = if key.uses_modseq() {
            Some(self.modseq_map(store.as_ref(), account_id)?)
        } else {
            None
        };
        let cache = MessageStoreCache::build(store.as_ref(), account_id)?;
        let mut members: Vec<(u32, &MessageCacheEntry)> = cache
            .in_mailbox(mailbox.id)
            .filter_map(|entry| entry.uid_in(mailbox.id).map(|uid| (uid, entry)))
            .collect();
        members.sort_by_key(|(uid, _)| *uid);
        let universe: Vec<u32> = members.iter().map(|(_, entry)| entry.document_id).collect();
        let saved: &[u32] = self.saved_search.as_deref().unwrap_or(&[]);
        let ctx = SearchCtx {
            total: members.len() as u32,
            uid_max: members.last().map(|(uid, _)| *uid).unwrap_or(0),
            members: &members,
            universe: &universe,
            store: store.as_ref(),
            blobs: self.blobs.as_deref(),
            account_id,
            modseqs: modseqs.as_ref(),
            saved_uids: saved,
        };
        let matched = eval_search(&ctx, key)?;

        let mut ids = Vec::new();
        let mut uids = Vec::new();
        let mut highest = None;
        for (index, (uid, entry)) in members.iter().enumerate() {
            if matched.contains(&entry.document_id) {
                ids.push(if uid_mode { *uid } else { index as u32 + 1 });
                uids.push(*uid);
                if let Some(map) = modseqs.as_ref() {
                    let modseq = map.get(&entry.document_id).copied().unwrap_or(1);
                    highest = Some(highest.unwrap_or(0).max(modseq));
                }
            }
        }
        ids.sort_unstable();
        uids.sort_unstable();
        Ok(SearchOutcome {
            ids,
            uids,
            modseq: highest,
        })
    }

    fn mailbox_named(&self, name: &str) -> Option<Mailbox> {
        self.mailboxes().into_iter().find(|mailbox| {
            let display = display_name(mailbox);
            display == name
                || (display.eq_ignore_ascii_case("INBOX") && name.eq_ignore_ascii_case("INBOX"))
        })
    }

    async fn reject(&mut self, line: &[u8], error: ParseError) -> Result<()> {
        let message = error.to_string();
        match leading_tag(line) {
            Some(tag) => self.reply(tag, "BAD", &message).await,
            None => {
                let out = format!("* BAD {message}\r\n");
                self.write(out.as_bytes()).await
            }
        }
    }

    async fn refuse_read_only(&mut self, tag: &str) -> Result<()> {
        self.reply(tag, "NO", "not permitted on a read-only mailbox")
            .await
    }

    async fn refuse_charset(&mut self, tag: &str) -> Result<()> {
        let text = format!("[BADCHARSET ({SUPPORTED_CHARSETS})] charset is not supported");
        self.reply(tag, "NO", &text).await
    }

    async fn reply(&mut self, tag: &str, status: &str, text: &str) -> Result<()> {
        let line = format!("{tag} {status} {text}\r\n");
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

struct LoadedBody {
    raw: Vec<u8>,
    header: std::ops::Range<usize>,
    text: std::ops::Range<usize>,
    metadata: irixmail_mail::MessageMetadata,
}

#[derive(Default)]
struct CopyResult {
    expunges: Vec<Vec<u8>>,
    src_uids: Vec<u32>,
    dst_uids: Vec<u32>,
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn uid_list(uids: &[u32]) -> String {
    uids.iter()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Default)]
struct SearchOutcome {
    ids: Vec<u32>,
    uids: Vec<u32>,
    modseq: Option<u64>,
}

struct SortableEntry {
    seqno: u32,
    uid: u32,
    size: u32,
    received_at: u64,
    sent_at: u64,
    thread_id: u32,
    subject: String,
    from: String,
    to: String,
    cc: String,
}

fn charset_supported(name: &str) -> bool {
    SUPPORTED_CHARSETS
        .split(' ')
        .any(|charset| charset.eq_ignore_ascii_case(name))
}

fn compare_entries(a: &SortableEntry, b: &SortableEntry, specs: &[SortSpec]) -> std::cmp::Ordering {
    for spec in specs {
        let ord = match spec.key {
            SortKey::Arrival => a.received_at.cmp(&b.received_at),
            SortKey::Date => date_of(a).cmp(&date_of(b)),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Subject => base_subject(&a.subject).cmp(&base_subject(&b.subject)),
            SortKey::From => a
                .from
                .to_ascii_lowercase()
                .cmp(&b.from.to_ascii_lowercase()),
            SortKey::To => a.to.to_ascii_lowercase().cmp(&b.to.to_ascii_lowercase()),
            SortKey::Cc => a.cc.to_ascii_lowercase().cmp(&b.cc.to_ascii_lowercase()),
        };
        let ord = if spec.reverse { ord.reverse() } else { ord };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    a.uid.cmp(&b.uid)
}

fn date_of(entry: &SortableEntry) -> u64 {
    if entry.sent_at != 0 {
        entry.sent_at
    } else {
        entry.received_at
    }
}

fn saved_selection(uids: &[u32], ret: &SearchReturn) -> Vec<u32> {
    if (ret.min || ret.max) && !ret.all && !ret.count {
        let mut selection = Vec::new();
        if ret.min {
            selection.extend(uids.first().copied());
        }
        if ret.max {
            selection.extend(uids.last().copied());
        }
        selection.dedup();
        selection
    } else {
        uids.to_vec()
    }
}

fn ranges_from(values: &[u32]) -> Vec<SeqRange> {
    let mut sorted: Vec<u32> = values.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut ranges: Vec<SeqRange> = Vec::new();
    for value in sorted {
        match ranges.last_mut() {
            Some(range) if matches!(range.to, SeqPoint::Num(end) if end + 1 == value) => {
                range.to = SeqPoint::Num(value);
            }
            _ => ranges.push(SeqRange {
                from: SeqPoint::Num(value),
                to: SeqPoint::Num(value),
            }),
        }
    }
    ranges
}

struct SearchCtx<'a> {
    members: &'a [(u32, &'a MessageCacheEntry)],
    total: u32,
    uid_max: u32,
    universe: &'a [u32],
    store: &'a dyn Store,
    blobs: Option<&'a dyn BlobStore>,
    account_id: u32,
    modseqs: Option<&'a HashMap<u32, u64>>,
    saved_uids: &'a [u32],
}

fn eval_search(ctx: &SearchCtx<'_>, key: &SearchKey) -> Result<HashSet<u32>> {
    let matched = match key {
        SearchKey::All => ctx.universe.iter().copied().collect(),
        SearchKey::Nothing => HashSet::new(),
        SearchKey::Flag(atom, present) => {
            let keyword = Keyword::from_imap(atom);
            ctx.members
                .iter()
                .filter(|(_, entry)| entry.has_keyword(&keyword) == *present)
                .map(|(_, entry)| entry.document_id)
                .collect()
        }
        SearchKey::Larger(size) => ctx
            .members
            .iter()
            .filter(|(_, entry)| entry.size > *size)
            .map(|(_, entry)| entry.document_id)
            .collect(),
        SearchKey::Smaller(size) => ctx
            .members
            .iter()
            .filter(|(_, entry)| entry.size < *size)
            .map(|(_, entry)| entry.document_id)
            .collect(),
        SearchKey::Uid(ranges) => ctx
            .members
            .iter()
            .filter(|(uid, _)| sequence_contains(ranges, *uid, ctx.uid_max))
            .map(|(_, entry)| entry.document_id)
            .collect(),
        SearchKey::Sequence(ranges) => ctx
            .members
            .iter()
            .enumerate()
            .filter(|(index, _)| sequence_contains(ranges, *index as u32 + 1, ctx.total))
            .map(|(_, (_, entry))| entry.document_id)
            .collect(),
        SearchKey::Text(text) => {
            let query = Query::term(text.clone());
            let candidates = FtsIndex::new(ctx.store).search(
                ctx.account_id,
                Collection::Email,
                &query,
                ctx.universe,
            )?;
            phrase_restrict(ctx, candidates, None, text)?
        }
        SearchKey::FieldText(field, text) => {
            let query = Query::field(*field, text.clone());
            let candidates = FtsIndex::new(ctx.store).search(
                ctx.account_id,
                Collection::Email,
                &query,
                ctx.universe,
            )?;
            phrase_restrict(ctx, candidates, Some(*field), text)?
        }
        SearchKey::Header(name, needle) => {
            let mut matched = HashSet::new();
            for (_, entry) in ctx.members {
                if let Some(metadata) = load_metadata(ctx.store, ctx.account_id, entry.document_id)?
                {
                    if header_matches(&metadata.raw_headers, name, needle) {
                        matched.insert(entry.document_id);
                    }
                }
            }
            matched
        }
        SearchKey::Before(date) => date_filter(ctx, |e| e.received_at, |ts| ts < *date),
        SearchKey::Since(date) => date_filter(ctx, |e| e.received_at, |ts| ts >= *date),
        SearchKey::On(date) => date_filter(
            ctx,
            |e| e.received_at,
            |ts| ts >= *date && ts < *date + 86_400,
        ),
        SearchKey::SentBefore(date) => date_filter(ctx, |e| e.sent_at, |ts| ts != 0 && ts < *date),
        SearchKey::SentSince(date) => date_filter(ctx, |e| e.sent_at, |ts| ts != 0 && ts >= *date),
        SearchKey::SentOn(date) => date_filter(
            ctx,
            |e| e.sent_at,
            |ts| ts != 0 && ts >= *date && ts < *date + 86_400,
        ),
        SearchKey::Not(inner) => {
            let sub = eval_search(ctx, inner)?;
            ctx.universe
                .iter()
                .copied()
                .filter(|id| !sub.contains(id))
                .collect()
        }
        SearchKey::Or(first, second) => {
            let left = eval_search(ctx, first)?;
            let right = eval_search(ctx, second)?;
            left.union(&right).copied().collect()
        }
        SearchKey::And(keys) => {
            let mut acc: Option<HashSet<u32>> = None;
            for inner in keys {
                let next = eval_search(ctx, inner)?;
                acc = Some(match acc {
                    Some(current) => current.intersection(&next).copied().collect(),
                    None => next,
                });
            }
            acc.unwrap_or_else(|| ctx.universe.iter().copied().collect())
        }
        SearchKey::ModSeq(value) => ctx
            .members
            .iter()
            .filter(|(_, entry)| {
                ctx.modseqs
                    .and_then(|map| map.get(&entry.document_id).copied())
                    .unwrap_or(1)
                    >= *value
            })
            .map(|(_, entry)| entry.document_id)
            .collect(),
        SearchKey::Saved => ctx
            .members
            .iter()
            .filter(|(uid, _)| ctx.saved_uids.contains(uid))
            .map(|(_, entry)| entry.document_id)
            .collect(),
    };
    Ok(matched)
}

fn header_matches(raw_headers: &[u8], name: &str, needle: &str) -> bool {
    let text = String::from_utf8_lossy(raw_headers);
    let needle = needle.to_ascii_lowercase();
    let mut collecting = false;
    let mut value = String::new();
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if collecting {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if collecting {
            if needle.is_empty() || value.to_ascii_lowercase().contains(&needle) {
                return true;
            }
            collecting = false;
            value.clear();
        }
        if let Some((field, rest)) = line.split_once(':') {
            if field.eq_ignore_ascii_case(name) {
                collecting = true;
                value.push_str(rest.trim_start());
            }
        }
    }
    collecting && (needle.is_empty() || value.to_ascii_lowercase().contains(&needle))
}

fn date_filter(
    ctx: &SearchCtx<'_>,
    select: impl Fn(&MessageCacheEntry) -> u64,
    matches: impl Fn(i64) -> bool,
) -> HashSet<u32> {
    ctx.members
        .iter()
        .filter(|(_, entry)| matches(select(entry) as i64))
        .map(|(_, entry)| entry.document_id)
        .collect()
}

// The FTS index has no positional data, so a multi-word query only proves co-occurrence;
// re-check candidates against the extracted text for the contiguous phrase.
fn phrase_restrict(
    ctx: &SearchCtx<'_>,
    candidates: Vec<u32>,
    field: Option<Field>,
    text: &str,
) -> Result<HashSet<u32>> {
    if fts_tokenize(text).len() < 2 {
        return Ok(candidates.into_iter().collect());
    }
    let Some(blobs) = ctx.blobs else {
        return Ok(candidates.into_iter().collect());
    };
    let needle = normalize_phrase(text);
    let mut matched = HashSet::new();
    for document_id in candidates {
        let Some(metadata) = load_metadata(ctx.store, ctx.account_id, document_id)? else {
            continue;
        };
        let Some(raw) = blobs.get_all(&metadata.blob_hash())? else {
            continue;
        };
        let Ok(extracted) = message_text(&raw) else {
            continue;
        };
        let spans: Vec<&str> = match field {
            None => vec![
                &extracted.subject,
                &extracted.body,
                &extracted.from,
                &extracted.to,
                &extracted.cc,
                &extracted.bcc,
            ],
            Some(Field::Subject) => vec![&extracted.subject],
            Some(Field::Body) => vec![&extracted.body],
            Some(Field::From) => vec![&extracted.from],
            Some(Field::To) => vec![&extracted.to],
            Some(Field::Cc) => vec![&extracted.cc],
            Some(Field::Bcc) => vec![&extracted.bcc],
            Some(Field::Combined) => vec![
                &extracted.subject,
                &extracted.body,
                &extracted.from,
                &extracted.to,
                &extracted.cc,
                &extracted.bcc,
            ],
        };
        if spans
            .iter()
            .any(|span| normalize_phrase(span).contains(&needle))
        {
            matched.insert(document_id);
        }
    }
    Ok(matched)
}

fn normalize_phrase(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn desired_keywords(current: &[Keyword], op: &StoreOp) -> Vec<Keyword> {
    let incoming: Vec<Keyword> = op
        .flags
        .iter()
        .map(|flag| Keyword::from_imap(flag))
        .filter(|keyword| *keyword != Keyword::Recent)
        .collect();
    match op.mode {
        StoreMode::Replace => {
            let mut out: Vec<Keyword> = Vec::new();
            for keyword in incoming {
                if !out.contains(&keyword) {
                    out.push(keyword);
                }
            }
            out
        }
        StoreMode::Add => {
            let mut out = current.to_vec();
            for keyword in incoming {
                if !out.contains(&keyword) {
                    out.push(keyword);
                }
            }
            out
        }
        StoreMode::Remove => current
            .iter()
            .filter(|keyword| !incoming.contains(keyword))
            .cloned()
            .collect(),
    }
}

fn count_unseen(cache: &MessageStoreCache, mailbox_id: u32) -> u32 {
    cache
        .in_mailbox(mailbox_id)
        .filter(|entry| !entry.has_keyword(&Keyword::Seen))
        .count() as u32
}

fn first_unseen_seqno(cache: &MessageStoreCache, mailbox_id: u32) -> Option<u32> {
    let mut members: Vec<(u32, bool)> = cache
        .in_mailbox(mailbox_id)
        .filter_map(|entry| {
            entry
                .uid_in(mailbox_id)
                .map(|uid| (uid, entry.has_keyword(&Keyword::Seen)))
        })
        .collect();
    members.sort_by_key(|(uid, _)| *uid);
    members
        .iter()
        .position(|(_, seen)| !seen)
        .map(|index| index as u32 + 1)
}

fn leading_tag(line: &[u8]) -> Option<&str> {
    let line = strip_crlf(line);
    let end = line.iter().position(|b| *b == b' ')?;
    if end == 0 {
        return None;
    }
    std::str::from_utf8(&line[..end]).ok()
}

fn strip_crlf(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    &line[..end]
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
        "127.0.0.1:1430".parse().unwrap()
    }

    struct HangingPipe {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl HangingPipe {
        fn new(input: &[u8]) -> Self {
            Self {
                input: Cursor::new(input.to_vec()),
                output: Vec::new(),
            }
        }
    }

    impl AsyncRead for HangingPipe {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let before = buf.filled().len();
            match std::pin::Pin::new(&mut self.input).poll_read(cx, buf) {
                std::task::Poll::Ready(Ok(())) if buf.filled().len() == before => {
                    std::task::Poll::Pending
                }
                other => other,
            }
        }
    }

    impl AsyncWrite for HangingPipe {
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

    #[tokio::test(start_paused = true)]
    async fn a_half_open_idle_connection_is_closed_after_the_timeout() {
        let mut session = Session::new(HangingPipe::new(b"a1 IDLE\r\n"), peer());
        session.state = State::Authenticated;

        let flow = tokio::time::timeout(IDLE_TIMEOUT * 2, session.run())
            .await
            .expect("the idle session should time out on its own")
            .unwrap();
        assert_eq!(flow, Flow::Close);

        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("+ idling"), "unexpected output: {out}");
        assert!(out.contains("* BYE"), "a timed-out idle says BYE: {out}");
    }

    async fn drive(script: &[u8]) -> (Flow, String, State) {
        let mut session = Session::new(Pipe::new(script), peer());
        let flow = session.run().await.unwrap();
        let state = session.state();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        (flow, out, state)
    }

    async fn run_authenticated(script: &[u8]) -> (Session<Pipe>, String) {
        let mut session = Session::new(Pipe::new(script), peer()).with_tls();
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        (session, out)
    }

    async fn drive_tls(script: &[u8]) -> String {
        let mut session = Session::new(Pipe::new(script), peer()).with_tls();
        session.run().await.unwrap();
        String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap()
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

    fn imap_log_text(logs: &irixmail_core::LogBuffer, sid: u64) -> String {
        let tagged = format!("sid={sid} ");
        logs.snapshot()
            .into_iter()
            .filter(|record| record.source == "irixmail::imap")
            .map(|record| record.message)
            .filter(|message| message.contains(&tagged))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn an_imap_session_logs_each_decision_under_one_session_id() {
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let logs = global_logs();
        let (directory, path) = account_directory("secret");
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let script = b"a1 LOGIN alice@example.com secret\r\na2 SELECT INBOX\r\na3 APPEND INBOX {14}\r\nSubject: x\r\n\r\n\r\na4 LOGOUT\r\n";
        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        let sid = session.session_id();
        session.run().await.unwrap();

        let text = imap_log_text(&logs, sid);
        for needle in [
            "connection accepted",
            "login succeeded",
            "mailbox selected",
            "message appended",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_failed_login_is_logged() {
        let logs = global_logs();
        let (directory, path) = account_directory("secret");
        let mut session = Session::new(Pipe::new(b"a LOGIN alice@example.com wrong\r\n"), peer())
            .with_tls()
            .with_directory(directory);
        let sid = session.session_id();
        session.run().await.unwrap();
        let text = imap_log_text(&logs, sid);
        assert!(text.contains("login refused"), "got:\n{text}");
        assert!(text.contains("alice@example.com"), "got:\n{text}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn the_id_command_reports_the_server() {
        let (_, out, _) = drive(b"a1 ID NIL\r\na2 LOGOUT\r\n").await;
        assert!(out.contains(r#"* ID ("name" "IRIXMAIL""#), "{out}");
        assert!(out.contains("a1 OK ID completed"), "{out}");
    }

    #[tokio::test]
    async fn the_id_command_accepts_a_client_parameter_list() {
        let (_, out, _) =
            drive(b"a1 ID (\"name\" \"Thunderbird\" \"version\" \"140\")\r\na2 LOGOUT\r\n").await;
        assert!(out.contains("a1 OK ID completed"), "{out}");
    }

    #[tokio::test]
    async fn the_greeting_carries_the_capability_list() {
        let (_, out, _) = drive(b"a LOGOUT\r\n").await;
        assert!(out.starts_with("* OK [CAPABILITY IMAP4rev1"), "{out}");
        assert!(out.contains("] IRIXMAIL"), "{out}");
    }

    #[tokio::test]
    async fn a_login_reply_carries_the_capability_code() {
        let (directory, path) = account_directory("secret");
        let mut session = Session::new(Pipe::new(b"a LOGIN alice@example.com secret\r\n"), peer())
            .with_tls()
            .with_directory(directory);
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("a OK [CAPABILITY IMAP4rev1"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn unselect_leaves_the_mailbox_without_expunging() {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, account, _account_id, path) = expunge_env(&[
            b"Subject: One\r\n\r\nbody one\r\n",
            b"Subject: Two\r\n\r\nbody two\r\n",
        ]);
        let mut session = Session::new(
            Pipe::new(
                b"b SELECT INBOX\r\nc STORE 1 +FLAGS.SILENT (\\Deleted)\r\nd UNSELECT\r\ne SELECT INBOX\r\n",
            ),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(blobs)
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("d OK UNSELECT completed"), "{out}");
        let reselect = out.split("d OK UNSELECT completed").nth(1).unwrap();
        assert!(
            reselect.contains("* 2 EXISTS"),
            "the deleted message survives UNSELECT: {out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn lsub_lists_every_folder_until_a_subscription_exists() {
        let (_, out) = run_authenticated(b"a1 LSUB \"\" \"*\"\r\n").await;
        assert!(out.contains("\"INBOX\""), "{out}");
        assert!(out.contains("\"Sent\""), "{out}");
        assert!(out.contains("a1 OK LSUB completed"), "{out}");
    }

    #[tokio::test]
    async fn creating_the_archive_folder_assigns_its_role() {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, account, _account_id, path) =
            expunge_env(&[b"Subject: One\r\n\r\nbody one\r\n"]);
        let mut session = Session::new(
            Pipe::new(b"a1 CREATE Archive\r\na2 LIST \"\" \"*\"\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(blobs)
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("a1 OK CREATE completed"), "{out}");
        let archive = out
            .lines()
            .find(|line| line.starts_with("* LIST") && line.contains("\"Archive\""))
            .unwrap_or_default();
        assert!(archive.contains("\\Archive"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_trycreate_refusal_drains_a_nonsync_literal() {
        let (_, out) =
            run_authenticated(b"a1 APPEND missing {9+}\r\na9 NOOP\r\n\r\na2 NOOP\r\n").await;
        assert!(out.contains("a1 NO [TRYCREATE]"), "{out}");
        assert!(out.contains("a2 OK NOOP completed"), "{out}");
        assert!(
            !out.contains("a9 OK"),
            "the literal must not be executed as a command: {out}"
        );
    }

    #[tokio::test]
    async fn an_append_without_a_store_is_refused_not_faked() {
        let (_, out) = run_authenticated(b"a1 APPEND INBOX {5}\r\nhello\r\n").await;
        assert!(out.contains("a1 NO"), "{out}");
        assert!(
            !out.contains("APPEND completed"),
            "an unstored message must not be acknowledged: {out}"
        );
    }

    #[test]
    fn commands_parse_case_insensitively() {
        assert_eq!(Command::from_word(b"login"), Command::Login);
        assert_eq!(Command::from_word(b"SELECT"), Command::Select);
        assert_eq!(Command::from_word(b"FeTcH"), Command::Fetch);
        assert_eq!(Command::from_word(b"NONSENSE"), Command::Unknown);
        assert_eq!(Command::from_word(b""), Command::Unknown);
    }

    #[tokio::test]
    async fn greeting_is_sent_before_any_command() {
        let (flow, out, _) = drive(b"a LOGOUT\r\n").await;
        assert_eq!(flow, Flow::Close);
        assert!(out.starts_with("* OK [CAPABILITY"));
    }

    #[tokio::test]
    async fn without_greeting_skips_the_banner() {
        let mut session = Session::new(Pipe::new(b"a NOOP\r\n"), peer()).without_greeting();
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(!out.contains("* OK"));
        assert!(out.contains("a OK NOOP completed"));
    }

    #[tokio::test]
    async fn capability_emits_an_untagged_line_and_a_tagged_ok() {
        let (_, out, _) = drive(b"a1 CAPABILITY\r\na2 LOGOUT\r\n").await;
        assert!(out.contains("* CAPABILITY IMAP4rev1"));
        assert!(out.contains("a1 OK CAPABILITY completed"));
    }

    #[tokio::test]
    async fn a_plaintext_login_is_refused_until_tls() {
        let (_, out, state) = drive(b"a LOGIN alice@example.com secret\r\n").await;
        assert_eq!(state, State::NotAuthenticated);
        assert!(out.contains("a NO [PRIVACYREQUIRED]"));
    }

    #[tokio::test]
    async fn a_login_over_tls_without_a_directory_is_rejected() {
        let mut session =
            Session::new(Pipe::new(b"a LOGIN alice@example.com secret\r\n"), peer()).with_tls();
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::NotAuthenticated);
        assert!(out.contains("a NO [AUTHENTICATIONFAILED]"));
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
            "irixmail-imap-auth-{}-{unique}",
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
    async fn a_login_with_valid_credentials_authenticates() {
        let (directory, path) = account_directory("secret");
        let mut session = Session::new(Pipe::new(b"a LOGIN alice@example.com secret\r\n"), peer())
            .with_tls()
            .with_directory(directory);
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::Authenticated);
        assert!(out.contains("LOGIN completed"));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_login_with_a_wrong_password_is_rejected() {
        let (directory, path) = account_directory("secret");
        let mut session = Session::new(Pipe::new(b"a LOGIN alice@example.com wrong\r\n"), peer())
            .with_tls()
            .with_directory(directory);
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::NotAuthenticated);
        assert!(out.contains("a NO [AUTHENTICATIONFAILED]"));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn repeated_failed_logins_lock_the_source() {
        let (directory, path) = account_directory("secret");
        let mut script = String::new();
        for attempt in 1..=5 {
            script.push_str(&format!("a{attempt} LOGIN alice@example.com wrong\r\n"));
        }
        script.push_str("a6 LOGIN alice@example.com secret\r\n");
        let mut session = Session::new(Pipe::new(script.as_bytes()), peer())
            .with_tls()
            .with_directory(directory);
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::NotAuthenticated);
        assert!(
            out.contains("a6 NO") && out.contains("Too many failed authentication attempts"),
            "the locked attempt should be refused, got: {out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_login_missing_a_password_is_bad() {
        let mut session = Session::new(Pipe::new(b"a LOGIN alice\r\n"), peer()).with_tls();
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("a BAD LOGIN expects a username and password"));
    }

    #[tokio::test]
    async fn authenticate_on_cleartext_is_refused() {
        let (_, out, state) = drive(b"a AUTHENTICATE PLAIN\r\n").await;
        assert_eq!(state, State::NotAuthenticated);
        assert!(out.contains("a NO [PRIVACYREQUIRED]"));
    }

    #[tokio::test]
    async fn authenticate_with_an_unsupported_mechanism_is_refused() {
        let mut session =
            Session::new(Pipe::new(b"a AUTHENTICATE CRAM-MD5\r\n"), peer()).with_tls();
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("a NO Unsupported"));
    }

    #[tokio::test]
    async fn authenticate_plain_without_a_directory_fails() {
        use base64::Engine as _;
        let payload =
            base64::engine::general_purpose::STANDARD.encode(b"\0alice@example.com\0secret");
        let script = format!("a AUTHENTICATE PLAIN {payload}\r\n");
        let mut session = Session::new(Pipe::new(script.as_bytes()), peer()).with_tls();
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert_eq!(session.state(), State::NotAuthenticated);
        assert!(out.contains("a NO [AUTHENTICATIONFAILED]"));
    }

    #[tokio::test]
    async fn authenticate_plain_challenges_then_cancels() {
        let mut session =
            Session::new(Pipe::new(b"a AUTHENTICATE PLAIN\r\n*\r\n"), peer()).with_tls();
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        assert!(out.contains("+ "));
        assert!(out.contains("a BAD authentication cancelled"));
    }

    #[tokio::test]
    async fn select_before_authentication_is_refused() {
        let (_, out, state) = drive(b"a SELECT INBOX\r\n").await;
        assert_eq!(state, State::NotAuthenticated);
        assert!(out.contains("a NO Authenticate first"));
    }

    #[tokio::test]
    async fn select_after_authentication_enters_the_selected_state() {
        let (session, out) = run_authenticated(b"b SELECT INBOX\r\n").await;
        assert_eq!(session.state(), State::Selected);
        assert_eq!(session.data().mailbox.as_deref(), Some("INBOX"));
        assert!(out.contains("* 0 EXISTS"));
        assert!(out.contains("[UIDVALIDITY"));
        assert!(out.contains("b OK [READ-WRITE] SELECT completed"));
    }

    #[tokio::test]
    async fn select_of_an_unknown_mailbox_is_refused() {
        let (session, out) = run_authenticated(b"b SELECT Nonsense\r\n").await;
        assert_eq!(session.state(), State::Authenticated);
        assert!(out.contains("b NO mailbox does not exist"));
    }

    fn account_directory_with_store(
        password: &str,
    ) -> (
        Directory,
        std::sync::Arc<dyn irixmail_store::Store>,
        Account,
        std::path::PathBuf,
    ) {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        use irixmail_core::IdGenerator;
        use irixmail_directory::{password as pw, Directory, Role};
        use irixmail_store::{RocksdbStore, Store};

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "irixmail-imap-counts-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();

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
            .set_primary_password(account.id, pw::hash(password).unwrap())
            .unwrap();
        (directory, store, account, path)
    }

    fn deliver_into(
        store: &dyn irixmail_store::Store,
        account_id: u32,
        mailbox: &Mailbox,
        document_id: u32,
        seen: bool,
    ) {
        use irixmail_mail::{Keyword, MessageData};
        use irixmail_store::{serialize, ChangeKind, ChangeLog, Collection, Key, Subspace};

        let uid = mailbox.next_uid(store, account_id).unwrap();
        let mut data = MessageData::new(1, 120);
        data.add_mailbox(mailbox.id, uid);
        if seen {
            data.add_keyword(Keyword::Seen);
        }
        let key = Key::new(
            Subspace::Property,
            account_id,
            Collection::Email,
            document_id,
        )
        .encode();
        store
            .put(&key, &serialize::archive(&data).unwrap())
            .unwrap();
        ChangeLog::new(store)
            .record(
                account_id,
                Collection::Email,
                document_id,
                ChangeKind::Insert,
            )
            .unwrap();
    }

    #[tokio::test]
    async fn select_reports_the_real_message_counts_from_the_store() {
        let (directory, store, account, path) = account_directory_with_store("secret");
        let account_id = account.id as u32;
        let inbox = provision_mailboxes(account.created_at)
            .into_iter()
            .find(|mailbox| mailbox.id == irixmail_mail::INBOX_ID)
            .unwrap();
        deliver_into(store.as_ref(), account_id, &inbox, 10, false);
        deliver_into(store.as_ref(), account_id, &inbox, 11, true);
        deliver_into(store.as_ref(), account_id, &inbox, 12, false);

        let mut session = Session::new(Pipe::new(b"b SELECT INBOX\r\n"), peer())
            .with_tls()
            .with_directory(directory);
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("* 3 EXISTS"), "{out}");
        assert!(out.contains("* 0 RECENT"), "{out}");
        assert!(out.contains("[UNSEEN 1]"), "{out}");
        assert!(out.contains("[UIDNEXT 4]"), "{out}");
        assert!(out.contains("b OK [READ-WRITE] SELECT completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn status_reports_the_real_counts_from_the_store() {
        let (directory, store, account, path) = account_directory_with_store("secret");
        let account_id = account.id as u32;
        let inbox = provision_mailboxes(account.created_at)
            .into_iter()
            .find(|mailbox| mailbox.id == irixmail_mail::INBOX_ID)
            .unwrap();
        deliver_into(store.as_ref(), account_id, &inbox, 10, false);
        deliver_into(store.as_ref(), account_id, &inbox, 11, true);
        deliver_into(store.as_ref(), account_id, &inbox, 12, false);

        let mut session = Session::new(
            Pipe::new(b"b STATUS INBOX (MESSAGES UNSEEN UIDNEXT)\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory);
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("MESSAGES 3"), "{out}");
        assert!(out.contains("UNSEEN 2"), "{out}");
        assert!(out.contains("UIDNEXT 4"), "{out}");
        assert!(out.contains("b OK STATUS completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn examine_opens_the_mailbox_read_only() {
        let (_, out) = run_authenticated(b"b EXAMINE Sent\r\n").await;
        assert!(out.contains("[PERMANENTFLAGS ()]"));
        assert!(out.contains("b OK [READ-ONLY] EXAMINE completed"));
    }

    #[tokio::test]
    async fn list_returns_the_default_folders() {
        let (_, out) = run_authenticated(b"a LIST \"\" \"*\"\r\n").await;
        assert!(out.contains("* LIST"));
        assert!(out.contains("\"INBOX\""));
        assert!(out.contains("\"Sent\""));
        assert!(out.contains("a OK LIST completed"));
    }

    #[tokio::test]
    async fn list_before_authentication_is_refused() {
        let (_, out, _) = drive(b"a LIST \"\" \"*\"\r\n").await;
        assert!(out.contains("a NO Authenticate first"));
    }

    #[tokio::test]
    async fn append_reads_the_literal_and_completes() {
        let (_, out) = run_authenticated(b"a APPEND INBOX {5}\r\nhello\r\n").await;
        assert!(out.contains("+ ready for literal data"));
        assert!(out.contains("a NO APPEND failed"), "{out}");
    }

    #[tokio::test]
    async fn append_to_an_unknown_mailbox_suggests_trycreate() {
        let (_, out) = run_authenticated(b"a APPEND Nowhere {5}\r\n").await;
        assert!(out.contains("a NO [TRYCREATE]"));
    }

    #[tokio::test]
    async fn append_without_a_literal_is_bad() {
        let (_, out) = run_authenticated(b"a APPEND INBOX\r\n").await;
        assert!(out.contains("a BAD"));
    }

    #[tokio::test]
    async fn a_non_synchronizing_append_skips_the_continuation() {
        let (_, out) = run_authenticated(b"a APPEND INBOX {5+}\r\nhello\r\n").await;
        assert!(!out.contains("+ ready for literal data"));
        assert!(out.contains("a NO APPEND failed"), "{out}");
    }

    #[tokio::test]
    async fn an_oversized_append_literal_is_refused_before_allocation() {
        let (_, out) = run_authenticated(b"a APPEND INBOX {30000000}\r\nb NOOP\r\n").await;
        assert!(out.contains("a NO [TOOBIG]"), "{out}");
        assert!(!out.contains("+ ready for literal data"), "{out}");
        assert!(out.contains("b OK"), "{out}");
    }

    #[tokio::test]
    async fn an_oversized_non_synchronizing_append_literal_closes_the_connection() {
        let (_, out) = run_authenticated(b"a APPEND INBOX {30000000+}\r\nb NOOP\r\n").await;
        assert!(out.contains("a NO [TOOBIG]"), "{out}");
        assert!(
            !out.contains("b OK"),
            "payload bytes were parsed as commands: {out}"
        );
    }

    #[tokio::test]
    async fn login_reads_literal_arguments_off_the_wire() {
        let out = drive_tls(b"a LOGIN {5}\r\nalice {6}\r\nsecret\r\n").await;
        assert_eq!(out.matches("+ ready for literal data").count(), 2);
        assert!(out.contains("a NO [AUTHENTICATIONFAILED]"));
    }

    #[tokio::test]
    async fn a_non_synchronizing_login_literal_needs_no_continuation() {
        let out = drive_tls(b"a LOGIN {5+}\r\nalice {6+}\r\nsecret\r\n").await;
        assert!(!out.contains("+ ready for literal data"));
        assert!(out.contains("a NO [AUTHENTICATIONFAILED]"));
    }

    #[tokio::test]
    async fn select_reads_a_literal_mailbox_name() {
        let (session, out) = run_authenticated(b"b SELECT {5}\r\nINBOX\r\n").await;
        assert!(out.contains("+ ready for literal data"));
        assert_eq!(session.state(), State::Selected);
        assert!(out.contains("b OK [READ-WRITE] SELECT completed"));
    }

    #[tokio::test]
    async fn an_overlong_literal_is_refused() {
        let out = drive_tls(b"a LOGIN {100000}\r\n").await;
        assert!(out.contains("a BAD literal too long"));
    }

    #[tokio::test]
    async fn fetch_before_select_is_refused() {
        let (_, out) = run_authenticated(b"b FETCH 1 FULL\r\n").await;
        assert!(out.contains("b NO Select a mailbox first"));
    }

    #[tokio::test]
    async fn fetch_on_a_selected_mailbox_completes() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc FETCH 1:* FULL\r\n").await;
        assert!(out.contains("c OK FETCH completed"));
    }

    #[tokio::test]
    async fn fetch_returns_real_flags_uid_and_size_from_the_store() {
        let (directory, store, account, path) = account_directory_with_store("secret");
        let account_id = account.id as u32;
        let inbox = provision_mailboxes(account.created_at)
            .into_iter()
            .find(|mailbox| mailbox.id == irixmail_mail::INBOX_ID)
            .unwrap();
        deliver_into(store.as_ref(), account_id, &inbox, 10, true);
        deliver_into(store.as_ref(), account_id, &inbox, 11, false);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1:* (FLAGS UID RFC822.SIZE)\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory);
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(
            out.contains("* 1 FETCH (FLAGS (\\Seen) UID 1 RFC822.SIZE 120)"),
            "{out}"
        );
        assert!(
            out.contains("* 2 FETCH (FLAGS () UID 2 RFC822.SIZE 120)"),
            "{out}"
        );
        assert!(out.contains("c OK FETCH completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn uid_fetch_targets_by_uid_and_always_includes_the_uid() {
        let (directory, store, account, path) = account_directory_with_store("secret");
        let account_id = account.id as u32;
        let inbox = provision_mailboxes(account.created_at)
            .into_iter()
            .find(|mailbox| mailbox.id == irixmail_mail::INBOX_ID)
            .unwrap();
        deliver_into(store.as_ref(), account_id, &inbox, 10, true);
        deliver_into(store.as_ref(), account_id, &inbox, 11, false);
        deliver_into(store.as_ref(), account_id, &inbox, 12, false);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc UID FETCH 2:3 (FLAGS)\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory);
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(!out.contains("* 1 FETCH"), "{out}");
        assert!(out.contains("* 2 FETCH (UID 2 FLAGS ())"), "{out}");
        assert!(out.contains("* 3 FETCH (UID 3 FLAGS ())"), "{out}");
        assert!(out.contains("c OK UID FETCH completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn fetch_returns_the_message_body_from_the_blob_store() {
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let notifier = ChangeNotifier::new();
        let mailboxes = provision_mailboxes(account.created_at);
        let raw = b"Subject: Hi\r\nFrom: a@example.com\r\n\r\nHello body\r\n";
        deliver(
            store.as_ref(),
            blobs.as_ref(),
            &notifier,
            &DeliveryRequest {
                account: &account,
                mailboxes: &mailboxes,
                mail_from: "a@example.com",
                recipient: "alice@example.com",
                document_id: 1,
                raw,
                target_override: None,
                received_at: 482_374_938,
            },
        )
        .unwrap();

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (BODY[] BODY[HEADER] BODY[TEXT])\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(
            out.contains("BODY[] {48}\r\nSubject: Hi\r\nFrom: a@example.com\r\n\r\nHello body\r\n"),
            "{out}"
        );
        assert!(
            out.contains("BODY[HEADER] {36}\r\nSubject: Hi\r\nFrom: a@example.com\r\n\r\n"),
            "{out}"
        );
        assert!(out.contains("BODY[TEXT] {12}\r\nHello body\r\n"), "{out}");
        assert!(out.contains("c OK FETCH completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn fetch_returns_the_envelope() {
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let notifier = ChangeNotifier::new();
        let mailboxes = provision_mailboxes(account.created_at);
        let raw = b"From: Alice <alice@example.com>\r\nTo: Bob <bob@example.org>\r\nSubject: Hello\r\nMessage-ID: <abc@example.com>\r\n\r\nBody\r\n";
        deliver(
            store.as_ref(),
            blobs.as_ref(),
            &notifier,
            &DeliveryRequest {
                account: &account,
                mailboxes: &mailboxes,
                mail_from: "alice@example.com",
                recipient: "alice@example.com",
                document_id: 1,
                raw,
                target_override: None,
                received_at: 482_374_938,
            },
        )
        .unwrap();

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (ENVELOPE)\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("* 1 FETCH (ENVELOPE ("), "{out}");
        assert!(out.contains("\"Hello\""), "{out}");
        assert!(
            out.contains("((\"Alice\" NIL \"alice\" \"example.com\"))"),
            "{out}"
        );
        assert!(
            out.contains("((\"Bob\" NIL \"bob\" \"example.org\"))"),
            "{out}"
        );
        assert!(out.contains("\"<abc@example.com>\""), "{out}");
        assert!(out.contains("c OK FETCH completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn fetch_returns_the_internaldate_from_the_stored_timestamp() {
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let notifier = ChangeNotifier::new();
        let mailboxes = provision_mailboxes(account.created_at);
        let raw = b"Subject: Hi\r\n\r\nbody\r\n";
        deliver(
            store.as_ref(),
            blobs.as_ref(),
            &notifier,
            &DeliveryRequest {
                account: &account,
                mailboxes: &mailboxes,
                mail_from: "a@example.com",
                recipient: "alice@example.com",
                document_id: 1,
                raw,
                target_override: None,
                received_at: 482_374_938,
            },
        )
        .unwrap();

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (INTERNALDATE)\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(
            out.contains("* 1 FETCH (INTERNALDATE \"15-Apr-1985 01:02:18 +0000\")"),
            "{out}"
        );
        assert!(out.contains("c OK FETCH completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn fetch_returns_the_body_structure() {
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let notifier = ChangeNotifier::new();
        let mailboxes = provision_mailboxes(account.created_at);
        let raw = b"From: a@example.com\r\nSubject: Hi\r\n\r\nHello body\r\n";
        deliver(
            store.as_ref(),
            blobs.as_ref(),
            &notifier,
            &DeliveryRequest {
                account: &account,
                mailboxes: &mailboxes,
                mail_from: "a@example.com",
                recipient: "alice@example.com",
                document_id: 1,
                raw,
                target_override: None,
                received_at: 482_374_938,
            },
        )
        .unwrap();

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (BODY BODYSTRUCTURE)\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(
            out.contains("BODY (\"text\" \"plain\" NIL NIL NIL \"7bit\" 12 1)"),
            "{out}"
        );
        assert!(
            out.contains(
                "BODYSTRUCTURE (\"text\" \"plain\" NIL NIL NIL \"7bit\" 12 1 NIL NIL NIL NIL)"
            ),
            "{out}"
        );
        assert!(out.contains("c OK FETCH completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn fetch_with_a_bad_sequence_set_is_rejected() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc FETCH bogus FULL\r\n").await;
        assert!(out.contains("c BAD FETCH expects a sequence set"));
    }

    #[tokio::test]
    async fn uid_fetch_on_a_selected_mailbox_completes() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc UID FETCH 1:* (FLAGS)\r\n").await;
        assert!(out.contains("c OK UID FETCH completed"));
    }

    #[tokio::test]
    async fn uid_search_returns_an_empty_result_and_completes() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc UID SEARCH ALL\r\n").await;
        assert!(out.contains("* SEARCH"));
        assert!(out.contains("c OK UID SEARCH completed"));
    }

    async fn search_session(script: &'static [u8]) -> String {
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let notifier = Arc::new(ChangeNotifier::new());
        let mailboxes = provision_mailboxes(account.created_at);
        let large = format!(
            "Subject: Cherry\r\nFrom: c@x.com\r\n\r\ninvoice {}\r\n",
            "x".repeat(120)
        );
        let messages: [&[u8]; 3] = [
            b"Subject: Apple\r\nFrom: a@x.com\r\n\r\ninvoice one\r\n",
            b"Subject: Banana\r\nFrom: b@x.com\r\n\r\nhello world\r\n",
            large.as_bytes(),
        ];
        for (index, raw) in messages.iter().enumerate() {
            let document_id = index as u32 + 1;
            deliver(
                store.as_ref(),
                blobs.as_ref(),
                &notifier,
                &DeliveryRequest {
                    account: &account,
                    mailboxes: &mailboxes,
                    mail_from: "x@x.com",
                    recipient: "alice@example.com",
                    document_id,
                    raw,
                    target_override: None,
                    received_at: 1_700_000_000,
                },
            )
            .unwrap();
        }

        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(Arc::clone(&blobs))
            .with_notifier(Arc::clone(&notifier));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        let _ = std::fs::remove_dir_all(&path);
        out
    }

    #[tokio::test]
    async fn search_finds_all_messages_and_text_matches() {
        let out =
            search_session(b"b SELECT INBOX\r\nc SEARCH ALL\r\nd SEARCH TEXT invoice\r\n").await;
        assert!(out.contains("* SEARCH 1 2 3\r\n"), "{out}");
        assert!(out.contains("* SEARCH 1 3\r\n"), "{out}");
        assert!(out.contains("c OK SEARCH completed"), "{out}");
        assert!(out.contains("d OK SEARCH completed"), "{out}");
    }

    #[tokio::test]
    async fn search_filters_by_flag_after_a_store() {
        let out = search_session(
            b"b SELECT INBOX\r\nc STORE 2 +FLAGS (\\Seen)\r\nd SEARCH SEEN\r\ne SEARCH UNSEEN\r\n",
        )
        .await;
        assert!(out.contains("* SEARCH 2\r\n"), "{out}");
        assert!(out.contains("* SEARCH 1 3\r\n"), "{out}");
        assert!(out.contains("e OK SEARCH completed"), "{out}");
    }

    #[tokio::test]
    async fn search_combines_not_and_or() {
        let out = search_session(
            b"b SELECT INBOX\r\nc SEARCH NOT TEXT invoice\r\nd SEARCH OR TEXT apple TEXT banana\r\n",
        )
        .await;
        assert!(out.contains("* SEARCH 2\r\n"), "{out}");
        assert!(out.contains("* SEARCH 1 2\r\n"), "{out}");
    }

    #[tokio::test]
    async fn search_field_scoped_text_matches_only_the_named_field() {
        let out = search_session(
            b"b SELECT INBOX\r\nc SEARCH SUBJECT Apple\r\nd SEARCH FROM Apple\r\ne SEARCH BODY invoice\r\nf SEARCH SUBJECT invoice\r\ng SEARCH TEXT invoice\r\n",
        )
        .await;
        assert!(
            out.contains("* SEARCH 1\r\nc OK SEARCH completed"),
            "SUBJECT Apple: {out}"
        );
        assert!(
            out.contains("* SEARCH\r\nd OK SEARCH completed"),
            "FROM Apple empty: {out}"
        );
        assert!(
            out.contains("* SEARCH 1 3\r\ne OK SEARCH completed"),
            "BODY invoice: {out}"
        );
        assert!(
            out.contains("* SEARCH\r\nf OK SEARCH completed"),
            "SUBJECT invoice empty: {out}"
        );
        assert!(
            out.contains("* SEARCH 1 3\r\ng OK SEARCH completed"),
            "TEXT invoice combined: {out}"
        );
    }

    #[test]
    fn header_matches_unfolds_continuations_and_treats_empty_needle_as_presence() {
        let raw = b"Subject: hello\r\nX-Long: part one\r\n  part two\r\nFrom: a@x.com\r\n";
        assert!(header_matches(raw, "x-long", "part two"));
        assert!(header_matches(raw, "Subject", ""));
        assert!(!header_matches(raw, "Subject", "world"));
        assert!(!header_matches(raw, "X-Absent", ""));
    }

    #[tokio::test]
    async fn search_header_substring_matches_the_named_header() {
        let out = search_session(
            b"b SELECT INBOX\r\nc SEARCH HEADER Subject Apple\r\nd SEARCH HEADER From b\r\ne SEARCH HEADER Subject an\r\nf SEARCH HEADER X-Missing nope\r\n",
        )
        .await;
        assert!(
            out.contains("* SEARCH 1\r\nc OK SEARCH completed"),
            "HEADER Subject Apple: {out}"
        );
        assert!(
            out.contains("* SEARCH 2\r\nd OK SEARCH completed"),
            "HEADER From b: {out}"
        );
        assert!(
            out.contains("* SEARCH 2\r\ne OK SEARCH completed"),
            "HEADER Subject an substring: {out}"
        );
        assert!(
            out.contains("* SEARCH\r\nf OK SEARCH completed"),
            "HEADER X-Missing empty: {out}"
        );
    }

    #[tokio::test]
    async fn search_filters_by_size() {
        let out =
            search_session(b"b SELECT INBOX\r\nc SEARCH LARGER 100\r\nd SEARCH SMALLER 100\r\n")
                .await;
        assert!(out.contains("* SEARCH 3\r\n"), "{out}");
        assert!(out.contains("* SEARCH 1 2\r\n"), "{out}");
    }

    #[tokio::test]
    async fn uid_search_reports_uids() {
        let out = search_session(b"b SELECT INBOX\r\nc UID SEARCH TEXT invoice\r\n").await;
        assert!(out.contains("* SEARCH 1 3\r\n"), "{out}");
        assert!(out.contains("c OK UID SEARCH completed"), "{out}");
    }

    #[tokio::test]
    async fn an_unsupported_search_criterion_is_rejected() {
        let out = search_session(b"b SELECT INBOX\r\nc SEARCH BEFORE not-a-date\r\n").await;
        assert!(out.contains("c BAD"), "{out}");
    }

    async fn custom_search_session(messages: &[&[u8]], script: &'static [u8]) -> String {
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
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
                    mail_from: "x@x.com",
                    recipient: "alice@example.com",
                    document_id,
                    raw,
                    target_override: None,
                    received_at: 1_700_000_000,
                },
            )
            .unwrap();
        }

        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(Arc::clone(&blobs))
            .with_notifier(Arc::clone(&notifier));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        let _ = std::fs::remove_dir_all(&path);
        out
    }

    #[tokio::test]
    async fn multiword_search_matches_only_a_contiguous_phrase() {
        let out = custom_search_session(
            &[
                b"Subject: Alpha\r\nFrom: a@x.com\r\n\r\nworld greets hello\r\n",
                b"Subject: Beta\r\nFrom: b@x.com\r\n\r\nsay hello world now\r\n",
                b"Subject: hello world\r\nFrom: c@x.com\r\n\r\nnothing here\r\n",
                b"Subject: world and hello\r\nFrom: d@x.com\r\n\r\nnothing there\r\n",
            ],
            b"b SELECT INBOX\r\nc SEARCH TEXT \"hello world\"\r\nd SEARCH SUBJECT \"hello world\"\r\n",
        )
        .await;
        assert!(
            out.contains("* SEARCH 2 3\r\nc OK SEARCH completed"),
            "TEXT phrase: {out}"
        );
        assert!(
            out.contains("* SEARCH 3\r\nd OK SEARCH completed"),
            "SUBJECT phrase: {out}"
        );
    }

    #[tokio::test]
    async fn dateless_messages_do_not_match_sent_date_criteria() {
        let out = search_session(
            b"b SELECT INBOX\r\nc SEARCH SENTBEFORE 1-Jan-2030\r\nd SEARCH SENTSINCE 1-Jan-2000\r\ne SEARCH SENTON 1-Jan-2020\r\n",
        )
        .await;
        assert!(
            out.contains("* SEARCH\r\nc OK SEARCH completed"),
            "SENTBEFORE: {out}"
        );
        assert!(
            out.contains("* SEARCH\r\nd OK SEARCH completed"),
            "SENTSINCE: {out}"
        );
        assert!(
            out.contains("* SEARCH\r\ne OK SEARCH completed"),
            "SENTON: {out}"
        );
    }

    #[tokio::test]
    async fn an_unsupported_search_charset_is_refused_with_badcharset() {
        let out = search_session(
            b"b SELECT INBOX\r\nc SEARCH CHARSET KOI8-R ALL\r\nd SEARCH CHARSET UTF-8 ALL\r\ne UID SEARCH CHARSET KOI8-R ALL\r\nf SEARCH CHARSET US-ASCII ALL\r\n",
        )
        .await;
        assert!(out.contains("c NO [BADCHARSET (US-ASCII UTF-8)]"), "{out}");
        assert!(
            out.contains("* SEARCH 1 2 3\r\nd OK SEARCH completed"),
            "{out}"
        );
        assert!(out.contains("e NO [BADCHARSET (US-ASCII UTF-8)]"), "{out}");
        assert!(
            out.contains("* SEARCH 1 2 3\r\nf OK SEARCH completed"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn recent_new_and_old_reflect_an_always_empty_recent_set() {
        let out = search_session(
            b"b SELECT INBOX\r\nc STORE 1 +FLAGS (\\Recent)\r\nd SEARCH RECENT\r\ne SEARCH NEW\r\nf SEARCH OLD\r\n",
        )
        .await;
        assert!(
            out.contains("* SEARCH\r\nd OK SEARCH completed"),
            "RECENT empty: {out}"
        );
        assert!(
            out.contains("* SEARCH\r\ne OK SEARCH completed"),
            "NEW empty: {out}"
        );
        assert!(
            out.contains("* SEARCH 1 2 3\r\nf OK SEARCH completed"),
            "OLD is all: {out}"
        );
    }

    async fn dated_search(script: &'static [u8]) -> String {
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let notifier = Arc::new(ChangeNotifier::new());
        let mailboxes = provision_mailboxes(account.created_at);
        let messages: [(&[u8], u64); 3] = [
            (b"Subject: Jan\r\nFrom: a@x.com\r\nDate: Fri, 01 Jan 2021 00:00:00 +0000\r\n\r\nfirst\r\n", 1_577_836_800),
            (b"Subject: Feb\r\nFrom: b@x.com\r\nDate: Mon, 01 Feb 2021 00:00:00 +0000\r\n\r\nsecond\r\n", 1_580_515_200),
            (b"Subject: Mar\r\nFrom: c@x.com\r\nDate: Mon, 01 Mar 2021 00:00:00 +0000\r\n\r\nthird\r\n", 1_583_020_800),
        ];
        for (index, (raw, received_at)) in messages.iter().enumerate() {
            let document_id = index as u32 + 1;
            deliver(
                store.as_ref(),
                blobs.as_ref(),
                &notifier,
                &DeliveryRequest {
                    account: &account,
                    mailboxes: &mailboxes,
                    mail_from: "x@x.com",
                    recipient: "alice@example.com",
                    document_id,
                    raw,
                    target_override: None,
                    received_at: *received_at,
                },
            )
            .unwrap();
        }

        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(Arc::clone(&blobs))
            .with_notifier(Arc::clone(&notifier));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        let _ = std::fs::remove_dir_all(&path);
        out
    }

    #[tokio::test]
    async fn search_since_before_on_filter_by_received_date() {
        let out = dated_search(
            b"b SELECT INBOX\r\nc SEARCH SINCE 1-Feb-2020\r\nd SEARCH BEFORE 1-Feb-2020\r\ne SEARCH ON 1-Feb-2020\r\n",
        )
        .await;
        assert!(out.contains("* SEARCH 2 3\r\n"), "SINCE: {out}");
        assert!(out.contains("* SEARCH 1\r\n"), "BEFORE: {out}");
        assert!(out.contains("* SEARCH 2\r\n"), "ON: {out}");
    }

    #[tokio::test]
    async fn search_sent_criteria_filter_by_the_date_header() {
        let out = dated_search(
            b"b SELECT INBOX\r\nc SEARCH SENTSINCE 1-Feb-2021\r\nd SEARCH SENTBEFORE 1-Feb-2021\r\ne SEARCH SENTON 1-Feb-2021\r\n",
        )
        .await;
        assert!(out.contains("* SEARCH 2 3\r\n"), "SENTSINCE: {out}");
        assert!(out.contains("* SEARCH 1\r\n"), "SENTBEFORE: {out}");
        assert!(out.contains("* SEARCH 2\r\n"), "SENTON: {out}");
    }

    #[tokio::test]
    async fn copy_files_the_message_into_the_target_and_reports_copyuid() {
        let out =
            search_session(b"b SELECT INBOX\r\nc COPY 1 Sent\r\nd SELECT Sent\r\ne SEARCH ALL\r\n")
                .await;
        assert!(out.contains("c OK [COPYUID"), "{out}");
        assert!(out.contains("COPY completed"), "{out}");
        assert!(out.contains("* SEARCH 1\r\n"), "{out}");
    }

    #[tokio::test]
    async fn copy_leaves_the_source_message_in_place() {
        let out = search_session(b"b SELECT INBOX\r\nc COPY 1 Sent\r\nd SEARCH ALL\r\n").await;
        assert!(out.contains("* SEARCH 1 2 3\r\n"), "{out}");
    }

    #[tokio::test]
    async fn move_relocates_the_message_with_expunge_and_copyuid() {
        let out = search_session(b"b SELECT INBOX\r\nc MOVE 1 Trash\r\nd SEARCH ALL\r\n").await;
        assert!(out.contains("* OK [COPYUID"), "{out}");
        assert!(out.contains("* 1 EXPUNGE\r\n"), "{out}");
        assert!(out.contains("c OK MOVE completed"), "{out}");
        assert!(out.contains("* SEARCH 1 2\r\n"), "{out}");
    }

    #[tokio::test]
    async fn a_uid_move_into_the_target_completes_and_expunges() {
        let out = search_session(b"b SELECT INBOX\r\nc UID MOVE 2 Trash\r\nd SEARCH ALL\r\n").await;
        assert!(out.contains("* 2 EXPUNGE\r\n"), "{out}");
        assert!(out.contains("c OK UID MOVE completed"), "{out}");
        assert!(out.contains("* SEARCH 1 2\r\n"), "{out}");
    }

    #[tokio::test]
    async fn a_move_into_the_selected_mailbox_is_refused_and_loses_nothing() {
        let out = search_session(
            b"b SELECT INBOX\r\nc UID MOVE 1 INBOX\r\nd SEARCH ALL\r\ne MOVE 1 INBOX\r\nf SEARCH ALL\r\n",
        )
        .await;
        assert!(out.contains("c NO [CANNOT]"), "{out}");
        assert!(out.contains("e NO [CANNOT]"), "{out}");
        assert_eq!(out.matches("* SEARCH 1 2 3\r\n").count(), 2, "{out}");
    }

    #[tokio::test]
    async fn a_copy_into_the_selected_mailbox_is_refused() {
        let out = search_session(b"b SELECT INBOX\r\nc COPY 1 INBOX\r\nd SEARCH ALL\r\n").await;
        assert!(out.contains("c NO [CANNOT]"), "{out}");
        assert!(out.contains("* SEARCH 1 2 3\r\n"), "{out}");
    }

    fn expunge_env(
        messages: &[&[u8]],
    ) -> (
        Directory,
        std::sync::Arc<dyn irixmail_store::Store>,
        Arc<dyn irixmail_store::BlobStore>,
        Account,
        u32,
        std::path::PathBuf,
    ) {
        use irixmail_mail::{deliver, DeliveryRequest};
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, store, account, path) = account_directory_with_store("secret");
        let account_id = account.id as u32;
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let notifier = ChangeNotifier::new();
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
                    mail_from: "a@example.com",
                    recipient: "alice@example.com",
                    document_id,
                    raw,
                    target_override: None,
                    received_at: 1_700_000_000,
                },
            )
            .unwrap();
        }
        (directory, store, blobs, account, account_id, path)
    }

    async fn drive_seeded(script: &[u8]) -> (String, std::path::PathBuf) {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, account, _account_id, path) = expunge_env(&[
            b"Subject: One\r\n\r\nbody one\r\n",
            b"Subject: Two\r\n\r\nbody two\r\n",
        ]);
        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        (out, path)
    }

    async fn drive_three(script: &[u8]) -> (String, std::path::PathBuf) {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, account, _account_id, path) = expunge_env(&[
            concat!(
                "From: carol@example.com\r\n",
                "Subject: zebra\r\n",
                "Message-ID: <one@example.com>\r\n",
                "Date: Sat, 01 Feb 2020 00:00:00 +0000\r\n",
                "\r\n",
                "a much much much longer body than the other two messages\r\n",
            )
            .as_bytes(),
            concat!(
                "From: alice@example.com\r\n",
                "Subject: unrelated\r\n",
                "Message-ID: <two@example.com>\r\n",
                "Date: Sat, 01 Feb 2021 00:00:00 +0000\r\n",
                "\r\n",
                "short\r\n",
            )
            .as_bytes(),
            concat!(
                "From: bob@example.com\r\n",
                "Subject: Re: zebra\r\n",
                "Message-ID: <three@example.com>\r\n",
                "In-Reply-To: <one@example.com>\r\n",
                "Date: Sat, 01 Feb 2022 00:00:00 +0000\r\n",
                "\r\n",
                "middle len\r\n",
            )
            .as_bytes(),
        ]);
        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        let _ = std::fs::remove_dir_all(&path);
        (out, path)
    }

    async fn drive_with_quota(script: &[u8], quota_bytes: u64, quota_messages: u64) -> String {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, mut account, _account_id, path) = expunge_env(&[
            b"Subject: One\r\n\r\nbody one\r\n",
            b"Subject: Two\r\n\r\nbody two\r\n",
        ]);
        account.quota_bytes = quota_bytes;
        account.quota_messages = quota_messages;
        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        let _ = std::fs::remove_dir_all(&path);
        out
    }

    #[tokio::test]
    async fn list_return_status_interleaves_status_lines() {
        let (out, path) =
            drive_seeded(b"b LIST \"\" * RETURN (STATUS (MESSAGES UNSEEN))\r\n").await;
        let inbox_list = out.find("\"INBOX\"").expect("inbox listed");
        let inbox_status = out
            .find("* STATUS \"INBOX\" (MESSAGES 2 UNSEEN 2)")
            .expect("inbox status");
        assert!(inbox_status > inbox_list, "{out}");
        assert!(
            out.contains("* STATUS \"Sent\" (MESSAGES 0 UNSEEN 0)"),
            "{out}"
        );
        assert!(out.contains("b OK LIST completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn list_subscribed_selection_lists_only_subscribed_mailboxes() {
        let (out, path) = drive_seeded(b"a SUBSCRIBE Sent\r\nb LIST (SUBSCRIBED) \"\" *\r\n").await;
        let tail = out.split("a OK").nth(1).unwrap_or("");
        assert!(tail.contains("\"Sent\""), "{out}");
        assert!(tail.contains("\\Subscribed"), "{out}");
        assert!(!tail.contains("\"INBOX\""), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn list_accepts_multiple_patterns() {
        let (out, path) = drive_seeded(b"b LIST \"\" (INBOX Sent)\r\n").await;
        assert!(out.contains("\"INBOX\""), "{out}");
        assert!(out.contains("\"Sent\""), "{out}");
        assert!(!out.contains("\"Drafts\""), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn recursivematch_requires_subscribed() {
        let (out, path) = drive_seeded(b"b LIST (RECURSIVEMATCH) \"\" *\r\n").await;
        assert!(out.contains("b BAD"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn special_use_selection_filters_plain_mailboxes() {
        let (out, path) = drive_seeded(b"b LIST (SPECIAL-USE) \"\" *\r\n").await;
        assert!(out.contains("\"Sent\""), "{out}");
        assert!(!out.contains("\"INBOX\""), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn recursivematch_reports_a_parent_with_a_subscribed_child() {
        let (out, path) = drive_seeded(
            b"a CREATE Projects/Deep\r\nb SUBSCRIBE Projects/Deep\r\nc LIST (SUBSCRIBED RECURSIVEMATCH) \"\" %\r\n",
        )
        .await;
        assert!(
            out.contains("\"Projects\" (\"CHILDINFO\" (\"SUBSCRIBED\"))"),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn getquotaroot_reports_storage_and_message_usage() {
        let out = drive_with_quota(b"b GETQUOTAROOT INBOX\r\n", 100 * 1024, 10).await;
        assert!(out.contains("* QUOTAROOT \"INBOX\" \"\"\r\n"), "{out}");
        assert!(
            out.contains("* QUOTA \"\" (STORAGE 0 100 MESSAGE 2 10)\r\n"),
            "{out}"
        );
        assert!(out.contains("b OK GETQUOTAROOT completed"), "{out}");
    }

    #[tokio::test]
    async fn getquota_reports_the_root_quota() {
        let out = drive_with_quota(b"b GETQUOTA \"\"\r\n", 0, 10).await;
        assert!(out.contains("* QUOTA \"\" (MESSAGE 2 10)\r\n"), "{out}");
        assert!(out.contains("b OK GETQUOTA completed"), "{out}");
    }

    #[tokio::test]
    async fn an_unlimited_account_reports_an_empty_quota_list() {
        let out = drive_with_quota(b"b GETQUOTA \"\"\r\n", 0, 0).await;
        assert!(out.contains("* QUOTA \"\" ()\r\n"), "{out}");
    }

    #[tokio::test]
    async fn an_unknown_quota_root_is_refused() {
        let out = drive_with_quota(b"b GETQUOTA bogus\r\n", 0, 0).await;
        assert!(out.contains("b NO"), "{out}");
    }

    #[tokio::test]
    async fn getquotaroot_on_a_missing_mailbox_is_refused() {
        let out = drive_with_quota(b"b GETQUOTAROOT NoSuch\r\n", 0, 0).await;
        assert!(out.contains("b NO"), "{out}");
    }

    #[tokio::test]
    async fn multiappend_stores_every_message_under_one_appenduid() {
        let (out, path) = drive_seeded(
            b"a APPEND INBOX {14+}\r\nSubject: x\r\n\r\n {14+}\r\nSubject: y\r\n\r\n\r\nb SELECT INBOX\r\n",
        )
        .await;
        assert!(out.contains("[APPENDUID"), "{out}");
        assert!(out.contains("3:4"), "{out}");
        assert!(out.contains("* 4 EXISTS"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn multiappend_applies_flags_and_date_per_message() {
        let (out, path) = drive_seeded(
            b"a APPEND INBOX {14+}\r\nSubject: x\r\n\r\n (\\Seen) \"15-Apr-1985 01:02:18 +0000\" {14+}\r\nSubject: y\r\n\r\n\r\nb SELECT INBOX\r\nc FETCH 4 (FLAGS INTERNALDATE)\r\n",
        )
        .await;
        assert!(out.contains("\\Seen"), "{out}");
        assert!(out.contains("15-Apr-1985"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_bad_multiappend_group_drains_the_remaining_literal() {
        let (out, path) = drive_seeded(
            b"a APPEND INBOX {14+}\r\nSubject: x\r\n\r\n \"not a date\" {9+}\r\na9 NOOP\r\n\r\na2 NOOP\r\n",
        )
        .await;
        assert!(out.contains("a BAD"), "{out}");
        assert!(!out.contains("a9 OK"), "{out}");
        assert!(out.contains("a2 OK"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn sort_by_size_orders_the_smallest_first() {
        let (out, _) = drive_three(b"b SELECT INBOX\r\nc SORT (SIZE) UTF-8 ALL\r\n").await;
        assert!(out.contains("* SORT 2 3 1\r\n"), "{out}");
        assert!(out.contains("c OK SORT completed"), "{out}");
    }

    #[tokio::test]
    async fn reverse_date_sorts_the_newest_first() {
        let (out, _) = drive_three(b"b SELECT INBOX\r\nc SORT (REVERSE DATE) UTF-8 ALL\r\n").await;
        assert!(out.contains("* SORT 3 2 1\r\n"), "{out}");
    }

    #[tokio::test]
    async fn uid_sort_by_from_orders_by_sender() {
        let (out, _) = drive_three(b"b SELECT INBOX\r\nc UID SORT (FROM) UTF-8 ALL\r\n").await;
        assert!(out.contains("* SORT 2 3 1\r\n"), "{out}");
        assert!(out.contains("c OK UID SORT completed"), "{out}");
    }

    #[tokio::test]
    async fn subject_sort_ignores_reply_prefixes() {
        let (out, _) = drive_three(b"b SELECT INBOX\r\nc SORT (SUBJECT DATE) UTF-8 ALL\r\n").await;
        assert!(out.contains("* SORT 2 1 3\r\n"), "{out}");
    }

    #[tokio::test]
    async fn an_unknown_sort_charset_is_refused() {
        let (out, _) = drive_three(b"b SELECT INBOX\r\nc SORT (SIZE) KOI8-R ALL\r\n").await;
        assert!(out.contains("c NO [BADCHARSET"), "{out}");
    }

    #[tokio::test]
    async fn thread_references_groups_a_reply_with_its_parent() {
        let (out, _) = drive_three(b"b SELECT INBOX\r\nc THREAD REFERENCES UTF-8 ALL\r\n").await;
        assert!(out.contains("* THREAD (1 3)(2)\r\n"), "{out}");
        assert!(out.contains("c OK THREAD completed"), "{out}");
    }

    #[tokio::test]
    async fn thread_orderedsubject_groups_by_base_subject() {
        let (out, _) =
            drive_three(b"b SELECT INBOX\r\nc UID THREAD ORDEREDSUBJECT UTF-8 ALL\r\n").await;
        assert!(out.contains("* THREAD (1 3)(2)\r\n"), "{out}");
    }

    #[tokio::test]
    async fn search_return_emits_an_esearch_response() {
        let (out, path) =
            drive_seeded(b"b SELECT INBOX\r\nc SEARCH RETURN (ALL COUNT) ALL\r\n").await;
        assert!(
            out.contains("* ESEARCH (TAG \"c\") COUNT 2 ALL 1:2"),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn uid_search_return_min_max_reports_uids() {
        let (out, path) =
            drive_seeded(b"b SELECT INBOX\r\nc UID SEARCH RETURN (MIN MAX) ALL\r\n").await;
        assert!(
            out.contains("* ESEARCH (TAG \"c\") UID MIN 1 MAX 2"),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn an_empty_return_option_list_defaults_to_all() {
        let (out, path) = drive_seeded(b"b SELECT INBOX\r\nc SEARCH RETURN () ALL\r\n").await;
        assert!(out.contains("* ESEARCH (TAG \"c\") ALL 1:2"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_saved_search_substitutes_into_uid_fetch() {
        let (out, path) = drive_seeded(
            b"b SELECT INBOX\r\nc UID SEARCH RETURN (SAVE) UID 2\r\nd UID FETCH $ (FLAGS)\r\n",
        )
        .await;
        let tail = out.split("c OK").nth(1).unwrap_or("");
        assert!(tail.contains("UID 2"), "{out}");
        assert!(!tail.contains("UID 1 "), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_saved_search_substitutes_into_a_seq_mode_fetch() {
        let (out, path) = drive_seeded(
            b"b SELECT INBOX\r\nc UID SEARCH RETURN (SAVE) UID 2\r\nd FETCH $ (FLAGS)\r\n",
        )
        .await;
        let tail = out.split("c OK").nth(1).unwrap_or("");
        assert!(tail.contains("* 2 FETCH"), "{out}");
        assert!(!tail.contains("* 1 FETCH"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_saved_search_substitutes_into_search_criteria() {
        let (out, path) = drive_seeded(
            b"b SELECT INBOX\r\nc UID SEARCH RETURN (SAVE) UID 2\r\nd UID SEARCH $\r\n",
        )
        .await;
        assert!(out.contains("* SEARCH 2\r\n"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn an_unset_saved_search_matches_nothing() {
        let (out, path) = drive_seeded(b"b SELECT INBOX\r\nc UID FETCH $ (FLAGS)\r\n").await;
        let tail = out.split("b OK").nth(1).unwrap_or("");
        assert!(!tail.contains("FETCH ("), "{out}");
        assert!(out.contains("c OK UID FETCH completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn enable_reports_the_enabled_extensions() {
        let (_, out) = run_authenticated(b"a ENABLE CONDSTORE QRESYNC\r\nb LOGOUT\r\n").await;
        assert!(out.contains("* ENABLED CONDSTORE QRESYNC\r\n"), "{out}");
        assert!(out.contains("a OK ENABLE completed"), "{out}");
    }

    #[tokio::test]
    async fn enable_ignores_unknown_extensions() {
        let (_, out) = run_authenticated(b"a ENABLE X-BOGUS CONDSTORE\r\nb LOGOUT\r\n").await;
        assert!(out.contains("* ENABLED CONDSTORE\r\n"), "{out}");
        assert!(!out.contains("X-BOGUS"), "{out}");
    }

    #[tokio::test]
    async fn select_reports_the_highest_modseq() {
        let (out, path) = drive_seeded(b"b SELECT INBOX\r\n").await;
        assert!(out.contains("* OK [HIGHESTMODSEQ 2]"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn status_reports_the_highest_modseq() {
        let (out, path) = drive_seeded(b"b STATUS INBOX (HIGHESTMODSEQ)\r\n").await;
        assert!(out.contains("(HIGHESTMODSEQ 2)"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn fetch_reports_a_per_message_modseq() {
        let (out, path) = drive_seeded(b"b SELECT INBOX\r\nc FETCH 1 (MODSEQ)\r\n").await;
        assert!(out.contains("* 1 FETCH (MODSEQ (1))"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn uid_fetch_changedsince_returns_only_changed_messages() {
        let (out, path) = drive_seeded(
            b"b SELECT INBOX\r\nc STORE 2 +FLAGS.SILENT (\\Seen)\r\nd UID FETCH 1:* (FLAGS) (CHANGEDSINCE 2)\r\n",
        )
        .await;
        let tail = out.split("c OK").nth(1).unwrap_or("");
        assert!(tail.contains("UID 2"), "{out}");
        assert!(tail.contains("MODSEQ (3)"), "{out}");
        assert!(!tail.contains("UID 1 "), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn uid_fetch_vanished_reports_earlier_expunges() {
        let (out, path) = drive_seeded(
            b"a ENABLE QRESYNC\r\nb SELECT INBOX\r\nc STORE 1 +FLAGS.SILENT (\\Deleted)\r\nd EXPUNGE\r\ne UID FETCH 1:* (FLAGS) (CHANGEDSINCE 2 VANISHED)\r\n",
        )
        .await;
        assert!(out.contains("* VANISHED (EARLIER) 1"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn store_unchangedsince_reports_modified_conflicts() {
        let (out, path) = drive_seeded(
            b"b SELECT INBOX\r\nc STORE 2 +FLAGS.SILENT (\\Seen)\r\nd STORE 1:2 (UNCHANGEDSINCE 2) +FLAGS (\\Flagged)\r\n",
        )
        .await;
        assert!(out.contains("d OK [MODIFIED 2]"), "{out}");
        assert!(out.contains("* 1 FETCH ("), "{out}");
        assert!(out.contains("MODSEQ (4)"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn expunge_emits_vanished_when_qresync_is_enabled() {
        let (out, path) = drive_seeded(
            b"a ENABLE QRESYNC\r\nb SELECT INBOX\r\nc STORE 1 +FLAGS.SILENT (\\Deleted)\r\nd EXPUNGE\r\n",
        )
        .await;
        assert!(out.contains("* VANISHED 1\r\n"), "{out}");
        assert!(!out.contains("* 1 EXPUNGE"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_qresync_select_replays_changes_since_the_client_modseq() {
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) = expunge_env(&[
            b"Subject: One\r\n\r\nbody one\r\n",
            b"Subject: Two\r\n\r\nbody two\r\n",
        ]);
        let notifier = ChangeNotifier::new();
        delete_message(store.as_ref(), blobs.as_ref(), &notifier, account_id, 1).unwrap();
        update_message(store.as_ref(), &notifier, account_id, 2, |data| {
            data.add_keyword(Keyword::Seen);
            Ok(())
        })
        .unwrap();

        let uidvalidity = assign_uid_validity(account.created_at);
        let script = format!("a ENABLE QRESYNC\r\nb SELECT INBOX (QRESYNC ({uidvalidity} 2))\r\n");
        let mut session = Session::new(Pipe::new(script.as_bytes()), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(notifier));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("* VANISHED (EARLIER) 1"), "{out}");
        assert!(out.contains("UID 2"), "{out}");
        assert!(out.contains("MODSEQ (5)"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn search_modseq_matches_recently_changed_messages() {
        let (out, path) = drive_seeded(
            b"b SELECT INBOX\r\nc STORE 2 +FLAGS.SILENT (\\Seen)\r\nd SEARCH MODSEQ 3\r\n",
        )
        .await;
        assert!(out.contains("* SEARCH 2 (MODSEQ 3)"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn expunge_destroys_deleted_messages_and_frees_their_storage() {
        use irixmail_mail::load_data;
        use irixmail_store::{ChangeNotifier, Quota};

        let (directory, store, blobs, account, account_id, path) = expunge_env(&[
            b"Subject: One\r\n\r\nbody one\r\n",
            b"Subject: Two\r\n\r\nbody two\r\n",
        ]);

        let mut session = Session::new(
            Pipe::new(
                b"b SELECT INBOX\r\nc STORE 1 +FLAGS (\\Deleted)\r\nd EXPUNGE\r\ne SEARCH TEXT one\r\nf SEARCH TEXT two\r\n",
            ),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("* 1 EXPUNGE\r\n"), "{out}");
        assert!(out.contains("d OK EXPUNGE completed"), "{out}");
        assert!(
            out.contains("* SEARCH\r\n"),
            "the unindexed term should miss: {out}"
        );
        assert!(
            out.contains("* SEARCH 1\r\n"),
            "the survivor stays searchable: {out}"
        );

        assert!(
            load_data(store.as_ref(), account_id, 1).unwrap().is_none(),
            "doc 1 destroyed"
        );
        assert!(
            load_data(store.as_ref(), account_id, 2).unwrap().is_some(),
            "doc 2 survives"
        );
        assert_eq!(
            Quota::new(store.as_ref())
                .usage(account_id)
                .unwrap()
                .messages,
            1
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn close_expunges_deleted_messages_without_untagged_responses() {
        use irixmail_mail::load_data;
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) = expunge_env(&[
            b"Subject: One\r\n\r\nbody one\r\n",
            b"Subject: Two\r\n\r\nbody two\r\n",
        ]);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc STORE 1 +FLAGS.SILENT (\\Deleted)\r\nd CLOSE\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("d OK CLOSE completed"), "{out}");
        assert!(
            !out.contains("EXPUNGE"),
            "CLOSE sends no untagged EXPUNGE: {out}"
        );
        assert!(
            load_data(store.as_ref(), account_id, 1).unwrap().is_none(),
            "doc 1 expunged"
        );
        assert!(
            load_data(store.as_ref(), account_id, 2).unwrap().is_some(),
            "doc 2 survives"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_non_peek_body_fetch_sets_the_seen_flag() {
        use irixmail_mail::load_data;
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) =
            expunge_env(&[b"Subject: One\r\n\r\nbody one\r\n"]);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (BODY[])\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(
            out.contains("FLAGS (\\Seen)"),
            "the flag change is pushed in the response: {out}"
        );
        let data = load_data(store.as_ref(), account_id, 1).unwrap().unwrap();
        assert!(
            data.keywords.contains(&Keyword::Seen),
            "\\Seen persists: {:?}",
            data.keywords
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_peek_body_fetch_leaves_the_seen_flag_unset() {
        use irixmail_mail::load_data;
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) =
            expunge_env(&[b"Subject: One\r\n\r\nbody one\r\n"]);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (BODY.PEEK[])\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(!out.contains("FLAGS (\\Seen)"), "{out}");
        let data = load_data(store.as_ref(), account_id, 1).unwrap().unwrap();
        assert!(
            !data.keywords.contains(&Keyword::Seen),
            "{:?}",
            data.keywords
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_body_fetch_on_an_examined_mailbox_does_not_set_seen() {
        use irixmail_mail::load_data;
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) =
            expunge_env(&[b"Subject: One\r\n\r\nbody one\r\n"]);

        let mut session = Session::new(
            Pipe::new(b"b EXAMINE INBOX\r\nc FETCH 1 (BODY[])\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();

        let data = load_data(store.as_ref(), account_id, 1).unwrap().unwrap();
        assert!(
            !data.keywords.contains(&Keyword::Seen),
            "{:?}",
            data.keywords
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_partial_body_fetch_returns_the_requested_byte_range() {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, account, _account_id, path) =
            expunge_env(&[b"Subject: One\r\n\r\nHello body\r\n"]);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (BODY.PEEK[TEXT]<2.4>)\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("BODY[TEXT]<2> {4}\r\nllo "), "{out}");
        assert!(out.contains("c OK FETCH completed"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    const MULTIPART_RAW: &[u8] = b"Subject: Parts\r\nFrom: a@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nplain part\r\n--B\r\nContent-Type: text/html\r\n\r\n<p>html part</p>\r\n--B--\r\n";

    #[tokio::test]
    async fn fetch_returns_header_fields_numeric_parts_and_mime_sections() {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, account, _account_id, path) = expunge_env(&[MULTIPART_RAW]);

        let mut session = Session::new(
            Pipe::new(
                b"b SELECT INBOX\r\nc FETCH 1 (BODY.PEEK[HEADER.FIELDS (SUBJECT)])\r\nd FETCH 1 (BODY.PEEK[1])\r\ne FETCH 1 (BODY.PEEK[2.MIME])\r\nf FETCH 1 (BODY.PEEK[9])\r\n",
            ),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(
            out.contains("BODY[HEADER.FIELDS (SUBJECT)] {18}\r\nSubject: Parts\r\n\r\n"),
            "{out}"
        );
        assert!(out.contains("BODY[1] {"), "{out}");
        assert!(out.contains("plain part"), "{out}");
        assert!(out.contains("BODY[2.MIME] {"), "{out}");
        assert!(out.contains("Content-Type: text/html\r\n\r\n"), "{out}");
        assert!(
            !out.contains("BODY[9]"),
            "an out-of-range part is omitted: {out}"
        );
        for tag in ["c", "d", "e", "f"] {
            assert!(out.contains(&format!("{tag} OK FETCH completed")), "{out}");
        }
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn fetch_header_fields_not_excludes_the_listed_fields() {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, account, _account_id, path) =
            expunge_env(&[b"Subject: One\r\nX-Keep: yes\r\n\r\nbody\r\n"]);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (BODY.PEEK[HEADER.FIELDS.NOT (SUBJECT)])\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("BODY[HEADER.FIELDS.NOT (SUBJECT)] {"), "{out}");
        assert!(out.contains("X-Keep: yes\r\n"), "{out}");
        assert!(!out.contains("Subject: One"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_numeric_part_fetch_of_a_plain_message_returns_the_body() {
        use irixmail_store::ChangeNotifier;

        let (directory, _store, blobs, account, _account_id, path) =
            expunge_env(&[b"Subject: One\r\n\r\nHello body\r\n"]);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc FETCH 1 (BODY.PEEK[1])\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("BODY[1] {12}\r\nHello body\r\n"), "{out}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn an_examined_mailbox_refuses_store_expunge_and_move() {
        use irixmail_mail::load_data;
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) = expunge_env(&[
            b"Subject: One\r\n\r\nbody one\r\n",
            b"Subject: Two\r\n\r\nbody two\r\n",
        ]);

        let mut session = Session::new(
            Pipe::new(
                b"a CREATE Work\r\nb EXAMINE INBOX\r\nc STORE 1 +FLAGS (\\Deleted)\r\nd UID STORE 1 +FLAGS (\\Deleted)\r\ne EXPUNGE\r\nf UID EXPUNGE 1\r\ng MOVE 1 Work\r\nh UID MOVE 1 Work\r\ni COPY 1 Work\r\nj SEARCH ALL\r\n",
            ),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        for tag in ["c", "d", "e", "f", "g", "h"] {
            assert!(
                out.contains(&format!("{tag} NO ")),
                "{tag} must be refused read-only: {out}"
            );
        }
        assert!(
            out.contains("i OK"),
            "COPY out of a read-only mailbox is allowed: {out}"
        );
        assert!(
            out.contains("* SEARCH 1 2\r\n"),
            "both messages remain: {out}"
        );
        let data = load_data(store.as_ref(), account_id, 1).unwrap().unwrap();
        assert!(
            !data.keywords.contains(&Keyword::Deleted),
            "no flag was written: {:?}",
            data.keywords
        );
        assert!(
            data.in_mailbox(irixmail_mail::INBOX_ID),
            "the message was not moved"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn close_after_examine_leaves_deleted_messages_alone() {
        use irixmail_mail::{load_data, update_message};
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) =
            expunge_env(&[b"Subject: One\r\n\r\nbody one\r\n"]);
        let notifier = ChangeNotifier::new();
        update_message(store.as_ref(), &notifier, account_id, 1, |data| {
            data.add_keyword(Keyword::Deleted);
            Ok(())
        })
        .unwrap();

        let mut session = Session::new(Pipe::new(b"b EXAMINE INBOX\r\nc CLOSE\r\n"), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(Arc::clone(&blobs))
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("c OK CLOSE completed"), "{out}");
        assert!(
            load_data(store.as_ref(), account_id, 1).unwrap().is_some(),
            "an EXAMINE session must not expunge on CLOSE"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    async fn crud_session(script: &'static [u8]) -> String {
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, _store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());
        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        let _ = std::fs::remove_dir_all(&path);
        out
    }

    fn segment<'a>(out: &'a str, from: &str, to: &str) -> &'a str {
        let start = out.find(from).map(|at| at + from.len()).unwrap_or(0);
        let end = out[start..]
            .find(to)
            .map(|at| start + at)
            .unwrap_or(out.len());
        &out[start..end]
    }

    #[tokio::test]
    async fn subscriptions_persist_and_lsub_lists_only_subscribed_mailboxes() {
        use irixmail_core::IdGenerator;
        use irixmail_store::ChangeNotifier;

        let (directory, store, account, path) = account_directory_with_store("secret");
        let account_again = account.clone();

        let mut session = Session::new(
            Pipe::new(
                b"a LSUB \"\" \"*\"\r\nb SUBSCRIBE INBOX\r\nc CREATE Work\r\nd SUBSCRIBE Work\r\ne LSUB \"\" \"*\"\r\nf UNSUBSCRIBE INBOX\r\ng LSUB \"\" \"*\"\r\nh SUBSCRIBE Nope\r\ni UNSUBSCRIBE Sent\r\n",
            ),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(
            segment(&out, "* OK [CAPABILITY", "a OK").contains("\"INBOX\""),
            "a fresh account lists every folder: {out}"
        );
        assert!(out.contains("b OK"), "{out}");
        assert!(out.contains("d OK"), "{out}");
        let both = segment(&out, "d OK", "e OK");
        assert!(
            both.contains("\"INBOX\"") && both.contains("\"Work\""),
            "{out}"
        );
        assert!(out.contains("f OK"), "{out}");
        let after = segment(&out, "f OK", "g OK");
        assert!(
            after.contains("\"Work\"") && !after.contains("\"INBOX\""),
            "{out}"
        );
        assert!(
            out.contains("h NO"),
            "subscribing a missing mailbox fails: {out}"
        );
        assert!(
            out.contains("i NO"),
            "unsubscribing a non-subscribed mailbox fails: {out}"
        );

        let directory = Directory::new(store, Arc::new(IdGenerator::new(0)), None);
        let mut second = Session::new(Pipe::new(b"a LSUB \"\" \"*\"\r\n"), peer())
            .with_tls()
            .with_directory(directory)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        second.account = Some(account_again);
        second.state = State::Authenticated;
        second.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut second.stream.get_mut().output)).unwrap();
        assert!(
            out.contains("* LSUB") && out.contains("\"Work\"") && !out.contains("\"INBOX\""),
            "subscriptions survive a new session: {out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn create_adds_a_selectable_listable_folder() {
        let out = crud_session(b"a CREATE Work\r\nb LIST \"\" \"*\"\r\nc SELECT Work\r\n").await;
        assert!(out.contains("a OK CREATE completed"), "{out}");
        assert!(out.contains("\"Work\""), "LIST shows the new folder: {out}");
        assert!(out.contains("c OK [READ-WRITE] SELECT completed"), "{out}");
    }

    #[tokio::test]
    async fn status_escapes_a_mailbox_name_that_needs_quoting() {
        let out =
            crud_session(b"a CREATE \"Quo\\\"te\"\r\nb STATUS \"Quo\\\"te\" (MESSAGES)\r\n").await;
        assert!(out.contains("a OK CREATE completed"), "{out}");
        assert!(out.contains("* STATUS \"Quo\\\"te\" (MESSAGES 0)"), "{out}");
        assert!(out.contains("b OK STATUS completed"), "{out}");
    }

    #[tokio::test]
    async fn create_of_an_existing_mailbox_is_refused() {
        let out = crud_session(b"a CREATE INBOX\r\nb CREATE Work\r\nc CREATE Work\r\n").await;
        assert!(out.contains("a NO"), "cannot recreate INBOX: {out}");
        assert!(out.contains("b OK CREATE completed"), "{out}");
        assert!(out.contains("c NO"), "duplicate is refused: {out}");
    }

    #[tokio::test]
    async fn delete_removes_a_user_folder_but_refuses_system_folders() {
        let out = crud_session(
            b"a CREATE Work\r\nb DELETE Work\r\nc LIST \"\" \"*\"\r\nd DELETE INBOX\r\n",
        )
        .await;
        assert!(out.contains("b OK DELETE completed"), "{out}");
        assert!(
            !out.contains("\"Work\""),
            "the folder is gone from LIST: {out}"
        );
        assert!(out.contains("d NO"), "INBOX cannot be deleted: {out}");
    }

    #[tokio::test]
    async fn rename_changes_the_name_and_refuses_system_folders() {
        let out = crud_session(
            b"a CREATE Work\r\nb RENAME Work Projects\r\nc LIST \"\" \"*\"\r\nd RENAME INBOX Archive\r\n",
        )
        .await;
        assert!(out.contains("b OK RENAME completed"), "{out}");
        assert!(out.contains("\"Projects\""), "the new name lists: {out}");
        assert!(!out.contains("\"Work\""), "the old name is gone: {out}");
        assert!(out.contains("d NO"), "INBOX cannot be renamed: {out}");
    }

    #[tokio::test]
    async fn append_stores_the_message_with_its_flag_and_reports_appenduid() {
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, _store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());

        let body: &[u8] = b"Subject: Appended\r\nFrom: a@example.com\r\n\r\nappended body\r\n";
        let mut script = format!("a APPEND INBOX (\\Seen) {{{}}}\r\n", body.len()).into_bytes();
        script.extend_from_slice(body);
        script.extend_from_slice(b"\r\nb SELECT INBOX\r\nc FETCH 1 (FLAGS RFC822.SIZE)\r\n");

        let mut session = Session::new(Pipe::new(&script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("a OK [APPENDUID"), "{out}");
        assert!(out.contains("APPEND completed"), "{out}");
        assert!(
            out.contains("* 1 EXISTS"),
            "the message lands in INBOX: {out}"
        );
        assert!(
            out.contains(&format!(
                "* 1 FETCH (FLAGS (\\Seen) RFC822.SIZE {})",
                body.len()
            )),
            "stored with its flag and size: {out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn append_stores_a_client_supplied_internaldate() {
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, _store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());

        let body = b"Subject: Dated\r\n\r\nhello\r\n";
        let mut script = format!(
            "a APPEND INBOX (\\Seen) \"15-Apr-1985 03:02:18 +0200\" {{{}+}}\r\n",
            body.len()
        )
        .into_bytes();
        script.extend_from_slice(body);
        script.extend_from_slice(b"\r\nb SELECT INBOX\r\nc FETCH 1 (INTERNALDATE)\r\n");

        let mut session = Session::new(Pipe::new(&script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("a OK [APPENDUID"), "{out}");
        assert!(
            out.contains("INTERNALDATE \"15-Apr-1985 01:02:18 +0000\""),
            "the supplied date-time is stored, normalized to UTC: {out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn append_with_a_malformed_internaldate_is_rejected_without_desync() {
        use irixmail_store::{BlobStore, ChangeNotifier, FsBlobStore};

        let (directory, _store, account, path) = account_directory_with_store("secret");
        std::fs::create_dir_all(path.join("blobs")).unwrap();
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::open(path.join("blobs")).unwrap());

        let mut script = b"a APPEND INBOX \"not a date\" {5+}\r\nhello".to_vec();
        script.extend_from_slice(b"\r\nb NOOP\r\n");

        let mut session = Session::new(Pipe::new(&script), peer())
            .with_tls()
            .with_directory(directory)
            .with_blobs(blobs)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("a BAD"), "{out}");
        assert!(
            out.contains("b OK NOOP completed"),
            "the literal is drained: {out}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn deleting_a_folder_destroys_messages_that_lived_only_there() {
        use irixmail_mail::load_data;
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) =
            expunge_env(&[b"Subject: Only\r\n\r\nbody only\r\n"]);

        let mut session = Session::new(
            Pipe::new(b"a CREATE Work\r\nb SELECT INBOX\r\nc MOVE 1 Work\r\nd DELETE Work\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("d OK DELETE completed"), "{out}");
        assert!(
            load_data(store.as_ref(), account_id, 1).unwrap().is_none(),
            "message destroyed"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn uid_expunge_only_removes_messages_in_the_uid_set() {
        use irixmail_mail::load_data;
        use irixmail_store::ChangeNotifier;

        let (directory, store, blobs, account, account_id, path) = expunge_env(&[
            b"Subject: One\r\n\r\nbody one\r\n",
            b"Subject: Two\r\n\r\nbody two\r\n",
        ]);

        let mut session = Session::new(
            Pipe::new(
                b"b SELECT INBOX\r\nc STORE 1:2 +FLAGS (\\Deleted)\r\nd UID EXPUNGE 2\r\ne SEARCH ALL\r\n",
            ),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_blobs(Arc::clone(&blobs))
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("* 2 EXPUNGE\r\n"), "{out}");
        assert!(out.contains("d OK UID EXPUNGE completed"), "{out}");
        assert!(out.contains("* SEARCH 1\r\n"), "{out}");
        assert!(
            load_data(store.as_ref(), account_id, 1).unwrap().is_some(),
            "uid 1 kept"
        );
        assert!(
            load_data(store.as_ref(), account_id, 2).unwrap().is_none(),
            "uid 2 expunged"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn uid_store_updates_flags_and_completes() {
        let (_, out) =
            run_authenticated(b"b SELECT INBOX\r\nc UID STORE 1 +FLAGS (\\Seen)\r\n").await;
        assert!(out.contains("c OK UID STORE completed"));
    }

    fn email_data_key(account_id: u32, document_id: u32) -> Vec<u8> {
        use irixmail_store::{Collection, Key, Subspace};
        Key::new(
            Subspace::Property,
            account_id,
            Collection::Email,
            document_id,
        )
        .encode()
    }

    struct RacingStore {
        inner: Arc<dyn irixmail_store::Store>,
        hook_key: Arc<std::sync::Mutex<Vec<u8>>>,
        hook: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl irixmail_store::Store for RacingStore {
        fn get(&self, key: &[u8]) -> irixmail_core::Result<Option<Vec<u8>>> {
            let value = self.inner.get(key)?;
            let armed = {
                let hook_key = self.hook_key.lock().unwrap();
                !hook_key.is_empty() && key == hook_key.as_slice()
            };
            if armed {
                if let Some(hook) = self.hook.lock().unwrap().take() {
                    hook();
                }
            }
            Ok(value)
        }

        fn put(&self, key: &[u8], value: &[u8]) -> irixmail_core::Result<()> {
            self.inner.put(key, value)
        }

        fn delete(&self, key: &[u8]) -> irixmail_core::Result<()> {
            self.inner.delete(key)
        }

        fn iterate(
            &self,
            prefix: &irixmail_store::KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> irixmail_core::Result<irixmail_store::Flow>,
        ) -> irixmail_core::Result<()> {
            self.inner.iterate(prefix, visit)
        }

        fn batch(&self, ops: &[irixmail_store::WriteOp]) -> irixmail_core::Result<()> {
            self.inner.batch(ops)
        }

        fn add_and_get(&self, key: &[u8], by: i64) -> irixmail_core::Result<i64> {
            self.inner.add_and_get(key, by)
        }

        fn counter(&self, key: &[u8]) -> irixmail_core::Result<i64> {
            self.inner.counter(key)
        }
    }

    struct FailingStore {
        inner: Arc<dyn irixmail_store::Store>,
        fail_key: Arc<std::sync::Mutex<Vec<u8>>>,
    }

    impl irixmail_store::Store for FailingStore {
        fn get(&self, key: &[u8]) -> irixmail_core::Result<Option<Vec<u8>>> {
            self.inner.get(key)
        }

        fn put(&self, key: &[u8], value: &[u8]) -> irixmail_core::Result<()> {
            self.inner.put(key, value)
        }

        fn delete(&self, key: &[u8]) -> irixmail_core::Result<()> {
            self.inner.delete(key)
        }

        fn iterate(
            &self,
            prefix: &irixmail_store::KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> irixmail_core::Result<irixmail_store::Flow>,
        ) -> irixmail_core::Result<()> {
            self.inner.iterate(prefix, visit)
        }

        fn batch(&self, ops: &[irixmail_store::WriteOp]) -> irixmail_core::Result<()> {
            let fail_key = self.fail_key.lock().unwrap().clone();
            if !fail_key.is_empty() && ops.iter().any(|op| op.key() == fail_key.as_slice()) {
                return Err(irixmail_core::Error::store("injected batch failure"));
            }
            self.inner.batch(ops)
        }

        fn add_and_get(&self, key: &[u8], by: i64) -> irixmail_core::Result<i64> {
            self.inner.add_and_get(key, by)
        }

        fn counter(&self, key: &[u8]) -> irixmail_core::Result<i64> {
            self.inner.counter(key)
        }
    }

    fn wrapped_store_env(
        label: &str,
        wrap: impl FnOnce(Arc<dyn irixmail_store::Store>) -> Arc<dyn irixmail_store::Store>,
    ) -> (
        Directory,
        Arc<dyn irixmail_store::Store>,
        Account,
        std::path::PathBuf,
    ) {
        use std::sync::atomic::{AtomicU32, Ordering};

        use irixmail_core::IdGenerator;
        use irixmail_directory::{password as pw, Role};
        use irixmail_store::RocksdbStore;

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "irixmail-imap-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();

        let inner: Arc<dyn irixmail_store::Store> =
            Arc::new(RocksdbStore::open(path.join("db")).unwrap());
        let wrapped = wrap(Arc::clone(&inner));
        let directory = Directory::new(wrapped, Arc::new(IdGenerator::new(0)), None);
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
        (directory, inner, account, path)
    }

    #[tokio::test]
    async fn a_store_merges_with_a_concurrent_flag_write_instead_of_clobbering_it() {
        use irixmail_mail::{load_data, MessageData};
        use irixmail_store::{serialize, ChangeNotifier};

        let hook_key: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (directory, inner, account, path) = {
            let hook_key = Arc::clone(&hook_key);
            wrapped_store_env("race", move |inner| {
                let racer = Arc::clone(&inner);
                let key_source = Arc::clone(&hook_key);
                Arc::new(RacingStore {
                    inner,
                    hook_key,
                    hook: std::sync::Mutex::new(Some(Box::new(move || {
                        let key = key_source.lock().unwrap().clone();
                        let bytes = racer.get(&key).unwrap().unwrap();
                        let mut data: MessageData = serialize::deserialize(&bytes).unwrap();
                        data.add_keyword(Keyword::Answered);
                        racer
                            .put(&key, &serialize::archive(&data).unwrap())
                            .unwrap();
                    }))),
                })
            })
        };
        let account_id = account.id as u32;

        let inbox = provision_mailboxes(account.created_at)
            .into_iter()
            .find(|mailbox| mailbox.id == irixmail_mail::INBOX_ID)
            .unwrap();
        deliver_into(inner.as_ref(), account_id, &inbox, 1, false);
        *hook_key.lock().unwrap() = email_data_key(account_id, 1);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc STORE 1 +FLAGS (\\Flagged)\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("c OK STORE completed"), "{out}");
        let data = load_data(inner.as_ref(), account_id, 1).unwrap().unwrap();
        assert!(
            data.keywords.contains(&Keyword::Answered),
            "the concurrent write survives: {:?}",
            data.keywords
        );
        assert!(
            data.keywords.contains(&Keyword::Flagged),
            "the store applies its own flag: {:?}",
            data.keywords
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn a_multi_message_store_applies_all_or_nothing() {
        use irixmail_mail::load_data;
        use irixmail_store::ChangeNotifier;

        let fail_key: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (directory, inner, account, path) = {
            let fail_key = Arc::clone(&fail_key);
            wrapped_store_env("atomic", move |inner| {
                Arc::new(FailingStore { inner, fail_key })
            })
        };
        let account_id = account.id as u32;

        let inbox = provision_mailboxes(account.created_at)
            .into_iter()
            .find(|mailbox| mailbox.id == irixmail_mail::INBOX_ID)
            .unwrap();
        deliver_into(inner.as_ref(), account_id, &inbox, 1, false);
        deliver_into(inner.as_ref(), account_id, &inbox, 2, false);
        *fail_key.lock().unwrap() = email_data_key(account_id, 2);

        let mut session = Session::new(
            Pipe::new(b"b SELECT INBOX\r\nc STORE 1:2 +FLAGS (\\Flagged)\r\nd NOOP\r\n"),
            peer(),
        )
        .with_tls()
        .with_directory(directory)
        .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();

        assert!(out.contains("c NO"), "a failed store reports NO: {out}");
        assert!(
            out.contains("d OK NOOP completed"),
            "the session survives: {out}"
        );
        for document_id in [1, 2] {
            let data = load_data(inner.as_ref(), account_id, document_id)
                .unwrap()
                .unwrap();
            assert!(
                !data.keywords.contains(&Keyword::Flagged),
                "doc {document_id} must not be partially flagged: {:?}",
                data.keywords
            );
        }
        let _ = std::fs::remove_dir_all(&path);
    }

    async fn store_session(script: &'static [u8], seen_first: bool) -> String {
        let (directory, store, account, path) = account_directory_with_store("secret");
        let account_id = account.id as u32;
        let inbox = provision_mailboxes(account.created_at)
            .into_iter()
            .find(|mailbox| mailbox.id == irixmail_mail::INBOX_ID)
            .unwrap();
        deliver_into(store.as_ref(), account_id, &inbox, 10, seen_first);

        let mut session = Session::new(Pipe::new(script), peer())
            .with_tls()
            .with_directory(directory)
            .with_notifier(Arc::new(ChangeNotifier::new()));
        session.account = Some(account);
        session.state = State::Authenticated;
        session.run().await.unwrap();
        let out = String::from_utf8(std::mem::take(&mut session.stream.get_mut().output)).unwrap();
        let _ = std::fs::remove_dir_all(&path);
        out
    }

    #[tokio::test]
    async fn store_adds_a_flag_persists_it_and_echoes_the_new_flags() {
        let out = store_session(
            b"b SELECT INBOX\r\nc STORE 1 +FLAGS (\\Seen)\r\nd FETCH 1 (FLAGS)\r\n",
            false,
        )
        .await;
        assert_eq!(
            out.matches("* 1 FETCH (FLAGS (\\Seen))").count(),
            2,
            "{out}"
        );
        assert!(out.contains("c OK STORE completed"), "{out}");
        assert!(out.contains("d OK FETCH completed"), "{out}");
    }

    #[tokio::test]
    async fn store_replace_sets_exactly_the_given_flags() {
        let out = store_session(
            b"b SELECT INBOX\r\nc STORE 1 FLAGS (\\Flagged)\r\nd FETCH 1 (FLAGS)\r\n",
            true,
        )
        .await;
        assert_eq!(
            out.matches("* 1 FETCH (FLAGS (\\Flagged))").count(),
            2,
            "{out}"
        );
        assert!(!out.contains("FETCH (FLAGS (\\Seen"), "{out}");
        assert!(out.contains("c OK STORE completed"), "{out}");
    }

    #[tokio::test]
    async fn a_silent_store_suppresses_the_untagged_fetch_but_still_persists() {
        let out = store_session(
            b"b SELECT INBOX\r\nc STORE 1 +FLAGS.SILENT (\\Seen)\r\nd FETCH 1 (FLAGS)\r\n",
            false,
        )
        .await;
        assert_eq!(
            out.matches("* 1 FETCH (FLAGS (\\Seen))").count(),
            1,
            "{out}"
        );
        assert!(out.contains("c OK STORE completed"), "{out}");
        assert!(out.contains("d OK FETCH completed"), "{out}");
    }

    #[tokio::test]
    async fn a_uid_store_echoes_the_uid_with_the_new_flags() {
        let out = store_session(
            b"b SELECT INBOX\r\nc UID STORE 1 +FLAGS (\\Seen)\r\n",
            false,
        )
        .await;
        assert!(out.contains("* 1 FETCH (UID 1 FLAGS (\\Seen))"), "{out}");
        assert!(out.contains("c OK UID STORE completed"), "{out}");
    }

    #[tokio::test]
    async fn uid_copy_into_an_existing_mailbox_completes() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc UID COPY 1:* Sent\r\n").await;
        assert!(out.contains("c OK UID COPY completed"));
    }

    #[tokio::test]
    async fn uid_copy_into_an_unknown_mailbox_suggests_trycreate() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc UID COPY 1:* Nowhere\r\n").await;
        assert!(out.contains("c NO [TRYCREATE]"));
    }

    #[tokio::test]
    async fn uid_move_into_an_existing_mailbox_completes() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc UID MOVE 1:* Trash\r\n").await;
        assert!(out.contains("c OK UID MOVE completed"));
    }

    #[tokio::test]
    async fn uid_expunge_with_a_sequence_set_completes() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc UID EXPUNGE 1:*\r\n").await;
        assert!(out.contains("c OK UID EXPUNGE completed"));
    }

    #[tokio::test]
    async fn an_unknown_uid_subcommand_is_bad() {
        let (_, out) = run_authenticated(b"b SELECT INBOX\r\nc UID FROB 1\r\n").await;
        assert!(out.contains("c BAD Unsupported UID subcommand"));
    }

    #[tokio::test]
    async fn idle_continues_then_completes_on_done() {
        let (_, out) = run_authenticated(b"c IDLE\r\nDONE\r\n").await;
        assert!(out.contains("+ idling"));
        assert!(out.contains("c OK IDLE completed"));
    }

    #[tokio::test]
    async fn close_returns_to_the_authenticated_state() {
        let (session, _) = run_authenticated(b"b SELECT INBOX\r\nc CLOSE\r\n").await;
        assert_eq!(session.state(), State::Authenticated);
        assert!(session.data().mailbox.is_none());
    }

    #[tokio::test]
    async fn starttls_requests_an_upgrade() {
        let (flow, out, _) = drive(b"a STARTTLS\r\n").await;
        assert_eq!(flow, Flow::Upgrade);
        assert!(out.contains("a OK Begin TLS negotiation now"));
    }

    #[tokio::test]
    async fn starttls_after_authentication_is_refused() {
        let (_, out) = run_authenticated(b"b STARTTLS\r\n").await;
        assert!(
            out.contains("b BAD STARTTLS not allowed now")
                || out.contains("b BAD TLS already active")
        );
    }

    #[tokio::test]
    async fn logout_emits_bye_and_closes() {
        let (flow, out, _) = drive(b"a LOGOUT\r\n").await;
        assert_eq!(flow, Flow::Close);
        assert!(out.contains("* BYE"));
        assert!(out.contains("a OK LOGOUT completed"));
    }

    #[tokio::test]
    async fn an_unknown_command_is_bad() {
        let (_, out, _) = drive(b"a FROBNICATE\r\n").await;
        assert!(out.contains("a BAD Command unrecognized"));
    }

    #[tokio::test]
    async fn a_line_without_a_tag_is_bad() {
        let (_, out, _) = drive(b" SELECT INBOX\r\n").await;
        assert!(out.contains("* BAD"));
        assert!(out.contains("missing command tag"));
    }
}
