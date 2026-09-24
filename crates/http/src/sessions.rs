use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use irixmail_core::{Error, Result};
use irixmail_store::{Flow, KeyPrefix, Store, Subspace, WriteOp};
use rand::RngCore;
use serde::{Deserialize, Serialize};

const TAG_SESSION: u8 = 0x36;

const WEBMAIL_IDLE_SECS: u64 = 90 * 24 * 60 * 60;

const ADMIN_IDLE_SECS: u64 = 12 * 60 * 60;

const RENEW_AFTER_SECS: u64 = 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Admin,
    Webmail,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Admin => "admin",
            SessionKind::Webmail => "webmail",
        }
    }

    fn idle_limit(self) -> u64 {
        match self {
            SessionKind::Admin => ADMIN_IDLE_SECS,
            SessionKind::Webmail => WEBMAIL_IDLE_SECS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenInfo {
    pub account_id: u64,
    pub username: String,
    pub is_admin: bool,
    pub kind: SessionKind,
}

#[derive(Serialize, Deserialize)]
struct SessionRecord {
    account_id: u64,
    username: String,
    is_admin: bool,
    kind: SessionKind,
    issued_at: u64,
    last_used_at: u64,
}

impl SessionRecord {
    fn expired(&self, now: u64) -> bool {
        now.saturating_sub(self.last_used_at) >= self.kind.idle_limit()
    }

    fn info(&self) -> TokenInfo {
        TokenInfo {
            account_id: self.account_id,
            username: self.username.clone(),
            is_admin: self.is_admin,
            kind: self.kind,
        }
    }
}

pub struct Sessions {
    store: Arc<dyn Store>,
}

impl Sessions {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    pub fn issue(&self, info: TokenInfo) -> Result<String> {
        self.issue_at(info, unix_now())
    }

    pub fn issue_at(&self, info: TokenInfo, now: u64) -> Result<String> {
        let token = random_token();
        let record = SessionRecord {
            account_id: info.account_id,
            username: info.username,
            is_admin: info.is_admin,
            kind: info.kind,
            issued_at: now,
            last_used_at: now,
        };
        self.store.put(&session_key(&token), &encode(&record)?)?;
        Ok(token)
    }

    pub fn validate(&self, token: &str) -> Option<TokenInfo> {
        self.validate_at(token, unix_now())
    }

    pub fn validate_at(&self, token: &str, now: u64) -> Option<TokenInfo> {
        let key = session_key(token);
        let bytes = self.store.get(&key).ok()??;
        let mut record = decode(&bytes).ok()?;
        if record.expired(now) {
            let _ = self.store.delete(&key);
            return None;
        }
        if now.saturating_sub(record.last_used_at) >= RENEW_AFTER_SECS {
            record.last_used_at = now;
            if let Ok(value) = encode(&record) {
                let _ = self.store.put(&key, &value);
            }
        }
        Some(record.info())
    }

    pub fn revoke(&self, token: &str) -> bool {
        let key = session_key(token);
        match self.store.exists(&key) {
            Ok(true) => self.store.delete(&key).is_ok(),
            _ => false,
        }
    }

    pub fn revoke_account(&self, account_id: u64) -> Result<usize> {
        self.remove_where(|record, _| record.account_id == account_id)
    }

    pub fn sweep_expired(&self) -> Result<usize> {
        self.sweep_expired_at(unix_now())
    }

    pub fn sweep_expired_at(&self, now: u64) -> Result<usize> {
        self.remove_where(|record, _| record.expired(now))
    }

    fn remove_where(&self, keep_out: impl Fn(&SessionRecord, &[u8]) -> bool) -> Result<usize> {
        let mut doomed = Vec::new();
        self.store.iterate_from(
            &KeyPrefix::subspace(Subspace::Registry),
            &[Subspace::Registry.as_byte(), TAG_SESSION],
            &mut |key, value| {
                if key.len() < 2 || key[1] != TAG_SESSION {
                    return Ok(if key.get(1).is_some_and(|tag| *tag > TAG_SESSION) {
                        Flow::Stop
                    } else {
                        Flow::Continue
                    });
                }
                match decode(value) {
                    Ok(record) if keep_out(&record, key) => {
                        doomed.push(WriteOp::Delete { key: key.to_vec() })
                    }
                    Ok(_) => {}
                    Err(_) => doomed.push(WriteOp::Delete { key: key.to_vec() }),
                }
                Ok(Flow::Continue)
            },
        )?;
        let removed = doomed.len();
        if removed > 0 {
            self.store.batch(&doomed)?;
        }
        Ok(removed)
    }
}

fn session_key(token: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + blake3::OUT_LEN);
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_SESSION);
    key.extend_from_slice(blake3::hash(token.as_bytes()).as_bytes());
    key
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn encode(record: &SessionRecord) -> Result<Vec<u8>> {
    serde_json::to_vec(record)
        .map_err(|err| Error::serialize(format!("could not encode a session: {err}")))
}

fn decode(bytes: &[u8]) -> Result<SessionRecord> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::serialize(format!("could not decode a session: {err}")))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{state, TempDir};

    const DAY: u64 = 24 * 60 * 60;

    fn info(account_id: u64, kind: SessionKind) -> TokenInfo {
        TokenInfo {
            account_id,
            username: format!("user{account_id}@example.com"),
            is_admin: kind == SessionKind::Admin,
            kind,
        }
    }

    #[test]
    fn a_session_survives_a_new_instance_over_the_same_store() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = shared.tokens.issue(info(7, SessionKind::Webmail)).unwrap();
        let reopened = Sessions::new(Arc::clone(&shared.store));
        assert_eq!(
            reopened.validate(&token),
            Some(info(7, SessionKind::Webmail))
        );
    }

    #[test]
    fn the_raw_token_is_not_stored() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = shared.tokens.issue(info(1, SessionKind::Admin)).unwrap();
        let mut found_raw = false;
        shared
            .store
            .iterate(
                &KeyPrefix::subspace(Subspace::Registry),
                &mut |key, value| {
                    if key.windows(token.len()).any(|w| w == token.as_bytes())
                        || value.windows(token.len()).any(|w| w == token.as_bytes())
                    {
                        found_raw = true;
                    }
                    Ok(Flow::Continue)
                },
            )
            .unwrap();
        assert!(!found_raw);
    }

    #[test]
    fn webmail_sessions_slide_and_expire_after_ninety_idle_days() {
        let dir = TempDir::new();
        let sessions = Sessions::new(Arc::clone(&state(&dir).store));
        let token = sessions.issue_at(info(1, SessionKind::Webmail), 0).unwrap();
        assert!(sessions.validate_at(&token, 89 * DAY).is_some());
        assert!(sessions.validate_at(&token, 89 * DAY + 89 * DAY).is_some());
        assert!(sessions
            .validate_at(&token, 89 * DAY + 89 * DAY + 91 * DAY)
            .is_none());
        assert!(sessions.validate_at(&token, 0).is_none());
    }

    #[test]
    fn admin_sessions_expire_after_twelve_idle_hours() {
        let dir = TempDir::new();
        let sessions = Sessions::new(Arc::clone(&state(&dir).store));
        let token = sessions.issue_at(info(1, SessionKind::Admin), 0).unwrap();
        assert!(sessions.validate_at(&token, 11 * 60 * 60).is_some());
        assert!(sessions
            .validate_at(&token, 11 * 60 * 60 + 13 * 60 * 60)
            .is_none());
    }

    #[test]
    fn renewal_is_throttled_to_once_an_hour() {
        let dir = TempDir::new();
        let sessions = Sessions::new(Arc::clone(&state(&dir).store));
        let token = sessions.issue_at(info(1, SessionKind::Admin), 0).unwrap();
        assert!(sessions.validate_at(&token, 30 * 60).is_some());
        assert!(sessions
            .validate_at(&token, 30 * 60 + 12 * 60 * 60)
            .is_none());
    }

    #[test]
    fn revoking_an_account_leaves_other_accounts_alone() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let alice_a = shared.tokens.issue(info(1, SessionKind::Webmail)).unwrap();
        let alice_b = shared.tokens.issue(info(1, SessionKind::Admin)).unwrap();
        let bob = shared.tokens.issue(info(2, SessionKind::Webmail)).unwrap();
        assert_eq!(shared.tokens.revoke_account(1).unwrap(), 2);
        assert!(shared.tokens.validate(&alice_a).is_none());
        assert!(shared.tokens.validate(&alice_b).is_none());
        assert!(shared.tokens.validate(&bob).is_some());
    }

    #[test]
    fn the_sweep_removes_only_expired_sessions() {
        let dir = TempDir::new();
        let sessions = Sessions::new(Arc::clone(&state(&dir).store));
        let stale = sessions.issue_at(info(1, SessionKind::Admin), 0).unwrap();
        let fresh = sessions.issue_at(info(2, SessionKind::Webmail), 0).unwrap();
        assert_eq!(sessions.sweep_expired_at(DAY).unwrap(), 1);
        assert!(sessions.validate_at(&stale, DAY).is_none());
        assert!(sessions.validate_at(&fresh, DAY).is_some());
    }

    #[test]
    fn revoke_reports_whether_a_session_existed() {
        let dir = TempDir::new();
        let shared = state(&dir);
        let token = shared.tokens.issue(info(1, SessionKind::Webmail)).unwrap();
        assert!(shared.tokens.revoke(&token));
        assert!(!shared.tokens.revoke(&token));
        assert!(shared.tokens.validate(&token).is_none());
    }
}
