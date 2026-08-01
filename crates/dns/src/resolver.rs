use std::sync::Arc;

use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::lookup::Lookup;
use hickory_resolver::lookup_ip::LookupIp;
use hickory_resolver::name_server::TokioConnectionProvider;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::TokioResolver;

use irixmail_core::{Error, Result};

#[derive(Clone)]
pub struct Resolver {
    inner: Backend,
}

#[derive(Clone)]
enum Backend {
    Live(Arc<TokioResolver>),
    Empty,
}

impl Resolver {
    pub fn from_system() -> Result<Self> {
        let resolver = TokioResolver::builder_tokio()
            .map_err(|err| {
                Error::internal(format!("could not initialize the DNS resolver: {err}"))
            })?
            .build();
        Ok(Self {
            inner: Backend::Live(Arc::new(resolver)),
        })
    }

    pub fn from_config(config: ResolverConfig, options: ResolverOpts) -> Self {
        let resolver =
            TokioResolver::builder_with_config(config, TokioConnectionProvider::default())
                .with_options(options)
                .build();
        Self {
            inner: Backend::Live(Arc::new(resolver)),
        }
    }

    pub fn empty() -> Self {
        Self {
            inner: Backend::Empty,
        }
    }

    pub async fn lookup(&self, name: &str, record_type: RecordType) -> Result<Option<Lookup>> {
        let Backend::Live(resolver) = &self.inner else {
            return Ok(None);
        };
        match resolver.lookup(name, record_type).await {
            Ok(lookup) => Ok(Some(lookup)),
            Err(err) if err.is_no_records_found() || err.is_nx_domain() => Ok(None),
            Err(err) => Err(Error::internal(format!(
                "DNS lookup of {record_type} for {name} failed: {err}"
            ))),
        }
    }

    pub async fn lookup_ip(&self, host: &str) -> Result<Option<LookupIp>> {
        let Backend::Live(resolver) = &self.inner else {
            return Ok(None);
        };
        match resolver.lookup_ip(host).await {
            Ok(ips) => Ok(Some(ips)),
            Err(err) if err.is_no_records_found() || err.is_nx_domain() => Ok(None),
            Err(err) => Err(Error::internal(format!(
                "DNS address lookup of {host} failed: {err}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_resolver_is_built_from_an_explicit_config_and_clones_cheaply() {
        let resolver = Resolver::from_config(ResolverConfig::default(), ResolverOpts::default());
        let clone = resolver.clone();
        let _ = (resolver, clone);
    }
}
