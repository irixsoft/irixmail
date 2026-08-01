use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub const TOKEN_TTL_SECS: u64 = 900;

struct Challenge {
    key_authorization: String,
    expires_at: u64,
}

#[derive(Clone, Default)]
pub struct Http01Challenges {
    tokens: Arc<RwLock<HashMap<String, Challenge>>>,
}

impl Http01Challenges {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &self,
        token: impl Into<String>,
        key_authorization: impl Into<String>,
        now_unix: u64,
    ) {
        let mut tokens = self.tokens.write().unwrap();
        tokens.retain(|_, challenge| challenge.expires_at > now_unix);
        tokens.insert(
            token.into(),
            Challenge {
                key_authorization: key_authorization.into(),
                expires_at: now_unix + TOKEN_TTL_SECS,
            },
        );
    }

    pub fn get(&self, token: &str, now_unix: u64) -> Option<String> {
        let tokens = self.tokens.read().unwrap();
        tokens
            .get(token)
            .filter(|challenge| challenge.expires_at > now_unix)
            .map(|challenge| challenge.key_authorization.clone())
    }

    pub fn remove(&self, token: &str) {
        self.tokens.write().unwrap().remove(token);
    }
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_served_until_removed() {
        let challenges = Http01Challenges::new();
        challenges.insert("tok", "tok.keyauth", 100);
        assert_eq!(challenges.get("tok", 100).as_deref(), Some("tok.keyauth"));
        assert_eq!(challenges.get("other", 100), None);

        challenges.remove("tok");
        assert_eq!(challenges.get("tok", 100), None);
    }

    #[test]
    fn an_expired_token_is_not_served() {
        let challenges = Http01Challenges::new();
        challenges.insert("tok", "tok.keyauth", 100);
        assert!(challenges.get("tok", 100 + TOKEN_TTL_SECS - 1).is_some());
        assert_eq!(challenges.get("tok", 100 + TOKEN_TTL_SECS), None);
    }

    #[test]
    fn expired_tokens_are_swept_on_later_inserts() {
        let challenges = Http01Challenges::new();
        challenges.insert("stale", "stale.keyauth", 100);
        challenges.insert("fresh", "fresh.keyauth", 100 + TOKEN_TTL_SECS + 1);

        assert!(challenges.tokens.read().unwrap().get("stale").is_none());
        assert!(challenges.get("fresh", 100 + TOKEN_TTL_SECS + 2).is_some());
    }
}
