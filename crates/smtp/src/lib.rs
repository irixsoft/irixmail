pub mod arc;
pub mod cmd_auth;
pub mod cmd_bdat;
pub mod cmd_data;
pub mod cmd_ehlo;
pub mod cmd_mail;
pub mod cmd_noop;
pub mod cmd_quit;
pub mod cmd_rcpt;
pub mod cmd_rset;
pub mod cmd_starttls;
pub mod concurrency;
pub mod deliver_hook;
pub mod deliver_out;
pub mod dkim_sign;
pub mod dkim_verify;
pub mod dmarc;
pub mod dnsbl;
pub mod dsn;
pub mod greylist;
pub mod inbound;
pub mod ip_guard;
pub mod listener_in;
pub mod listener_in_tls;
pub mod listener_sub;
pub mod listener_sub_465;
pub mod listener_sub_587;
pub mod loop_detect;
pub mod mtasts_publish;
pub mod mx_resolve;
pub mod parser;
pub mod queue_deliver;
pub mod queue_enqueue;
pub mod queue_lease;
pub mod queue_local;
pub mod queue_manager;
pub mod queue_model;
pub mod ratelimit_in;
pub mod ratelimit_out;
pub mod retry;
pub mod session;
pub mod session_services;
pub mod spam_decision;
pub mod spf;
pub mod sub_auth;
pub mod sub_enqueue;
pub mod sub_from;
pub mod sub_headers;

pub use arc::{ArcDecision, ArcVerifier};
pub use cmd_auth::{
    credentials_invalid_reply, success_reply, Credentials, SaslExchange, SaslStart, SaslStep,
};
pub use cmd_bdat::{
    bdat_reply, chunk_disposal, chunk_ok_reply, BdatOutcome, ChunkDisposal, ChunkReceiver,
    ChunkStep,
};
pub use cmd_data::{
    accepted_reply, data_reply, too_large_reply, BodyReceiver, BodyStep, DataOutcome,
};
pub use cmd_ehlo::{ehlo_response, helo_response, EhloContext};
pub use cmd_mail::{mail_reply, MailOutcome, ReversePath};
pub use cmd_noop::noop_reply;
pub use cmd_quit::quit_reply;
pub use cmd_rcpt::{rcpt_reply, ForwardPath, RcptOutcome, Recipient, DEFAULT_MAX_RECIPIENTS};
pub use cmd_rset::rset_reply;
pub use cmd_starttls::{build_acceptor, starttls_reply, upgrade, StartTlsReply};
pub use concurrency::{ConcurrencyLimiter, DeliverySlot, DEFAULT_MAX_PER_DESTINATION};
pub use deliver_hook::{
    day_number, deliver_inbound, inbound_total, record_inbound, InboundOutcome,
};
pub use deliver_out::{deliver, outbound_total, record_outbound, DeliveryAttempt};
pub use dkim_sign::DomainSigner;
pub use dkim_verify::{DkimDecision, DkimSignatureResult, DkimVerdict, DkimVerifier};
pub use dmarc::{DmarcAction, DmarcDecision, DmarcVerifier};
pub use dnsbl::{DnsblConfig, DnsblDecision, DEFAULT_ZONE};
pub use dsn::build_dsn;
pub use greylist::{Greylist, GreylistConfig, GreylistDecision};
pub use inbound::{build_received, prepend_header, run_gauntlet, GauntletOutcome};
pub use listener_in::{register_inbound, InboundListener};
pub use listener_in_tls::{register_inbound_tls, InboundTlsListener};
pub use listener_sub::register_submission;
pub use listener_sub_465::{register_submission_465, ImplicitTlsListener};
pub use listener_sub_587::{register_submission_587, SubmissionListener};
pub use loop_detect::{LoopConfig, LoopDecision, DEFAULT_MAX_RECEIVED};
pub use mtasts_publish::{publish, publish_with, PublishedPolicy};
pub use mx_resolve::{resolve, MxResolution, MxTarget};
pub use parser::{parse_command, AuthMechanism, Command, MailParams, ParseError, RcptParams};
pub use queue_deliver::OutboundDelivery;
pub use queue_enqueue::{enqueue, load, persist, remove, retry_now, Enqueue, Enqueued};
pub use queue_lease::{Lease, LeaseRegistry, DEFAULT_LEASE};
pub use queue_local::LocalDelivery;
pub use queue_manager::{
    next_wake, register_outbound, run, scan_all, scan_due, wakeup_channel, DueBatch, DueMessage,
    Wakeup, REFRESH_INTERVAL,
};
pub use queue_model::{
    Expiry, NotifySchedule, QueueRecipient, QueuedMessage, RecipientStatus, RetrySchedule,
};
pub use ratelimit_in::{
    RateDecision, RateLimiter, RateLimits, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_MESSAGES,
    DEFAULT_WINDOW,
};
pub use ratelimit_out::{
    Axis, OutboundDecision, OutboundLimiter, OutboundLimits, DEFAULT_MAX_PER_DOMAIN,
    DEFAULT_MAX_PER_SENDER,
};
pub use retry::{backoff, next_after_deferral, RetryDecision, BASE_BACKOFF, MAX_BACKOFF};
pub use session::{
    AcceptedMessage, Flow, Session, SessionData, SessionServices, SmtpMode, Stage, Verb,
};
pub use session_services::{InboundServices, SubmissionServices};
pub use spam_decision::{decide, AuthSummary, Disposition, ReputationSummary, SpamDecision};
pub use spf::{SpfConfig, SpfDecision, SpfStage, SpfVerifier};
pub use sub_auth::{guard_submission, SubmissionGate};
pub use sub_enqueue::{enqueue_submission, Submission, DEFAULT_MAX_AGE};
pub use sub_from::{guard_from, OwnershipGate};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
