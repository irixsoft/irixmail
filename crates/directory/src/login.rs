use crate::account::Account;
use crate::authenticate::{authenticate, AuthenticatedBy, Authentication, LoginPurpose};
use crate::password;
use crate::server::Directory;
use irixmail_core::{Error, Result};

#[derive(Debug, Clone)]
pub enum LoginAttempt {
    Granted(Box<Account>, AuthenticatedBy),
    Denied,
    Throttled,
}

pub async fn attempt_login_blocking(
    directory: &Directory,
    ip: Option<&str>,
    username: &str,
    secret: &str,
    purpose: LoginPurpose,
) -> Result<LoginAttempt> {
    let directory = directory.clone();
    let ip = ip.map(str::to_string);
    let username = username.to_string();
    let secret = secret.to_string();
    tokio::task::spawn_blocking(move || {
        attempt_login(&directory, ip.as_deref(), &username, &secret, purpose)
    })
    .await
    .map_err(|err| Error::Internal(format!("the login task failed: {err}")))?
}

pub fn attempt_login(
    directory: &Directory,
    ip: Option<&str>,
    username: &str,
    secret: &str,
    purpose: LoginPurpose,
) -> Result<LoginAttempt> {
    let throttle = directory.throttle();
    if throttle.is_locked(ip, None) {
        return Ok(LoginAttempt::Throttled);
    }
    let Some(account) = resolve(directory, username)? else {
        password::verify_dummy(secret);
        throttle.record_failure(ip, None);
        return Ok(LoginAttempt::Denied);
    };
    if throttle.is_locked(ip, Some(account.id)) {
        return Ok(LoginAttempt::Throttled);
    }
    let stored = directory.credentials().list(account.id)?;
    match authenticate(&account, &stored, purpose, secret)? {
        Authentication::Granted(by) => {
            throttle.record_success(ip, Some(account.id));
            Ok(LoginAttempt::Granted(Box::new(account), by))
        }
        Authentication::Denied => {
            throttle.record_failure(ip, Some(account.id));
            Ok(LoginAttempt::Denied)
        }
    }
}

