pub mod attachment_backfill;
pub mod auth_results;
pub mod blob_store_msg;
pub mod cache;
pub mod compose;
pub mod deliver;
pub mod forward;
pub mod html_text;
pub mod index;
pub mod ingest;
pub mod keywords;
pub mod mailbox;
pub mod mailbox_admin;
pub mod message_data;
pub mod metadata;
pub mod provision;
pub mod purge;
pub mod quota_enforce;
pub mod read;
pub mod resolve;
pub mod sanitize;
pub mod server;
pub mod sieve_compile;
pub mod sieve_exec;
pub mod subscriptions;
pub mod thread_backfill;
pub mod threading;
pub mod vacation;

pub use attachment_backfill::backfill_attachment_keywords;
pub use auth_results::{
    ArcVerdict, AuthResults, DkimVerdict, DmarcVerdict, MethodResult, SpfIdentity, SpfVerdict,
};
pub use blob_store_msg::{
    account_link_op, account_references_blob, add_reference, has_live_reservation, reference_count,
    reference_op, release_message, reserve_upload, store_blob, store_message, RESERVATION_TTL_SECS,
};
pub use cache::{MessageCacheEntry, MessageStoreCache};
pub use compose::{build_message, Attachment, Compose, Mailbox as ComposeMailbox};
pub use deliver::{
    append_message, deliver, AppendOutcome, AppendRequest, DeliveryOutcome, DeliveryRequest,
    DeliveryTarget,
};
pub use forward::{forward_to, plan_forward, ForwardPlan, ForwardRelay};
pub use html_text::text_from_html;
pub use index::{
    index_message, index_ops, index_text, message_sent_at, message_text, reindex_account,
    unindex_message, unindex_ops, MessageText,
};
pub use ingest::ingest;
pub use mailbox::{assign_uid_validity, Mailbox, SpecialUse};
pub use mailbox_admin::{create_mailbox, delete_mailbox, rename_mailbox, MailboxDelete};
pub use message_data::{Keyword, MailboxUid, MessageData};
pub use metadata::{
    ByteRange, HeaderName, MessageMetadata, MessagePart, PartBody, PartEncoding, PartHeader,
};
pub use provision::{
    load_mailboxes, mailbox_ops, provision_mailboxes, provision_ops, DRAFTS_ID,
    FIRST_USER_MAILBOX_ID, INBOX_ID, SENT_ID, SPAM_ID, SYSTEM_MAILBOX_COUNT, TRASH_ID,
};
pub use purge::{purge_orphans, PURGE_GRACE_SECS};
pub use quota_enforce::{enforce_quota, limits_for, QuotaVerdict};
pub use read::{
    allocate_document_id, delete_message, load_data, load_metadata, load_raw, update_message,
    update_messages, UpdatedMessage,
};
pub use resolve::{resolve, Resolution};
pub use sanitize::{sanitize_html, Sanitized};
pub use server::MailServices;
pub use sieve_compile::{
    compile_active_script, compile_rules, compile_source, compile_stored_script, emit_script,
    script_source, stored_rule_set, Action, Comparator, CompiledScript, Condition, Field,
    MatchType, Rule, RuleSet,
};
pub use sieve_exec::{execute_sieve, SieveOutcome};
pub use thread_backfill::backfill_threads;
pub use vacation::{
    evaluate_vacation, last_vacation_reply, record_vacation_reply, SuppressReason, VacationConfig,
    VacationDecision, VacationReply, DEFAULT_PERIOD_SECONDS,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
