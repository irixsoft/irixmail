use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

use irixmail_core::{Error, Result};

use crate::cert_store::CertMaterial;
use crate::issue::{issue, IssueRequest};

pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(60),
        }
    }
}

pub async fn issue_with_retry(
    request: &IssueRequest<'_>,
    policy: &RetryPolicy,
) -> Result<CertMaterial> {
    retry(policy, || issue(request)).await
}

async fn retry<F, Fut>(policy: &RetryPolicy, mut attempt: F) -> Result<CertMaterial>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<CertMaterial>>,
{
    let mut backoff = policy.initial_backoff;
    let mut last = None;
    for n in 1..=policy.max_attempts {
        match attempt().await {
            Ok(material) => return Ok(material),
            Err(err) => {
                last = Some(err);
                if n < policy.max_attempts {
                    sleep(backoff).await;
                    backoff = (backoff * 2).min(policy.max_backoff);
                }
            }
        }
    }
    Err(actionable_error(last))
}

fn actionable_error(last: Option<Error>) -> Error {
    let detail = last.map(|err| err.to_string()).unwrap_or_default();
    Error::internal(format!(
        "certificate issuance failed after retries: {detail}. Re-check that the DNS A/AAAA record \
         points to this server and that port 80 is reachable, wait for DNS to propagate, then \
         re-run `irixmail setup`."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::self_signed;

    fn fast_policy(max_attempts: usize) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
        }
    }

    fn material() -> CertMaterial {
        self_signed::generate(vec!["localhost".to_string()]).unwrap()
    }

    #[tokio::test]
    async fn it_succeeds_once_an_attempt_passes() {
        let mut attempts = 0;
        let result = retry(&fast_policy(5), || {
            attempts += 1;
            let n = attempts;
            async move {
                if n < 3 {
                    Err(Error::internal("not yet"))
                } else {
                    Ok(material())
                }
            }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(attempts, 3);
    }

    #[tokio::test]
    async fn exhausting_attempts_yields_actionable_guidance() {
        let mut attempts = 0;
        let result = retry(&fast_policy(2), || {
            attempts += 1;
            async { Err(Error::internal("dns not propagated")) }
        })
        .await;
        assert_eq!(attempts, 2);
        let message = match result {
            Ok(_) => panic!("expected issuance to fail"),
            Err(err) => err.to_string(),
        };
        assert!(message.contains("re-run `irixmail setup`"));
        assert!(message.contains("dns not propagated"));
    }
}
