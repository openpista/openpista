//! QUIC server bootstrap and in-process gateway helpers.

use std::net::SocketAddr;
use std::sync::Arc;

use proto::{ChannelEvent, GatewayError};
use quinn::{Endpoint, ServerConfig};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::session::AgentSession;

/// Async callback that processes inbound [`ChannelEvent`] and returns optional text.
pub type AgentHandler = Arc<
    dyn Fn(
            ChannelEvent,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>>
        + Send
        + Sync,
>;

/// QUIC server that accepts connections and spawns agent sessions
pub struct QuicServer {
    endpoint: Endpoint,
    handler: AgentHandler,
}

impl QuicServer {
    /// Create a new QUIC server with auto-generated self-signed certificate
    pub fn new_self_signed(addr: SocketAddr, handler: AgentHandler) -> Result<Self, GatewayError> {
        let (cert, key) = generate_self_signed_cert()?;
        let server_config = make_server_config(cert, key)?;
        let endpoint = Endpoint::server(server_config, addr)
            .map_err(|e| GatewayError::Endpoint(e.to_string()))?;

        info!("QUIC server listening on {addr}");
        Ok(Self { endpoint, handler })
    }

    /// Create a QUIC server with provided PEM cert and key
    pub fn new_with_certs(
        addr: SocketAddr,
        cert_pem: &[u8],
        key_pem: &[u8],
        handler: AgentHandler,
    ) -> Result<Self, GatewayError> {
        let cert: Vec<rustls::pki_types::CertificateDer<'static>> =
            rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e: std::io::Error| GatewayError::Tls(e.to_string()))?;
        let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem))
            .map_err(|e: std::io::Error| GatewayError::Tls(e.to_string()))?
            .ok_or_else(|| GatewayError::Tls("No private key found".into()))?;

        let server_config = make_server_config(cert, key)?;
        let endpoint = Endpoint::server(server_config, addr)
            .map_err(|e| GatewayError::Endpoint(e.to_string()))?;

        info!("QUIC server listening on {addr} (custom cert)");
        Ok(Self { endpoint, handler })
    }

    /// Accept loop: accept incoming connections and spawn sessions
    pub async fn run(self) {
        info!("QUIC server accept loop started");
        loop {
            match self.endpoint.accept().await {
                Some(incoming) => {
                    let handler = self.handler.clone();
                    tokio::spawn(async move {
                        match incoming.await {
                            Ok(conn) => {
                                let remote = conn.remote_address();
                                info!("New QUIC connection from {remote}");
                                let session = AgentSession::new(conn, handler);
                                if let Err(e) = session.run().await {
                                    warn!("Session error from {remote}: {e}");
                                }
                            }
                            Err(e) => {
                                error!("Failed to accept connection: {e}");
                            }
                        }
                    });
                }
                None => {
                    info!("QUIC endpoint closed");
                    break;
                }
            }
        }
    }

    /// Local address the server is bound to
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }
}

/// Generates a localhost self-signed certificate and private key pair.
fn generate_self_signed_cert() -> Result<
    (
        Vec<rustls::pki_types::CertificateDer<'static>>,
        rustls::pki_types::PrivateKeyDer<'static>,
    ),
    GatewayError,
> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .map_err(|e| GatewayError::Tls(e.to_string()))?;

    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.key_pair.serialize_der())
        .map_err(|e| GatewayError::Tls(e.to_string()))?;

    Ok((vec![cert_der], key_der))
}

/// Builds a QUIC server config from DER certificates and private key.
fn make_server_config(
    certs: Vec<rustls::pki_types::CertificateDer<'static>>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
) -> Result<ServerConfig, GatewayError> {
    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| GatewayError::Tls(e.to_string()))?;

    // Enable 0-RTT
    tls_config.max_early_data_size = u32::MAX;

    let server_config = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
            .map_err(|e| GatewayError::Tls(e.to_string()))?,
    ));

    Ok(server_config)
}

/// Simple in-process "gateway" using tokio channels (for CLI/testing without QUIC)
pub struct InProcessGateway {
    tx: mpsc::Sender<ChannelEvent>,
    rx: mpsc::Receiver<ChannelEvent>,
}

impl InProcessGateway {
    /// Creates a bounded in-process gateway.
    pub fn new(buffer: usize) -> Self {
        let (tx, rx) = mpsc::channel(buffer);
        Self { tx, rx }
    }

    /// Returns a cloneable sender used to enqueue inbound events.
    pub fn sender(&self) -> mpsc::Sender<ChannelEvent> {
        self.tx.clone()
    }

    /// Receives the next inbound event from the queue.
    pub async fn recv(&mut self) -> Option<ChannelEvent> {
        self.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ensure_crypto_provider() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    fn noop_handler() -> AgentHandler {
        Arc::new(|_event| Box::pin(async move { Some("ok".to_string()) }))
    }

    #[test]
    fn generate_cert_and_server_config_work() {
        ensure_crypto_provider();
        let (certs, key) =
            generate_self_signed_cert().expect("self-signed cert should be generated");
        assert_eq!(certs.len(), 1);
        let config = make_server_config(certs, key).expect("server config should be created");
        let _ = config; // compile-time assertion that config is usable
    }

    #[test]
    fn new_with_certs_rejects_invalid_pem() {
        ensure_crypto_provider();
        let addr: SocketAddr = "127.0.0.1:0".parse().expect("socket addr");
        let result =
            QuicServer::new_with_certs(addr, b"invalid cert", b"invalid key", noop_handler());
        assert!(result.is_err(), "invalid pem should fail");
        let err = result.err().expect("error is expected");
        assert!(err.to_string().contains("TLS error"));
    }

    #[tokio::test]
    async fn new_with_certs_accepts_generated_pem() {
        ensure_crypto_provider();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("certificate generation");
        let cert_pem = cert.cert.pem();
        let key_pem = cert.key_pair.serialize_pem();

        let addr: SocketAddr = "127.0.0.1:0".parse().expect("socket addr");
        let server = QuicServer::new_with_certs(
            addr,
            cert_pem.as_bytes(),
            key_pem.as_bytes(),
            noop_handler(),
        )
        .expect("valid generated cert should work");
        let local = server.local_addr().expect("local addr");
        assert!(local.port() > 0);
    }

    #[tokio::test]
    async fn in_process_gateway_forwards_events() {
        let mut gateway = InProcessGateway::new(4);
        let sender = gateway.sender();
        let event = ChannelEvent::new(
            proto::ChannelId::from("cli:local"),
            proto::SessionId::from("s1"),
            "hello",
        );
        sender.send(event.clone()).await.expect("send should work");

        let received = gateway.recv().await.expect("event should be received");
        assert_eq!(received.channel_id, event.channel_id);
        assert_eq!(received.session_id, event.session_id);
        assert_eq!(received.user_message, "hello");
    }
}
