use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use irixmail_core::{Error, Result};

pub async fn upgrade<S>(acceptor: &TlsAcceptor, stream: S) -> Result<TlsStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    acceptor
        .accept(stream)
        .await
        .map_err(|err| Error::protocol(format!("STARTTLS handshake failed: {err}")))
}
