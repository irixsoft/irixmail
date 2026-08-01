use irixmail_core::{Result, Server};

use crate::resolver::Resolver;

#[derive(Clone)]
pub struct DnsServices {
    resolver: Resolver,
}

impl DnsServices {
    pub fn new(resolver: Resolver) -> Self {
        Self { resolver }
    }

    pub fn from_server(_server: &Server) -> Result<Self> {
        Ok(Self::new(Resolver::from_system()?))
    }

    pub fn resolver(&self) -> &Resolver {
        &self.resolver
    }
}