fn resolve(directory: &Directory, username: &str) -> Result<Option<Account>> {
    let Some((local_part, domain_name)) = username.rsplit_once('@') else {
        return Ok(None);
    };
    let Some(domain) = directory.domains().get_by_name(domain_name)? else {
        return Ok(None);
    };
    directory.accounts().get_by_address(local_part, domain.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use irixmail_core::IdGenerator;
    use irixmail_store::{Flow, KeyPrefix, Store, WriteOp};

    use crate::account::Role;
    use crate::throttle::DEFAULT_MAX_FAILURES;

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
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            let bound = prefix.encode();
            let map = self.map.lock().unwrap();
            for (key, value) in map.iter() {
                if key.starts_with(&bound) && visit(key, value)? == Flow::Stop {
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
                    WriteOp::Add { .. } => unreachable!("logins do not use counters"),
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("logins do not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("logins do not use counters")
        }
    }

    fn directory_with_account(secret: &str) -> Directory {
        let directory = Directory::new(
            Arc::new(MemStore::default()),
            Arc::new(IdGenerator::new(0)),
            None,
        );
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
            .set_primary_password(account.id, password::hash(secret).unwrap())
            .unwrap();
        directory
    }

    fn attempt(directory: &Directory, ip: Option<&str>, secret: &str) -> LoginAttempt {
        attempt_login(
            directory,
            ip,
            "alice@example.com",
            secret,
            LoginPurpose::Interactive,
        )
        .unwrap()
    }

    #[test]
    fn a_correct_secret_is_granted() {
        let directory = directory_with_account("secret");
        assert!(matches!(
            attempt(&directory, None, "secret"),
            LoginAttempt::Granted(_, _)
        ));
    }

    #[test]
    fn a_wrong_secret_is_denied() {
        let directory = directory_with_account("secret");
        assert!(matches!(
            attempt(&directory, None, "wrong"),
            LoginAttempt::Denied
        ));
    }

    #[test]
    fn an_unknown_account_is_denied_without_an_error() {
        let directory = directory_with_account("secret");
        let outcome = attempt_login(
            &directory,
            None,
            "ghost@example.com",
            "anything",
            LoginPurpose::Interactive,
        )
        .unwrap();
        assert!(matches!(outcome, LoginAttempt::Denied));
    }

    #[test]
    fn a_username_without_a_domain_is_denied() {
        let directory = directory_with_account("secret");
        let outcome = attempt_login(
            &directory,
            None,
            "alice",
            "secret",
            LoginPurpose::Interactive,
        )
        .unwrap();
        assert!(matches!(outcome, LoginAttempt::Denied));
    }

    #[test]
    fn repeated_failures_lock_the_account_even_for_the_right_secret() {
        let directory = directory_with_account("secret");
        for _ in 0..DEFAULT_MAX_FAILURES {
            assert!(matches!(
                attempt(&directory, None, "wrong"),
                LoginAttempt::Denied
            ));
        }
        assert!(matches!(
            attempt(&directory, None, "secret"),
            LoginAttempt::Throttled
        ));
    }

    #[test]
    fn repeated_unknown_account_failures_lock_the_source_ip() {
        let directory = directory_with_account("secret");
        for _ in 0..DEFAULT_MAX_FAILURES {
            let outcome = attempt_login(
                &directory,
                Some("203.0.113.9"),
                "ghost@example.com",
                "anything",
                LoginPurpose::Interactive,
            )
            .unwrap();
            assert!(matches!(outcome, LoginAttempt::Denied));
        }
        assert!(matches!(
            attempt(&directory, Some("203.0.113.9"), "secret"),
            LoginAttempt::Throttled
        ));
        assert!(matches!(
            attempt(&directory, Some("198.51.100.1"), "secret"),
            LoginAttempt::Granted(_, _)
        ));
    }

    #[test]
    fn a_success_clears_the_failure_tally() {
        let directory = directory_with_account("secret");
        for _ in 0..DEFAULT_MAX_FAILURES - 1 {
            attempt(&directory, None, "wrong");
        }
        assert!(matches!(
            attempt(&directory, None, "secret"),
            LoginAttempt::Granted(_, _)
        ));
        assert!(matches!(
            attempt(&directory, None, "wrong"),
            LoginAttempt::Denied
        ));
        assert!(matches!(
            attempt(&directory, None, "secret"),
            LoginAttempt::Granted(_, _)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verification_runs_off_the_async_runtime() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::Duration;

        let directory = directory_with_account("secret");
        let domain = directory
            .domains()
            .get_by_name("example.com")
            .unwrap()
            .unwrap();
        let account = directory
            .accounts()
            .get_by_address("alice", domain.id)
            .unwrap()
            .unwrap();
        for index in 0..12 {
            let minted =
                crate::app_password::generate(index, &format!("client-{index}"), 0).unwrap();
            directory
                .credentials()
                .add_app_password(account.id, minted.record)
                .unwrap();
        }

        let beats = Arc::new(AtomicU32::new(0));
        let heart = tokio::spawn({
            let beats = Arc::clone(&beats);
            async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    beats.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        let outcome = attempt_login_blocking(
            &directory,
            None,
            "alice@example.com",
            "wrong",
            LoginPurpose::Mail,
        )
        .await
        .unwrap();
        heart.abort();
        assert!(matches!(outcome, LoginAttempt::Denied));
        assert!(
            beats.load(Ordering::SeqCst) >= 3,
            "the reactor made no progress while a password was being verified"
        );
    }

    #[test]
    fn an_app_password_is_granted_only_for_mail() {
        use crate::app_password;

        let directory = directory_with_account("secret");
        let domain = directory
            .domains()
            .get_by_name("example.com")
            .unwrap()
            .unwrap();
        let account = directory
            .accounts()
            .get_by_address("alice", domain.id)
            .unwrap()
            .unwrap();
        let minted = app_password::generate(1, "client", 0).unwrap();
        let plaintext = minted.plaintext.clone();
        directory
            .credentials()
            .add_app_password(account.id, minted.record)
            .unwrap();

        let mail = attempt_login(
            &directory,
            None,
            "alice@example.com",
            &plaintext,
            LoginPurpose::Mail,
        )
        .unwrap();
        assert!(matches!(
            mail,
            LoginAttempt::Granted(_, AuthenticatedBy::AppPassword { .. })
        ));

        let interactive = attempt_login(
            &directory,
            None,
            "alice@example.com",
            &plaintext,
            LoginPurpose::Interactive,
        )
        .unwrap();
        assert!(matches!(interactive, LoginAttempt::Denied));
    }
}
